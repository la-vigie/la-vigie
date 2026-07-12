//! Remote axum server: GET / (static page), GET /api/state, POST /api/tasks.
//! Auth + Host checks read `RemoteState` per request. The write action reuses
//! the shared launch core + `task_launched` event (TASK-89), so the desktop
//! frontend spawns the agent. Glue is not unit-tested (verify live); the pure
//! body→args mapping is.

use axum::body::Bytes;
use axum::extract::{Path, Query, State as AxumState};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tauri::Manager as _;

use crate::remote::auth;
use crate::state::AppState;

const INDEX_HTML: &str = include_str!("index.html");

/// `POST /api/tasks` body (camelCase from the phone client).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskBody {
    pub repo_id: String,
    pub title: String,
    pub ticket_key: Option<String>,
    pub prompt: Option<String>,
}

/// `GET /api/tasks/:id/session` query: byte offset to read from (default 0).
#[derive(serde::Deserialize)]
pub struct SessionQuery {
    pub since: Option<usize>,
}

/// Map the request body to launch args + the prompt that rides `task_launched`.
/// `agent`/`model`/`base_branch`/`after_merge_of` default (repo/global agent,
/// no queueing) — matching the minimal New-Task path.
pub fn launch_args_from(body: CreateTaskBody) -> (crate::launch::LaunchArgs, Option<String>) {
    (
        crate::launch::LaunchArgs {
            repo_id: body.repo_id,
            title: body.title,
            base_branch: None,
            ticket_key: body.ticket_key,
            agent: None,
            model: None,
            auto_approve: None,
            after_merge_of: None,
            prompt: None,
        },
        body.prompt,
    )
}

/// Reject unless: remote is active, the Host matches the MagicDNS allowlist,
/// and the bearer token constant-time-equals the active token.
fn authorize(app: &tauri::AppHandle, headers: &HeaderMap) -> Result<(), StatusCode> {
    let state = app.state::<AppState>();
    let remote = state.remote.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let active = remote.active.as_ref().ok_or(StatusCode::UNAUTHORIZED)?;

    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    if !auth::host_allowed(host, &active.magic_dns) {
        return Err(StatusCode::FORBIDDEN);
    }
    let presented = headers.get(header::AUTHORIZATION).and_then(|h| h.to_str().ok());
    let token = auth::parse_bearer(presented).ok_or(StatusCode::UNAUTHORIZED)?;
    if !auth::constant_time_eq(token.as_bytes(), active.token.as_bytes()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

/// Host-check only (the page carries no secret) for `GET /`.
fn host_ok(app: &tauri::AppHandle, headers: &HeaderMap) -> bool {
    let state = app.state::<AppState>();
    let Ok(remote) = state.remote.lock() else { return false };
    let Some(active) = remote.active.as_ref() else { return false };
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    auth::host_allowed(host, &active.magic_dns)
}

fn no_store(mut resp: Response) -> Response {
    resp.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    resp
}

async fn index_handler(AxumState(app): AxumState<tauri::AppHandle>, headers: HeaderMap) -> Response {
    if !host_ok(&app, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    no_store(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], INDEX_HTML).into_response())
}

async fn state_handler(AxumState(app): AxumState<tauri::AppHandle>, headers: HeaderMap) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let state = app.state::<AppState>();
    match crate::commands::build_snapshot(state.inner()) {
        Ok(snapshot) => no_store(Json(snapshot).into_response()),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn create_task_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let parsed: CreateTaskBody = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    };
    let (launch_args, prompt) = launch_args_from(parsed);

    let task = {
        let state = app.state::<AppState>();
        crate::commands::launch_and_kickoff_setup(state.inner(), &app, launch_args).await
    };
    let task = match task {
        Ok(t) => t,
        Err(e) => return (StatusCode::UNPROCESSABLE_ENTITY, e).into_response(),
    };

    // Mirror TASK-89: the running desktop frontend's `useTaskLaunch` hook starts
    // the agent on this event (prompt rides the event, not the task row).
    use tauri::Emitter as _;
    let _ = app.emit(
        "task_launched",
        json!({ "taskId": task.id, "initialPrompt": prompt }),
    );

    no_store(Json(json!({ "taskId": task.id, "branch": task.branch })).into_response())
}

/// `GET /api/tasks/:id/session?since=<byteOffset>` — incremental transcript read.
/// Absent transcript (no hook yet) → `{ messages: [], cursor: 0 }` so the client
/// keeps polling. The `no_store_all` layer sets Cache-Control on the response.
async fn session_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    Path(task_id): Path<String>,
    Query(q): Query<SessionQuery>,
    headers: HeaderMap,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let state = app.state::<AppState>();
    crate::concierge::note_concierge_activity(state.inner(), &task_id);
    let read = match crate::session::read_session(state.inner(), &task_id, q.since.unwrap_or(0)) {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    Json(json!({ "messages": read.messages, "cursor": read.cursor })).into_response()
}

/// `POST /api/tasks/:id/reply` (plain-text body) — deliver a reply to the task's
/// live agent via bracketed paste + Enter. `409` if no agent is running.
async fn reply_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let text = String::from_utf8_lossy(&body).to_string();
    let state = app.state::<AppState>();
    crate::concierge::note_concierge_activity(state.inner(), &task_id);

    // Snapshot each map separately (clone, drop the guard) — never hold two PTY
    // locks at once — then resolve against the owned snapshots.
    let agent_id = {
        let agent_tasks = match state.agent_tasks.lock() {
            Ok(g) => g.clone(),
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        let live: std::collections::HashSet<String> = match state.sessions.lock() {
            Ok(g) => g.keys().cloned().collect(),
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        crate::session::resolve_live_agent(&agent_tasks, &live, &task_id)
    };
    let Some(agent_id) = agent_id else {
        return (StatusCode::CONFLICT, "no running agent for task").into_response();
    };

    // Deliver the reply as a bracketed paste, then submit with Enter. The Enter
    // MUST be a separate PTY read from the paste: if the two writes coalesce into
    // one read, Claude's TUI consumes the `\r` as part of the paste-end and the
    // text sits unsubmitted in the input buffer (replies then pile up and only
    // flush, concatenated, on a later input event). A short gap forces a distinct
    // read so the `\r` registers as a standalone submit — mirroring how a human
    // pastes and then presses Enter a moment later.
    let paste = crate::session::bracketed_paste(&text);
    if let Err(e) = crate::agent::write_to_session(state.inner(), &agent_id, &paste) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    if let Err(e) = crate::agent::write_to_session(state.inner(), &agent_id, "\r") {
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }
    StatusCode::OK.into_response()
}

/// `POST /api/concierge` — ensure a live concierge session (create or resume).
/// Idempotent. Returns the sentinel id the client uses to address the concierge
/// via the existing `/api/tasks/:id/session` + `/reply` routes.
async fn concierge_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    headers: HeaderMap,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let state = app.state::<AppState>();
    match crate::concierge::ensure_concierge(state.inner()) {
        Ok(()) => Json(json!({ "id": crate::concierge::CONCIERGE_SENTINEL })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// Middleware that sets `Cache-Control: no-store` on every response, including
/// error responses, so browsers and proxies never cache remote-control data.
async fn no_store_all(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let mut resp = next.run(req).await;
    resp.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    resp
}

/// Bind an ephemeral loopback port, serve until `shutdown` fires, and return
/// the port + the shutdown sender. The token/Host checks live in the handlers
/// (reading `RemoteState`), so the server holds no secret itself.
pub async fn start_remote_server(
    app: tauri::AppHandle,
) -> std::io::Result<(u16, tokio::sync::oneshot::Sender<()>)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();

    let router: Router = Router::new()
        .route("/", get(index_handler))
        .route("/api/state", get(state_handler))
        .route("/api/tasks", post(create_task_handler))
        .route("/api/concierge", post(concierge_handler))
        .route("/api/tasks/{id}/session", get(session_handler))
        .route("/api/tasks/{id}/reply", post(reply_handler))
        .with_state(app)
        .layer(axum::middleware::from_fn(no_store_all));

    tauri::async_runtime::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            })
            .await;
    });

    Ok((port, tx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_args_from_maps_fields_and_splits_prompt() {
        let body = CreateTaskBody {
            repo_id: "r1".into(),
            title: "Do thing".into(),
            ticket_key: Some("TASK-99".into()),
            prompt: Some("go".into()),
        };
        let (args, prompt) = launch_args_from(body);
        assert_eq!(args.repo_id, "r1");
        assert_eq!(args.title, "Do thing");
        assert_eq!(args.ticket_key.as_deref(), Some("TASK-99"));
        assert_eq!(args.agent, None);
        assert_eq!(args.base_branch, None);
        assert_eq!(args.model, None);
        assert_eq!(args.after_merge_of, None);
        assert_eq!(prompt.as_deref(), Some("go"));
    }
}
