//! HookBridge: receive Claude Code hook callbacks and emit agent status events.
//!
//! Claude Code is configured (per spawn, via `--settings`) to POST its hook
//! payloads to a local axum server started at app startup. Each POST carries
//! a JSON body whose `hook_event_name` (and optional `notification_type`) are
//! mapped to a `StatusEvent`, run through the state machine, persisted, and
//! forwarded to the frontend via a `StatusSink`.
//!
//! The `StatusSink` trait decouples the bridge from a live Tauri app handle so
//! the route handler can be tested without any Tauri infrastructure.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::post;
use axum::Router;

use crate::agent::status::{apply_event, to_task_status, StatusEvent};
use crate::state::AppState;

// ── Pure Claude hook adapter ──────────────────────────────────────────────────

/// Claude Code hook adapter: map a hook event name (+ optional notification_type)
/// to a normalized `StatusEvent`. `PreToolUse` and `UserPromptSubmit` both mean
/// "the agent is actively working" — mapping `PreToolUse` closes the AC2-47 gap
/// where a permission approval (which resumes via `PreToolUse`, not a prompt
/// submit) left the run-state stuck on NeedsAttention. `SubagentStart`/
/// `SubagentStop` map to background-subagent events (AC2-85) so the state machine
/// keeps the pill active while a backgrounded subagent runs past the main `Stop`.
/// Returns `None` for events that carry no status meaning.
pub fn claude_event(
    hook_event_name: &str,
    notification_type: Option<&str>,
) -> Option<StatusEvent> {
    match hook_event_name {
        "UserPromptSubmit" | "PreToolUse" => Some(StatusEvent::Working),
        "Notification" => match notification_type? {
            "permission_prompt" | "elicitation_dialog" => Some(StatusEvent::NeedsAttention),
            "idle_prompt" => Some(StatusEvent::Idle),
            _ => None,
        },
        "Stop" => Some(StatusEvent::Idle),
        "StopFailure" => Some(StatusEvent::Failed),
        // AC2-85: track in-flight background subagents so the pill stays active
        // while a backgrounded subagent runs past the main loop's Stop.
        "SubagentStart" => Some(StatusEvent::SubagentStarted),
        "SubagentStop" => Some(StatusEvent::SubagentStopped),
        _ => None,
    }
}

// ── Console status type + parsers ─────────────────────────────────────────────

/// Provider-neutral console snapshot for the status banner. Fields are optional
/// so partial updates (statusLine vs hook permission_mode) can merge on the
/// frontend; absent fields are omitted from the emitted event.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_remaining_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// Parse the Claude Code statusLine JSON into a `ConsoleStatus` (model + context
/// remaining %). `remaining_percentage` is preferred; otherwise derive from
/// `used_percentage`; otherwise `None` (e.g. null early in a session).
pub fn parse_status_line(body: &[u8]) -> ConsoleStatus {
    let v: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return ConsoleStatus::default(),
    };
    let model = v["model"]["display_name"].as_str().map(str::to_string);
    let cw = &v["context_window"];
    let context_remaining_percent = cw["remaining_percentage"]
        .as_f64()
        .or_else(|| cw["used_percentage"].as_f64().map(|used| 100.0 - used));
    ConsoleStatus { model, context_remaining_percent, mode: None }
}

/// Extract the `permission_mode` common field from a hook payload, if present.
pub fn parse_permission_mode(body: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v["permission_mode"].as_str().map(str::to_string)
}

// ── Task-rename name sanitizer (AC2-40) ───────────────────────────────────────

/// Maximum length (in chars) of an agent-set task name; longer input is truncated.
const MAX_TASK_NAME_LEN: usize = 200;

/// Normalize an agent-supplied task name (AC2-40): collapse every run of
/// whitespace — including embedded newlines, tabs, and other control
/// whitespace — to single spaces, trim the ends, and cap the length at a char
/// boundary. Returns `None` when nothing usable remains, so a blank/whitespace
/// payload rejects the rename instead of blanking the task title.
pub fn sanitize_task_name(raw: &str) -> Option<String> {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    Some(collapsed.chars().take(MAX_TASK_NAME_LEN).collect())
}

// ── Status sink trait ─────────────────────────────────────────────────────────

/// Accepts a normalized status event for an agent (from the Claude hook adapter
/// or, later, other provider adapters) and a console snapshot. Implementations
/// run the state machine, persist, and forward to the webview — or collect for
/// tests.
pub trait StatusSink: Send + Sync + 'static {
    fn record(&self, agent_id: &str, event: StatusEvent);
    fn emit_console(&self, agent_id: &str, console: ConsoleStatus);
    /// Rename the task owned by `agent_id` to the (already-sanitized) `name`
    /// (AC2-40). The agent can only ever rename its own task — implementations
    /// resolve `agent_id → task_id` and no-op for an unknown id.
    fn set_task_name(&self, agent_id: &str, name: &str);
    /// Record the filesystem path of `agent_id`'s transcript (AC2-108).
    /// Implementations resolve `agent_id → task_id` and store `task_id → path`;
    /// no-op for an unknown agent id.
    fn set_transcript(&self, agent_id: &str, transcript_path: &str);
}

// ── Production sink (Tauri AppHandle) ────────────────────────────────────────

/// Tauri event payload for `"agent_status"`. `status` is the normalized run-state;
/// for Claude it only ever takes the refined values (working/idle/needs_attention/
/// error), which are wire-compatible with the frontend's existing `AgentActivity`.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AgentStatusPayload {
    agent_id: String,
    status: crate::agent::status::AgentRunState,
}

/// Tauri event payload for `"agent_console"`.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AgentConsolePayload {
    agent_id: String,
    #[serde(flatten)]
    console: ConsoleStatus,
}

/// Tauri event payload for `"task_renamed"` (AC2-40): the task whose title an
/// agent changed, and its new title. The frontend patches the task in place so
/// the sidebar/header refresh live.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TaskRenamedPayload {
    task_id: String,
    title: String,
}

/// Production `StatusSink` that emits a Tauri event to all webview listeners.
pub struct TauriSink {
    app: tauri::AppHandle,
}

impl TauriSink {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl StatusSink for TauriSink {
    fn record(&self, agent_id: &str, event: StatusEvent) {
        use tauri::{Emitter as _, Manager as _};

        let st = self.app.state::<AppState>();

        // 1. Run the state machine (change-detected). No emit/persist if unchanged.
        let new_state = {
            let mut states = match st.agent_states.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            apply_event(&mut states, agent_id, event)
        };
        let Some(new_state) = new_state else { return };

        // 2. Persist the mapped status. Mutex-safe: lock → write → drop, no await.
        if let Some(task_status) = to_task_status(new_state) {
            let task_id = st
                .agent_tasks
                .lock()
                .ok()
                .and_then(|m| m.get(agent_id).cloned());
            if let Some(task_id) = task_id {
                if let Ok(store) = st.store.lock() {
                    let _ = store.update_task_status(&task_id, task_status);
                }
            }
        }

        // 3. Emit the normalized state to the webview.
        let _ = self.app.emit(
            "agent_status",
            AgentStatusPayload {
                agent_id: agent_id.to_string(),
                status: new_state,
            },
        );
    }

    fn emit_console(&self, agent_id: &str, console: ConsoleStatus) {
        use tauri::Emitter as _;
        let _ = self.app.emit(
            "agent_console",
            AgentConsolePayload {
                agent_id: agent_id.to_string(),
                console,
            },
        );
    }

    fn set_task_name(&self, agent_id: &str, name: &str) {
        use tauri::{Emitter as _, Manager as _};

        let st = self.app.state::<AppState>();

        // Resolve the agent's OWN task; unknown id → no-op (an agent can never
        // rename a task that isn't the one it was spawned for).
        let task_id = st
            .agent_tasks
            .lock()
            .ok()
            .and_then(|m| m.get(agent_id).cloned());
        let Some(task_id) = task_id else { return };

        // Persist the new title. Mutex-safe: lock → write → drop, no await.
        {
            let Ok(store) = st.store.lock() else { return };
            if store.update_task_title(&task_id, name).is_err() {
                return;
            }
        }

        // Tell the webview so the sidebar/header update live.
        let _ = self.app.emit(
            "task_renamed",
            TaskRenamedPayload {
                task_id,
                title: name.to_string(),
            },
        );
    }

    fn set_transcript(&self, agent_id: &str, transcript_path: &str) {
        use tauri::Manager as _;
        // Resolve the agent's own task; unknown id → no-op.
        let task_id = self.app.state::<AppState>()
            .agent_tasks
            .lock()
            .ok()
            .and_then(|m| m.get(agent_id).cloned());
        let Some(task_id) = task_id else { return };
        // Capture transcript. Mutex-safe: lock → insert → drop guard.
        let _ = self.app.state::<AppState>()
            .transcripts
            .lock()
            .map(|mut map| map.insert(task_id, transcript_path.to_string()));
    }
}

// ── Axum route handler ────────────────────────────────────────────────────────

/// Minimal subset of the hook JSON body we care about.
#[derive(serde::Deserialize)]
struct HookBody {
    hook_event_name: Option<String>,
    notification_type: Option<String>,
    transcript_path: Option<String>,
}

/// `POST /hook/:agent_id` — receive a hook payload and (maybe) emit a status.
/// Always returns 200 so the hook command never sees an error.
async fn hook_handler(
    Path(agent_id): Path<String>,
    State(sink): State<Arc<dyn StatusSink>>,
    body: axum::body::Bytes,
) -> axum::http::StatusCode {
    // Best-effort parse — ignore malformed bodies entirely.
    if let Ok(parsed) = serde_json::from_slice::<HookBody>(&body) {
        if let Some(event) = parsed.hook_event_name.as_deref() {
            if let Some(status_event) = claude_event(event, parsed.notification_type.as_deref()) {
                sink.record(&agent_id, status_event);
            }
        }
        if let Some(path) = parsed.transcript_path.as_deref() {
            sink.set_transcript(&agent_id, path);
        }
    }
    if let Some(mode) = parse_permission_mode(&body) {
        sink.emit_console(&agent_id, ConsoleStatus { mode: Some(mode), ..Default::default() });
    }
    axum::http::StatusCode::OK
}

/// `POST /rename/:agent_id` — rename the calling agent's own task (AC2-40).
/// The request body is the raw new name (plain text). On success returns 200
/// with the applied (sanitized) name; a blank/whitespace-only name returns 400
/// so the caller knows the rename was rejected rather than silently dropped.
async fn rename_handler(
    Path(agent_id): Path<String>,
    State(sink): State<Arc<dyn StatusSink>>,
    body: axum::body::Bytes,
) -> (axum::http::StatusCode, String) {
    let raw = String::from_utf8_lossy(&body);
    match sanitize_task_name(&raw) {
        Some(name) => {
            sink.set_task_name(&agent_id, &name);
            (axum::http::StatusCode::OK, name)
        }
        None => (
            axum::http::StatusCode::BAD_REQUEST,
            "task name is empty after trimming".to_string(),
        ),
    }
}

/// `POST /status/:agent_id` — receive a statusLine payload and emit a console update.
async fn status_handler(
    Path(agent_id): Path<String>,
    State(sink): State<Arc<dyn StatusSink>>,
    body: axum::body::Bytes,
) -> axum::http::StatusCode {
    sink.emit_console(&agent_id, parse_status_line(&body));
    axum::http::StatusCode::OK
}

// ── Server startup ────────────────────────────────────────────────────────────

/// Start the hook bridge server. Binds to an ephemeral port on loopback,
/// spawns the axum serve loop on the Tauri (tokio) runtime, and returns the
/// chosen port.
pub async fn start_hook_server(sink: Arc<dyn StatusSink>) -> std::io::Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    let app: Router = Router::new()
        .route("/hook/{agent_id}", post(hook_handler))
        .route("/status/{agent_id}", post(status_handler))
        .route("/rename/{agent_id}", post(rename_handler))
        .with_state(sink);

    tauri::async_runtime::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Ok(port)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use axum::body::Body;
    use tower::ServiceExt as _;

    // ── claude_event ──────────────────────────────────────────────────────────

    #[test]
    fn claude_pretooluse_is_working() {
        assert_eq!(claude_event("PreToolUse", None), Some(StatusEvent::Working));
    }

    #[test]
    fn claude_user_prompt_submit_is_working() {
        assert_eq!(claude_event("UserPromptSubmit", None), Some(StatusEvent::Working));
    }

    #[test]
    fn claude_permission_prompt_is_needs_attention() {
        assert_eq!(
            claude_event("Notification", Some("permission_prompt")),
            Some(StatusEvent::NeedsAttention)
        );
        assert_eq!(
            claude_event("Notification", Some("elicitation_dialog")),
            Some(StatusEvent::NeedsAttention)
        );
    }

    #[test]
    fn claude_idle_and_stop_are_idle() {
        assert_eq!(claude_event("Notification", Some("idle_prompt")), Some(StatusEvent::Idle));
        assert_eq!(claude_event("Stop", None), Some(StatusEvent::Idle));
    }

    #[test]
    fn claude_stop_failure_is_failed() {
        assert_eq!(claude_event("StopFailure", None), Some(StatusEvent::Failed));
    }

    #[test]
    fn claude_subagent_start_and_stop_map_to_background_events() {
        // AC2-85: background-subagent lifecycle drives the in-flight counter so the
        // pill stays active while a backgrounded subagent runs past the main Stop.
        assert_eq!(claude_event("SubagentStart", None), Some(StatusEvent::SubagentStarted));
        assert_eq!(claude_event("SubagentStop", None), Some(StatusEvent::SubagentStopped));
    }

    #[test]
    fn claude_unknown_or_no_type_is_none() {
        assert_eq!(claude_event("Notification", Some("auth_success")), None);
        assert_eq!(claude_event("Notification", None), None);
        assert_eq!(claude_event("RandomThing", None), None);
    }

    // ── Collecting test sink ──────────────────────────────────────────────────

    struct CollectingSink {
        events: Mutex<Vec<(String, StatusEvent)>>,
        consoles: Mutex<Vec<(String, ConsoleStatus)>>,
        renames: Mutex<Vec<(String, String)>>,
        transcripts: Mutex<Vec<(String, String)>>,
    }

    impl CollectingSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                events: Mutex::new(Vec::new()),
                consoles: Mutex::new(Vec::new()),
                renames: Mutex::new(Vec::new()),
                transcripts: Mutex::new(Vec::new()),
            })
        }

        fn collected(&self) -> Vec<(String, StatusEvent)> {
            self.events.lock().unwrap().clone()
        }

        fn collected_consoles(&self) -> Vec<(String, ConsoleStatus)> {
            self.consoles.lock().unwrap().clone()
        }

        fn collected_renames(&self) -> Vec<(String, String)> {
            self.renames.lock().unwrap().clone()
        }

        fn collected_transcripts(&self) -> Vec<(String, String)> {
            self.transcripts.lock().unwrap().clone()
        }
    }

    impl StatusSink for CollectingSink {
        fn record(&self, agent_id: &str, event: StatusEvent) {
            self.events.lock().unwrap().push((agent_id.to_string(), event));
        }

        fn emit_console(&self, agent_id: &str, console: ConsoleStatus) {
            self.consoles.lock().unwrap().push((agent_id.to_string(), console));
        }

        fn set_task_name(&self, agent_id: &str, name: &str) {
            self.renames.lock().unwrap().push((agent_id.to_string(), name.to_string()));
        }

        fn set_transcript(&self, agent_id: &str, transcript_path: &str) {
            self.transcripts.lock().unwrap().push((agent_id.to_string(), transcript_path.to_string()));
        }
    }

    fn make_router(sink: Arc<dyn StatusSink>) -> Router {
        Router::new()
            .route("/hook/{agent_id}", post(hook_handler))
            .route("/status/{agent_id}", post(status_handler))
            .route("/rename/{agent_id}", post(rename_handler))
            .with_state(sink)
    }

    async fn post_hook_status(router: &Router, path: &str, body: &str) -> (axum::http::StatusCode, String) {
        let req = axum::http::Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "text/plain")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    async fn post_hook(router: &Router, path: &str, body: &str) -> axum::http::StatusCode {
        let req = axum::http::Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        let resp = router.clone().oneshot(req).await.unwrap();
        resp.status()
    }

    // ── HookBridge handler tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn mapped_event_records_status_and_returns_200() {
        let sink = CollectingSink::new();
        let router = make_router(Arc::clone(&sink) as Arc<dyn StatusSink>);
        let body = r#"{"hook_event_name":"Notification","notification_type":"permission_prompt"}"#;
        let status = post_hook(&router, "/hook/agent-1", body).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(sink.collected(), vec![("agent-1".to_string(), StatusEvent::NeedsAttention)]);
    }

    #[tokio::test]
    async fn pretooluse_records_working() {
        // AC2-47: PreToolUse (the resume-after-approval event) is now delivered as Working.
        let sink = CollectingSink::new();
        let router = make_router(Arc::clone(&sink) as Arc<dyn StatusSink>);
        let body = r#"{"hook_event_name":"PreToolUse","permission_mode":"default"}"#;
        let status = post_hook(&router, "/hook/agent-1", body).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(sink.collected(), vec![("agent-1".to_string(), StatusEvent::Working)]);
    }

    #[tokio::test]
    async fn user_prompt_submit_records_working() {
        let sink = CollectingSink::new();
        let router = make_router(Arc::clone(&sink) as Arc<dyn StatusSink>);
        let body = r#"{"hook_event_name":"UserPromptSubmit","session_id":"s1","cwd":"/tmp"}"#;
        post_hook(&router, "/hook/my-agent", body).await;
        assert_eq!(sink.collected(), vec![("my-agent".to_string(), StatusEvent::Working)]);
    }

    #[tokio::test]
    async fn stop_records_idle() {
        let sink = CollectingSink::new();
        let router = make_router(Arc::clone(&sink) as Arc<dyn StatusSink>);
        let body = r#"{"hook_event_name":"Stop","permission_mode":"default"}"#;
        post_hook(&router, "/hook/agent-2", body).await;
        assert_eq!(sink.collected(), vec![("agent-2".to_string(), StatusEvent::Idle)]);
    }

    #[tokio::test]
    async fn stop_failure_records_failed() {
        let sink = CollectingSink::new();
        let router = make_router(Arc::clone(&sink) as Arc<dyn StatusSink>);
        let body = r#"{"hook_event_name":"StopFailure"}"#;
        post_hook(&router, "/hook/agent-3", body).await;
        assert_eq!(sink.collected(), vec![("agent-3".to_string(), StatusEvent::Failed)]);
    }

    #[tokio::test]
    async fn unmapped_event_returns_200_and_nothing_collected() {
        let sink = CollectingSink::new();
        let router = make_router(Arc::clone(&sink) as Arc<dyn StatusSink>);

        let body = r#"{"hook_event_name":"SomeRandomEvent"}"#;
        let status = post_hook(&router, "/hook/agent-1", body).await;

        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(sink.collected().is_empty());
    }

    #[tokio::test]
    async fn malformed_body_returns_200_and_nothing_collected() {
        let sink = CollectingSink::new();
        let router = make_router(Arc::clone(&sink) as Arc<dyn StatusSink>);

        let status = post_hook(&router, "/hook/agent-1", "not json at all").await;

        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(sink.collected().is_empty());
    }

    // ── sanitize_task_name ────────────────────────────────────────────────────

    #[test]
    fn sanitize_trims_and_collapses_whitespace() {
        assert_eq!(sanitize_task_name("  Build the thing  ").as_deref(), Some("Build the thing"));
        assert_eq!(
            sanitize_task_name("Build\n\tthe   thing").as_deref(),
            Some("Build the thing"),
        );
    }

    #[test]
    fn sanitize_rejects_blank_and_whitespace_only() {
        assert_eq!(sanitize_task_name(""), None);
        assert_eq!(sanitize_task_name("   \n\t  "), None);
    }

    #[test]
    fn sanitize_truncates_overlong_names_at_char_boundary() {
        let long = "é".repeat(MAX_TASK_NAME_LEN + 50);
        let out = sanitize_task_name(&long).unwrap();
        assert_eq!(out.chars().count(), MAX_TASK_NAME_LEN);
    }

    // ── rename_handler ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn rename_valid_name_records_and_echoes_sanitized() {
        let sink = CollectingSink::new();
        let router = make_router(Arc::clone(&sink) as Arc<dyn StatusSink>);
        let (status, body) = post_hook_status(&router, "/rename/agent-7", "  New title \n").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body, "New title");
        assert_eq!(sink.collected_renames(), vec![("agent-7".to_string(), "New title".to_string())]);
    }

    #[tokio::test]
    async fn rename_blank_name_is_rejected_and_records_nothing() {
        let sink = CollectingSink::new();
        let router = make_router(Arc::clone(&sink) as Arc<dyn StatusSink>);
        let (status, _body) = post_hook_status(&router, "/rename/agent-7", "   \n  ").await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
        assert!(sink.collected_renames().is_empty());
    }

    #[tokio::test]
    async fn hook_with_transcript_path_is_captured() {
        let sink = CollectingSink::new();
        let router = make_router(Arc::clone(&sink) as Arc<dyn StatusSink>);
        let body = r#"{"hook_event_name":"PreToolUse","transcript_path":"/tmp/x/sess.jsonl"}"#;
        let code = post_hook(&router, "/hook/agent-42", body).await;
        assert_eq!(code, axum::http::StatusCode::OK);
        assert_eq!(
            sink.collected_transcripts(),
            vec![("agent-42".to_string(), "/tmp/x/sess.jsonl".to_string())]
        );
    }

    // ── parse_status_line ─────────────────────────────────────────────────────

    #[test]
    fn parse_status_line_extracts_model_and_remaining() {
        let body = br#"{"model":{"display_name":"Opus"},"context_window":{"used_percentage":8,"remaining_percentage":92}}"#;
        let c = parse_status_line(body);
        assert_eq!(c.model.as_deref(), Some("Opus"));
        assert_eq!(c.context_remaining_percent, Some(92.0));
        assert_eq!(c.mode, None);
    }

    #[test]
    fn parse_status_line_falls_back_to_100_minus_used_when_no_remaining() {
        let body = br#"{"model":{"display_name":"Opus"},"context_window":{"used_percentage":30}}"#;
        assert_eq!(parse_status_line(body).context_remaining_percent, Some(70.0));
    }

    #[test]
    fn parse_status_line_handles_null_context_early_session() {
        let body = br#"{"model":{"display_name":"Opus"},"context_window":{"used_percentage":null,"remaining_percentage":null}}"#;
        assert_eq!(parse_status_line(body).context_remaining_percent, None);
    }

    #[test]
    fn parse_status_line_on_garbage_is_empty() {
        let c = parse_status_line(b"not json");
        assert_eq!(c, ConsoleStatus::default());
    }

    #[test]
    fn parse_permission_mode_reads_field_or_none() {
        assert_eq!(parse_permission_mode(br#"{"permission_mode":"auto"}"#).as_deref(), Some("auto"));
        assert_eq!(parse_permission_mode(br#"{"hook_event_name":"Stop"}"#), None);
    }

    // ── status_handler + hook_handler console tests ───────────────────────────

    #[tokio::test]
    async fn status_handler_emits_console() {
        let sink = CollectingSink::new();
        let router = make_router(Arc::clone(&sink) as Arc<dyn StatusSink>);
        let body = r#"{"model":{"display_name":"Opus"},"context_window":{"remaining_percentage":81}}"#;
        let code = post_hook(&router, "/status/agent-9", body).await;
        assert_eq!(code, axum::http::StatusCode::OK);
        let consoles = sink.collected_consoles();
        assert_eq!(consoles.len(), 1);
        assert_eq!(consoles[0].0, "agent-9");
        assert_eq!(consoles[0].1.model.as_deref(), Some("Opus"));
        assert_eq!(consoles[0].1.context_remaining_percent, Some(81.0));
    }

    #[tokio::test]
    async fn hook_handler_emits_mode_when_permission_mode_present() {
        let sink = CollectingSink::new();
        let router = make_router(Arc::clone(&sink) as Arc<dyn StatusSink>);
        let body = r#"{"hook_event_name":"PreToolUse","permission_mode":"auto"}"#;
        post_hook(&router, "/hook/agent-9", body).await;
        let consoles = sink.collected_consoles();
        assert_eq!(consoles.iter().filter(|(_, c)| c.mode.as_deref() == Some("auto")).count(), 1);
    }

}
