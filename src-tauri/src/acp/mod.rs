//! ACP (Agent Client Protocol, agentclientprotocol.com) backend engine: a
//! second agent-execution backend living side-by-side with the PTY backend
//! (`agent/`). See `docs/superpowers/specs/2026-07-17-acp-backend-engine-design.md`.
//!
//! The pure core lives in `events` (the frontend-facing event contract) and
//! `translate` (the pure `SessionUpdate` -> `AcpEvent` / ACP-semantic ->
//! `StatusEvent` translation). This module is the connection driver:
//! spawning the agent process, running the ACP connection as one `Send`
//! future (`tauri::async_runtime::spawn`, no dedicated thread, no
//! `LocalSet`), and the `#[tauri::command]` glue that registers an ACP
//! session into the same `state.sessions`/`state.agent_tasks` id space the
//! PTY path uses — so teardown, MCP `finish_task`, and status persistence all
//! work unchanged (`crate::agent::SessionBackend::Acp`).
//!
//! Scope: the new-session happy path end to end; capability-gated MCP-server
//! injection into `session/new`/`load`/`resume`
//! (`lavigie_mcp_server`/`should_inject_http`); persistence of `acp_session_id`
//! on session start (`store::set_task_acp_session_id`); the raw JSONL event log
//! (`acp::log`); and **resume** — consuming the persisted id via
//! `session/load` ▸ `session/resume` ▸ event-log replay ▸ fresh on relaunch,
//! chosen by `select_resume_source` from the agent's advertised capabilities.
//!
//! Like `agent/mod.rs`, the process-spawn/live-connection glue here is thin
//! glue over a running Tauri app + a live agent subprocess and is not
//! unit-tested (see the project's testing convention); it is exercised by the
//! live smoke test (`examples/acp_smoke.rs`). Only the pure helpers below
//! (`mode_for_auto_approve`, `lavigie_mcp_server`, `should_inject_http`,
//! `select_resume_source`) are unit-tested directly.

pub mod events;
pub mod log;
pub mod translate;
pub mod usage;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    CancelNotification, HttpHeader, Implementation, InitializeRequest, InitializeResponse,
    LoadSessionRequest, McpServer, McpServerHttp, NewSessionRequest,
    PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, ResumeSessionRequest, SelectedPermissionOutcome, SessionId,
    SessionNotification, SetSessionModeRequest,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::util::MatchDispatch;
use agent_client_protocol::{
    ActiveSession, Agent, ByteStreams, Client, ConnectionTo, SessionMessage,
};
use futures::channel::{mpsc, oneshot};
use futures::{FutureExt as _, StreamExt as _};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::agent::spec::AgentSpec;
use crate::agent::{SessionBackend, SessionHandle, SessionKind};
use crate::hooks::{StatusSink, TauriSink};
use crate::state::AppState;

pub use events::AcpEvent;
use events::{MessageRole, PermissionOptionEvent};
use translate::{current_model_of, map_status, stop_reason_label, translate_update, AcpPhase};

/// A command sent from a Tauri command handler into the driver's `select!`
/// loop (`spawn_acp_session`'s connection task) via
/// `SessionBackend::Acp::cmd_tx`.
#[derive(Debug, Clone)]
pub enum DriverCommand {
    /// Send a new user prompt on the session (`acp_prompt`).
    Prompt(String),
    /// Cancel the in-flight turn (`acp_cancel`).
    Cancel,
    /// Switch the session's active mode (`acp_set_mode`).
    SetMode(String),
}

/// Build the `initialize` request's `InitializeRequest`, always carrying
/// `client_info` — the ACP schema documents this as becoming a *required*
/// field in a future protocol version, and `vibe-acp` (Mistral) rejects a
/// prompt turn with a `422` when it's absent (empty `client_name`/
/// `client_version` forwarded into Mistral's own API metadata). Set
/// unconditionally rather than gated to Mistral: it's protocol-correct for
/// every agent, `claude-acp` included. Note this is unrelated to the
/// `Client::builder().name(...)` call elsewhere in this module, which is
/// debug-log-only and never crosses the wire.
fn initialize_request() -> InitializeRequest {
    InitializeRequest::new(ProtocolVersion::V1)
        .client_info(Implementation::new("la-vigie", env!("CARGO_PKG_VERSION")))
}

/// Which mechanism `drive_connection` uses to re-establish a stopped session
/// when a caller asks to resume. Chosen purely from the agent's
/// advertised capabilities by `select_resume_source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeSource {
    /// `session/load`: the agent replays its full native history as
    /// `session/update` notifications during the load. The real path for both
    /// curated engines (`claude-acp`/`mistral-acp` advertise `loadSession`).
    Load,
    /// `session/resume`: lighter reconnect, no replay — the agent re-establishes
    /// state itself. Fallback for agents that advertise `sessionCapabilities
    /// .resume` but not `loadSession`.
    Resume,
    /// Neither native path is available, but our app-side raw JSONL log exists:
    /// replay it to restore the **visual** timeline, then open a fresh
    /// `session/new` (the agent has no memory — honest degradation).
    EventLog,
    /// Nothing to restore from: a plain fresh `session/new`.
    Fresh,
}

/// Pick the resume mechanism from the agent's advertised capabilities, in
/// strict preference order load ▸ resume ▸ event-log ▸ fresh (design doc,
/// "Persistence & resume"). `load_session` is
/// `initialize`'s `agentCapabilities.loadSession`; `resume` is
/// `agentCapabilities.sessionCapabilities.resume`; `has_event_log` is whether a
/// non-empty app-side replay log was found for the task. Degrades gracefully:
/// an agent that advertises nothing and has no log falls to `Fresh` rather than
/// hanging on a replay that will never come (the Cursor/Copilot-CLI classes).
pub fn select_resume_source(load_session: bool, resume: bool, has_event_log: bool) -> ResumeSource {
    if load_session {
        ResumeSource::Load
    } else if resume {
        ResumeSource::Resume
    } else if has_event_log {
        ResumeSource::EventLog
    } else {
        ResumeSource::Fresh
    }
}

/// Everything `drive_connection` needs to resume a stopped session: the ACP
/// session id persisted on the task (`tasks.acp_session_id`), a cheap flag for
/// whether an app-side event log exists (drives `select_resume_source` without
/// reading the file), and a thunk that reads + parses that log into
/// `replay`-marked events. The thunk is called **only** on the `EventLog` tier,
/// so the common `Load` path (both curated engines) never pays the full
/// read + JSON-decode of the log.
pub struct ResumeContext {
    pub stored_session_id: String,
    pub has_event_log: bool,
    pub load_event_log: Box<dyn FnOnce() -> Vec<AcpEvent> + Send>,
}

/// Map the effective auto-approve setting to the ACP session mode id to set
/// right after session creation, per engine: `claude-acp` uses
/// `bypassPermissions`, `mistral-acp` uses `auto-approve`. `false` (or an
/// unrecognized engine name — e.g. a future generic ACP engine) yields
/// `None`, leaving the session on its default mode; the
/// `session/request_permission` plumbing stays wired regardless, so turning
/// auto-approve off just means more permission prompts, never a broken
/// session.
pub fn mode_for_auto_approve(spec_name: &str, auto_approve: bool) -> Option<&'static str> {
    if !auto_approve {
        return None;
    }
    match spec_name {
        "claude-acp" => Some("bypassPermissions"),
        "mistral-acp" => Some("auto-approve"),
        _ => None,
    }
}

/// Build the ACP `McpServer::Http` entry for La Vigie's own loopback MCP
/// server — the exact loopback URL + bearer header shape `agent::build_mcp_config`
/// hands the PTY-backed Claude CLI via `--mcp-config` (`agent/mod.rs`'s
/// `build_mcp_config`), reproduced as an ACP-native entry rather than JSON.
pub fn lavigie_mcp_server(mcp_port: u16, token: &str) -> McpServer {
    McpServer::Http(
        McpServerHttp::new("lavigie", crate::agent::mcp_loopback_url(mcp_port)).headers(vec![
            HttpHeader::new("Authorization", crate::agent::mcp_bearer_value(token)),
        ]),
    )
}

/// Whether the agent's `initialize` response advertises HTTP MCP-server
/// support (`agentCapabilities.mcpCapabilities.http`) — the gate for sending
/// La Vigie's `McpServer::Http` entry in `session/new`. `claude-agent-acp`
/// advertises it; `vibe-acp` (Mistral) does not (stdio-only, v1 limitation —
/// see the design doc's MCP-injection section).
pub fn should_inject_http(resp: &InitializeResponse) -> bool {
    resp.agent_capabilities.mcp_capabilities.http
}

/// Label a `PermissionOptionKind` for the frontend event contract. Mirrors
/// `translate::tool_kind_label`'s pattern: the schema enum is
/// `#[non_exhaustive]`, so a future variant falls back to `"other"` rather
/// than failing to compile.
fn permission_option_kind_label(kind: &PermissionOptionKind) -> &'static str {
    match kind {
        PermissionOptionKind::AllowOnce => "allow_once",
        PermissionOptionKind::AllowAlways => "allow_always",
        PermissionOptionKind::RejectOnce => "reject_once",
        PermissionOptionKind::RejectAlways => "reject_always",
        _ => "other",
    }
}

/// Forward one update read during a `session/load`/`session/resume` replay
/// window to the frontend, translating it with `replay = true` so the reducer
/// restores it as history without flipping "turn running" affordances. A
/// `StopReason` (not expected during replay) is ignored; a read error
/// propagates.
async fn forward_replay_update(
    update: Result<SessionMessage, agent_client_protocol::Error>,
    on_event: &(dyn Fn(AcpEvent) + Sync),
) -> Result<(), agent_client_protocol::Error> {
    match update? {
        SessionMessage::SessionMessage(dispatch) => {
            MatchDispatch::new(dispatch)
                .if_notification(async move |notif: SessionNotification| {
                    for ev in translate_update(&notif.update, true) {
                        on_event(ev);
                    }
                    Ok(())
                })
                .await
                .otherwise_ignore()?;
            Ok(())
        }
        // `SessionMessage` is `#[non_exhaustive]`; a `StopReason` during replay
        // is unexpected and carries no history — ignore it.
        _ => Ok(()),
    }
}

/// Attempt to re-establish the stored session via `session/load` (`source ==
/// Load`) or `session/resume`, forwarding the history the agent replays before
/// its response to `on_replay` (marked `replay = true`). The returned session
/// carries the modes/config options of the load/resume response.
///
/// Returns `Err` when the agent can't load the id (GC'd/expired/unknown to the
/// freshly-spawned process) so the caller can fall back to a fresh session
/// rather than stranding the tab — `select_resume_source` only knows the
/// advertised capability, not whether *this* id is still loadable. The crate
/// removes the provisional session route on that failure. A malformed replayed
/// update is non-fatal so it never discards an otherwise-loaded session: the
/// drain stops there, and any replay still queued behind it reaches the live
/// loop and is emitted as live (`replay = false`).
async fn try_reconnect(
    cx: &ConnectionTo<Agent>,
    source: ResumeSource,
    stored_session_id: &str,
    cwd: &Path,
    mcp_servers: &[McpServer],
    on_replay: &(dyn Fn(AcpEvent) + Sync),
) -> Result<ActiveSession<'static, Agent>, agent_client_protocol::Error> {
    let session_id = SessionId::new(stored_session_id.to_string());
    let servers = mcp_servers.to_vec();
    // The restore builders install the route for the known id before publishing
    // the request, so everything the agent replays while handling it is already
    // buffered on `active`'s update channel when the response resolves.
    let mut active = match source {
        ResumeSource::Load => cx
            .load_session_from(LoadSessionRequest::new(session_id, cwd).mcp_servers(servers))
            .block_task()
            .start_session()
            .await?
            .into_session(),
        _ => cx
            .resume_session_from(ResumeSessionRequest::new(session_id, cwd).mcp_servers(servers))
            .block_task()
            .start_session()
            .await?
            .into_session(),
    };

    // Forward the buffered replay (`session/load` delivers the whole history;
    // `session/resume` typically none) before the caller emits SessionStarted.
    // This drains only updates already enqueued, so it assumes the agent orders
    // its whole replay strictly before the load/resume response
    // (claude-agent-acp does). A late in-transit chunk would fall through to the
    // live loop marked live — a benign timeline glitch (the status pill is
    // StatusEvent-driven, untouched by this path).
    while let Some(update) = active.read_update().now_or_never() {
        if let Err(e) = forward_replay_update(update, on_replay).await {
            eprintln!("[acp] error during resume replay (continuing): {e:#}");
            break;
        }
    }

    Ok(active)
}

/// Drive one ACP session end to end, from the `initialize`/`session/new`
/// handshake through the pull-based `select!` loop, until the connection
/// closes or `cmd_rx` closes (the session was stopped). This is the actual
/// protocol core — generic over the event/status sinks (`&dyn Fn`, cheap to
/// re-borrow across `.await` points and into nested closures) so both the
/// live Tauri driver (`spawn_acp_session`'s `connect_with` closure, which
/// adapts a `tauri::ipc::Channel` + `StatusSink` into these callbacks) and the
/// live smoke test (`examples/acp_smoke.rs`, which has no Tauri app to
/// construct) exercise this exact code path — no second, hand-rolled client.
///
/// `cwd` is the session's working directory (the ACP `session/new.cwd`, not
/// necessarily the same as the process's OS cwd, though callers set both to
/// the same worktree path). `auto_approve_mode`, when `Some`, is the mode id
/// set via `session/set_mode` right after session creation (see
/// `mode_for_auto_approve`). `initial_prompt`, when non-blank, is sent as the
/// session's first prompt — but only for a *fresh* session; a reconnect
/// (`resume`) continues an existing conversation and never re-sends one.
/// `mcp_port`/`mcp_token`: when `mcp_token` is `Some` AND the agent's
/// `initialize` response advertises HTTP MCP support (`should_inject_http`),
/// La Vigie's own MCP server (`lavigie_mcp_server`) is injected into
/// `session/new`/`load`/`resume`; otherwise the session gets no MCP servers
/// and, if a token existed but HTTP wasn't advertised, a note is logged (the
/// `mistral-acp`/stdio-only case — a documented v1 limitation).
///
/// `resume`, when `Some`, asks to re-establish the task's stored
/// session instead of opening a fresh one: `select_resume_source` picks
/// `session/load` ▸ `session/resume` ▸ event-log visual replay ▸ fresh from
/// the agent's advertised capabilities, and a runtime reconnect failure falls
/// back to fresh — degrading gracefully at every step.
///
/// `on_replay` is a second event sink for *restored history* (native replay +
/// event-log tier). It must forward to the frontend but **not** append to the
/// app-side log — re-logging replayed events would grow the per-task log
/// unbounded and duplicate the restored timeline on the next resume. Live
/// events go through `on_event` (frontend + log).
#[allow(clippy::too_many_arguments)]
pub async fn drive_connection(
    cx: ConnectionTo<Agent>,
    cwd: std::path::PathBuf,
    initial_prompt: Option<String>,
    auto_approve_mode: Option<String>,
    mcp_port: u16,
    mcp_token: Option<String>,
    resume: Option<ResumeContext>,
    cmd_rx: mpsc::UnboundedReceiver<DriverCommand>,
    on_event: &(dyn Fn(AcpEvent) + Sync),
    on_replay: &(dyn Fn(AcpEvent) + Sync),
    on_status: &(dyn Fn(AcpPhase) + Sync),
) -> Result<(), agent_client_protocol::Error> {
    let init_resp = cx.send_request(initialize_request()).block_task().await?;

    // La Vigie's own MCP server rides session/new|load|resume only when a token
    // exists AND the agent advertises HTTP MCP; otherwise it's omitted (and,
    // when a token existed, noted once — the mistral-acp/stdio-only case).
    let mcp_servers: Vec<McpServer> = match &mcp_token {
        Some(token) if should_inject_http(&init_resp) => vec![lavigie_mcp_server(mcp_port, token)],
        _ => Vec::new(),
    };
    if mcp_token.is_some() && mcp_servers.is_empty() {
        eprintln!(
            "[acp] agent does not advertise http MCP; La Vigie tools unavailable (stdio bridge is a follow-up)"
        );
    }

    // Decide how to establish the session: fresh, or — when the caller asked to
    // resume — the best reconnect path the agent's capabilities allow.
    let plan = match &resume {
        None => ResumeSource::Fresh,
        Some(ctx) => select_resume_source(
            init_resp.agent_capabilities.load_session,
            init_resp.agent_capabilities.session_capabilities.resume.is_some(),
            ctx.has_event_log,
        ),
    };

    // Try the chosen reconnect first. On ANY failure (a stale/GC'd/unknown
    // stored id the freshly-spawned agent can't load) fall through to a fresh
    // session rather than stranding the tab on "starting" — the runtime
    // complement to `select_resume_source`'s capability-only choice.
    let reconnected = match plan {
        ResumeSource::Load | ResumeSource::Resume => {
            let stored_id = resume
                .as_ref()
                .expect("resume context is present for a reconnect plan")
                .stored_session_id
                .clone();
            match try_reconnect(&cx, plan, &stored_id, &cwd, &mcp_servers, on_replay).await {
                Ok(active) => Some(active),
                Err(e) => {
                    eprintln!(
                        "[acp] session/{} for {stored_id} failed ({e:#}); starting a fresh session instead",
                        if plan == ResumeSource::Load { "load" } else { "resume" }
                    );
                    on_event(AcpEvent::Error {
                        message: format!("could not resume the previous session ({e:#}); started a fresh one"),
                    });
                    None
                }
            }
        }
        ResumeSource::EventLog | ResumeSource::Fresh => None,
    };

    match reconnected {
        Some(active) => {
            run_established(
                &cx,
                active,
                true,
                Vec::new(),
                initial_prompt,
                auto_approve_mode,
                cmd_rx,
                on_event,
                on_replay,
                on_status,
            )
            .await
        }
        None => {
            if plan == ResumeSource::Fresh && resume.is_some() {
                eprintln!(
                    "[acp] resume requested but agent advertises no load/resume and no event log; starting fresh"
                );
            }
            // EventLog tier: read + parse the app-side log now (via the thunk —
            // never called on the Load path, so that common case pays nothing).
            let pending_replay = match plan {
                ResumeSource::EventLog => resume.map(|c| (c.load_event_log)()).unwrap_or_default(),
                _ => Vec::new(),
            };
            // `run_until` sends `session/new` on this task, so an agent's
            // rejection propagates verbatim. `start_session` would send it from
            // a spawned task and surface only a generic internal error.
            let session_cx = cx.clone();
            cx.build_session_from(NewSessionRequest::new(cwd).mcp_servers(mcp_servers))
                .block_task()
                .run_until(async move |active| {
                    run_established(
                        &session_cx,
                        active,
                        false,
                        pending_replay,
                        initial_prompt,
                        auto_approve_mode,
                        cmd_rx,
                        on_event,
                        on_replay,
                        on_status,
                    )
                    .await
                })
                .await
        }
    }
}

/// The `SessionStarted` payload of an established session: its modes as JSON,
/// its config options as JSON (the event's `models` field), and the current
/// model id taken from those config options.
fn session_start_payload(
    active: &ActiveSession<'_, Agent>,
) -> (Option<serde_json::Value>, Option<serde_json::Value>, Option<String>) {
    (
        active.modes().and_then(|m| serde_json::to_value(m).ok()),
        active.config_options().and_then(|c| serde_json::to_value(c).ok()),
        active.config_options().and_then(current_model_of),
    )
}

/// Run an established session to completion: announce it (`SessionStarted`,
/// plus `ModelSelected` so usage attribution has a model from the first
/// `UsageUpdate`), restore and settle any history, apply the auto-approve mode,
/// send the kickoff prompt, then pump session updates and driver commands until
/// the session or the command channel closes.
///
/// `is_reconnect` marks a native `session/load`|`resume` of the stored id: its
/// replayed history was already forwarded by `try_reconnect`, and it continues
/// an existing conversation so no kickoff prompt is sent. `pending_replay` is
/// the event-log tier's restored timeline, emitted after `SessionStarted`.
#[allow(clippy::too_many_arguments)]
async fn run_established(
    cx: &ConnectionTo<Agent>,
    mut active: ActiveSession<'_, Agent>,
    is_reconnect: bool,
    pending_replay: Vec<AcpEvent>,
    initial_prompt: Option<String>,
    auto_approve_mode: Option<String>,
    mut cmd_rx: mpsc::UnboundedReceiver<DriverCommand>,
    on_event: &(dyn Fn(AcpEvent) + Sync),
    on_replay: &(dyn Fn(AcpEvent) + Sync),
    on_status: &(dyn Fn(AcpPhase) + Sync),
) -> Result<(), agent_client_protocol::Error> {
    let session_id_str = active.session_id().0.to_string();
    let (modes_json, models_json, initial_model) = session_start_payload(&active);

    on_event(AcpEvent::SessionStarted {
        session_id: session_id_str,
        modes: modes_json,
        models: models_json,
    });
    if let Some(model_id) = initial_model {
        on_event(AcpEvent::ModelSelected { model_id });
    }
    on_status(AcpPhase::SessionCreated);

    // Restore the visual timeline from the app-side log (EventLog tier only),
    // after SessionStarted so it lands on the fresh session's empty timeline.
    // Via `on_replay` so these restored events are NOT re-appended to the log.
    let restored_history = is_reconnect || !pending_replay.is_empty();
    for ev in pending_replay {
        on_replay(ev);
    }

    // A restored session is idle, not mid-turn — settle it with a synthetic
    // `TurnEnded`. Replayed `toolCall`/`plan` events set the reducer's
    // `turnActive` true and carry no `replay` flag for it to guard on, and
    // native `session/load` replays no `StopReason`; without this, a resumed
    // tool-using conversation would strand `turnActive` true and show a
    // spurious "Cancel turn" affordance. Via `on_replay` (frontend only, not
    // logged); the reducer ignores the stop reason for `turnEnded`.
    if restored_history {
        on_replay(AcpEvent::TurnEnded { stop_reason: "resumed".to_string() });
    }

    if let Some(mode_id) = auto_approve_mode {
        if let Err(e) = cx
            .send_request(SetSessionModeRequest::new(active.session_id().clone(), mode_id))
            .block_task()
            .await
        {
            // Non-fatal, but surface it: the session stays on its default
            // mode, so the user will get permission prompts they expected to
            // be auto-approved. Mirror the `DriverCommand::SetMode` arm rather
            // than swallowing with `let _`.
            on_event(AcpEvent::Error {
                message: format!("failed to set auto-approve mode: {e:#}"),
            });
        }
    }

    // A fresh session gets its kickoff prompt; a reconnect continues an
    // existing conversation and must not re-send one.
    if !is_reconnect {
        if let Some(prompt) = initial_prompt.as_deref() {
            if !prompt.trim().is_empty() {
                active.send_prompt(prompt)?;
                // The kickoff prompt is delivered backend-side, and agents don't
                // echo live prompts back as user chunks (user_message_chunk is a
                // replay-path thing) — so emit the user bubble ourselves or the
                // timeline's first turn appears out of thin air. Composer prompts
                // don't need this: the frontend adds an optimistic local bubble.
                on_event(AcpEvent::MessageChunk {
                    role: MessageRole::User,
                    text: prompt.to_string(),
                    message_id: None,
                    replay: false,
                });
                on_status(AcpPhase::TurnStarted);
            }
        }
    }

    loop {
        tokio::select! {
            update = active.read_update() => {
                match update {
                    Ok(SessionMessage::SessionMessage(dispatch)) => {
                        MatchDispatch::new(dispatch)
                            .if_notification(async move |notif: SessionNotification| {
                                // Status is NOT touched here. A turn's
                                // Working/Idle span is defined solely by its
                                // boundaries — a prompt we send (-> Working)
                                // and its `StopReason` (-> Idle). Streaming
                                // notifications arrive *within* that span, and
                                // some (a startup `current_mode_update`, a
                                // trailing `usage_update` after `StopReason`)
                                // arrive with no turn in flight at all — using
                                // any of them to set Working would strand the
                                // session as busy with nothing to clear it.
                                for ev in translate_update(&notif.update, false) {
                                    on_event(ev);
                                }
                                Ok(())
                            })
                            .await
                            .otherwise_ignore()?;
                    }
                    Ok(SessionMessage::StopReason(reason)) => {
                        on_event(AcpEvent::TurnEnded { stop_reason: stop_reason_label(&reason).to_string() });
                        on_status(AcpPhase::TurnCompleted);
                    }
                    // `SessionMessage` is `#[non_exhaustive]`.
                    Ok(_) => {}
                    Err(e) => {
                        on_event(AcpEvent::Error { message: format!("{e:#}") });
                        on_status(AcpPhase::TurnFailed);
                        break;
                    }
                }
            }
            cmd = cmd_rx.next() => {
                match cmd {
                    Some(DriverCommand::Prompt(text)) => {
                        match active.send_prompt(text) {
                            Ok(()) => on_status(AcpPhase::TurnStarted),
                            Err(e) => on_event(AcpEvent::Error { message: format!("{e:#}") }),
                        }
                    }
                    Some(DriverCommand::Cancel) => {
                        let _ = cx.send_notification(CancelNotification::new(active.session_id().clone()));
                    }
                    Some(DriverCommand::SetMode(mode_id)) => {
                        if let Err(e) = cx
                            .send_request(SetSessionModeRequest::new(active.session_id().clone(), mode_id))
                            .block_task()
                            .await
                        {
                            on_event(AcpEvent::Error { message: format!("{e:#}") });
                        }
                    }
                    // The command channel closes when the session is
                    // stopped/dropped — end the driver loop.
                    None => break,
                }
            }
        }
    }

    on_event(AcpEvent::Exit { code: None });
    on_status(AcpPhase::ConnectionClosed);
    Ok(())
}

/// Spawn the ACP agent process for `spec` in `cwd`, run its connection as one
/// spawned `Send` future, and register the resulting session under
/// `agent_id` in `state.sessions`/`state.agent_tasks` — the same id space
/// `start_agent`'s PTY sessions use, so teardown/MCP/status all keep working.
///
/// `task_id` and `mcp_token` are handed over (rather than re-derived) because
/// the caller (`start_acp_agent`) already resolved them and owns their
/// lifecycle (the token's removal on stop is handled generically by
/// `stop_session_inner`, same as the PTY path). `resume`, when `Some`, asks
/// `drive_connection` to re-establish the task's stored session.
#[allow(clippy::too_many_arguments)]
pub fn spawn_acp_session(
    app: &AppHandle,
    state: &AppState,
    agent_id: &str,
    task_id: &str,
    spec: &AgentSpec,
    cwd: &Path,
    initial_prompt: Option<String>,
    auto_approve: bool,
    mcp_token: Option<String>,
    resume: Option<ResumeContext>,
    on_event: Channel<AcpEvent>,
) -> Result<(), String> {
    let resolved = crate::claude_path::find_binary(&spec.binary);

    let mut argv: Vec<String> = Vec::new();
    argv.extend(spec.base_args.iter().cloned());
    argv.extend(spec.extra_args.iter().cloned());

    // `tokio::process` needs a reactor: sync Tauri commands run on the main
    // thread, OUTSIDE the async runtime, where `Command::spawn` panics with
    // "there is no reactor running" and takes the whole app down. Enter the
    // shared runtime's context for the rest of this fn so the spawn (and the
    // stdio pipe registration it performs) works from any calling thread.
    let runtime = tauri::async_runtime::handle();
    let _runtime_context = runtime.inner().enter();

    let mut command = tokio::process::Command::new(&resolved);
    command
        .args(&argv)
        .current_dir(cwd)
        .env("LAVIGIE", "1")
        .env("LAVIGIE_HOOK_PORT", state.hook_port.to_string())
        .env("LAVIGIE_AGENT_ID", agent_id)
        .env("LAVIGIE_TASK_ID", task_id)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut child = command
        .spawn()
        .map_err(|e| format!("spawning {}: {e:#}", resolved.display()))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "ACP child stdin not piped".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "ACP child stdout not piped".to_string())?;

    // Forward stderr line-by-line to our own stderr with an `[acp stderr]`
    // prefix. ACP speaks only over stdin/stdout, so the child's stderr is
    // pure diagnostics — but they're the ONLY signal when the agent fails to
    // start (e.g. `npx` can't resolve the package) or crashes mid-session, so
    // surfacing them (rather than draining to a void) is what makes those
    // failures debuggable. Reading it also keeps the pipe from filling.
    if let Some(stderr) = child.stderr.take() {
        let stderr_agent_id = agent_id.to_string();
        tauri::async_runtime::spawn(async move {
            use tokio::io::{AsyncBufReadExt as _, BufReader};
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                eprintln!("[acp stderr {stderr_agent_id}] {line}");
            }
        });
    }

    let child = Arc::new(Mutex::new(child));
    let transport = ByteStreams::new(stdin.compat_write(), stdout.compat());

    let (cmd_tx, cmd_rx) = mpsc::unbounded::<DriverCommand>();
    let pending_permissions: Arc<Mutex<HashMap<String, oneshot::Sender<Option<String>>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let sink: Arc<dyn StatusSink> = Arc::new(TauriSink::new(app.clone()));
    let auto_approve_mode = mode_for_auto_approve(&spec.name, auto_approve).map(str::to_string);
    let cwd_owned = cwd.to_path_buf();
    let agent_id_owned = agent_id.to_string();
    let task_id_owned = task_id.to_string();
    let mcp_port = state.mcp_port;
    let mcp_token_for_conn = mcp_token.clone();
    let acp_logs_root = state.acp_logs_root.clone();

    // Captures for the permission-request handler (called once per
    // `session/request_permission`, potentially many times over the
    // session's life).
    let perm_on_event = on_event.clone();
    let perm_sink = Arc::clone(&sink);
    let perm_agent_id = agent_id_owned.clone();
    let perm_pending = Arc::clone(&pending_permissions);

    // Captures for the one-shot main driver task.
    let main_on_event = on_event.clone();
    // A second frontend channel handle for RESTORED history (native replay +
    // event-log tier): forwards to the frontend but must NOT append to the
    // app-side log — see `drive_connection`'s `on_replay`.
    let replay_on_event = on_event.clone();
    let main_sink = Arc::clone(&sink);
    let main_agent_id = agent_id_owned.clone();
    let log_agent_id = agent_id_owned.clone();
    // Captures for the failure-notify path: if the connection future ends in
    // `Err` (before/without a clean `Exit`), tell the frontend so it tears the
    // session down instead of stranding the surface.
    let fail_on_event = on_event.clone();
    let fail_sink = Arc::clone(&sink);
    let fail_agent_id = agent_id_owned.clone();
    // Captures for the event-log/persist-session-id wrapper around `on_event`:
    // re-derive `AppState` from the `AppHandle` at call time, mirroring
    // `TauriSink::record`'s pattern, since `drive_connection` is sink-generic
    // and has no direct store access. The JSONL log file is opened ONCE here
    // (not per event) — `AcpLog::create` returns `None` on failure, in which
    // case logging is silently skipped.
    let event_log_app = app.clone();
    let event_log = log::AcpLog::create(&acp_logs_root, task_id, agent_id).map(Arc::new);
    let event_log_task_id = task_id_owned;
    // The provider (engine) name is the always-known attribution dimension for
    // persisted usage; the model rides alongside via `current_model_slot`.
    let usage_provider = spec.name.clone();
    let usage_agent_id = agent_id_owned.clone();

    // NOTE deliberately NO builder-level `on_receive_notification` here: user
    // handlers run BEFORE the session's dynamic handler in the crate's
    // dispatch chain (jsonrpc.rs "Message Flow"), so a catch-all that returns
    // Ok(()) CLAIMS every `session/update` and starves `read_update` — the
    // timeline goes silent while turns still complete (StopReason rides the
    // request/response path). Unrouted session notifications are queued by
    // the role default (`retry` on `has_session_id`) until the session builder
    // registers the session's dynamic handler; genuinely stray ones are ignored
    // at the end of the chain, which is all the removed catch-all ever did.
    let connection_future = Client
        .builder()
        .name("La Vigie")
        .on_receive_request(
            move |request: RequestPermissionRequest,
                  responder: agent_client_protocol::Responder<RequestPermissionResponse>,
                  _connection| {
                let on_event = perm_on_event.clone();
                let sink = Arc::clone(&perm_sink);
                let agent_id = perm_agent_id.clone();
                let pending = Arc::clone(&perm_pending);
                async move {
                    let request_id = uuid::Uuid::new_v4().to_string();
                    let (tx, rx) = oneshot::channel::<Option<String>>();
                    if let Ok(mut map) = pending.lock() {
                        map.insert(request_id.clone(), tx);
                    }

                    let options = request
                        .options
                        .iter()
                        .map(|o| PermissionOptionEvent {
                            option_id: o.option_id.0.to_string(),
                            name: o.name.clone(),
                            kind: permission_option_kind_label(&o.kind).to_string(),
                        })
                        .collect();
                    let tool_call =
                        serde_json::to_value(&request.tool_call).unwrap_or(serde_json::Value::Null);
                    let _ = on_event.send(AcpEvent::PermissionRequest {
                        request_id: request_id.clone(),
                        options,
                        tool_call,
                    });
                    sink.record(&agent_id, map_status(AcpPhase::PermissionRequested));

                    // Awaiting here is safe: this handler runs as its own
                    // task, not inline in the dispatch loop, so blocking on
                    // the user's answer doesn't stall other session traffic.
                    let answer = rx.await.unwrap_or(None);
                    // Drop our own entry so the map doesn't accumulate stale
                    // requests. `acp_respond_permission` already removes it on
                    // the answered path (this is then a no-op); this also
                    // covers the abandoned path where the sender was dropped
                    // (`rx` errored -> `answer = None`) without anyone removing
                    // it.
                    if let Ok(mut map) = pending.lock() {
                        map.remove(&request_id);
                    }
                    sink.record(&agent_id, map_status(AcpPhase::PermissionAnswered));

                    let outcome = match answer {
                        Some(option_id) => {
                            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id))
                        }
                        None => RequestPermissionOutcome::Cancelled,
                    };
                    responder.respond(RequestPermissionResponse::new(outcome))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, move |cx: ConnectionTo<Agent>| {
            let on_event = main_on_event;
            let replay_on_event = replay_on_event;
            let sink = main_sink;
            let agent_id = main_agent_id;
            let mcp_token = mcp_token_for_conn;
            let resume = resume;
            let event_log_app = event_log_app;
            let event_log = event_log;
            let event_log_task_id = event_log_task_id;
            let usage_provider = usage_provider;
            let usage_agent_id = usage_agent_id;
            async move {
                // Tracks the session id once `SessionStarted` arrives, so
                // every subsequent logged event (and the store persist below)
                // can be tagged with it; `None` before that point.
                let session_id_slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
                // The current model, updated on every `ModelSelected`, so a
                // `Usage` event — which the protocol sends with no model — can
                // be attributed.
                let current_model_slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

                let emit_event = move |ev: AcpEvent| {
                    if let AcpEvent::SessionStarted { session_id, .. } = &ev {
                        if let Ok(mut slot) = session_id_slot.lock() {
                            *slot = Some(session_id.clone());
                        }

                        // Best-effort: persist the id so a future relaunch can
                        // consume it via session/load|resume (not yet wired).
                        // A failure here never disrupts the live session.
                        use tauri::Manager as _;
                        let app_state = event_log_app.state::<AppState>();
                        let persisted = app_state
                            .store
                            .lock()
                            .map_err(|e| format!("store mutex poisoned: {e}"))
                            .and_then(|store| {
                                store
                                    .set_task_acp_session_id(&event_log_task_id, Some(session_id.as_str()))
                                    .map_err(|e| format!("{e:#}"))
                            });
                        if let Err(e) = persisted {
                            eprintln!("[acp] persisting acp_session_id for task {event_log_task_id}: {e}");
                        }
                    }

                    // Track the active model, and persist one usage row per
                    // `Usage` event attributed to (provider, current model,
                    // task, session). Best-effort — a persist failure is logged
                    // and swallowed, like the JSONL log below.
                    match &ev {
                        AcpEvent::ModelSelected { model_id } => {
                            if let Ok(mut slot) = current_model_slot.lock() {
                                *slot = Some(model_id.clone());
                            }
                        }
                        AcpEvent::Usage { used, size, cost, rate_limit } => {
                            let session_id = session_id_slot.lock().ok().and_then(|g| g.clone());
                            let model = current_model_slot.lock().ok().and_then(|g| g.clone());
                            let row = crate::store::AcpUsageEvent {
                                agent_id: usage_agent_id.clone(),
                                task_id: event_log_task_id.clone(),
                                session_id,
                                provider: usage_provider.clone(),
                                model,
                                used: *used as i64,
                                size: *size as i64,
                                cost_amount: cost.as_ref().map(|c| c.amount),
                                currency: cost.as_ref().map(|c| c.currency.clone()),
                                rate_status: rate_limit.as_ref().map(|r| r.status.clone()),
                                ts: chrono::Utc::now().timestamp(),
                            };
                            use tauri::Manager as _;
                            let app_state = event_log_app.state::<AppState>();
                            let persisted = app_state
                                .store
                                .lock()
                                .map_err(|e| format!("store mutex poisoned: {e}"))
                                .and_then(|store| {
                                    store.insert_acp_usage_event(&row).map_err(|e| format!("{e:#}"))
                                });
                            if let Err(e) = persisted {
                                eprintln!("[acp] persisting usage event for task {event_log_task_id}: {e}");
                            }
                        }
                        _ => {}
                    }

                    if let Some(logger) = event_log.as_ref() {
                        let session_id = session_id_slot.lock().ok().and_then(|g| g.clone());
                        logger.append(session_id.as_deref(), &ev);
                    }

                    let _ = on_event.send(ev);
                };
                let emit_status = move |phase: AcpPhase| sink.record(&agent_id, map_status(phase));
                // Restored-history sink: straight to the frontend, no log
                // append and no `acp_session_id` persist (replayed events carry
                // neither a live turn nor a new SessionStarted).
                let emit_replay_event = move |ev: AcpEvent| {
                    let _ = replay_on_event.send(ev);
                };
                drive_connection(
                    cx,
                    cwd_owned,
                    initial_prompt,
                    auto_approve_mode,
                    mcp_port,
                    mcp_token,
                    resume,
                    cmd_rx,
                    &emit_event,
                    &emit_replay_event,
                    &emit_status,
                )
                .await
            }
        });

    let join_handle = tauri::async_runtime::spawn(async move {
        if let Err(e) = connection_future.await {
            eprintln!("[acp] connection for agent {log_agent_id} ended with error: {e:#}");
            // The connection died before `drive_connection` emitted a clean
            // `Exit` (e.g. the agent process never came up, `initialize`
            // failed, or a resume's reconnect AND its fresh-session fallback
            // both failed). Emit `Error` + `Exit` ourselves so the frontend's
            // exit handler tears the session down (reverts the surface to the
            // Start placeholder) instead of stranding it — otherwise a resume
            // stays wedged on "Restoring…" (`sessionId` never lands). The Ok
            // path already emits `Exit` from within `drive_connection`, so this
            // never double-fires.
            let _ = fail_on_event.send(AcpEvent::Error { message: format!("{e:#}") });
            let _ = fail_on_event.send(AcpEvent::Exit { code: None });
            fail_sink.record(&fail_agent_id, map_status(AcpPhase::TurnFailed));
        }
    });
    let abort = join_handle.inner().abort_handle();

    let handle = SessionHandle {
        backend: SessionBackend::Acp { child, cmd_tx, abort, pending_permissions },
        mcp_token,
        kind: SessionKind::Task,
        repo_id: None,
        last_activity: std::time::Instant::now(),
        has_frontend_channel: true,
    };
    state
        .sessions
        .lock()
        .map_err(|e| e.to_string())?
        .insert(agent_id.to_string(), handle);

    Ok(())
}

/// Look up a registered session's ACP command sender, erroring if the id is
/// unknown or names a PTY (non-ACP) session.
fn acp_cmd_tx(state: &AppState, session_id: &str) -> Result<mpsc::UnboundedSender<DriverCommand>, String> {
    let sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    let handle = sessions
        .get(session_id)
        .ok_or_else(|| format!("session not found: {session_id}"))?;
    match &handle.backend {
        SessionBackend::Acp { cmd_tx, .. } => Ok(cmd_tx.clone()),
        SessionBackend::Pty { .. } => Err(format!("{session_id} is not an ACP session")),
    }
}

/// Spawn the task's resolved ACP agent and stream its structured events to
/// the frontend over `on_event`. Mirrors `agent::start_agent`'s
/// task -> repo -> default resolution, but is ACP-only: a task/repo resolving
/// to a PTY engine errors here (the frontend should call `start_agent`
/// instead, choosing by `spec.execution`).
///
/// `resume`: when set, re-establish the task's stored ACP session
/// (`tasks.acp_session_id`) rather than starting fresh — `drive_connection`
/// picks `session/load` ▸ `session/resume` ▸ event-log replay ▸ fresh from the
/// agent's advertised capabilities. Resume with no stored id (or an unsupported
/// agent) degrades to a fresh session.
#[tauri::command]
pub fn start_acp_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
    resume: bool,
    initial_prompt: Option<String>,
    on_event: Channel<AcpEvent>,
) -> Result<String, String> {
    let (worktree_path, repo_id, task_agent, task_auto_approve, acp_session_id, repo_default, repo_auto_approve, custom_agents) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let task = store
            .get_task(&task_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("task not found: {task_id}"))?;
        let repo = store.get_repo(&task.repo_id).map_err(|e| e.to_string())?;
        let repo_default = repo.as_ref().and_then(|r| r.default_agent.clone());
        let repo_auto_approve = repo.as_ref().and_then(|r| r.auto_approve);
        let custom = store.list_custom_agents().map_err(|e| e.to_string())?;
        (
            task.worktree_path,
            task.repo_id,
            task.agent,
            task.auto_approve,
            task.acp_session_id,
            repo_default,
            repo_auto_approve,
            custom,
        )
    };

    use crate::agent::spec::{effective_auto_approve, resolve_for_task, ExecutionMode};

    let spec = resolve_for_task(task_agent.as_deref(), repo_default.as_deref(), &custom_agents);
    if spec.execution != ExecutionMode::Acp {
        return Err(format!(
            "'{}' is a PTY agent; use start_agent, not start_acp_agent",
            spec.name
        ));
    }
    let auto_approve = effective_auto_approve(task_auto_approve, repo_auto_approve);

    // Build the resume plan up front (before spawning). `drive_connection`
    // chooses load/resume/event-log/fresh once it sees the agent's caps; here
    // we only supply the stored id, a cheap "does a log exist" flag, and a
    // thunk that reads + parses the app-side log lazily — the thunk runs only
    // on the EventLog tier, so the common Load path never reads the file. A
    // resume with no stored id can't reconnect — fall through to fresh.
    let resume_ctx = if resume {
        match acp_session_id {
            Some(stored_session_id) => {
                let log_path = log::log_path(&state.acp_logs_root, &task_id);
                let has_event_log =
                    std::fs::metadata(&log_path).map(|m| m.len() > 0).unwrap_or(false);
                Some(ResumeContext {
                    stored_session_id,
                    has_event_log,
                    load_event_log: Box::new(move || {
                        let contents = std::fs::read_to_string(&log_path).unwrap_or_default();
                        log::parse_replay_events(&contents)
                    }),
                })
            }
            None => {
                eprintln!(
                    "[acp] resume requested for task {task_id} but no stored ACP session id; starting fresh"
                );
                None
            }
        }
    } else {
        None
    };

    let agent_id = uuid::Uuid::new_v4().to_string();
    state
        .agent_tasks
        .lock()
        .map_err(|e| e.to_string())?
        .insert(agent_id.clone(), task_id.clone());

    // Mint an Agent-tier MCP token for lifecycle parity with the PTY path
    // (teardown/`finish_task` already know how to release it).
    let mcp_token = uuid::Uuid::new_v4().to_string();
    state
        .mcp_tokens
        .lock()
        .map_err(|e| e.to_string())?
        .insert(
            mcp_token.clone(),
            crate::state::McpToken::Agent(crate::state::AgentLaunchContext {
                task_id: task_id.clone(),
                repo_id,
            }),
        );

    if let Err(e) = spawn_acp_session(
        &app,
        state.inner(),
        &agent_id,
        &task_id,
        &spec,
        Path::new(&worktree_path),
        initial_prompt,
        auto_approve,
        Some(mcp_token.clone()),
        resume_ctx,
        on_event,
    ) {
        // Spawn failed before registration, whose success owns the token's
        // lifecycle from here on — revoke it and the task mapping so a
        // failed launch doesn't orphan either.
        let _ = state.mcp_tokens.lock().map(|mut m| m.remove(&mcp_token));
        let _ = state.agent_tasks.lock().map(|mut m| m.remove(&agent_id));
        return Err(e);
    }

    Ok(agent_id)
}

/// Per-model usage/cost/rate-limit summary for ACP sessions over the last
/// `window_days` (default 7): reads the persisted `acp_usage_events` and runs
/// the pure `usage::summarize_usage` collapse.
#[tauri::command]
pub fn get_acp_usage_summary(
    state: State<'_, AppState>,
    window_days: Option<u32>,
) -> Result<usage::AcpUsageSummary, String> {
    let days = window_days.unwrap_or(7).max(1) as i64;
    let since_ts = chrono::Utc::now().timestamp() - days * 86_400;
    let events = {
        let store = state.store.lock().map_err(|e| format!("store mutex poisoned: {e}"))?;
        store.acp_usage_events_since(since_ts).map_err(|e| format!("{e:#}"))?
    };
    Ok(usage::summarize_usage(&events))
}

/// Send a new user prompt on a running ACP session (the ACP counterpart of
/// `write_session`, which raw PTY bytes take instead).
#[tauri::command]
pub fn acp_prompt(state: State<'_, AppState>, session_id: String, text: String) -> Result<(), String> {
    acp_cmd_tx(state.inner(), &session_id)?
        .unbounded_send(DriverCommand::Prompt(text))
        .map_err(|e| format!("ACP session {session_id} closed: {e}"))
}

/// Cancel the in-flight turn on a running ACP session.
#[tauri::command]
pub fn acp_cancel(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    acp_cmd_tx(state.inner(), &session_id)?
        .unbounded_send(DriverCommand::Cancel)
        .map_err(|e| format!("ACP session {session_id} closed: {e}"))
}

/// Switch a running ACP session's active mode (e.g. toggling permission
/// modes mid-session).
#[tauri::command]
pub fn acp_set_mode(state: State<'_, AppState>, session_id: String, mode_id: String) -> Result<(), String> {
    acp_cmd_tx(state.inner(), &session_id)?
        .unbounded_send(DriverCommand::SetMode(mode_id))
        .map_err(|e| format!("ACP session {session_id} closed: {e}"))
}

/// Answer a pending `session/request_permission` on a running ACP session:
/// `option_id` selects that option, `None` cancels the request. Errors if the
/// session or the specific `request_id` is unknown (already answered, or
/// never existed) — an honest signal to the frontend rather than a silent
/// no-op.
#[tauri::command]
pub fn acp_respond_permission(
    state: State<'_, AppState>,
    session_id: String,
    request_id: String,
    option_id: Option<String>,
) -> Result<(), String> {
    let sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    let handle = sessions
        .get(&session_id)
        .ok_or_else(|| format!("session not found: {session_id}"))?;
    match &handle.backend {
        SessionBackend::Acp { pending_permissions, .. } => {
            let mut pending = pending_permissions.lock().map_err(|e| e.to_string())?;
            match pending.remove(&request_id) {
                Some(tx) => {
                    let _ = tx.send(option_id);
                    Ok(())
                }
                None => Err(format!("no pending permission request: {request_id}")),
            }
        }
        SessionBackend::Pty { .. } => Err(format!("{session_id} is not an ACP session")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{AgentCapabilities, McpCapabilities};

    #[test]
    fn lavigie_mcp_server_builds_http_entry_with_bearer_header() {
        let server = lavigie_mcp_server(4321, "tok-123");
        match server {
            McpServer::Http(http) => {
                assert_eq!(http.name, "lavigie");
                assert_eq!(http.url, "http://127.0.0.1:4321/mcp");
                assert_eq!(http.headers.len(), 1);
                assert_eq!(http.headers[0].name, "Authorization");
                assert_eq!(http.headers[0].value, "Bearer tok-123");
            }
            other => panic!("expected McpServer::Http, got {other:?}"),
        }
    }

    #[test]
    fn should_inject_http_true_when_agent_advertises_it() {
        let resp = InitializeResponse::new(ProtocolVersion::V1)
            .agent_capabilities(AgentCapabilities::new().mcp_capabilities(McpCapabilities::new().http(true)));
        assert!(should_inject_http(&resp));
    }

    #[test]
    fn should_inject_http_false_when_agent_does_not_advertise_it() {
        // Default AgentCapabilities has mcp_capabilities.http = false — the
        // `mistral-acp` (vibe-acp) case, which has no `mcpCapabilities` at all.
        let resp = InitializeResponse::new(ProtocolVersion::V1);
        assert!(!should_inject_http(&resp));
    }

    #[test]
    fn initialize_request_carries_non_empty_client_info() {
        // Regression for the Mistral 422: `vibe-acp` forwards client_info
        // straight into its own API metadata and rejects an empty
        // client_name/client_version.
        let req = initialize_request();
        let client_info = req.client_info.expect("client_info must be set");
        assert_eq!(client_info.name, "la-vigie");
        assert!(!client_info.version.is_empty());
        assert_eq!(client_info.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn mode_for_auto_approve_maps_known_engines() {
        assert_eq!(mode_for_auto_approve("claude-acp", true), Some("bypassPermissions"));
        assert_eq!(mode_for_auto_approve("mistral-acp", true), Some("auto-approve"));
    }

    #[test]
    fn mode_for_auto_approve_is_none_when_off() {
        assert_eq!(mode_for_auto_approve("claude-acp", false), None);
        assert_eq!(mode_for_auto_approve("mistral-acp", false), None);
    }

    #[test]
    fn mode_for_auto_approve_is_none_for_unknown_engine() {
        // A future generic ACP engine (config-driven, not one of the two
        // built-ins) has no known mode vocabulary yet — leave the session on
        // its default mode rather than guessing.
        assert_eq!(mode_for_auto_approve("some-custom-acp-engine", true), None);
    }

    #[test]
    fn select_resume_source_prefers_load_over_everything() {
        // Both curated engines (claude-acp/mistral-acp) advertise loadSession —
        // load wins even when resume/event-log are also available.
        assert_eq!(select_resume_source(true, true, true), ResumeSource::Load);
        assert_eq!(select_resume_source(true, false, false), ResumeSource::Load);
    }

    #[test]
    fn select_resume_source_falls_back_to_resume_then_event_log_then_fresh() {
        assert_eq!(select_resume_source(false, true, true), ResumeSource::Resume);
        assert_eq!(select_resume_source(false, true, false), ResumeSource::Resume);
        // No native path, but an app-side log exists → visual replay.
        assert_eq!(select_resume_source(false, false, true), ResumeSource::EventLog);
        // Nothing to restore from → fresh (never hang on a replay that won't
        // come — the Cursor/Copilot-CLI classes).
        assert_eq!(select_resume_source(false, false, false), ResumeSource::Fresh);
    }

    #[test]
    fn permission_option_kind_label_covers_all_known_kinds() {
        assert_eq!(permission_option_kind_label(&PermissionOptionKind::AllowOnce), "allow_once");
        assert_eq!(permission_option_kind_label(&PermissionOptionKind::AllowAlways), "allow_always");
        assert_eq!(permission_option_kind_label(&PermissionOptionKind::RejectOnce), "reject_once");
        assert_eq!(permission_option_kind_label(&PermissionOptionKind::RejectAlways), "reject_always");
    }
}
