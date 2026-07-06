//! Concierge session primitive (AC2-112): a worktree-less, singleton Claude
//! session for the mobile concierge. This module holds the pure policy core
//! (sentinel id, idle predicate, idle-victim collection) plus glue
//! (token minting, spawn, activity tracking, reaper, desktop listing) added in
//! later tasks.
//!
//! Pure functions are unit-tested here; the glue (`ensure_concierge`,
//! `spawn_reaper`, the Tauri command) is verified live, per project convention.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::state::{AppState, McpToken};

/// One rootless/remote-spawned session for the desktop "Remote sessions"
/// surface. camelCase over IPC.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSessionInfo {
    pub id: String,
    /// Lowercase session kind label, e.g. "concierge".
    pub kind: String,
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

/// Pure idle predicate: has `last_activity` aged to or past `threshold` by `now`?
pub fn is_idle(last_activity: Instant, now: Instant, threshold: Duration) -> bool {
    now.duration_since(last_activity) >= threshold
}

/// Pure: from `(id, is_concierge, last_activity)` triples, collect the ids of
/// concierge sessions idle past `threshold` as of `now`. Non-concierge sessions
/// are never reaped.
pub fn collect_idle_concierge(
    sessions: &[(String, bool, Instant)],
    now: Instant,
    threshold: Duration,
) -> Vec<String> {
    sessions
        .iter()
        .filter(|(_, is_concierge, last)| *is_concierge && is_idle(*last, now, threshold))
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
    // Serialize the create path: concurrent POSTs must not both pass the
    // liveness check below and stack processes. ensure_concierge is fully
    // synchronous, so this guard never crosses an `.await`.
    let _spawn_guard = state.concierge_spawn.lock().map_err(|e| format!("{e:#}"))?;

    // 1. Idempotent: bail if a concierge session is already live.
    {
        let sessions = state.sessions.lock().map_err(|e| format!("{e:#}"))?;
        if sessions.values().any(|h| h.kind == crate::agent::SessionKind::Concierge) {
            return Ok(());
        }
    }

    // 2. Resume vs fresh, from the persisted marker (survives app restart).
    let resume = {
        let store = state.store.lock().map_err(|e| format!("{e:#}"))?;
        store
            .get_app_setting("concierge_started")
            .map_err(|e| format!("{e:#}"))?
            .is_some()
    };

    // 3. Mint the broad-scope token + build the inline configs.
    let token = mint_concierge_token(state)?;
    let mcp_config = crate::agent::build_mcp_config(state.mcp_port, &token);

    let agent_id = uuid::Uuid::new_v4().to_string();
    let hook_settings = crate::agent::build_hook_settings(state.hook_port, &agent_id);

    // 4. claude argv: [--continue?] --mcp-config <cfg> --settings <hooks>.
    //    `--mcp-config` is variadic, so `--settings` must follow it; no
    //    positional prompt is appended for the concierge.
    let mut args: Vec<String> = Vec::new();
    if resume {
        args.push("--continue".to_string());
    }
    args.push("--mcp-config".to_string());
    args.push(mcp_config);
    args.push("--settings".to_string());
    args.push(hook_settings);

    let program = crate::claude_path::find_binary("claude");

    // 5. Map agent → sentinel BEFORE spawn so the first hook resolves the
    //    transcript path under CONCIERGE_SENTINEL.
    state
        .agent_tasks
        .lock()
        .map_err(|e| format!("{e:#}"))?
        .insert(agent_id.clone(), CONCIERGE_SENTINEL.to_string());

    // 6. Spawn in the neutral cwd; hand the agent its HookBridge coords (AC2-40
    //    parity); register with NO frontend channel (drained to a sink).
    let agent_env: [(&str, String); 2] = [
        ("LAVIGIE_HOOK_PORT", state.hook_port.to_string()),
        ("LAVIGIE_AGENT_ID", agent_id.clone()),
    ];
    let session = match crate::agent::spawn_pty(
        &program,
        &args,
        Some(Path::new(&state.concierge_root)),
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
        None,
        Some(token.clone()),
        crate::agent::SessionKind::Concierge,
    ) {
        let _ = state.agent_tasks.lock().map(|mut m| m.remove(&agent_id));
        let _ = state.mcp_tokens.lock().map(|mut m| m.remove(&token));
        return Err(e);
    }

    // 7. Record the marker so the next start resumes.
    {
        let store = state.store.lock().map_err(|e| format!("{e:#}"))?;
        let _ = store.set_app_setting("concierge_started", "1");
    }

    Ok(())
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

/// One reaper pass: stop every concierge session idle past `IDLE_TIMEOUT`. Locks
/// are captured-then-dropped before the (synchronous) stop calls.
fn reap_once(state: &AppState) {
    let now = Instant::now();
    let victims = {
        let sessions = match state.sessions.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let snapshot: Vec<(String, bool, Instant)> = sessions
            .iter()
            .map(|(id, h)| (id.clone(), h.kind == crate::agent::SessionKind::Concierge, h.last_activity))
            .collect();
        collect_idle_concierge(&snapshot, now, IDLE_TIMEOUT)
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
    sessions: &[(String, &'static str, Instant)],
    now: Instant,
) -> Vec<RemoteSessionInfo> {
    sessions
        .iter()
        .map(|(id, kind, last)| RemoteSessionInfo {
            id: id.clone(),
            kind: (*kind).to_string(),
            idle_secs: now.duration_since(*last).as_secs(),
        })
        .collect()
}

/// List rootless/remote-spawned sessions (currently: concierge) for the desktop
/// "Remote sessions" surface. Brief `sessions` lock, no await.
#[tauri::command]
pub fn list_remote_sessions(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RemoteSessionInfo>, String> {
    let now = Instant::now();
    let sessions = state.sessions.lock().map_err(|e| format!("{e:#}"))?;
    let rows: Vec<(String, &'static str, Instant)> = sessions
        .iter()
        .filter(|(_, h)| h.kind == crate::agent::SessionKind::Concierge)
        .map(|(id, h)| (id.clone(), "concierge", h.last_activity))
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
        let victims = collect_idle_concierge(&sessions, now, IDLE_TIMEOUT);
        assert_eq!(victims, vec!["c-old".to_string()]);
    }

    #[test]
    fn remote_session_infos_maps_idle_secs_and_camelcase() {
        let now = Instant::now();
        let rows = vec![("c1".to_string(), "concierge", now - Duration::from_secs(125))];
        let infos = remote_session_infos(&rows, now);
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].id, "c1");
        assert_eq!(infos[0].kind, "concierge");
        assert_eq!(infos[0].idle_secs, 125);
        let v = serde_json::to_value(&infos[0]).unwrap();
        assert!(v.get("idleSecs").is_some(), "must serialize camelCase idleSecs");
    }
}
