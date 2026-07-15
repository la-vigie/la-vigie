//! Shared launch-task core (TASK-88). One place that turns a launch request into
//! a worktree-backed task, so every entry point (the `create_task` Tauri command
//! today; a future MCP server (TASK-89) and REST endpoint (TASK-70)) shares exactly
//! one launch path.
//!
//! Two layers:
//!   * pure (`resolve_launch`, `LaunchDecision`): arg normalization + the
//!     immediate-vs-pending decision — unit-tested here.
//!   * async glue (`launch_task`): git/store side effects — verified via
//!     the app, not unit-tested (Task 2). Setup is NOT run here: it's a
//!     non-blocking background job the caller kicks off after launch (TASK-96).
//!
//! Shape A: this core never spawns the agent/PTY. It returns the created task and
//! the decision; the caller starts the agent (the frontend via `start_agent`).

use std::path::{Path, PathBuf};

use crate::commands::{
    effective_fetch_remote_base, has_task_identity, is_valid_base_branch, parse_bool_setting,
    task_branch, worktree_base_ref, worktree_path_for,
};
use crate::git;
use crate::state::AppState;
use crate::store::{Repo, Task, TaskStatus};

/// Raw launch request before resolution. Mirrors the design's
/// `{ repo, title, ticketKey?, agent?, afterMergeOf? }` plus a base-branch override.
pub struct LaunchArgs {
    pub repo_id: String,
    pub title: String,
    pub base_branch: Option<String>,
    pub ticket_key: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub after_merge_of: Vec<String>,
    /// Seed prompt for a queued task; stored as `pending_prompt` and emitted at
    /// promote-time. Ignored for immediate launches (the caller emits the
    /// prompt via the task_launched event). TASK-90.
    pub prompt: Option<String>,
    pub auto_approve: Option<bool>,
    /// TASK-163: run in the repo's existing checkout (no worktree).
    pub in_place: bool,
    /// TASK-163: optional new branch to `checkout -b` in the checkout. `None`/empty
    /// ⇒ adopt the checkout's current branch. Ignored when `in_place` is false.
    pub branch_name: Option<String>,
}

/// Whether a launch happens now or is queued behind another task's merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchDecision {
    Immediate,
    Pending { after_merge_of: Vec<String> },
}

impl LaunchDecision {
    /// An empty or all-whitespace list ⇒ `Immediate`; otherwise `Pending`
    /// carrying the trimmed, de-duplicated (first-seen order) dependency ids.
    pub fn from(after_merge_of: &[String]) -> LaunchDecision {
        let mut ids: Vec<String> = Vec::new();
        for raw in after_merge_of {
            let id = raw.trim();
            if id.is_empty() || ids.iter().any(|existing| existing == id) {
                continue;
            }
            ids.push(id.to_string());
        }
        if ids.is_empty() {
            LaunchDecision::Immediate
        } else {
            LaunchDecision::Pending { after_merge_of: ids }
        }
    }
}

/// A fully-resolved launch ready for side effects: normalized fields, computed
/// branch/base/worktree path, and the immediate-vs-pending decision. No I/O.
#[derive(Debug)]
pub struct ResolvedLaunch {
    pub repo_id: String,
    pub title: String,
    pub branch: String,
    pub base_branch: String,
    pub worktree_path: PathBuf,
    pub ticket_key: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub auto_approve: Option<bool>,
    pub decision: LaunchDecision,
    /// Normalized seed prompt for a queued (pending) launch. Not read by
    /// `launch_task`'s `Immediate` path; consumed by the pending-insert path
    /// (TASK-90).
    pub pending_prompt: Option<String>,
    /// TASK-163: whether this launch targets the repo's checkout in place.
    pub in_place: bool,
    /// TASK-163: the trimmed new-branch name, or `None` to adopt the current
    /// branch.
    pub branch_name: Option<String>,
}

/// Pure resolution: trim/normalize `ticket_key` and `agent` (empty ⇒ `None`),
/// enforce task identity, compute the branch (ticket-key-or-title), default the
/// base branch to the repo default, reject a flag-like base, compose the worktree
/// path, and compute the launch decision. Mirrors today's `create_task` ordering
/// so the refactor is behavior-identical.
pub fn resolve_launch(
    args: LaunchArgs,
    repo: &Repo,
    worktrees_root: &Path,
) -> Result<ResolvedLaunch, String> {
    let ticket_key = args
        .ticket_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());
    let agent = args
        .agent
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty());
    let model = args
        .model
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());
    let pending_prompt = args
        .prompt
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());

    if !has_task_identity(&args.title, ticket_key.as_deref()) {
        return Err("a task needs a title or a ticket ID".to_string());
    }

    let base_branch = args
        .base_branch
        .unwrap_or_else(|| repo.default_branch.clone());
    if !is_valid_base_branch(&base_branch) {
        return Err(format!("invalid base branch: {base_branch}"));
    }

    let branch_name = args
        .branch_name
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty());

    // TASK-163: an in-place task lives in the repo's own checkout — no worktree
    // path is derived, and its branch is either the requested new branch
    // (created at launch) or empty, signalling launch_task to adopt the
    // checkout's current branch (that read needs git I/O, so it can't happen
    // in this pure function).
    let (branch, worktree_path) = if args.in_place {
        (branch_name.clone().unwrap_or_default(), std::path::PathBuf::from(&repo.path))
    } else {
        let branch = task_branch(&args.title, ticket_key.as_deref());
        let wt = worktree_path_for(worktrees_root, repo.worktree_root.as_deref(), &repo.id, &branch);
        (branch, wt)
    };
    let decision = LaunchDecision::from(&args.after_merge_of);

    Ok(ResolvedLaunch {
        repo_id: repo.id.clone(),
        title: args.title,
        branch,
        base_branch,
        worktree_path,
        ticket_key,
        agent,
        model,
        auto_approve: args.auto_approve,
        decision,
        pending_prompt,
        in_place: args.in_place,
        branch_name,
    })
}

/// The subset of `blockers` that currently exists (per `exists`), order
/// preserved. An empty result means every requested blocker is dangling
/// (missing / already-merged-and-deleted) — nothing would ever promote the
/// task, so the caller launches immediately instead of queuing (TASK-177).
pub fn live_blockers<'a>(blockers: &'a [String], exists: impl Fn(&str) -> bool) -> Vec<&'a str> {
    blockers
        .iter()
        .filter(|id| exists(id))
        .map(|id| id.as_str())
        .collect()
}

/// True when `tasks` already contains a live (non-Done) in-place task. A repo's
/// single checkout can host only one in-place agent at a time (TASK-163), so
/// `launch_task` rejects a second one.
pub fn has_active_in_place(tasks: &[Task]) -> bool {
    tasks.iter().any(|t| t.in_place && t.status != TaskStatus::Done)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// The result of a launch: the created task and the decision that produced it.
pub struct LaunchOutcome {
    pub task: Task,
    /// The decision that produced this launch. Not read by `create_task` today
    /// but will be consumed by the MCP (TASK-89) and REST (TASK-70) callers.
    #[allow(dead_code)]
    pub decision: LaunchDecision,
}

/// Create `worktree_path` for `branch`, based on the up-to-date remote base when
/// the effective fetch-remote-base setting is on and the repo has a remote
/// (falling back to the local base so creation never blocks on the network).
/// Shared by the immediate launch and the TASK-90 start-on-merge promote path.
pub(crate) async fn prepare_worktree(
    state: &AppState,
    repo: &Repo,
    base_branch: &str,
    branch: &str,
    worktree_path: &Path,
) -> Result<(), String> {
    let app_fetch_remote_base = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .get_app_setting("fetch_remote_base")
            .map_err(|e| format!("{e:#}"))?
            .as_deref()
            .and_then(parse_bool_setting)
    };
    let repo_path = Path::new(&repo.path);
    let use_remote = effective_fetch_remote_base(repo.fetch_remote_base, app_fetch_remote_base);
    let fetch_ok = if use_remote && repo.remote_url.is_some() {
        git::fetch(repo_path, "origin", base_branch).await.is_ok()
    } else {
        false
    };
    let start_ref = worktree_base_ref(use_remote, fetch_ok, "origin", base_branch);
    git::create_worktree(repo_path, worktree_path, branch, &start_ref)
        .await
        .map_err(|e| format!("{e:#}"))?;
    Ok(())
}

/// The single launch path. Looks up the repo, resolves args, and — for an
/// `Immediate` decision (or a `Pending` decision whose dependency is already
/// gone) — creates the worktree and inserts the task row (rolling back the
/// worktree on insert failure). For a `Pending` decision with a live
/// dependency, queues a dependency-less-yet task row (no worktree/branch) plus
/// a `task_dependencies` edge instead, and returns early. Returns the created
/// task plus its decision.
///
/// Setup is intentionally NOT run here: TASK-96 made it a non-blocking background
/// job the caller kicks off after the task exists (the task is inserted first so
/// it's immediately navigable). This keeps the core UI-agnostic.
///
/// Shape A: this does NOT spawn the agent — the caller starts it (the frontend
/// via `start_agent`).
///
/// Locking: the store Mutex is captured-then-dropped before each `.await` (and
/// before/after the synchronous pending insert+edge, which needs no `.await`
/// between them).
pub async fn launch_task(
    state: &AppState,
    args: LaunchArgs,
) -> Result<LaunchOutcome, String> {
    // 1. Look up the repo (lock → capture → drop before any await).
    let repo = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .get_repo(&args.repo_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("repo not found: {}", args.repo_id))?
    };

    // 2. Pure resolution (normalization, identity, branch/base/path, decision).
    let resolved = resolve_launch(args, &repo, &state.worktrees_root)?;

    // TASK-163: in-place task — run in the repo's own checkout, no worktree.
    // Immediate-only for v1 (ignores after_merge_of queuing).
    if resolved.in_place {
        // Guard: a single checkout can host only one live in-place task.
        {
            let store = state.store.lock().map_err(|e| e.to_string())?;
            let existing = store
                .list_tasks_for_repo(&repo.id)
                .map_err(|e| e.to_string())?;
            if has_active_in_place(&existing) {
                return Err(
                    "this repo already has an in-place task — finish it before starting another \
                     (a single checkout can't host two agents)".to_string(),
                );
            }
        }

        // Resolve the branch: create the requested new branch in the checkout, or
        // adopt whatever branch is currently checked out. Errors surface git stderr.
        let repo_path = Path::new(&repo.path);
        let branch = if let Some(name) = &resolved.branch_name {
            git::create_branch_in_place(repo_path, name)
                .await
                .map_err(|e| format!("{e:#}"))?;
            name.clone()
        } else {
            git::current_branch(repo_path)
                .await
                .map_err(|e| format!("{e:#}"))?
        };

        let now = now_secs();
        let task = Task {
            id: uuid::Uuid::new_v4().to_string(),
            repo_id: resolved.repo_id,
            title: resolved.title,
            worktree_path: repo.path.clone(),
            branch,
            base_branch: resolved.base_branch,
            status: TaskStatus::Idle,
            created_at: now,
            updated_at: now,
            pr_number: None,
            pr_url: None,
            ticket_key: resolved.ticket_key,
            agent: resolved.agent,
            model: resolved.model,
            setup_status: None,
            hidden: false,
            pending_prompt: None,
            auto_approve: resolved.auto_approve,
            in_place: true,
        };
        let insert_result = {
            let store = state.store.lock().map_err(|e| e.to_string())?;
            store.insert_task(&task)
        };
        insert_result.map_err(|e| e.to_string())?;
        return Ok(LaunchOutcome { task, decision: resolved.decision });
    }

    // 3. Pending launch (TASK-90/TASK-177): queue the task behind one or more
    //    other tasks' merges. Only the LIVE blockers get an edge; if every
    //    requested blocker is dangling, nothing would ever promote us, so we
    //    fall through to the immediate path below.
    if let LaunchDecision::Pending { after_merge_of } = &resolved.decision {
        let now = now_secs();
        let task = Task {
            id: uuid::Uuid::new_v4().to_string(),
            repo_id: resolved.repo_id.clone(),
            title: resolved.title.clone(),
            worktree_path: String::new(), // no worktree until promoted
            branch: String::new(),        // derived at promote off updated base
            base_branch: resolved.base_branch.clone(),
            status: TaskStatus::Pending,
            created_at: now,
            updated_at: now,
            pr_number: None,
            pr_url: None,
            ticket_key: resolved.ticket_key.clone(),
            agent: resolved.agent.clone(),
            model: resolved.model.clone(),
            setup_status: None,
            hidden: false,
            pending_prompt: resolved.pending_prompt.clone(),
            auto_approve: resolved.auto_approve,
            in_place: false,
        };
        // Defensive: a fresh uuid can never equal an existing id, so a
        // self-cycle is unreachable — assert it anyway.
        debug_assert!(!after_merge_of.iter().any(|dep| dep == &task.id));
        // Live-filter + insert + edges under a SINGLE lock so the queue op is
        // self-atomic: a Pending row and all its dependency edges appear
        // together or not at all (the rollback below deletes the row on any
        // edge failure), and we only queue behind blockers that still exist at
        // lock time. (No `.await` inside the guard.) TASK-182: the queue-vs-finish
        // window is closed — both finish surfaces delete the blocker's row BEFORE
        // promoting, and `promote_dependents_of` captures its promotion set
        // atomically with removing the edges (`take_dependents_on`). So an edge we
        // insert here is either (a) queued before the row is deleted and thus seen
        // and promoted, or (b) rejected because the blocker no longer exists (we
        // fall through to the immediate path) — never orphaned.
        let queued = {
            let store = state.store.lock().map_err(|e| e.to_string())?;
            let exists = |dep: &str| matches!(store.get_task(dep), Ok(Some(_)));
            let live = live_blockers(after_merge_of, exists);
            if live.is_empty() {
                false // all dangling → fall through to the immediate path below
            } else {
                store.insert_task(&task).map_err(|e| e.to_string())?;
                for dep in &live {
                    if let Err(e) = store.add_task_dependency(&task.id, dep) {
                        // Roll back the just-inserted row so a Pending task never
                        // exists without its dependency edges (all-or-nothing).
                        let _ = store.delete_task(&task.id);
                        return Err(e.to_string());
                    }
                }
                true
            }
        };
        if queued {
            return Ok(LaunchOutcome { task, decision: resolved.decision });
        }
        // else: every blocker dangling → fall through to the immediate path below.
    }

    let repo_path = Path::new(&repo.path);

    // 4. Adopt-or-create (TASK-125). If the target worktree path is already a
    //    registered worktree on the intended branch, reuse it instead of failing
    //    on `git worktree add`. If it's occupied by something that doesn't match
    //    (bare dir, different branch, already owned by another task), return a
    //    clear error rather than a cryptic git one — the caller/UI warns first.
    let already_task_owned = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .list_tasks()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|t| Path::new(&t.worktree_path) == resolved.worktree_path)
    };
    let adoption =
        git::worktree_state(repo_path, &resolved.worktree_path, &resolved.branch, already_task_owned)
            .await;

    // A leftover/orphaned (or stale-registered) worktree must be cleaned up before
    // recreating; captured before the match consumes `adoption`.
    let needs_reclaim = matches!(adoption, git::WorktreeAdoption::Reclaim { .. });

    // `created_here` gates rollback: never remove a worktree we merely adopted.
    let created_here = match adoption {
        // Existing worktree on the intended branch: adopt it, skip `worktree add`.
        git::WorktreeAdoption::Adopt => false,
        git::WorktreeAdoption::Conflict { reason } => return Err(reason),
        // Vacant → create fresh. Reclaim → clean up the leftover, then create fresh
        // (create_worktree reuses the existing branch, preserving its commits).
        git::WorktreeAdoption::Vacant | git::WorktreeAdoption::Reclaim { .. } => {
            if needs_reclaim {
                // Removes the orphaned directory + prunes any stale admin entry
                // (TASK-118 teardown handles the deregistered case). `{:#}` keeps
                // git's stderr on the error path.
                git::remove_worktree(repo_path, &resolved.worktree_path, true)
                    .await
                    .map_err(|e| format!("{e:#}"))?;
            }

            // Create the worktree via the shared helper (fetch-remote-base decision
            // + create_worktree), also used by the TASK-90 promote path. The stored
            // `base_branch` stays the logical local base; only the start ref changes.
            prepare_worktree(
                state,
                &repo,
                &resolved.base_branch,
                &resolved.branch,
                &resolved.worktree_path,
            )
            .await?;
            true
        }
    };

    // 5. Build and insert the task row. (Setup runs as a background job the
    //    caller starts after this returns — see TASK-96.) This runs for both a
    //    genuine `Immediate` decision and a dangling `Pending` fall-through.
    let now = now_secs();
    let worktree_path = resolved
        .worktree_path
        .to_str()
        .ok_or("worktree path is not valid UTF-8")?
        .to_string();
    let task = Task {
        id: uuid::Uuid::new_v4().to_string(),
        repo_id: resolved.repo_id,
        title: resolved.title,
        worktree_path,
        branch: resolved.branch,
        base_branch: resolved.base_branch,
        status: TaskStatus::Idle,
        created_at: now,
        updated_at: now,
        pr_number: None,
        pr_url: None,
        ticket_key: resolved.ticket_key,
        agent: resolved.agent,
        model: resolved.model,
        setup_status: None,
        hidden: false,
        pending_prompt: None,
        auto_approve: resolved.auto_approve,
        in_place: false,
    };

    let insert_result = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store.insert_task(&task)
    };
    if let Err(e) = insert_result {
        // Worktree we created here exists on disk but the DB insert failed:
        // best-effort rollback so we don't leave disk state with no DB record.
        // An *adopted* worktree pre-existed — leave it untouched.
        if created_here {
            let _ = git::remove_worktree(repo_path, Path::new(&task.worktree_path), true).await;
        }
        return Err(e.to_string());
    }

    Ok(LaunchOutcome { task, decision: resolved.decision })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Repo;

    fn repo() -> Repo {
        Repo {
            id: "repo-1".to_string(),
            name: "r".to_string(),
            path: "/repo".to_string(),
            default_branch: "main".to_string(),
            remote_url: None,
            worktree_root: None,
            setup_command: None,
            default_agent: None,
            auto_start_agent: false,
            initial_prompt: None,
            default_model: None,
            sound_settings: None,
            fetch_remote_base: None,
            auto_approve: None,
            in_place_default: false,
        }
    }

    fn args(title: &str) -> LaunchArgs {
        LaunchArgs {
            repo_id: "repo-1".to_string(),
            title: title.to_string(),
            base_branch: None,
            ticket_key: None,
            agent: None,
            model: None,
            after_merge_of: vec![],
            prompt: None,
            auto_approve: None,
            in_place: false,
            branch_name: None,
        }
    }

    fn repo_fixture() -> Repo {
        Repo {
            id: "r1".into(),
            name: "r".into(),
            path: "/checkout/root".into(),
            default_branch: "main".into(),
            remote_url: None,
            worktree_root: None,
            setup_command: None,
            default_agent: None,
            auto_start_agent: false,
            initial_prompt: None,
            default_model: None,
            sound_settings: None,
            fetch_remote_base: None,
            auto_approve: None,
            in_place_default: false,
        }
    }

    fn base_args() -> LaunchArgs {
        LaunchArgs {
            repo_id: "r1".into(),
            title: "My Task".into(),
            base_branch: None,
            ticket_key: None,
            agent: None,
            model: None,
            after_merge_of: Vec::new(),
            prompt: None,
            auto_approve: None,
            in_place: false,
            branch_name: None,
        }
    }

    #[test]
    fn resolve_in_place_uses_repo_path_and_empty_branch_when_no_name() {
        let repo = repo_fixture();
        let args = LaunchArgs { in_place: true, ..base_args() };
        let r = resolve_launch(args, &repo, Path::new("/worktrees")).unwrap();
        assert_eq!(r.worktree_path, std::path::PathBuf::from("/checkout/root"));
        assert_eq!(r.base_branch, "main");
        assert!(r.in_place);
        assert_eq!(r.branch, ""); // empty ⇒ launch_task adopts the current branch
        assert_eq!(r.branch_name, None);
    }

    #[test]
    fn resolve_in_place_uses_provided_branch_name_verbatim() {
        let repo = repo_fixture();
        let args = LaunchArgs { in_place: true, branch_name: Some("  hotfix/api  ".into()), ..base_args() };
        let r = resolve_launch(args, &repo, Path::new("/worktrees")).unwrap();
        assert_eq!(r.worktree_path, std::path::PathBuf::from("/checkout/root"));
        assert_eq!(r.branch, "hotfix/api"); // trimmed, NOT slugified
        assert_eq!(r.branch_name.as_deref(), Some("hotfix/api"));
    }

    #[test]
    fn resolve_non_in_place_unchanged() {
        let repo = repo_fixture();
        let r = resolve_launch(base_args(), &repo, Path::new("/worktrees")).unwrap();
        assert!(!r.in_place);
        assert_eq!(r.worktree_path, std::path::PathBuf::from("/worktrees/r1/my-task"));
        assert_eq!(r.branch, "my-task");
    }

    #[test]
    fn has_active_in_place_true_only_for_live_in_place_task() {
        let mk = |in_place: bool, status: TaskStatus| Task {
            id: "x".into(), repo_id: "r1".into(), title: "t".into(),
            worktree_path: "/checkout/root".into(), branch: "main".into(),
            base_branch: "main".into(), status, created_at: 0, updated_at: 0,
            pr_number: None, pr_url: None, ticket_key: None, agent: None, model: None,
            setup_status: None, hidden: false, pending_prompt: None, auto_approve: None,
            in_place,
        };
        assert!(!has_active_in_place(&[mk(false, TaskStatus::Idle)]));
        assert!(!has_active_in_place(&[mk(true, TaskStatus::Done)]));
        assert!(has_active_in_place(&[mk(true, TaskStatus::Idle)]));
        assert!(has_active_in_place(&[mk(true, TaskStatus::Working)]));
    }

    #[test]
    fn decision_empty_is_immediate() {
        assert_eq!(LaunchDecision::from(&[]), LaunchDecision::Immediate);
    }

    #[test]
    fn decision_all_whitespace_is_immediate() {
        let ids = vec!["   ".to_string(), "".to_string()];
        assert_eq!(LaunchDecision::from(&ids), LaunchDecision::Immediate);
    }

    #[test]
    fn decision_trims_dedups_and_drops_empties() {
        let ids = vec![
            "  TASK-7 ".to_string(),
            "".to_string(),
            "TASK-7".to_string(), // dup after trim
            "TASK-8".to_string(),
        ];
        assert_eq!(
            LaunchDecision::from(&ids),
            LaunchDecision::Pending { after_merge_of: vec!["TASK-7".to_string(), "TASK-8".to_string()] }
        );
    }

    #[test]
    fn resolve_defaults_base_and_branch_from_title() {
        let r = resolve_launch(args("My Task"), &repo(), Path::new("/wt")).unwrap();
        assert_eq!(r.base_branch, "main");
        assert_eq!(r.branch, "my-task");
        assert_eq!(r.worktree_path, Path::new("/wt/repo-1/my-task"));
        assert_eq!(r.decision, LaunchDecision::Immediate);
    }

    #[test]
    fn resolve_honors_base_override() {
        let mut a = args("My Task");
        a.base_branch = Some("develop".to_string());
        let r = resolve_launch(a, &repo(), Path::new("/wt")).unwrap();
        assert_eq!(r.base_branch, "develop");
    }

    #[test]
    fn resolve_branch_from_ticket_key() {
        let mut a = args("My Task");
        a.ticket_key = Some("TASK-88".to_string());
        let r = resolve_launch(a, &repo(), Path::new("/wt")).unwrap();
        assert_eq!(r.branch, "task-88");
        assert_eq!(r.ticket_key.as_deref(), Some("TASK-88"));
    }

    #[test]
    fn resolve_normalizes_whitespace_ticket_and_agent_to_none() {
        let mut a = args("My Task");
        a.ticket_key = Some("   ".to_string());
        a.agent = Some("  ".to_string());
        let r = resolve_launch(a, &repo(), Path::new("/wt")).unwrap();
        assert_eq!(r.ticket_key, None);
        assert_eq!(r.agent, None);
        assert_eq!(r.branch, "my-task"); // empty ticket ⇒ branch from title
    }

    #[test]
    fn resolve_rejects_empty_identity() {
        let err = resolve_launch(args("   "), &repo(), Path::new("/wt")).unwrap_err();
        assert!(err.contains("needs a title or a ticket ID"), "got: {err}");
    }

    #[test]
    fn resolve_rejects_flaglike_base() {
        let mut a = args("My Task");
        a.base_branch = Some("-x".to_string());
        let err = resolve_launch(a, &repo(), Path::new("/wt")).unwrap_err();
        assert!(err.contains("invalid base branch"), "got: {err}");
    }

    #[test]
    fn resolve_worktree_path_with_repo_override() {
        let mut r = repo();
        r.worktree_root = Some("/custom".to_string());
        let res = resolve_launch(args("My Task"), &r, Path::new("/wt")).unwrap();
        assert_eq!(res.worktree_path, Path::new("/custom/my-task"));
    }

    #[test]
    fn resolve_carries_pending_decision() {
        let mut a = args("My Task");
        a.after_merge_of = vec!["TASK-7".to_string(), "TASK-8".to_string()];
        let r = resolve_launch(a, &repo(), Path::new("/wt")).unwrap();
        assert_eq!(
            r.decision,
            LaunchDecision::Pending { after_merge_of: vec!["TASK-7".to_string(), "TASK-8".to_string()] }
        );
    }

    #[test]
    fn resolve_normalizes_pending_prompt() {
        let mut a = args("My Task");
        a.prompt = Some("  seed me  ".to_string());
        let r = resolve_launch(a, &repo(), Path::new("/wt")).unwrap();
        assert_eq!(r.pending_prompt.as_deref(), Some("seed me"));

        let mut blank = args("My Task");
        blank.prompt = Some("   ".to_string());
        let r2 = resolve_launch(blank, &repo(), Path::new("/wt")).unwrap();
        assert_eq!(r2.pending_prompt, None);
    }

    #[test]
    fn resolve_launch_passes_through_auto_approve() {
        let mut a = args("My Task");
        a.auto_approve = Some(false);
        let r = resolve_launch(a, &repo(), Path::new("/wt")).unwrap();
        assert_eq!(r.auto_approve, Some(false));
    }

    #[test]
    fn live_blockers_keeps_only_existing_ids() {
        let ids = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let live = live_blockers(&ids, |id| id == "a" || id == "c");
        assert_eq!(live, vec!["a", "c"]);
    }

    #[test]
    fn live_blockers_all_dangling_is_empty() {
        let ids = vec!["gone1".to_string(), "gone2".to_string()];
        let live = live_blockers(&ids, |_| false);
        assert!(live.is_empty());
    }
}
