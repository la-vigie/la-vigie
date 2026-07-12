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
    pub after_merge_of: Option<String>,
    /// Seed prompt for a queued task; stored as `pending_prompt` and emitted at
    /// promote-time. Ignored for immediate launches (the caller emits the
    /// prompt via the task_launched event). TASK-90.
    pub prompt: Option<String>,
    pub auto_approve: Option<bool>,
}

/// Whether a launch happens now or is queued behind another task's merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchDecision {
    Immediate,
    Pending { after_merge_of: String },
}

impl LaunchDecision {
    /// `None` or whitespace-only `after_merge_of` ⇒ `Immediate`; otherwise
    /// `Pending` carrying the trimmed dependency id.
    pub fn from(after_merge_of: Option<&str>) -> LaunchDecision {
        match after_merge_of {
            Some(dep) if !dep.trim().is_empty() => LaunchDecision::Pending {
                after_merge_of: dep.trim().to_string(),
            },
            _ => LaunchDecision::Immediate,
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

    let branch = task_branch(&args.title, ticket_key.as_deref());
    let base_branch = args
        .base_branch
        .unwrap_or_else(|| repo.default_branch.clone());
    if !is_valid_base_branch(&base_branch) {
        return Err(format!("invalid base branch: {base_branch}"));
    }

    let worktree_path =
        worktree_path_for(worktrees_root, repo.worktree_root.as_deref(), &repo.id, &branch);
    let decision = LaunchDecision::from(args.after_merge_of.as_deref());

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
    })
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

    // 3. Pending launch (TASK-90): queue the task behind another task's merge.
    if let LaunchDecision::Pending { after_merge_of } = &resolved.decision {
        // Dangling dependency (missing / already-merged-and-deleted) ⇒ nothing
        // will ever promote us, so launch immediately instead of queuing.
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
        };
        // Defensive: a fresh uuid can never equal an existing id, so a
        // self-cycle is unreachable — assert it anyway.
        debug_assert_ne!(&task.id, after_merge_of);
        // Dangling-check + insert + edge under a SINGLE lock so the queue op is
        // atomic w.r.t. finish_task(merge): since finish deletes the dependency
        // before reading its dependents, any interleaving resolves to either
        // "queued and later promoted" or "dangling → immediate" — never a
        // stranded Pending row with an edge to an already-merged dependency.
        // (No `.await` inside the guard, so holding it here is legal.)
        let queued = {
            let store = state.store.lock().map_err(|e| e.to_string())?;
            if store.get_task(after_merge_of).map_err(|e| e.to_string())?.is_some() {
                store.insert_task(&task).map_err(|e| e.to_string())?;
                if let Err(e) = store.add_task_dependency(&task.id, after_merge_of) {
                    // Roll back the just-inserted row so a Pending task never
                    // exists without its dependency edge (all-or-nothing).
                    let _ = store.delete_task(&task.id);
                    return Err(e.to_string());
                }
                true
            } else {
                false // dangling → fall through to the immediate path below
            }
        };
        if queued {
            return Ok(LaunchOutcome { task, decision: resolved.decision });
        }
        // else: dangling → fall through to the immediate path below.
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
            after_merge_of: None,
            prompt: None,
            auto_approve: None,
        }
    }

    #[test]
    fn decision_none_is_immediate() {
        assert_eq!(LaunchDecision::from(None), LaunchDecision::Immediate);
    }

    #[test]
    fn decision_whitespace_is_immediate() {
        assert_eq!(LaunchDecision::from(Some("   ")), LaunchDecision::Immediate);
    }

    #[test]
    fn decision_some_is_pending_trimmed() {
        assert_eq!(
            LaunchDecision::from(Some("  TASK-7 ")),
            LaunchDecision::Pending { after_merge_of: "TASK-7".to_string() }
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
        a.after_merge_of = Some("TASK-7".to_string());
        let r = resolve_launch(a, &repo(), Path::new("/wt")).unwrap();
        assert_eq!(r.decision, LaunchDecision::Pending { after_merge_of: "TASK-7".to_string() });
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
}
