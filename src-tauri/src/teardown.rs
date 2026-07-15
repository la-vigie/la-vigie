//! Shared task-teardown core (TASK-139): stop an agent's PTY, remove its
//! worktree, and delete its DB row — behind a safety gate. Reused by the
//! HookBridge `/finish/{agentId}` route and (TASK-140) the MCP `finish_task`.

use std::path::Path;

use crate::state::AppState;

/// Tauri event payload for `"task_removed"` (TASK-139): the task the backend
/// tore down. The frontend deselects it (if selected) and refreshes so the
/// sidebar drops it live, mirroring the GUI's own delete-task flow.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TaskRemovedPayload {
    task_id: String,
}

/// Result of a teardown attempt. `Unsafe` carries a human-readable reason for
/// the 409 body; `UnknownTask` maps to 404; `Done` to 200.
#[derive(Debug, Clone, PartialEq)]
pub enum TeardownOutcome {
    Done,
    UnknownTask,
    Unsafe(String),
}

/// Pure safety decision. Returns `Some(reason)` when teardown must be refused,
/// `None` when it is safe. A dirty tree is always unsafe; committed work is only
/// unsafe when it is not yet merged (the branch is kept, so merged commits
/// survive regardless).
fn unsafe_reason(dirty: bool, commits_ahead: bool, pr_merged: bool) -> Option<String> {
    if dirty {
        return Some("worktree has uncommitted or untracked changes".to_string());
    }
    if commits_ahead && !pr_merged {
        return Some("branch has commits not merged to base (no merged PR)".to_string());
    }
    None
}

/// Whether teardown should physically `git worktree remove` the task's path.
/// False for in-place tasks (TASK-163) — their `worktree_path` IS the repo's main
/// checkout, which must never be removed — and for path-less queued tasks.
pub fn should_remove_worktree(in_place: bool, worktree_path: &str) -> bool {
    !in_place && !worktree_path.is_empty()
}

/// Captured, safety-passed data for a task committed to teardown. Produced by
/// [`prepare_teardown`], consumed by [`perform_teardown`]. All-owned so it can
/// be moved into a detached task. `agent_id` is the task's live agent (whose PTY
/// must be stopped), or `None` when no agent is currently live — e.g. after an
/// app restart, where the process is already gone and only the durable task row
/// (worktree + DB) remains to clean up (TASK-151).
pub struct TeardownPlan {
    pub agent_id: Option<String>,
    pub task_id: String,
    pub worktree_path: String,
    pub repo_path: String,
    pub branch: String,
    /// TASK-163: an in-place task's `worktree_path` is the repo's main checkout;
    /// `perform_teardown` must skip `remove_worktree` for it.
    pub in_place: bool,
}

/// Outcome of the prepare phase: either an early terminal result (nothing to
/// tear down) or a plan that is safe to perform.
pub enum TeardownStep {
    Early(TeardownOutcome),
    Ready(TeardownPlan),
}

/// Phase 1 (cancellation-safe: mutates nothing): capture task/repo by task_id and
/// run the safety gate. Returns `Early` for UnknownTask/Unsafe, or `Ready(plan)`
/// when it is safe to proceed.
///
/// The task is resolved directly from the DB by its `task_id` — the durable key
/// (TASK-151) — not through the in-memory agent→task map. So teardown resolves
/// correctly even after an app restart empties that map, and the caller (the
/// `/finished` skill) proves which task it owns by presenting its `LAVIGIE_TASK_ID`.
pub async fn prepare_teardown(
    state: &AppState,
    task_id: &str,
    force: bool,
) -> Result<TeardownStep, String> {
    // 1. Capture task + repo + fetch setting by task_id; drop the store guard before await.
    let (worktree_path, branch, base_branch, repo_path, use_remote, has_remote, in_place) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let task = match store.get_task(task_id).map_err(|e| format!("{e:#}"))? {
            Some(t) => t,
            None => return Ok(TeardownStep::Early(TeardownOutcome::UnknownTask)),
        };
        let repo = match store.get_repo(&task.repo_id).map_err(|e| format!("{e:#}"))? {
            Some(r) => r,
            None => return Ok(TeardownStep::Early(TeardownOutcome::UnknownTask)),
        };
        let app_setting = store
            .get_app_setting("fetch_remote_base")
            .map_err(|e| format!("{e:#}"))?
            .as_deref()
            .and_then(crate::commands::parse_bool_setting);
        let use_remote =
            crate::commands::effective_fetch_remote_base(repo.fetch_remote_base, app_setting);
        (
            task.worktree_path,
            task.branch,
            task.base_branch,
            repo.path,
            use_remote,
            repo.remote_url.is_some(),
            task.in_place,
        )
    };

    // 2. Resolve the task's live agent (if any) whose PTY must be stopped. After a
    //    restart the map/sessions are empty → None → nothing to stop (the process is
    //    already gone), and teardown proceeds on the durable task row regardless.
    let live_agent = {
        let live: std::collections::HashSet<String> = state
            .sessions
            .lock()
            .map_err(|e| e.to_string())?
            .keys()
            .cloned()
            .collect();
        let map = state.agent_tasks.lock().map_err(|e| e.to_string())?;
        crate::session::resolve_live_agent(&map, &live, task_id)
    };

    // 3. Safety gate. In-place tasks never destroy anything (teardown detaches
    //    only), so the gate never blocks them (TASK-163).
    if !force && !in_place {
        let dirty = crate::git::working_tree_dirty(Path::new(&worktree_path))
            .await
            .map_err(|e| format!("{e:#}"))?;
        let (commits_ahead, pr_merged) = if dirty {
            (false, false) // short-circuit: no network when already unsafe
        } else {
            // Compare against freshly-fetched origin/<base> so commits merged in
            // from upstream (rebase/merge for conflict resolution) don't count as
            // this task's unmerged work (TASK-144). Best-effort: a fetch failure or
            // absent tracking ref degrades to the local base — never blocks/errors.
            if use_remote && has_remote {
                let _ = crate::git::fetch(Path::new(&worktree_path), "origin", &base_branch).await;
            }
            let origin_ref_exists = use_remote
                && has_remote
                && crate::git::ref_exists(
                    Path::new(&worktree_path),
                    &format!("origin/{base_branch}"),
                )
                .await;
            let compare_base = crate::commands::comparison_base_ref(
                use_remote,
                origin_ref_exists,
                "origin",
                &base_branch,
            );

            let ahead = crate::git::commits_ahead_of_base(Path::new(&worktree_path), &compare_base)
                .await
                .map_err(|e| format!("{e:#}"))?;
            let merged = if ahead {
                matches!(
                    crate::github::pr_status(Path::new(&worktree_path), &branch).await,
                    Ok(Some(pr)) if pr.state == "MERGED"
                )
            } else {
                false
            };
            (ahead, merged)
        };
        if let Some(reason) = unsafe_reason(dirty, commits_ahead, pr_merged) {
            return Ok(TeardownStep::Early(TeardownOutcome::Unsafe(reason)));
        }
    }

    Ok(TeardownStep::Ready(TeardownPlan {
        agent_id: live_agent,
        task_id: task_id.to_string(),
        worktree_path,
        repo_path,
        branch,
        in_place,
    }))
}

/// Phase 2 (must run to completion once started): stop the agent's PTY, abort
/// its background setup job, remove the worktree, then delete the DB row. The
/// self-teardown HTTP path spawns this detached so a request cancellation
/// cannot interrupt it midway. NOTE: `stop_session_inner` runs before the git
/// op (a live agent cwd'd in the worktree would complicate removal); if
/// `remove_worktree` then fails, the task row is left intact with its agent
/// already stopped — an agent-less but otherwise intact task, recoverable via
/// the GUI delete flow.
pub async fn perform_teardown(
    state: &AppState,
    app: &tauri::AppHandle,
    plan: &TeardownPlan,
) -> Result<(), String> {
    // 4. Stop the task's live agent PTY, if one is still running. None after a
    //    restart (the process is already gone). Ignore "already stopped".
    if let Some(agent_id) = &plan.agent_id {
        let _ = crate::agent::stop_session_inner(state, agent_id);
    }

    // 5. Abort a still-running background setup job (mirrors delete_task).
    if let Ok(mut setups) = state.setups.lock() {
        if let Some(s) = setups.remove(&plan.task_id) {
            if let Some(h) = s.handle {
                h.abort();
            }
        }
    }

    // 6. Remove the worktree, then delete the DB row (worktree-first so a git
    //    failure leaves the row intact rather than orphaning a live worktree).
    // TASK-163: in-place tasks skip this — `worktree_path` is the repo's main
    // checkout, never a task-owned worktree to remove.
    if should_remove_worktree(plan.in_place, &plan.worktree_path) {
        crate::git::remove_worktree(Path::new(&plan.repo_path), Path::new(&plan.worktree_path), true)
            .await
            .map_err(|e| format!("{e:#}"))?;
    }
    {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store.delete_task(&plan.task_id).map_err(|e| format!("{e:#}"))?;
    }

    use tauri::Emitter as _;
    // Tell the webview the task is gone so the sidebar drops it live (backend-initiated
    // teardown has no webview round-trip that would otherwise refresh). Best-effort.
    let _ = app.emit("task_removed", TaskRemovedPayload { task_id: plan.task_id.clone() });

    // TASK-204: drop the torn-down task from the system-tray menu live.
    crate::tray::refresh(app);

    Ok(())
}

/// Compose prepare + perform synchronously. Awaited directly by non-self-teardown
/// callers (e.g. TASK-140 MCP finish_task) that tear down a DIFFERENT task and so
/// have no request-cancellation hazard. `perform_teardown` (which deletes the
/// blocker's DB row) runs BEFORE promoting any dependents queued on this task
/// (TASK-90); `promote` is the `?promote` bypass (skip the landed check, e.g.
/// no-PR flows).
///
/// TASK-182: promote AFTER the row is deleted, mirroring the direct `finish_task`
/// path. Once the blocker row is gone, a concurrent `launch_task` can no longer
/// queue a new dependent behind it (`live_blockers` filters the now-dangling
/// blocker and launches immediately instead), so the queue-vs-finish window that
/// left a waiter stranded Pending — an edge inserted during teardown's
/// `git worktree remove` await and then orphaned — is closed. A dependent queued
/// before the row deletion is still captured by `promote_dependents_of`.
pub async fn teardown_task(
    state: &AppState,
    app: &tauri::AppHandle,
    task_id: &str,
    force: bool,
    promote: bool,
) -> Result<TeardownOutcome, String> {
    match prepare_teardown(state, task_id, force).await? {
        TeardownStep::Early(outcome) => Ok(outcome),
        TeardownStep::Ready(plan) => {
            perform_teardown(state, app, &plan).await?;
            crate::commands::promote_dependents_of(
                state, app, &plan.task_id, &plan.branch, &plan.repo_path, promote,
            ).await;
            Ok(TeardownOutcome::Done)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_tree_no_commits_is_safe() {
        assert_eq!(unsafe_reason(false, false, false), None);
    }

    #[test]
    fn dirty_tree_is_unsafe() {
        assert!(unsafe_reason(true, false, false).is_some());
    }

    #[test]
    fn unmerged_commits_are_unsafe() {
        assert!(unsafe_reason(false, true, false).is_some());
    }

    #[test]
    fn merged_commits_are_safe() {
        // Squash-merge: commits ahead of base but the PR is merged → safe.
        assert_eq!(unsafe_reason(false, true, true), None);
    }

    #[test]
    fn should_remove_worktree_never_for_in_place() {
        // In-place task: worktree_path IS the repo's main checkout — never remove.
        assert!(!should_remove_worktree(true, "/repo/root"));
        // Queued/path-less task: nothing to remove.
        assert!(!should_remove_worktree(false, ""));
        // Normal worktree task: remove it.
        assert!(should_remove_worktree(false, "/worktrees/r1/branch"));
    }

    // ── TASK-151: teardown resolves by task_id, independent of the in-memory map ──

    use crate::store::{Repo, Task, TaskStatus, TaskStore};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Build a minimal `AppState` backed by a temp SQLite store holding one repo +
    /// one task, with **empty** `agent_tasks`/`sessions` maps — the exact state after
    /// an app restart. `prepare_teardown` here must still resolve the task by its id.
    fn state_with_task(dir: &TempDir, task_id: &str, worktree_path: &str) -> AppState {
        let store = TaskStore::open(&dir.path().join("db.sqlite")).expect("open store");
        store
            .insert_repo(&Repo {
                id: "repo-1".into(),
                name: "r".into(),
                path: dir.path().to_string_lossy().into_owned(),
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
            })
            .unwrap();
        store
            .insert_task(&Task {
                id: task_id.into(),
                repo_id: "repo-1".into(),
                title: "t".into(),
                worktree_path: worktree_path.into(),
                branch: "b".into(),
                base_branch: "main".into(),
                status: TaskStatus::Idle,
                created_at: 0,
                updated_at: 0,
                pr_number: None,
                pr_url: None,
                ticket_key: None,
                agent: None,
                model: None,
                setup_status: None,
                hidden: false,
                pending_prompt: None,
                auto_approve: None,
                in_place: false,
            })
            .unwrap();

        AppState {
            store: Mutex::new(store),
            worktrees_root: dir.path().to_path_buf(),
            sounds_root: dir.path().to_path_buf(),
            concierge_root: dir.path().to_path_buf(),
            sessions: Mutex::new(HashMap::new()),
            hook_port: 0,
            agent_states: Mutex::new(HashMap::new()),
            agent_tasks: Mutex::new(HashMap::new()),
            setups: Mutex::new(HashMap::new()),
            mcp_port: 0,
            mcp_tokens: Mutex::new(HashMap::new()),
            remote: Mutex::new(crate::remote::RemoteState::default()),
            transcripts: Mutex::new(HashMap::new()),
            pending_questions: Mutex::new(HashMap::new()),
            concierge_spawn: Mutex::new(()),
            base_fetch_at: Mutex::new(HashMap::new()),
        }
    }

    #[tokio::test]
    async fn restart_teardown_resolves_task_by_id_with_empty_map() {
        // Post-restart: agent_tasks is empty and no PTY is live, but /finish for the
        // still-known task_id must resolve to the correct worktree (force=true skips
        // the git safety gate, so this stays a pure store-backed unit test).
        let dir = TempDir::new().unwrap();
        let state = state_with_task(&dir, "task-1", "/tmp/wt-task-1");

        let step = prepare_teardown(&state, "task-1", true).await.unwrap();
        match step {
            TeardownStep::Ready(plan) => {
                assert_eq!(plan.task_id, "task-1");
                assert_eq!(plan.worktree_path, "/tmp/wt-task-1");
                // No live agent after a restart → nothing to stop.
                assert_eq!(plan.agent_id, None);
            }
            TeardownStep::Early(o) => panic!("expected Ready, got {o:?}"),
        }
    }

    #[tokio::test]
    async fn teardown_unknown_task_id_is_early_unknown_task() {
        let dir = TempDir::new().unwrap();
        let state = state_with_task(&dir, "task-1", "/tmp/wt-task-1");

        let step = prepare_teardown(&state, "does-not-exist", true).await.unwrap();
        assert!(matches!(
            step,
            TeardownStep::Early(TeardownOutcome::UnknownTask)
        ));
    }
}
