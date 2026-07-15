//! Concierge session primitive (TASK-112): a worktree-less, singleton Claude
//! session for the mobile concierge. This module holds the pure policy core
//! (sentinel id, idle predicate, idle-victim collection) plus glue
//! (token minting, spawn, activity tracking, reaper, desktop listing) added in
//! later tasks.
//!
//! Pure functions are unit-tested here; the glue (`ensure_concierge`,
//! `spawn_reaper`, the Tauri command) is verified live, per project convention.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tauri::ipc::Channel;

use crate::agent::PtyEvent;
use crate::state::{AppState, McpToken};

/// One rootless/remote-spawned session for the desktop "Remote sessions"
/// surface. camelCase over IPC.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSessionInfo {
    pub id: String,
    /// Lowercase session kind label, e.g. "concierge" or "orchestrator".
    pub kind: String,
    /// The repo this session is scoped to, if any. `Some` for a per-repo
    /// orchestrator (TASK-180), `None` for the legacy global concierge.
    pub repo_id: Option<String>,
    /// Seconds since the session's last client activity.
    pub idle_secs: u64,
}

/// Stable "task id" the singleton concierge is addressed by in `agent_tasks`,
/// transcripts, and the existing task-keyed remote routes. It deliberately has
/// no DB Task row; `build_snapshot` is DB-only, so the concierge never appears
/// in the task list. Greppable and obviously-not-a-real-task on purpose.
pub const CONCIERGE_SENTINEL: &str = "concierge";

/// Idle window after which a silent concierge session is reaped. The poll-based
/// remote transport gives no disconnect signal, so silence is the only signal.
/// Channel-bound (desktop-attached) sessions are exempt regardless of idle time
/// — see [`is_reapable`] — since a live frontend `Channel` is itself the
/// disconnect signal (TASK-126).
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Pure: install a fresh Concierge token into the MCP token map, evicting any
/// existing Concierge token first (the concierge is a singleton — at most one
/// concierge token ever exists). Agent-tier tokens are untouched.
pub fn install_concierge_token(map: &mut HashMap<String, McpToken>, token: String) {
    map.retain(|_, t| !matches!(t, McpToken::Concierge));
    map.insert(token, McpToken::Concierge);
}

/// Mint a new broad-scope concierge MCP bearer token and register it as the
/// singleton concierge token. Returns the token string.
pub fn mint_concierge_token(state: &AppState) -> Result<String, String> {
    let token = uuid::Uuid::new_v4().to_string();
    let mut guard = state.mcp_tokens.lock().map_err(|e| format!("{e:#}"))?;
    install_concierge_token(&mut guard, token.clone());
    Ok(token)
}

/// Pure: install a fresh Orchestrator token for `repo_id`, evicting only a prior
/// orchestrator token for the SAME repo (never the concierge, never other repos).
// Wired by the `open_orchestrator` command / remote route (TASK-180 Task D1).
pub fn install_orchestrator_token(map: &mut HashMap<String, McpToken>, token: String, repo_id: &str) {
    map.retain(|_, t| !matches!(t, McpToken::Orchestrator { repo_id: r } if r == repo_id));
    map.insert(token, McpToken::Orchestrator { repo_id: repo_id.to_string() });
}

/// Mint a new per-repo orchestrator MCP bearer token and register it, evicting
/// only a prior orchestrator token for the same repo. Returns the token string.
// Wired by the `open_orchestrator` command / remote route (TASK-180 Task D1).
pub fn mint_orchestrator_token(state: &AppState, repo_id: &str) -> Result<String, String> {
    let token = uuid::Uuid::new_v4().to_string();
    let mut guard = state.mcp_tokens.lock().map_err(|e| format!("{e:#}"))?;
    install_orchestrator_token(&mut guard, token.clone(), repo_id);
    Ok(token)
}

/// Pure idle predicate: has `last_activity` aged to or past `threshold` by `now`?
pub fn is_idle(last_activity: Instant, now: Instant, threshold: Duration) -> bool {
    now.duration_since(last_activity) >= threshold
}

/// Pure: from `(id, is_rootless, last_activity)` triples, collect the ids of
/// reapable rootless sessions (concierge OR per-repo orchestrator) idle past
/// `threshold` as of `now`. Task/shell sessions (`is_rootless == false`) are
/// never reaped.
pub fn collect_idle_rootless(
    sessions: &[(String, bool, Instant)],
    now: Instant,
    threshold: Duration,
) -> Vec<String> {
    sessions
        .iter()
        .filter(|(_, is_rootless, last)| *is_rootless && is_idle(*last, now, threshold))
        .map(|(id, _, _)| id.clone())
        .collect()
}

/// Pure: a rootless session is reapable only when it has NO live frontend
/// channel. A desktop-attached orchestrator (channel-bound) is driven directly
/// (no poll to refresh last_activity), so the idle reaper must not kill it —
/// same as task/shell terminals. Sink-drained rootless sessions (mobile poll
/// transport) remain reapable on silence.
pub fn is_reapable(kind: crate::agent::SessionKind, has_frontend_channel: bool) -> bool {
    matches!(kind, crate::agent::SessionKind::Concierge | crate::agent::SessionKind::Orchestrator)
        && !has_frontend_channel
}

/// Pure (TASK-126): ids of live `Orchestrator` sessions scoped to `repo_id`, from
/// `(id, kind, repo_id)` triples. The desktop `open_orchestrator_terminal` stops
/// these before respawning the session bound to a frontend channel (a PTY already
/// draining to a sink can't have a channel attached retroactively).
pub fn orchestrator_agents_for_repo(
    sessions: &[(String, crate::agent::SessionKind, Option<String>)],
    repo_id: &str,
) -> Vec<String> {
    sessions
        .iter()
        .filter(|(_, kind, rid)| {
            *kind == crate::agent::SessionKind::Orchestrator && rid.as_deref() == Some(repo_id)
        })
        .map(|(id, _, _)| id.clone())
        .collect()
}

/// Ensure a live concierge session exists, creating or resuming one if not.
///
/// Idempotent: if a concierge session is already running, returns immediately
/// without spawning — the guard against repeated mobile logins stacking
/// processes. Otherwise it mints a concierge token, spawns a rootless `claude`
/// in `concierge_root` wired with hooks (so transcripts are captured for reads)
/// and the concierge MCP config, maps the new agent to `CONCIERGE_SENTINEL`, and
/// records the `concierge_started` marker so future starts resume via
/// `--continue`. Fully synchronous (no `.await`); glue, verified live.
pub fn ensure_concierge(state: &AppState) -> Result<(), String> {
    ensure_session_inner(
        state,
        CONCIERGE_SENTINEL,
        crate::agent::SessionKind::Concierge,
        None,
        mint_concierge_token,
        Some("concierge_started"),
    )
}

/// Ensure a live per-repo orchestrator session exists for `repo_id`, creating one
/// if not. The repo-scoped sibling of [`ensure_concierge`] (TASK-180): keyed by
/// `orchestrator:{repo_id}`, liveness-gated on an existing `Orchestrator` session
/// for the *same* repo. Never auto-resumes (TASK-206): it runs in the repo's own
/// checkout (`repo.path`), which is also where an TASK-163 in-place task runs, so a
/// cwd-keyed `--continue` could reattach to that task's conversation — passing a
/// `None` marker forecloses it. A user who wants a prior thread uses `claude
/// --resume` manually. Glue, verified live.
// Wired by the `open_orchestrator` command / remote route (TASK-180 Task D1).
pub fn ensure_orchestrator(state: &AppState, repo_id: &str) -> Result<(), String> {
    let key = format!("orchestrator:{repo_id}");
    ensure_session_inner(
        state,
        &key,
        crate::agent::SessionKind::Orchestrator,
        Some(repo_id.to_string()),
        |st| mint_orchestrator_token(st, repo_id),
        None,
    )
}

/// Pure (TASK-206): the working directory for a rootless session. The global
/// concierge (`None`) uses the shared `concierge_root`; a per-repo orchestrator
/// runs in the repo's own checkout (`repo_path`) so `claude` auto-loads the
/// repo's `CLAUDE.md` and can read the tree when dispatching tasks.
pub fn session_cwd(concierge_root: &Path, repo_path: Option<&Path>) -> PathBuf {
    match repo_path {
        None => concierge_root.to_path_buf(),
        Some(p) => p.to_path_buf(),
    }
}

/// Pure (TASK-207): the always-on briefing injected via `claude
/// --append-system-prompt` into every per-repo orchestrator session, so it knows
/// its manager role and its repo-scoped MCP powers without discovering them by
/// trial. Templated with the repo's `name` (D3); states the dispatch-not-edit
/// posture in prose only (D2 — no hard `--disallowedTools` block, so the
/// orchestrator can still read the tree and handle legitimate one-offs). The tool
/// rundown is pinned to `mcp::authz::registry()` by a unit test, so a
/// newly-registered orchestrator tool must be surfaced here too or the test fails.
pub fn orchestrator_briefing(repo_name: &str) -> String {
    format!(
        "You are the orchestrator for the \"{repo_name}\" repository — its manager, \
not one of its coding agents. Your job is to dispatch, inspect, finish, and \
schedule work; the actual coding happens in isolated task worktrees run by other \
agents.\n\n\
You are running inside this repo's MAIN checkout. Do NOT edit files here — no code \
changes, commits, or branch switches in this working copy. When something needs \
changing, dispatch a task (start_task) so the work happens in its own worktree; \
never make the change yourself in place.\n\n\
You act through La Vigie's MCP tools, all scoped to the \"{repo_name}\" repo:\n\
- start_task — dispatch a new task (its own worktree + agent) in this repo\n\
- queue_dependency — dispatch a new task QUEUED behind other tasks (auto-starts \
once they merge); clearer than start_task's afterMergeOf\n\
- finish_task — tear a task down (keep / discard / merge its work)\n\
- list_tasks — list this repo's tasks\n\
- task_status — inspect one task's current state\n\
- get_task_activity — see a task's live agent activity\n\
- send_task_message — message a running task's agent to keep it moving (unblock, \
redirect, or nudge a waiting agent); requires a live agent\n\
- schedule_task — launch a task once, now or deferred\n\
- create_schedule / list_schedules / update_schedule / set_schedule_enabled / \
delete_schedule — manage recurring (cron) task dispatch\n\
- list_repos — list known repos (the one cross-repo, read-only call)\n\n\
Everything you do is confined to the \"{repo_name}\" repo; you cannot act on other \
repos' tasks or schedules."
    )
}

/// Spawn a rootless (concierge/orchestrator) `claude` session and register it,
/// optionally streaming its PTY to the frontend over `on_event` (desktop) or
/// draining to a sink when `None` (mobile/headless). Assumes the caller already
/// holds `state.concierge_spawn` and has verified no conflicting live session
/// exists. Returns the new agent id. Fully synchronous; glue, verified live.
fn spawn_rootless_session(
    state: &AppState,
    key: &str,
    kind: crate::agent::SessionKind,
    repo_id: Option<String>,
    mint_token: impl FnOnce(&AppState) -> Result<String, String>,
    marker_key: Option<&str>,
    on_event: Option<Channel<PtyEvent>>,
) -> Result<String, String> {
    // Resume vs fresh, from the persisted marker (survives app restart). A `None`
    // marker means "always spawn fresh, never persist a resume marker" — used by
    // the desktop terminal path (TASK-126), where `claude --continue` would fail if
    // the marker were set by a session that never produced a conversation (e.g. a
    // React StrictMode throwaway mount), exiting immediately.
    let resume = match marker_key {
        Some(k) => {
            let store = state.store.lock().map_err(|e| format!("{e:#}"))?;
            store.get_app_setting(k).map_err(|e| format!("{e:#}"))?.is_some()
        }
        None => false,
    };

    // TASK-206: a per-repo orchestrator runs in the repo's own checkout so it can
    // read the repo's CLAUDE.md / tree. Resolve the path up front — before minting
    // the token or registering anything — so a missing repo fails with nothing to
    // roll back. The global concierge (`None`) stays in `concierge_root`.
    // Capture the repo's `path` (for cwd) and `name` (for the TASK-207 briefing) in
    // one lock — no extra store round-trip for the name.
    let (repo_path, repo_name): (Option<PathBuf>, Option<String>) = match repo_id.as_deref() {
        None => (None, None),
        Some(rid) => {
            let store = state.store.lock().map_err(|e| format!("{e:#}"))?;
            let repo = store
                .get_repo(rid)
                .map_err(|e| format!("{e:#}"))?
                .ok_or_else(|| format!("repo not found: {rid}"))?;
            (Some(PathBuf::from(repo.path)), Some(repo.name))
        }
    };

    let token = mint_token(state)?;
    let mcp_config = crate::agent::build_mcp_config(state.mcp_port, &token);

    let agent_id = uuid::Uuid::new_v4().to_string();
    let hook_settings = crate::agent::build_hook_settings(state.hook_port, &agent_id);

    let mut args: Vec<String> = Vec::new();
    if resume {
        args.push("--continue".to_string());
    }
    args.push("--mcp-config".to_string());
    args.push(mcp_config);
    args.push("--settings".to_string());
    args.push(hook_settings);

    // TASK-207: give every per-repo orchestrator an always-on manager-role +
    // capability briefing so it knows its powers without trial-and-error. Gated to
    // the Orchestrator kind — never the legacy scopeless concierge. An orchestrator
    // always carries a `repo_id`, so `repo_name` is `Some` here; if it were somehow
    // absent we simply skip the briefing rather than fabricate one.
    if kind == crate::agent::SessionKind::Orchestrator {
        if let Some(name) = repo_name.as_deref() {
            args.push("--append-system-prompt".to_string());
            args.push(orchestrator_briefing(name));
        }
    }

    let program = crate::claude_path::find_binary("claude");

    state
        .agent_tasks
        .lock()
        .map_err(|e| format!("{e:#}"))?
        .insert(agent_id.clone(), key.to_string());

    let agent_env: [(&str, String); 2] = [
        ("LAVIGIE_HOOK_PORT", state.hook_port.to_string()),
        ("LAVIGIE_AGENT_ID", agent_id.clone()),
    ];
    // Per-repo orchestrators spawn in the repo's own checkout (TASK-206) so Claude
    // auto-loads the repo's CLAUDE.md; the global concierge keeps its shared dir.
    let cwd = session_cwd(&state.concierge_root, repo_path.as_deref());
    if let Err(e) = std::fs::create_dir_all(&cwd) {
        let _ = state.agent_tasks.lock().map(|mut m| m.remove(&agent_id));
        let _ = state.mcp_tokens.lock().map(|mut m| m.remove(&token));
        return Err(format!("{e:#}"));
    }
    let session = match crate::agent::spawn_pty(
        &program,
        &args,
        Some(&cwd),
        80,
        24,
        &agent_env,
    ) {
        Ok(s) => s,
        Err(e) => {
            let _ = state.agent_tasks.lock().map(|mut m| m.remove(&agent_id));
            let _ = state.mcp_tokens.lock().map(|mut m| m.remove(&token));
            return Err(format!("{e:#}"));
        }
    };

    if let Err(e) = crate::agent::register_streaming_session(
        state,
        &agent_id,
        session,
        on_event,
        Some(token.clone()),
        kind,
        repo_id,
    ) {
        let _ = state.agent_tasks.lock().map(|mut m| m.remove(&agent_id));
        let _ = state.mcp_tokens.lock().map(|mut m| m.remove(&token));
        return Err(e);
    }

    if let Some(k) = marker_key {
        let store = state.store.lock().map_err(|e| format!("{e:#}"))?;
        let _ = store.set_app_setting(k, "1");
    }

    Ok(agent_id)
}

/// Shared create-or-resume core for the rootless concierge/orchestrator sessions.
///
/// Idempotent: if a session of `kind` scoped to `repo_id` is already running,
/// returns immediately without spawning. Otherwise it mints the session's MCP
/// token (`mint_token`, evaluated only on the create path so the idempotent bail
/// never evicts a live session's token), spawns a rootless `claude` in
/// `concierge_root` wired with hooks + the MCP config, maps the new agent to
/// `key`, and (when `marker_key` is `Some`) records that app-setting so future
/// starts resume via `--continue`. A `None` marker means the session never
/// resumes and never writes a marker. Fully synchronous (no `.await`); glue,
/// verified live.
fn ensure_session_inner(
    state: &AppState,
    key: &str,
    kind: crate::agent::SessionKind,
    repo_id: Option<String>,
    mint_token: impl FnOnce(&AppState) -> Result<String, String>,
    marker_key: Option<&str>,
) -> Result<(), String> {
    // Serialize the create path: concurrent POSTs must not both pass the
    // liveness check below and stack processes. This path is fully synchronous,
    // so this guard never crosses an `.await`.
    let _spawn_guard = state.concierge_spawn.lock().map_err(|e| format!("{e:#}"))?;

    // Idempotent: bail if a session of this kind + repo scope is already live.
    {
        let sessions = state.sessions.lock().map_err(|e| format!("{e:#}"))?;
        if sessions.values().any(|h| h.kind == kind && h.repo_id == repo_id) {
            return Ok(());
        }
    }

    // Headless callers (mobile / the existing `open_orchestrator` button) drain
    // to a sink — no frontend channel — and use the marker for cross-restart resume.
    spawn_rootless_session(state, key, kind, repo_id, mint_token, marker_key, None)?;
    Ok(())
}

/// Pure (TASK-180): given every `app_settings` key and the ids of the repos that
/// still exist, return the `orchestrator_started:{id}` markers whose repo is
/// gone (so a deleted repo never resurrects an orchestrator on restart). Only
/// keys carrying the `orchestrator_started:` prefix are candidates — any other
/// marker (e.g. `concierge_started`) is left untouched.
pub fn orphan_markers(markers: &[&str], live_repo_ids: &[&str]) -> Vec<String> {
    const PREFIX: &str = "orchestrator_started:";
    markers
        .iter()
        .filter_map(|m| {
            let id = m.strip_prefix(PREFIX)?;
            if live_repo_ids.contains(&id) {
                None
            } else {
                Some((*m).to_string())
            }
        })
        .collect()
}

/// Day-2 (TASK-180): revoke a repo's orchestrator when the repo is deleted. Stops
/// any live `orchestrator:{repo_id}` session (which drops its MCP token via
/// [`crate::agent::stop_session_inner`]) and deletes the
/// `orchestrator_started:{repo_id}` resume marker. Locks are captured then
/// dropped before the (synchronous) stop calls. Glue; verified live.
pub fn revoke_orchestrator_for_repo(state: &AppState, repo_id: &str) {
    let victims: Vec<String> = {
        let sessions = match state.sessions.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        sessions
            .iter()
            .filter(|(_, h)| {
                h.kind == crate::agent::SessionKind::Orchestrator
                    && h.repo_id.as_deref() == Some(repo_id)
            })
            .map(|(id, _)| id.clone())
            .collect()
    };
    for id in victims {
        let _ = crate::agent::stop_session_inner(state, &id);
    }
    if let Ok(store) = state.store.lock() {
        let _ = store.delete_app_setting(&format!("orchestrator_started:{repo_id}"));
    }
}

/// On startup (TASK-180): remove `orchestrator_started:*` markers whose repo no
/// longer exists — a repo deleted while the app was closed must not resurrect
/// its orchestrator. Glue over the pure [`orphan_markers`]; brief store lock,
/// no await.
pub fn prune_orphan_orchestrator_markers(state: &AppState) {
    let store = match state.store.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let live: Vec<String> = match store.list_repos() {
        Ok(repos) => repos.into_iter().map(|r| r.id).collect(),
        Err(_) => return,
    };
    let keys = match store.list_app_setting_keys() {
        Ok(k) => k,
        Err(_) => return,
    };
    let live_refs: Vec<&str> = live.iter().map(|s| s.as_str()).collect();
    let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
    for key in orphan_markers(&key_refs, &live_refs) {
        let _ = store.delete_app_setting(&key);
    }
}

/// If `task_id` addresses the concierge, bump the live concierge agent's
/// activity timestamp. No-op for ordinary task ids (only the concierge is
/// reaped). Called from the remote read/reply handlers.
pub fn note_concierge_activity(state: &AppState, task_id: &str) {
    if task_id != CONCIERGE_SENTINEL {
        return;
    }
    let agent_tasks = match state.agent_tasks.lock() {
        Ok(g) => g.clone(),
        Err(_) => return,
    };
    let live: std::collections::HashSet<String> = match state.sessions.lock() {
        Ok(g) => g.keys().cloned().collect(),
        Err(_) => return,
    };
    if let Some(agent_id) = crate::session::resolve_live_agent(&agent_tasks, &live, task_id) {
        crate::agent::bump_activity(state, &agent_id);
    }
}

/// One reaper pass: stop every concierge/orchestrator session idle past
/// `IDLE_TIMEOUT` — excluding any that has a live frontend channel attached
/// (a desktop-attached terminal; see [`is_reapable`]). Locks are
/// captured-then-dropped before the (synchronous) stop calls.
fn reap_once(state: &AppState) {
    let now = Instant::now();
    let victims = {
        let sessions = match state.sessions.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let snapshot: Vec<(String, bool, Instant)> = sessions
            .iter()
            .map(|(id, h)| (id.clone(), is_reapable(h.kind, h.has_frontend_channel), h.last_activity))
            .collect();
        collect_idle_rootless(&snapshot, now, IDLE_TIMEOUT)
    };
    for id in victims {
        let _ = crate::agent::stop_session_inner(state, &id);
    }
}

/// Spawn the background idle reaper: every 60s, stop concierge sessions idle past
/// `IDLE_TIMEOUT`. Runs for the app's lifetime on the Tauri runtime.
pub fn spawn_reaper(app: tauri::AppHandle) {
    use tauri::Manager as _;
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let state = app.state::<AppState>();
            reap_once(state.inner());
        }
    });
}

/// Pure: project `(id, kind_label, last_activity)` triples into
/// `RemoteSessionInfo`, computing idle seconds as of `now`.
pub fn remote_session_infos(
    sessions: &[(String, &'static str, Option<String>, Instant)],
    now: Instant,
) -> Vec<RemoteSessionInfo> {
    sessions
        .iter()
        .map(|(id, kind, repo_id, last)| RemoteSessionInfo {
            id: id.clone(),
            kind: (*kind).to_string(),
            repo_id: repo_id.clone(),
            idle_secs: now.duration_since(*last).as_secs(),
        })
        .collect()
}

/// Ensure a live per-repo orchestrator session for `repo_id` (create or resume).
/// Desktop sibling of the `POST /api/orchestrator/{repoId}` remote route
/// (TASK-180 Task D1). Idempotent; thin glue over [`ensure_orchestrator`].
#[tauri::command]
pub fn open_orchestrator(
    state: tauri::State<'_, AppState>,
    repo_id: String,
) -> Result<(), String> {
    ensure_orchestrator(state.inner(), &repo_id)
}

/// Ensure a live per-repo orchestrator session **bound to a frontend channel**
/// so the desktop can render + drive it (TASK-126). Any existing orchestrator for
/// this repo (a sink-drained mobile/`open_orchestrator` one, or a prior desktop
/// mount) is stopped, then a FRESH session is spawned bound to `on_event` (no
/// `--continue` — see the marker note below). Returns the new agent id, which the
/// frontend passes to `write_session` / `resize_session` / `stop_session`.
/// Thin glue over the pure `orchestrator_agents_for_repo` + `spawn_rootless_session`;
/// verified live (needs a real `Channel`).
#[tauri::command]
pub fn open_orchestrator_terminal(
    state: tauri::State<'_, AppState>,
    repo_id: String,
    on_event: Channel<PtyEvent>,
) -> Result<String, String> {
    let state = state.inner();
    let _spawn_guard = state.concierge_spawn.lock().map_err(|e| format!("{e:#}"))?;

    // Stop any live orchestrator for this repo so we can bind a fresh channel.
    let victims = {
        let sessions = state.sessions.lock().map_err(|e| format!("{e:#}"))?;
        let rows: Vec<(String, crate::agent::SessionKind, Option<String>)> = sessions
            .iter()
            .map(|(id, h)| (id.clone(), h.kind, h.repo_id.clone()))
            .collect();
        orchestrator_agents_for_repo(&rows, &repo_id)
    };
    for id in victims {
        let _ = crate::agent::stop_session_inner(state, &id);
    }

    // Desktop opens spawn a FRESH session (`None` marker → no `--continue`): the
    // resume marker is set the instant a session spawns, so a StrictMode throwaway
    // mount would poison it and the real mount's `--continue` would find no
    // conversation and exit. A mounted desktop terminal is reaper-exempt (TASK-126),
    // so it survives without needing marker-based resume. Cross-restart resume for
    // the desktop is a follow-up (attach a viewer channel to a persistent session).
    let key = format!("orchestrator:{repo_id}");
    spawn_rootless_session(
        state,
        &key,
        crate::agent::SessionKind::Orchestrator,
        Some(repo_id.clone()),
        |st| mint_orchestrator_token(st, &repo_id),
        None,
        Some(on_event),
    )
}

/// Pure: back-compat wire label for a rootless remote session's `kind`, or
/// `None` for kinds that are not surfaced as remote sessions (`Task`/`Shell`).
///
/// TASK-180 back-compat alias contract: the legacy global concierge keeps
/// emitting the wire `kind:"concierge"` value (the mobile client and the desktop
/// Remote-sessions UI key off it, disambiguated by `repo_id == None`), while the
/// per-repo orchestrator reports the new `"orchestrator"` label alongside its
/// `repo_id`. This mapping is the single source of truth for those wire values —
/// keep the strings stable (no storage/route renames, global constraint).
pub fn remote_kind_label(kind: crate::agent::SessionKind) -> Option<&'static str> {
    match kind {
        crate::agent::SessionKind::Concierge => Some("concierge"),
        crate::agent::SessionKind::Orchestrator => Some("orchestrator"),
        crate::agent::SessionKind::Task | crate::agent::SessionKind::Shell => None,
    }
}

/// List rootless/remote-spawned sessions (concierge + per-repo orchestrators) for
/// the desktop "Remote sessions" surface. The global concierge keeps the wire
/// `kind:"concierge"` value (disambiguated by `repo_id == None` in the UI);
/// orchestrators report `kind:"orchestrator"` with their `repo_id`. Brief
/// `sessions` lock, no await.
#[tauri::command]
pub fn list_remote_sessions(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RemoteSessionInfo>, String> {
    let now = Instant::now();
    let sessions = state.sessions.lock().map_err(|e| format!("{e:#}"))?;
    let rows: Vec<(String, &'static str, Option<String>, Instant)> = sessions
        .iter()
        .filter_map(|(id, h)| {
            remote_kind_label(h.kind)
                .map(|label| (id.clone(), label, h.repo_id.clone(), h.last_activity))
        })
        .collect();
    Ok(remote_session_infos(&rows, now))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::state::{AgentLaunchContext, McpToken};

    #[test]
    fn install_concierge_token_is_singleton_and_keeps_agent_tokens() {
        let mut map: std::collections::HashMap<String, McpToken> = std::collections::HashMap::new();
        map.insert(
            "agent-tok".into(),
            McpToken::Agent(AgentLaunchContext { task_id: "t1".into(), repo_id: "r1".into() }),
        );
        install_concierge_token(&mut map, "c1".into());
        install_concierge_token(&mut map, "c2".into()); // evicts c1

        assert!(map.contains_key("agent-tok"), "agent token must survive");
        assert!(!map.contains_key("c1"), "old concierge token must be evicted");
        assert!(matches!(map.get("c2"), Some(McpToken::Concierge)));
        let concierge_count = map.values().filter(|t| matches!(t, McpToken::Concierge)).count();
        assert_eq!(concierge_count, 1, "exactly one concierge token");
    }

    #[test]
    fn install_orchestrator_token_evicts_only_same_repo() {
        let mut map: HashMap<String, McpToken> = HashMap::new();
        install_orchestrator_token(&mut map, "tok-r1a".into(), "r1");
        install_orchestrator_token(&mut map, "tok-r2".into(), "r2");
        // re-open r1 → evicts tok-r1a, keeps r2 and any concierge
        map.insert("concierge-tok".into(), McpToken::Concierge);
        install_orchestrator_token(&mut map, "tok-r1b".into(), "r1");
        assert!(!map.contains_key("tok-r1a"), "old r1 orchestrator evicted");
        assert!(map.contains_key("tok-r1b"), "new r1 orchestrator present");
        assert!(map.contains_key("tok-r2"), "r2 orchestrator untouched");
        assert!(map.contains_key("concierge-tok"), "global concierge untouched");
    }

    #[test]
    fn is_idle_true_at_and_past_threshold() {
        let now = Instant::now();
        let past = now - Duration::from_secs(60);
        assert!(is_idle(past, now, Duration::from_secs(60)));
        assert!(is_idle(past, now, Duration::from_secs(30)));
    }

    #[test]
    fn is_idle_false_before_threshold() {
        let now = Instant::now();
        let recent = now - Duration::from_secs(10);
        assert!(!is_idle(recent, now, Duration::from_secs(60)));
    }

    #[test]
    fn collect_idle_concierge_picks_only_idle_concierge_sessions() {
        let now = Instant::now();
        let old = now - Duration::from_secs(20 * 60);
        let fresh = now - Duration::from_secs(60);
        let sessions = vec![
            ("c-old".to_string(), true, old),    // idle concierge → reap
            ("c-fresh".to_string(), true, fresh), // active concierge → keep
            ("task-old".to_string(), false, old), // idle but not concierge → keep
        ];
        let victims = collect_idle_rootless(&sessions, now, IDLE_TIMEOUT);
        assert_eq!(victims, vec!["c-old".to_string()]);
    }

    #[test]
    fn is_reapable_concierge_no_channel_is_reapable() {
        use crate::agent::SessionKind;
        assert!(is_reapable(SessionKind::Concierge, false));
    }

    #[test]
    fn is_reapable_orchestrator_no_channel_is_reapable() {
        use crate::agent::SessionKind;
        assert!(is_reapable(SessionKind::Orchestrator, false));
    }

    #[test]
    fn is_reapable_orchestrator_with_channel_is_not_reapable() {
        use crate::agent::SessionKind;
        // Channel-bound (desktop-attached) orchestrator: exempt from the reaper.
        assert!(!is_reapable(SessionKind::Orchestrator, true));
    }

    #[test]
    fn is_reapable_concierge_with_channel_is_not_reapable() {
        use crate::agent::SessionKind;
        assert!(!is_reapable(SessionKind::Concierge, true));
    }

    #[test]
    fn is_reapable_task_and_shell_never_reapable_regardless_of_channel() {
        use crate::agent::SessionKind;
        assert!(!is_reapable(SessionKind::Task, false));
        assert!(!is_reapable(SessionKind::Task, true));
        assert!(!is_reapable(SessionKind::Shell, false));
        assert!(!is_reapable(SessionKind::Shell, true));
    }

    #[test]
    fn reaps_idle_orchestrators_and_concierge_not_tasks() {
        let now = Instant::now();
        let old = now - Duration::from_secs(20 * 60);
        let sessions = vec![
            ("concierge".to_string(), true, old),       // reapable rootless, idle
            ("orchestrator:r1".to_string(), true, old), // reapable rootless, idle
            ("task-agent".to_string(), false, old),     // task — never reaped
        ];
        let victims = collect_idle_rootless(&sessions, now, Duration::from_secs(15 * 60));
        assert!(victims.contains(&"concierge".to_string()));
        assert!(victims.contains(&"orchestrator:r1".to_string()));
        assert!(!victims.contains(&"task-agent".to_string()));
    }

    #[test]
    fn prunes_markers_for_missing_repos() {
        let live = ["r1", "r3"];
        let markers = ["orchestrator_started:r1", "orchestrator_started:r2", "concierge_started"];
        let to_delete = orphan_markers(&markers, &live);
        assert_eq!(to_delete, vec!["orchestrator_started:r2".to_string()]);
    }

    #[test]
    fn remote_session_infos_maps_idle_secs_and_camelcase() {
        let now = Instant::now();
        let rows = vec![("c1".to_string(), "concierge", None, now - Duration::from_secs(125))];
        let infos = remote_session_infos(&rows, now);
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].id, "c1");
        assert_eq!(infos[0].kind, "concierge");
        assert_eq!(infos[0].repo_id, None);
        assert_eq!(infos[0].idle_secs, 125);
        let v = serde_json::to_value(&infos[0]).unwrap();
        assert!(v.get("idleSecs").is_some(), "must serialize camelCase idleSecs");
    }

    #[test]
    fn remote_kind_label_keeps_backcompat_wire_values() {
        use crate::agent::SessionKind;
        // TASK-180 back-compat: the legacy global concierge MUST keep emitting the
        // wire `kind:"concierge"` value (the mobile client + Remote-sessions UI key
        // off it; disambiguated by `repoId == None`). Orchestrators report the new
        // `"orchestrator"` label alongside their `repoId`. Task/Shell are not
        // rootless remote sessions and are filtered out (None).
        assert_eq!(remote_kind_label(SessionKind::Concierge), Some("concierge"));
        assert_eq!(remote_kind_label(SessionKind::Orchestrator), Some("orchestrator"));
        assert_eq!(remote_kind_label(SessionKind::Task), None);
        assert_eq!(remote_kind_label(SessionKind::Shell), None);
    }

    #[test]
    fn orchestrator_agents_for_repo_selects_only_that_repos_orchestrators() {
        use crate::agent::SessionKind;
        let rows = vec![
            ("a1".to_string(), SessionKind::Orchestrator, Some("r1".to_string())),
            ("a2".to_string(), SessionKind::Orchestrator, Some("r2".to_string())),
            ("a3".to_string(), SessionKind::Concierge, None),
            ("a4".to_string(), SessionKind::Task, Some("r1".to_string())), // task in r1 — not an orchestrator
        ];
        assert_eq!(orchestrator_agents_for_repo(&rows, "r1"), vec!["a1".to_string()]);
        assert_eq!(orchestrator_agents_for_repo(&rows, "r2"), vec!["a2".to_string()]);
        assert!(orchestrator_agents_for_repo(&rows, "r3").is_empty());
    }

    #[test]
    fn orchestrator_briefing_names_repo_and_states_manager_role() {
        let b = orchestrator_briefing("alpha");
        assert!(b.contains("alpha"), "briefing must name the repo it manages");
        assert!(
            b.to_lowercase().contains("orchestrator") && b.to_lowercase().contains("manager"),
            "briefing must establish the orchestrator/manager role"
        );
    }

    #[test]
    fn orchestrator_briefing_states_dispatch_not_edit_posture() {
        let b = orchestrator_briefing("alpha");
        // D2 prose-only edit-safety: it must tell the orchestrator not to edit the
        // main checkout and point at dispatch as the alternative.
        assert!(b.contains("Do NOT edit"), "briefing must state the do-not-edit posture");
        assert!(
            b.contains("start_task"),
            "briefing must point at dispatch (start_task) as the alternative to editing"
        );
    }

    #[test]
    fn orchestrator_briefing_enumerates_every_repo_scoped_power() {
        let b = orchestrator_briefing("beta");
        // Pin the briefing's tool rundown to the authz registry (TASK-180's source of
        // truth): a newly-registered orchestrator tool must be surfaced in the
        // briefing too, or this fails — no silent capability drift.
        for entry in crate::mcp::authz::registry() {
            let tool = entry.0;
            assert!(b.contains(tool), "briefing must enumerate the `{tool}` power");
        }
        // list_repos is the one ungated tool (absent from the registry) but still a
        // power the orchestrator holds.
        assert!(b.contains("list_repos"), "briefing must mention list_repos");
    }

    #[test]
    fn session_cwd_is_concierge_root_for_global_and_repo_path_for_orchestrator() {
        let root = Path::new("/data/concierge");
        // The global concierge (no repo) keeps the shared root dir.
        assert_eq!(session_cwd(root, None), PathBuf::from("/data/concierge"));
        // A per-repo orchestrator runs in the repo's own checkout, verbatim —
        // so `claude` auto-loads the repo's CLAUDE.md and can read the tree
        // (TASK-206). No neutral scratch dir.
        assert_eq!(
            session_cwd(root, Some(Path::new("/repos/alpha"))),
            PathBuf::from("/repos/alpha")
        );
        assert_eq!(
            session_cwd(root, Some(Path::new("/repos/beta"))),
            PathBuf::from("/repos/beta")
        );
    }

    #[test]
    fn remote_session_infos_carries_repo_id_for_orchestrator() {
        let now = Instant::now();
        let rows = vec![(
            "orchestrator:r1".to_string(),
            "orchestrator",
            Some("r1".to_string()),
            now - Duration::from_secs(3),
        )];
        let infos = remote_session_infos(&rows, now);
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].kind, "orchestrator");
        assert_eq!(infos[0].repo_id.as_deref(), Some("r1"));
        let v = serde_json::to_value(&infos[0]).unwrap();
        assert_eq!(v.get("repoId").and_then(|x| x.as_str()), Some("r1"),
            "must serialize camelCase repoId");
    }
}
