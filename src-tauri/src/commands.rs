//! Tauri commands: thin glue between the frontend, the git layer, and the
//! task store. Pure logic (slugify, worktree path composition) is extracted
//! into standalone functions so it can be unit-tested without a Tauri app
//! harness.

use std::path::{Path, PathBuf};

use tauri::Emitter as _;
use tauri::State;

use crate::docs::{self, DocRef};
use crate::git::{self, FileChange};
use crate::github;
use crate::setup::{self, SetupEvent, SetupOutcome};
use crate::state::{AppState, SetupState};
use crate::store::{Repo, SetupStatus, Task, TaskStatus};

/// Returns `false` if `branch` looks like a git flag (starts with `'-'`),
/// which would be misinterpreted as a positional argument by git. This is an
/// app-originated value, not a security boundary, but rejecting it early
/// gives a clearer error than a confusing git failure.
pub fn is_valid_base_branch(branch: &str) -> bool {
    !branch.starts_with('-')
}

/// App-level default for "base new worktrees on the fetched remote base branch"
/// when neither the repo override nor the app setting is set.
pub const DEFAULT_FETCH_REMOTE_BASE: bool = true;

/// Resolve the effective "use remote base" flag: repo override wins, else the
/// app setting, else the const default.
pub fn effective_fetch_remote_base(repo_override: Option<bool>, app_setting: Option<bool>) -> bool {
    repo_override.or(app_setting).unwrap_or(DEFAULT_FETCH_REMOTE_BASE)
}

/// App-level default for "inject La Vigie's bundled default skills into launched
/// agents" (TASK-153). OFF — the operator opts in.
pub const DEFAULT_INJECT_LAVIGIE_SKILLS: bool = false;

/// Resolve whether to inject the bundled skill plugin. v1 is app-level only;
/// repo/task overrides are TASK-154 (this signature will gain them then).
pub fn effective_inject_lavigie_skills(app_setting: Option<bool>) -> bool {
    app_setting.unwrap_or(DEFAULT_INJECT_LAVIGIE_SKILLS)
}

/// The worktree start-point ref: the fetched remote-tracking ref only when we
/// both want it and the fetch succeeded; otherwise the local base branch.
pub fn worktree_base_ref(use_remote: bool, fetch_ok: bool, remote: &str, base: &str) -> String {
    if use_remote && fetch_ok {
        format!("{remote}/{base}")
    } else {
        base.to_string()
    }
}

/// App-level throttle window for the Diff tab's background base fetch (TASK-144):
/// the tab re-renders often, so fetch `origin/<base>` at most once per window.
pub const BASE_FETCH_THROTTLE: std::time::Duration = std::time::Duration::from_secs(15);

/// The ref to compare a task's `HEAD` against for diffs and the teardown gate:
/// the remote-tracking base (`<remote>/<base>`) when we both want the remote base
/// AND that tracking ref is present, otherwise the local `<base>`. Freshness is
/// the caller's job (a best-effort fetch); this only picks the ref, so a failed or
/// never-run fetch degrades to the last-known `<remote>/<base>` (if present) or the
/// local base — never an error.
pub fn comparison_base_ref(
    use_remote: bool,
    origin_ref_exists: bool,
    remote: &str,
    base: &str,
) -> String {
    if use_remote && origin_ref_exists {
        format!("{remote}/{base}")
    } else {
        base.to_string()
    }
}

/// Throttle decision for the Diff-tab background base fetch: fetch when we have
/// never fetched (`None`) or the last fetch is at least `window` old.
pub fn should_fetch(since_last: Option<std::time::Duration>, window: std::time::Duration) -> bool {
    match since_last {
        None => true,
        Some(elapsed) => elapsed >= window,
    }
}

/// Parse a stored bool app-setting value. Unknown strings -> None (treated as unset).
pub fn parse_bool_setting(s: &str) -> Option<bool> {
    match s {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Encode a bool app-setting value for storage.
pub fn bool_setting_str(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

/// Lowercase `title`, collapse runs of non-alphanumeric characters into a
/// single `-`, and trim leading/trailing `-`. Returns `"task"` if the result
/// would otherwise be empty.
pub fn slugify(title: &str) -> String {
    let mut slug = String::with_capacity(title.len());
    let mut last_was_dash = false;

    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }

    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "task".to_string()
    } else {
        trimmed.to_string()
    }
}

/// The worktree branch for a task: the slugified ticket key when one is present
/// (non-whitespace), otherwise the slugified title. Mirrors today's title-only
/// behavior when there is no key.
pub fn task_branch(title: &str, ticket_key: Option<&str>) -> String {
    match ticket_key {
        Some(key) if !key.trim().is_empty() => slugify(key),
        _ => slugify(title),
    }
}

/// A task needs at least one human-or-machine identifier: a non-empty title or
/// a non-empty ticket key.
pub fn has_task_identity(title: &str, ticket_key: Option<&str>) -> bool {
    !title.trim().is_empty() || ticket_key.map_or(false, |k| !k.trim().is_empty())
}

/// Compose the on-disk worktree path for a task. With a per-repo override:
/// `<base>/<branch>`. Otherwise the global default: `<worktrees_root>/<repo_id>/<branch>`.
pub fn worktree_path_for(
    worktrees_root: &Path,
    repo_worktree_root: Option<&str>,
    repo_id: &str,
    branch: &str,
) -> PathBuf {
    match repo_worktree_root {
        Some(base) => Path::new(base).join(branch),
        None => worktrees_root.join(repo_id).join(branch),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub repos: Vec<Repo>,
    pub tasks: Vec<Task>,
    pub worktrees_root: String,
    pub sound_settings: Option<String>,
    pub fetch_remote_base: Option<bool>,
    pub inject_lavigie_skills: Option<bool>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupOutputPayload {
    task_id: String,
    data: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupStatusPayload {
    task_id: String,
    status: SetupStatus,
    exit_code: Option<i32>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStateDto {
    status: Option<SetupStatus>,
    log: String,
    exit_code: Option<i32>,
}

/// Spawn the repo's setup as a detached background job for `task_id`. Streams
/// output via the `setup_output` event + the in-memory registry log, and on
/// completion persists + emits the terminal `setup_status`. Never holds the
/// store or setups Mutex across the `run_setup(...).await`.
fn spawn_setup_job(
    app: tauri::AppHandle,
    task_id: String,
    worktree_path: std::path::PathBuf,
    setup_command: Option<String>,
) {
    use tauri::Manager as _;

    let handle = tauri::async_runtime::spawn({
        let app = app.clone();
        let task_id = task_id.clone();
        async move {
            // Emit the initial running status (registry was seeded by the caller).
            let _ = app.emit(
                "setup_status",
                SetupStatusPayload { task_id: task_id.clone(), status: SetupStatus::Running, exit_code: None },
            );

            let outcome = setup::run_setup(&worktree_path, setup_command.as_deref(), {
                let app = app.clone();
                let task_id = task_id.clone();
                move |ev| {
                    if let SetupEvent::Output { data } = ev {
                        // Brief lock: append to the live log, then drop before emit.
                        {
                            let state = app.state::<AppState>();
                            if let Ok(mut setups) = state.setups.lock() {
                                if let Some(s) = setups.get_mut(&task_id) {
                                    s.log.push_str(&data);
                                }
                            };
                        }
                        let _ = app.emit(
                            "setup_output",
                            SetupOutputPayload { task_id: task_id.clone(), data },
                        );
                    }
                }
            })
            .await;

            // Map the outcome to a terminal status (+ surface spawn errors as output).
            let (status, exit_code) = match outcome {
                Ok(SetupOutcome::Ran { code }) if code == 0 => (SetupStatus::Succeeded, Some(code)),
                Ok(SetupOutcome::Ran { code }) => (SetupStatus::Failed, Some(code)),
                Ok(SetupOutcome::Skipped) => (SetupStatus::Succeeded, None),
                Err(e) => {
                    let msg = format!("setup failed: {e:#}\n");
                    {
                        let state = app.state::<AppState>();
                        if let Ok(mut setups) = state.setups.lock() {
                            if let Some(s) = setups.get_mut(&task_id) {
                                s.log.push_str(&msg);
                            }
                        };
                    }
                    let _ = app.emit(
                        "setup_output",
                        SetupOutputPayload { task_id: task_id.clone(), data: msg },
                    );
                    (SetupStatus::Failed, None)
                }
            };

            // Update the in-memory status (brief lock).
            {
                let state = app.state::<AppState>();
                if let Ok(mut setups) = state.setups.lock() {
                    if let Some(s) = setups.get_mut(&task_id) {
                        s.status = status;
                        s.exit_code = exit_code;
                    }
                };
            }
            // Persist the terminal status (brief lock, dropped immediately).
            {
                let state = app.state::<AppState>();
                if let Ok(store) = state.store.lock() {
                    let _ = store.set_task_setup_status(&task_id, Some(status));
                };
            }
            let _ = app.emit(
                "setup_status",
                SetupStatusPayload { task_id, status, exit_code },
            );
        }
    });

    // Record the handle so delete_task can abort a running job.
    let state = app.state::<AppState>();
    if let Ok(mut setups) = state.setups.lock() {
        if let Some(s) = setups.get_mut(&task_id) {
            s.handle = Some(handle);
        }
    };
}

#[tauri::command]
pub async fn add_repo(state: State<'_, AppState>, path: String) -> Result<Repo, String> {
    let repo = git::add_repo(Path::new(&path))
        .await
        .map_err(|e| format!("{e:#}"))?;

    {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store.insert_repo(&repo).map_err(|e| e.to_string())?;
    }

    Ok(repo)
}

#[tauri::command]
pub async fn update_repo(
    state: State<'_, AppState>,
    repo_id: String,
    name: String,
    default_branch: String,
    worktree_root: Option<String>,
    setup_command: Option<String>,
    auto_start_agent: bool,
    initial_prompt: Option<String>,
    sound_settings: Option<String>,
    fetch_remote_base: Option<bool>,
    default_agent: Option<String>,
    auto_approve: Option<bool>,
) -> Result<Repo, String> {
    let name = name.trim().to_string();
    let default_branch = default_branch.trim().to_string();
    // Empty / whitespace-only means "unset" (inherit the global default agent).
    let default_agent = default_agent
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    // Empty / whitespace-only override means "use the global default" (unset).
    let worktree_root = worktree_root
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    // Empty / whitespace-only means "unset" (fall back to .vigie/setup.sh).
    let setup_command = setup_command
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let initial_prompt = initial_prompt
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if name.is_empty() {
        return Err("repo name cannot be empty".to_string());
    }
    if default_branch.is_empty() {
        return Err("default base branch cannot be empty".to_string());
    }
    // Reject a base branch that could be misread as a git flag (starts with '-').
    if !is_valid_base_branch(&default_branch) {
        return Err(format!("invalid base branch: {default_branch}"));
    }

    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .update_repo(
            &repo_id,
            &name,
            &default_branch,
            worktree_root.as_deref(),
            setup_command.as_deref(),
            auto_start_agent,
            initial_prompt.as_deref(),
            default_agent.as_deref(),
        )
        .map_err(|e| format!("{e:#}"))?;
    store
        .set_repo_sound_settings(&repo_id, sound_settings.as_deref())
        .map_err(|e| format!("{e:#}"))?;
    store
        .set_repo_fetch_remote_base(&repo_id, fetch_remote_base)
        .map_err(|e| format!("{e:#}"))?;
    store
        .set_repo_auto_approve(&repo_id, auto_approve)
        .map_err(|e| format!("{e:#}"))?;
    let repo = store
        .get_repo(&repo_id)
        .map_err(|e| format!("{e:#}"))?
        .ok_or_else(|| format!("repo not found: {repo_id}"))?;
    Ok(repo)
}

/// Persist the app-level sound-notification settings (raw JSON, validated by
/// the frontend). Stored under the `sound_notifications` app_settings key.
#[tauri::command]
pub async fn set_sound_settings(
    state: State<'_, AppState>,
    settings: String,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .set_app_setting("sound_notifications", &settings)
        .map_err(|e| format!("{e:#}"))?;
    Ok(())
}

/// True when the user is in a meeting (mic or camera capturing anywhere on the
/// system). Used by the frontend to optionally suppress notification sounds
/// (TASK-105). Native probe on macOS; always `false` on other platforms.
#[tauri::command]
pub fn is_meeting_active() -> bool {
    crate::meeting::is_meeting_active()
}

/// Persist the app-level "base new worktrees on the fetched remote base" flag,
/// stored under the `fetch_remote_base` app_settings key.
#[tauri::command]
pub async fn set_fetch_remote_base(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .set_app_setting("fetch_remote_base", bool_setting_str(enabled))
        .map_err(|e| format!("{e:#}"))?;
    Ok(())
}

/// Persist the app-level "inject La Vigie default skills" flag, stored under the
/// `inject_lavigie_skills` app_settings key (TASK-153).
#[tauri::command]
pub async fn set_inject_lavigie_skills(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .set_app_setting("inject_lavigie_skills", bool_setting_str(enabled))
        .map_err(|e| format!("{e:#}"))?;
    Ok(())
}

#[tauri::command]
pub async fn list_prompts(state: State<'_, AppState>) -> Result<Vec<crate::store::Prompt>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.list_prompts().map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn create_prompt(
    state: State<'_, AppState>,
    label: String,
    body: String,
) -> Result<crate::store::Prompt, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let prompts = store.list_prompts().map_err(|e| format!("{e:#}"))?;
    let position = prompts.iter().map(|p| p.position).max().map_or(0, |m| m + 1);
    let prompt = crate::store::Prompt {
        id: uuid::Uuid::new_v4().to_string(),
        label,
        body,
        position,
    };
    store.insert_prompt(&prompt).map_err(|e| format!("{e:#}"))?;
    Ok(prompt)
}

#[tauri::command]
pub async fn update_prompt(
    state: State<'_, AppState>,
    id: String,
    label: String,
    body: String,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.update_prompt(&id, &label, &body).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn delete_prompt(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.delete_prompt(&id).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn reorder_prompts(
    state: State<'_, AppState>,
    ordered_ids: Vec<String>,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.reorder_prompts(&ordered_ids).map_err(|e| format!("{e:#}"))
}

/// Remove a repo from La Vigie: delete the DB rows (repo + cascade its tasks)
/// and clean up the worktrees La Vigie created for those tasks. The user's
/// original checkout at `repo.path` is never touched.
///
/// Ordering rationale (mirrors `delete_task`, avoids a stale DB row):
///   1. Capture the repo path + its tasks' worktree paths under the lock.
///   2. Delete the repo row (cascades task rows) — DB is now consistent.
///   3. Best-effort `git worktree remove` each captured worktree (ignore
///      per-worktree errors so one bad worktree doesn't block the rest).
/// The lock is dropped before the async git work (locking invariant).
#[tauri::command]
pub async fn remove_repo(state: State<'_, AppState>, repo_id: String) -> Result<(), String> {
    let (repo_path, worktree_paths) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let repo = store
            .get_repo(&repo_id)
            .map_err(|e| format!("{e:#}"))?
            .ok_or_else(|| format!("repo not found: {repo_id}"))?;
        let worktree_paths: Vec<String> = store
            .list_tasks_for_repo(&repo_id)
            .map_err(|e| format!("{e:#}"))?
            .into_iter()
            .map(|t| t.worktree_path)
            .collect();
        (repo.path, worktree_paths)
    };

    {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store.delete_repo(&repo_id).map_err(|e| format!("{e:#}"))?;
    }

    for worktree_path in worktree_paths {
        // Best-effort: a worktree the user already removed by hand shouldn't
        // block detaching the rest of the repo.
        let _ = git::remove_worktree(Path::new(&repo_path), Path::new(&worktree_path), true).await;
    }

    Ok(())
}

/// List the local git branches for a repo, so the settings UI can offer the
/// default base branch as a dropdown. Locks the store only to resolve the
/// repo path, then drops the guard before the async git call (locking invariant).
#[tauri::command]
pub async fn list_repo_branches(
    state: State<'_, AppState>,
    repo_id: String,
) -> Result<Vec<String>, String> {
    let repo_path = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .get_repo(&repo_id)
            .map_err(|e| format!("{e:#}"))?
            .ok_or_else(|| format!("repo not found: {repo_id}"))?
            .path
    };
    git::list_branches(Path::new(&repo_path))
        .await
        .map_err(|e| format!("{e:#}"))
}

/// Shared launch path: run the TASK-88 launch core, then kick off the repo's
/// background setup job (TASK-96). Used by both the `create_task` command and the
/// MCP `start_task` tool so worktree/row creation and setup stay one path.
///
/// Locking: each store lock is captured-then-dropped before the next `.await`.
pub async fn launch_and_kickoff_setup(
    state: &AppState,
    app: &tauri::AppHandle,
    args: crate::launch::LaunchArgs,
) -> Result<Task, String> {
    let task = crate::launch::launch_task(state, args).await?.task;

    // A queued (Pending) task has no worktree yet — setup runs at promote-time.
    if task.status == TaskStatus::Pending {
        return Ok(task);
    }

    kickoff_setup(state, app, &task)?;
    Ok(task)
}

/// Kick off the repo's background setup job for a freshly-created/promoted task
/// (TASK-96). No-op when the worktree has no setup to run. Shared by the create
/// path and the TASK-90 promote path.
///
/// Locking: each store lock is captured-then-dropped; this fn does not `.await`.
pub(crate) fn kickoff_setup(
    state: &AppState,
    app: &tauri::AppHandle,
    task: &Task,
) -> Result<(), String> {
    // Look up the repo's setup command (brief lock).
    let setup_command = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .get_repo(&task.repo_id)
            .map_err(|e| e.to_string())?
            .and_then(|r| r.setup_command)
    };

    // Kick off the repo's setup as a non-blocking background job (per-repo
    // command takes precedence; falls back to .vigie/setup.sh). The task already
    // exists + is navigable; setup streams output via the setup_* events. A
    // failure marks the task's setup_status failed but does NOT roll back the
    // worktree (the task and its output stay for the user to inspect/retry).
    let worktree_path = PathBuf::from(&task.worktree_path);
    if setup::will_run(&worktree_path, setup_command.as_deref()) {
        {
            let mut setups = state.setups.lock().map_err(|e| e.to_string())?;
            setups.insert(
                task.id.clone(),
                SetupState { status: SetupStatus::Running, log: String::new(), exit_code: None, handle: None },
            );
        }
        // Persist Running so a restart mid-setup still shows "running".
        {
            let store = state.store.lock().map_err(|e| e.to_string())?;
            let _ = store.set_task_setup_status(&task.id, Some(SetupStatus::Running));
        }
        spawn_setup_job(app.clone(), task.id.clone(), worktree_path, setup_command);
    }

    Ok(())
}

#[tauri::command]
pub async fn create_task(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    repo_id: String,
    title: String,
    base_branch: Option<String>,
    ticket_key: Option<String>,
    agent: Option<String>,
    model: Option<String>,
    auto_approve: Option<bool>,
) -> Result<Task, String> {
    // Durable creation + background setup go through the shared launch path so
    // the `create_task` command and the MCP `start_task` tool share one path.
    let args = crate::launch::LaunchArgs {
        repo_id,
        title,
        base_branch,
        ticket_key,
        agent,
        model,
        auto_approve,
        // The frontend create path never queues on another task (TASK-90).
        after_merge_of: None,
        prompt: None,
    };
    let task = launch_and_kickoff_setup(state.inner(), &app, args).await?;
    Ok(task)
}

/// Preview of what creating a task at the derived worktree path would do, for the
/// New Task modal warning (TASK-125). `state` is `"vacant"` (path free), `"adopt"`
/// (an existing worktree on the intended branch will be reused), or `"conflict"`
/// (the path is occupied by something that doesn't match — creation would fail).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreePreview {
    pub state: String,
    pub path: String,
    pub message: Option<String>,
}

/// Check whether the worktree path derived from the given task inputs already
/// exists on disk, so the New Task modal can warn before submit (TASK-125). Never
/// errors on incomplete input — an unresolvable request (e.g. no title yet)
/// returns a `vacant` preview with no message so the UI simply shows nothing.
#[tauri::command]
pub async fn check_worktree_path(
    state: State<'_, AppState>,
    repo_id: String,
    title: String,
    base_branch: Option<String>,
    ticket_key: Option<String>,
) -> Result<WorktreePreview, String> {
    let repo = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .get_repo(&repo_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("repo not found: {repo_id}"))?
    };

    let args = crate::launch::LaunchArgs {
        repo_id,
        title,
        base_branch,
        ticket_key,
        agent: None,
        model: None,
        after_merge_of: None,
        prompt: None,
        auto_approve: None,
    };
    // Resolution can fail on incomplete input (no identity yet, flag-like base):
    // there's nothing to warn about, so present a vacant preview.
    let resolved = match crate::launch::resolve_launch(args, &repo, &state.worktrees_root) {
        Ok(r) => r,
        Err(_) => {
            return Ok(WorktreePreview { state: "vacant".to_string(), path: String::new(), message: None })
        }
    };

    let already_task_owned = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .list_tasks()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|t| Path::new(&t.worktree_path) == resolved.worktree_path)
    };

    let repo_path = Path::new(&repo.path);
    let adoption =
        git::worktree_state(repo_path, &resolved.worktree_path, &resolved.branch, already_task_owned)
            .await;

    let path = resolved.worktree_path.to_string_lossy().to_string();
    let preview = match adoption {
        git::WorktreeAdoption::Vacant => {
            // The path is free, but the branch may already exist (a leftover from a
            // deleted task): note that its commits will be reused (TASK-125).
            let branch_exists =
                git::ref_exists(repo_path, &format!("refs/heads/{}", resolved.branch)).await;
            if branch_exists {
                WorktreePreview {
                    state: "reuse-branch".to_string(),
                    message: Some(format!(
                        "Branch {} already exists — its commits will be reused.",
                        resolved.branch
                    )),
                    path,
                }
            } else {
                WorktreePreview { state: "vacant".to_string(), path, message: None }
            }
        }
        git::WorktreeAdoption::Adopt => WorktreePreview {
            state: "adopt".to_string(),
            message: Some(format!("A worktree already exists at {path} — it will be reused.")),
            path,
        },
        git::WorktreeAdoption::Reclaim { reason } => {
            WorktreePreview { state: "reclaim".to_string(), path, message: Some(reason) }
        }
        git::WorktreeAdoption::Conflict { reason } => {
            WorktreePreview { state: "conflict".to_string(), path, message: Some(reason) }
        }
    };
    Ok(preview)
}

#[tauri::command]
pub async fn delete_task(
    state: State<'_, AppState>,
    task_id: String,
    delete_branch: bool,
) -> Result<(), String> {
    let (task, repo) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let task = store
            .get_task(&task_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("task not found: {task_id}"))?;
        let repo = store
            .get_repo(&task.repo_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("repo not found: {}", task.repo_id))?;
        (task, repo)
    };

    // Delete the DB row before touching the worktree on disk: if worktree
    // removal fails afterwards, the DB is still left in a consistent state
    // (task gone) rather than a stale row pointing at a deleted/half-removed
    // path. The lock is dropped before the async git call below.
    {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store.delete_task(&task_id).map_err(|e| e.to_string())?;
    }

    // Abort a still-running background setup and drop its registry entry so it
    // stops writing to a worktree we're about to remove (kill_on_drop kills the
    // child when the aborted future drops).
    if let Ok(mut setups) = state.setups.lock() {
        if let Some(s) = setups.remove(&task_id) {
            if let Some(h) = s.handle {
                h.abort();
            }
        }
    }

    // A queued (Pending, TASK-90) task has no worktree/branch on disk — skip the
    // git teardown; the DB row delete above already cascaded its dependency
    // edges away.
    if !task.worktree_path.is_empty() {
        git::remove_worktree(
            Path::new(&repo.path),
            Path::new(&task.worktree_path),
            true,
        )
        .await
        .map_err(|e| format!("{e:#}"))?;

        // Optional best-effort branch deletion (the modal's opt-in checkbox).
        // Mirrors Finish "discard": force-delete, ignore failure.
        if delete_branch && !task.branch.is_empty() {
            let _ = git::delete_branch(Path::new(&repo.path), &task.branch, true).await;
        }
    }

    Ok(())
}

/// Returns a task's current setup state for the UI to hydrate on (re)mount.
/// Prefers the in-memory registry (live log + status); falls back to the
/// persisted DB status with an empty log (e.g. after an app restart).
#[tauri::command]
pub fn get_setup_state(state: State<'_, AppState>, task_id: String) -> Result<SetupStateDto, String> {
    {
        let setups = state.setups.lock().map_err(|e| e.to_string())?;
        if let Some(s) = setups.get(&task_id) {
            return Ok(SetupStateDto { status: Some(s.status), log: s.log.clone(), exit_code: s.exit_code });
        }
    }
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let status = store
        .get_task(&task_id)
        .map_err(|e| e.to_string())?
        .and_then(|t| t.setup_status);
    Ok(SetupStateDto { status, log: String::new(), exit_code: None })
}

/// Finish a task: remove its worktree, delete the DB row, and (in discard/merge
/// mode) best-effort delete the branch.
///
/// `mode` must be `"keep"`, `"discard"`, or `"merge"`.
///
/// Ordering rationale (avoids a stale DB row):
///   1. merge only: squash-merge the PR via `gh`.
///   2. Remove the worktree (git) — if this fails nothing else has changed
///      (for keep/discard; for merge, the PR is already merged).
///   3. Delete the DB row      — task is now finished from the app's view.
///   4. discard/merge only: best-effort delete the branch (ignore errors).
#[tauri::command]
pub async fn finish_task(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    task_id: String,
    mode: String,
) -> Result<(), String> {
    if mode != "keep" && mode != "discard" && mode != "merge" {
        return Err(format!("invalid finish mode: {mode}"));
    }

    // Lock, capture what we need, drop lock before any await.
    let (worktree_path, branch, repo_path) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let task = store
            .get_task(&task_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("task not found: {task_id}"))?;
        let repo = store
            .get_repo(&task.repo_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("repo not found: {}", task.repo_id))?;
        (task.worktree_path.clone(), task.branch.clone(), repo.path.clone())
    };

    // merge mode: resolve PR number and squash-merge before cleanup.
    if mode == "merge" {
        let pr = github::pr_status(Path::new(&worktree_path), &branch)
            .await
            .map_err(|e| format!("{e:#}"))?;
        let pr_number = pr
            .ok_or_else(|| "no PR found for this task".to_string())?
            .number;
        github::merge_pr(Path::new(&worktree_path), pr_number)
            .await
            .map_err(|e| format!("{e:#}"))?;
    }

    // Remove worktree — propagate error; DB row not yet deleted.
    git::remove_worktree(
        Path::new(&repo_path),
        Path::new(&worktree_path),
        true,
    )
    .await
    .map_err(|e| format!("{e:#}"))?;

    // Delete the DB row.
    {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store.delete_task(&task_id).map_err(|e| e.to_string())?;
    }

    // discard/merge mode — best-effort branch deletion (ignore error).
    if mode == "discard" || mode == "merge" {
        let _ = git::delete_branch(Path::new(&repo_path), &branch, true).await;
    }

    // TASK-90 (revised): promote dependents when this task's work has landed
    // (PR MERGED auto-detected). Merge mode already squash-merged the PR above,
    // so the landed check is redundant — bypass it. Keep/discard still verify
    // via `gh pr view`. Cheap early-out when there are no dependents.
    promote_dependents_of(state.inner(), &app, &task_id, &branch, &repo_path, mode == "merge").await;

    Ok(())
}

/// Promote one queued (Pending) task now that its dependency has merged: create
/// the worktree off the up-to-date base, flip it live, kick off setup, and emit
/// `task_launched` so the frontend starts the agent (TASK-90). Store-Mutex is
/// captured-then-dropped before each await.
async fn promote_pending_task(
    state: &AppState,
    app: &tauri::AppHandle,
    dep_id: &str,
) -> Result<(), String> {
    // 1. Capture the pending row + repo, and its seeded prompt.
    let (task, repo) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let task = store
            .get_task(dep_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("pending task not found: {dep_id}"))?;
        let repo = store
            .get_repo(&task.repo_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("repo not found: {}", task.repo_id))?;
        (task, repo)
    };

    // Idempotency: if this row is no longer Pending, another finish surface (or
    // the TASK-91 poller) already promoted it — do nothing, so we never re-create
    // a worktree or double-start an agent for an already-live task.
    if task.status != crate::store::TaskStatus::Pending {
        return Ok(());
    }

    let initial_prompt = task.pending_prompt.clone();

    // 2. Re-resolve branch/worktree off the now-updated base (after_merge_of=None).
    let args = crate::launch::LaunchArgs {
        repo_id: task.repo_id.clone(),
        title: task.title.clone(),
        base_branch: Some(task.base_branch.clone()),
        ticket_key: task.ticket_key.clone(),
        agent: task.agent.clone(),
        model: task.model.clone(),
        after_merge_of: None,
        prompt: None,
        auto_approve: None,
    };
    let resolved = crate::launch::resolve_launch(args, &repo, &state.worktrees_root)?;

    // 3. Create the worktree (fetches origin/<base>, now containing the merge).
    crate::launch::prepare_worktree(
        state,
        &repo,
        &resolved.base_branch,
        &resolved.branch,
        &resolved.worktree_path,
    )
    .await?;

    // 4. Flip the row live + kick off setup.
    let worktree_path = resolved
        .worktree_path
        .to_str()
        .ok_or("worktree path is not valid UTF-8")?
        .to_string();
    let promote_result = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store.promote_task(dep_id, &worktree_path, &resolved.branch)
    };
    if let Err(e) = promote_result {
        // Worktree exists on disk but the DB promote failed: best-effort rollback
        // so a retry isn't blocked by a leftover worktree with no DB record
        // (mirrors launch_task's insert-failure rollback in launch.rs).
        let _ = git::remove_worktree(Path::new(&repo.path), &resolved.worktree_path, true).await;
        return Err(e.to_string());
    }
    let live_task = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .get_task(dep_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("promoted task vanished: {dep_id}"))?
    };
    kickoff_setup(state, app, &live_task)?;

    // 5. Tell the frontend to start the agent (reuses TASK-89 useTaskLaunch).
    crate::mcp::emit_task_launched(app, dep_id.to_string(), initial_prompt);
    Ok(())
}

/// TASK-90 (revised): promote every dependent queued on `task_id` when its work
/// has LANDED. Dependents-first early-out — the gh landed-check runs ONLY when a
/// dependent is actually waiting (the common case exits after one indexed SELECT).
/// `landed = bypass || (this task's PR state == MERGED)`. Infallible + error-
/// isolated: a promote failure marks that dependent Error and never affects the
/// finishing/teardown caller.
pub(crate) async fn promote_dependents_of(
    state: &AppState,
    app: &tauri::AppHandle,
    task_id: &str,
    branch: &str,
    repo_path: &str,
    bypass: bool,
) {
    // 1. Cheap indexed lookup. No dependents ⇒ exit with no gh call, no work.
    let dependents = {
        let store = match state.store.lock() { Ok(s) => s, Err(e) => { eprintln!("TASK-90: store lock: {e}"); return; } };
        match store.dependents_of(task_id) {
            Ok(d) => d,
            Err(e) => { eprintln!("TASK-90: dependents_of({task_id}): {e:#}"); return; }
        }
    };
    if dependents.is_empty() {
        return;
    }
    // 2. Only now pay for the landed check. bypass short-circuits (no PR / no gh).
    let landed = bypass
        || matches!(
            crate::github::pr_status(Path::new(repo_path), branch).await,
            Ok(Some(pr)) if pr.state == "MERGED"
        );
    if !landed {
        return; // dependents stay queued; edges left intact
    }
    // 3. Satisfied ⇒ clear this dependency's edges, promote each 0-unmet dependent.
    {
        let store = match state.store.lock() { Ok(s) => s, Err(e) => { eprintln!("TASK-90: store lock: {e}"); return; } };
        if let Err(e) = store.remove_dependencies_on(task_id) {
            eprintln!("TASK-90: remove_dependencies_on({task_id}): {e:#}");
            return;
        }
    }
    for dep_id in dependents {
        let ready = {
            let store = match state.store.lock() { Ok(s) => s, Err(_) => continue };
            matches!(store.unmet_dependency_count(&dep_id), Ok(0))
        };
        if !ready { continue; }
        if let Err(e) = promote_pending_task(state, app, &dep_id).await {
            eprintln!("TASK-90: promoting dependent {dep_id}: {e:#}");
            if let Ok(store) = state.store.lock() {
                let _ = store.update_task_status(&dep_id, crate::store::TaskStatus::Error);
                let _ = store.set_task_setup_status(&dep_id, Some(SetupStatus::Failed));
            }
        }
    }
}

/// Build the full app snapshot from the store (brief lock, no await). Shared by
/// the `list_state` Tauri command and the remote `GET /api/state` handler.
pub fn build_snapshot(state: &AppState) -> Result<AppSnapshot, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let repos = store.list_repos().map_err(|e| e.to_string())?;
    let tasks = store.list_tasks().map_err(|e| e.to_string())?;
    let sound_settings = store
        .get_app_setting("sound_notifications")
        .map_err(|e| format!("{e:#}"))?;
    let fetch_remote_base = store
        .get_app_setting("fetch_remote_base")
        .map_err(|e| format!("{e:#}"))?
        .as_deref()
        .and_then(parse_bool_setting);
    let inject_lavigie_skills = store
        .get_app_setting("inject_lavigie_skills")
        .map_err(|e| format!("{e:#}"))?
        .as_deref()
        .and_then(parse_bool_setting);
    let worktrees_root = state.worktrees_root.to_string_lossy().to_string();
    Ok(AppSnapshot { repos, tasks, worktrees_root, sound_settings, fetch_remote_base, inject_lavigie_skills })
}

#[tauri::command]
pub async fn list_state(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    build_snapshot(state.inner())
}

// ── Diff / review commands ────────────────────────────────────────────────────

/// Resolve the diff target ref for a review scope.
/// - `"uncommitted"` → `HEAD` (working tree vs last commit: the uncommitted,
///   commit-able changes).
/// - anything else (default `"base"`) → the task's base branch (the overall
///   branch diff vs base: read-only "compared to <base>").
fn diff_target<'a>(scope: Option<&str>, base_branch: &'a str) -> &'a str {
    match scope {
        Some("uncommitted") => "HEAD",
        _ => base_branch,
    }
}

/// Captured inputs (lock → capture → drop) for resolving a task's diff base.
struct DiffBaseCtx {
    worktree_path: String,
    base_branch: String,
    /// Effective `fetch_remote_base`: repo override → app setting → default.
    use_remote: bool,
    /// Whether the repo has an `origin` remote at all.
    has_remote: bool,
    /// Throttle-map key: `"<repo_id>:<base_branch>"`.
    throttle_key: String,
}

/// Capture everything needed to resolve a task's diff base under one brief store
/// lock, then drop the guard before any git/await work.
fn diff_base_context(state: &State<'_, AppState>, task_id: &str) -> Result<DiffBaseCtx, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let task = store
        .get_task(task_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("task not found: {task_id}"))?;
    let repo = store
        .get_repo(&task.repo_id)
        .map_err(|e| format!("{e:#}"))?
        .ok_or_else(|| format!("repo not found: {}", task.repo_id))?;
    let app_setting = store
        .get_app_setting("fetch_remote_base")
        .map_err(|e| format!("{e:#}"))?
        .as_deref()
        .and_then(parse_bool_setting);
    Ok(DiffBaseCtx {
        worktree_path: task.worktree_path.clone(),
        base_branch: task.base_branch.clone(),
        use_remote: effective_fetch_remote_base(repo.fetch_remote_base, app_setting),
        has_remote: repo.remote_url.is_some(),
        throttle_key: format!("{}:{}", task.repo_id, task.base_branch),
    })
}

/// Resolve the ref the Diff tab should compare `HEAD` against. Best-effort and
/// non-blocking: when we want the remote base and the repo has a remote, a
/// throttled background fetch refreshes `origin/<base>` (spawned, never awaited —
/// so the diff never waits on the network and works offline); resolution itself
/// uses the *last-known* `origin/<base>` tracking ref. Degrades to the local base
/// when we don't want the remote, there's no remote, or the tracking ref is absent.
async fn resolve_base_ref(state: &AppState, ctx: &DiffBaseCtx) -> String {
    if ctx.use_remote && ctx.has_remote {
        // Throttle: decide + stamp under a brief lock, never across `.await`.
        let should = {
            let mut map = match state.base_fetch_at.lock() {
                Ok(m) => m,
                Err(_) => return comparison_base_ref(ctx.use_remote, false, "origin", &ctx.base_branch),
            };
            let since = map.get(&ctx.throttle_key).map(|t| t.elapsed());
            if should_fetch(since, BASE_FETCH_THROTTLE) {
                map.insert(ctx.throttle_key.clone(), std::time::Instant::now());
                true
            } else {
                false
            }
        };
        if should {
            // Fire-and-forget: refreshes the ref for the *next* render. Failure
            // (offline / no upstream branch) is intentionally ignored.
            let wt = ctx.worktree_path.clone();
            let base = ctx.base_branch.clone();
            tauri::async_runtime::spawn(async move {
                let _ = git::fetch(Path::new(&wt), "origin", &base).await;
            });
        }
    }

    let origin_ref_exists = ctx.use_remote
        && ctx.has_remote
        && git::ref_exists(Path::new(&ctx.worktree_path), &format!("origin/{}", ctx.base_branch)).await;

    comparison_base_ref(ctx.use_remote, origin_ref_exists, "origin", &ctx.base_branch)
}

/// Return the unified diff of the task's worktree for the given review scope
/// (`uncommitted` vs `base`). Defaults to `base`. For the `base` scope the diff
/// is taken against the freshly-fetched `origin/<base>` when available (TASK-144),
/// else the local base; the `uncommitted` scope is unaffected (always vs `HEAD`).
#[tauri::command]
pub async fn get_diff(
    state: State<'_, AppState>,
    task_id: String,
    scope: Option<String>,
) -> Result<String, String> {
    let ctx = diff_base_context(&state, &task_id)?;

    // Only the base scope compares against origin/<base>; skip the fetch/resolve
    // for the uncommitted scope (diff_target returns HEAD there regardless).
    let base_ref = if scope.as_deref() == Some("uncommitted") {
        ctx.base_branch.clone()
    } else {
        resolve_base_ref(state.inner(), &ctx).await
    };
    let target = diff_target(scope.as_deref(), &base_ref);

    let mut diff = git::diff_against_base(Path::new(&ctx.worktree_path), target)
        .await
        .map_err(|e| e.to_string())?;

    if scope.as_deref() == Some("uncommitted") {
        let untracked_diff = git::diff_untracked(Path::new(&ctx.worktree_path))
            .await
            .map_err(|e| e.to_string())?;
        diff.push_str(&untracked_diff);
    }

    Ok(diff)
}

/// Return the list of files changed in the task's worktree for the given review
/// scope (`uncommitted` vs `base`). Defaults to `base`. Base scope compares
/// against the freshly-fetched `origin/<base>` when available (TASK-144).
#[tauri::command]
pub async fn get_changed_files(
    state: State<'_, AppState>,
    task_id: String,
    scope: Option<String>,
) -> Result<Vec<FileChange>, String> {
    let ctx = diff_base_context(&state, &task_id)?;

    let base_ref = if scope.as_deref() == Some("uncommitted") {
        ctx.base_branch.clone()
    } else {
        resolve_base_ref(state.inner(), &ctx).await
    };
    let target = diff_target(scope.as_deref(), &base_ref);

    let mut changes = git::changed_files(Path::new(&ctx.worktree_path), target)
        .await
        .map_err(|e| e.to_string())?;

    // Untracked (new, non-ignored) files are invisible to `git diff`; surface
    // them as additions in the uncommitted/Iteration view only.
    if scope.as_deref() == Some("uncommitted") {
        let untracked = git::untracked_files(Path::new(&ctx.worktree_path))
            .await
            .map_err(|e| e.to_string())?;
        for path in untracked {
            changes.push(FileChange {
                path,
                change: git::ChangeKind::Added,
            });
        }
    }

    Ok(changes)
}

/// Stage the given paths in the task's worktree.
#[tauri::command]
pub async fn stage_files(
    state: State<'_, AppState>,
    task_id: String,
    paths: Vec<String>,
) -> Result<(), String> {
    let (worktree_path, _) = task_paths(&state, &task_id)?;
    git::stage(Path::new(&worktree_path), &paths)
        .await
        .map_err(|e| e.to_string())
}

/// Commit staged changes in the task's worktree with the given message.
#[tauri::command]
pub async fn commit_task(
    state: State<'_, AppState>,
    task_id: String,
    message: String,
) -> Result<(), String> {
    let (worktree_path, _) = task_paths(&state, &task_id)?;
    git::commit(Path::new(&worktree_path), &message)
        .await
        .map_err(|e| e.to_string())
}

/// Worktree-relative paths that are new on this branch relative to `base_branch`:
/// files added/modified vs the merge-base, plus still-untracked files. Deletions
/// are excluded (a removed doc is not something to show). This is the "not on
/// main" set that `docs::resolve_docs` filters down to the doc directories.
async fn branch_doc_paths(worktree_path: &str, base_branch: &str) -> Result<Vec<String>, String> {
    let wt = Path::new(worktree_path);
    let mut paths: Vec<String> = git::changed_files(wt, base_branch)
        .await
        .map_err(|e| format!("{e:#}"))?
        .into_iter()
        .filter(|fc| fc.change != git::ChangeKind::Deleted)
        .map(|fc| fc.path)
        .collect();
    paths.extend(
        git::untracked_files(wt)
            .await
            .map_err(|e| format!("{e:#}"))?,
    );
    Ok(paths)
}

/// List the spec/design/plan docs available for a task (worktree-relative ids).
#[tauri::command]
pub async fn list_task_docs(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Vec<DocRef>, String> {
    let (worktree_path, base_branch, ticket_key) = task_doc_context(&state, &task_id)?;
    let branch_docs = branch_doc_paths(&worktree_path, &base_branch).await?;
    Ok(docs::resolve_docs(
        Path::new(&worktree_path),
        ticket_key.as_deref(),
        &branch_docs,
    ))
}

/// Read one task doc's markdown by its worktree-relative id. The id is validated
/// against the resolved allow-list, so arbitrary files cannot be read.
#[tauri::command]
pub async fn read_task_doc(
    state: State<'_, AppState>,
    task_id: String,
    id: String,
) -> Result<String, String> {
    let (worktree_path, base_branch, ticket_key) = task_doc_context(&state, &task_id)?;
    let branch_docs = branch_doc_paths(&worktree_path, &base_branch).await?;
    docs::read_doc(
        Path::new(&worktree_path),
        ticket_key.as_deref(),
        &branch_docs,
        &id,
    )
}

// ── GitHub / PR commands ──────────────────────────────────────────────────────

/// Return whether `gh` is on PATH and the current user is authenticated.
#[tauri::command]
pub async fn gh_status() -> Result<github::GhStatus, String> {
    Ok(github::gh_status().await)
}

/// Push the task's branch, open a PR, persist the PR reference, and return it.
#[tauri::command]
pub async fn create_pr(
    state: State<'_, AppState>,
    task_id: String,
    title: String,
    body: String,
    draft: bool,
) -> Result<github::PrRef, String> {
    // Lock, capture, drop before any await.
    let (worktree_path, branch, base_branch) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let task = store
            .get_task(&task_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("task not found: {task_id}"))?;
        (task.worktree_path.clone(), task.branch.clone(), task.base_branch.clone())
    };

    let pr = github::create_pr(
        Path::new(&worktree_path),
        &branch,
        &base_branch,
        &title,
        &body,
        draft,
    )
    .await
    .map_err(|e| format!("{e:#}"))?;

    // Persist the PR reference.
    {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .set_task_pr(&task_id, pr.number, &pr.url)
            .map_err(|e| format!("{e:#}"))?;
    }

    Ok(pr)
}

/// Return the current PR status for the task's branch, or `None` if no PR exists.
#[tauri::command]
pub async fn get_pr_status(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Option<github::PrStatus>, String> {
    let (worktree_path, branch) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let task = store
            .get_task(&task_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("task not found: {task_id}"))?;
        (task.worktree_path.clone(), task.branch.clone())
    };

    github::pr_status(Path::new(&worktree_path), &branch)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// Set the hidden state of a task. Non-destructive visibility toggle.
#[tauri::command]
pub fn set_task_hidden(
    state: tauri::State<'_, AppState>,
    task_id: String,
    hidden: bool,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| format!("{e:#}"))?;
    // Verify task exists first
    let _task = store
        .get_task(&task_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("task not found: {task_id}"))?;

    store
        .set_task_hidden(&task_id, hidden)
        .map_err(|e| format!("Failed to update task hidden: {e:#}"))?;

    Ok(())
}

/// Return all comments/reviews for the task's PR (issue + review + inline),
/// in chronological order. Returns an empty list when no PR exists for the branch.
#[tauri::command]
pub async fn get_pr_comments(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Vec<github::PrComment>, String> {
    let (worktree_path, branch) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let task = store
            .get_task(&task_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("task not found: {task_id}"))?;
        (task.worktree_path.clone(), task.branch.clone())
    };

    let worktree = Path::new(&worktree_path);
    match github::pr_status(worktree, &branch)
        .await
        .map_err(|e| format!("{e:#}"))?
    {
        Some(s) => github::pr_comments(worktree, s.number)
            .await
            .map_err(|e| format!("{e:#}")),
        None => Ok(vec![]),
    }
}

/// Lock store, look up task, drop lock, return (worktree_path, base_branch).
fn task_paths(state: &State<'_, AppState>, task_id: &str) -> Result<(String, String), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let task = store
        .get_task(task_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("task not found: {task_id}"))?;
    Ok((task.worktree_path.clone(), task.base_branch.clone()))
}

/// Capture a task's worktree path, base branch, and ticket key (lock → capture
/// → drop guard).
fn task_doc_context(
    state: &State<'_, AppState>,
    task_id: &str,
) -> Result<(String, String, Option<String>), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let task = store
        .get_task(task_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("task not found: {task_id}"))?;
    Ok((
        task.worktree_path.clone(),
        task.base_branch.clone(),
        task.ticket_key.clone(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_lowercases_and_dashes_punctuation() {
        assert_eq!(slugify("Fix Login Bug!"), "fix-login-bug");
    }

    #[test]
    fn slugify_trims_and_collapses_non_alphanumeric_runs() {
        assert_eq!(slugify("  C++  "), "c");
    }

    #[test]
    fn slugify_empty_title_falls_back_to_task() {
        assert_eq!(slugify(""), "task");
    }

    #[test]
    fn slugify_all_punctuation_falls_back_to_task() {
        assert_eq!(slugify("!!!"), "task");
    }

    #[test]
    fn slugify_collapses_multiple_separators_into_one_dash() {
        assert_eq!(slugify("foo___bar   baz"), "foo-bar-baz");
    }

    #[test]
    fn worktree_path_for_uses_root_repo_id_and_branch_when_unset() {
        let root = Path::new("/data/worktrees");
        let path = worktree_path_for(root, None, "repo-123", "fix-login-bug");
        assert_eq!(path, Path::new("/data/worktrees/repo-123/fix-login-bug"));
    }

    #[test]
    fn worktree_path_for_uses_custom_base_without_repo_id() {
        let root = Path::new("/data/worktrees");
        let path = worktree_path_for(root, Some("/Users/me/wt"), "repo-123", "fix-login-bug");
        assert_eq!(path, Path::new("/Users/me/wt/fix-login-bug"));
    }

    #[test]
    fn is_valid_base_branch_accepts_normal_branch_names() {
        assert!(is_valid_base_branch("main"));
        assert!(is_valid_base_branch("feature/my-branch"));
        assert!(is_valid_base_branch("release-1.0"));
    }

    #[test]
    fn is_valid_base_branch_rejects_dash_prefix() {
        assert!(!is_valid_base_branch("-f"));
        assert!(!is_valid_base_branch("--force"));
        assert!(!is_valid_base_branch("-"));
    }

    #[test]
    fn is_valid_base_branch_accepts_empty_string() {
        // An empty string is not a flag; other layers handle empty validation.
        assert!(is_valid_base_branch(""));
    }

    #[test]
    fn task_branch_uses_the_ticket_key_when_present() {
        assert_eq!(task_branch("Fix login bug", Some("TST-1")), "tst-1");
    }

    #[test]
    fn task_branch_falls_back_to_the_title_without_a_key() {
        assert_eq!(task_branch("Fix login bug", None), "fix-login-bug");
        // A whitespace-only key is treated as absent.
        assert_eq!(task_branch("Fix login bug", Some("   ")), "fix-login-bug");
    }

    #[test]
    fn has_task_identity_requires_a_title_or_a_key() {
        assert!(has_task_identity("Fix login", None));
        assert!(has_task_identity("", Some("TST-1")));
        assert!(!has_task_identity("", None));
        assert!(!has_task_identity("   ", Some("   ")));
    }

    #[test]
    fn effective_inject_defaults_off() {
        assert!(!effective_inject_lavigie_skills(None));
        assert!(effective_inject_lavigie_skills(Some(true)));
        assert!(!effective_inject_lavigie_skills(Some(false)));
    }

    #[test]
    fn effective_fetch_remote_base_precedence() {
        // repo override wins
        assert!(effective_fetch_remote_base(Some(true), Some(false)));
        assert!(!effective_fetch_remote_base(Some(false), Some(true)));
        // None repo -> inherit app
        assert!(effective_fetch_remote_base(None, Some(true)));
        assert!(!effective_fetch_remote_base(None, Some(false)));
        // both None -> const default (true)
        assert_eq!(effective_fetch_remote_base(None, None), DEFAULT_FETCH_REMOTE_BASE);
    }

    #[test]
    fn worktree_base_ref_picks_remote_only_when_used_and_fetched() {
        assert_eq!(worktree_base_ref(true, true, "origin", "main"), "origin/main");
        assert_eq!(worktree_base_ref(true, false, "origin", "main"), "main");
        assert_eq!(worktree_base_ref(false, true, "origin", "main"), "main");
        assert_eq!(worktree_base_ref(false, false, "origin", "main"), "main");
    }

    #[test]
    fn comparison_base_ref_prefers_origin_only_when_wanted_and_present() {
        assert_eq!(comparison_base_ref(true, true, "origin", "main"), "origin/main");
        // want remote but ref absent (fetch failed / never fetched) -> local base
        assert_eq!(comparison_base_ref(true, false, "origin", "main"), "main");
        // don't want remote -> local base even if the ref happens to exist
        assert_eq!(comparison_base_ref(false, true, "origin", "main"), "main");
        assert_eq!(comparison_base_ref(false, false, "origin", "develop"), "develop");
    }

    #[test]
    fn should_fetch_true_when_never_fetched_or_window_elapsed() {
        use std::time::Duration;
        let window = Duration::from_secs(15);
        assert!(should_fetch(None, window), "never fetched -> fetch");
        assert!(should_fetch(Some(Duration::from_secs(15)), window), "exactly at window -> fetch");
        assert!(should_fetch(Some(Duration::from_secs(30)), window), "past window -> fetch");
        assert!(!should_fetch(Some(Duration::from_secs(5)), window), "inside window -> skip");
    }

    #[test]
    fn bool_setting_round_trips() {
        assert_eq!(parse_bool_setting("true"), Some(true));
        assert_eq!(parse_bool_setting("false"), Some(false));
        assert_eq!(parse_bool_setting("garbage"), None);
        assert_eq!(bool_setting_str(true), "true");
        assert_eq!(bool_setting_str(false), "false");
    }


}
