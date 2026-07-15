//! Remote axum server: GET / (static page), GET /api/state, POST /api/tasks.
//! Auth + Host checks read `RemoteState` per request. The write action reuses
//! the shared launch core + `task_launched` event (TASK-89), so the desktop
//! frontend spawns the agent. Glue is not unit-tested (verify live); the pure
//! body→args mapping is.

use axum::body::Bytes;
use axum::extract::{Path, Query, State as AxumState};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::json;
use tauri::Manager as _;

use crate::remote::auth;
use crate::state::AppState;

const INDEX_HTML: &str = include_str!("index.html");

/// `POST /api/tasks` body (camelCase from the phone client). The `agent`, `model`,
/// `base_branch`, and `auto_approve` fields are all optional (TASK-199) — omitting
/// any of them preserves the minimal default-launch behavior exactly (each maps to
/// the same `None` the field carried before).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskBody {
    pub repo_id: String,
    pub title: String,
    pub ticket_key: Option<String>,
    pub prompt: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub base_branch: Option<String>,
    #[serde(default)]
    pub auto_approve: Option<bool>,
}

/// `GET /api/tasks/:id/session` query: byte offset to read from (default 0).
#[derive(serde::Deserialize)]
pub struct SessionQuery {
    pub since: Option<usize>,
}

/// Map the request body to launch args + the prompt that rides `task_launched`.
/// `agent`/`model`/`base_branch`/`auto_approve` are threaded from the (optional)
/// body fields (TASK-199); when absent they stay `None`, which `resolve_launch`
/// treats identically to the pre-TASK-199 minimal path (repo/global agent, repo
/// default branch, inherited auto-approve). `after_merge_of` stays empty — the
/// remote-control API does not expose dependency queueing.
pub fn launch_args_from(body: CreateTaskBody) -> (crate::launch::LaunchArgs, Option<String>) {
    (
        crate::launch::LaunchArgs {
            repo_id: body.repo_id,
            title: body.title,
            base_branch: body.base_branch,
            ticket_key: body.ticket_key,
            agent: body.agent,
            model: body.model,
            auto_approve: body.auto_approve,
            after_merge_of: Vec::new(),
            prompt: None,
            // TASK-163: placeholder default — the remote-control API doesn't expose
            // in-place launches.
            in_place: false,
            branch_name: None,
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

/// `GET /api/agents` — the selectable agents (built-ins + custom), so the mobile
/// New-task form can offer an agent picker (TASK-199). Reuses the desktop
/// `list_agents` core. Lazy: fetched when the form opens, not on every poll.
async fn agents_handler(AxumState(app): AxumState<tauri::AppHandle>, headers: HeaderMap) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let state = app.state::<AppState>();
    match crate::agent_commands::agents_list(state.inner()) {
        Ok(agents) => no_store(Json(agents).into_response()),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// `GET /api/agents/{name}/models` — the models the named agent enumerates (empty
/// when it advertises none, e.g. Claude → free-text on the client). Reuses the
/// desktop `list_agent_models` core (TASK-199).
async fn agent_models_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    Path(agent_name): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let state = app.state::<AppState>();
    match crate::agent_commands::agent_models(state.inner(), &agent_name) {
        Ok(models) => no_store(Json(models).into_response()),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// `GET /api/repos/{id}/branches` — the repo's branches for the base-branch picker
/// (default = repo default). Reuses the desktop `list_repo_branches` core (TASK-199).
async fn repo_branches_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    Path(repo_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let state = app.state::<AppState>();
    match crate::commands::repo_branches(state.inner(), &repo_id).await {
        Ok(branches) => no_store(Json(branches).into_response()),
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
    let pending = state
        .pending_questions
        .lock()
        .ok()
        .and_then(|m| m.get(&task_id).cloned());
    Json(json!({ "messages": read.messages, "cursor": read.cursor, "pendingQuestion": pending })).into_response()
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

/// `POST /api/tasks/:id/answer` body: one answer per pending question, in order.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnswerBody {
    answers: Vec<crate::session::question::Answer>,
}

/// `POST /api/repos/:id/schedules` body (TASK-196). A non-empty `cron` ⇒ recurring;
/// `inSeconds`/`atUnix` ⇒ one-time. Exactly one mode must be given. Optional
/// `agent`/`model`/`baseBranch` mirror the desktop create forms.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScheduleCreateBody {
    name: String,
    prompt: String,
    cron: Option<String>,
    in_seconds: Option<i64>,
    at_unix: Option<i64>,
    agent: Option<String>,
    model: Option<String>,
    base_branch: Option<String>,
    // TASK-181: skip prepending the repo's initial prompt when this schedule fires.
    // Omitted ⇒ None ⇒ the core defaults it to `true` (the store/desktop default).
    skip_repo_prompt: Option<bool>,
}

enum ScheduleCreateKind {
    Recurring { cron: String },
    OneShot { in_seconds: Option<i64>, at_unix: Option<i64> },
}

/// Decide recurring vs one-time from the create body. Pure — the unit-test target.
/// A non-empty `cron` ⇒ recurring; `inSeconds`/`atUnix` ⇒ one-time. Supplying both
/// (or neither) is a client error. Cron / fire-time validation itself stays in the
/// schedule cores (`validate_schedule_fields` / `resolve_fire_at`).
fn schedule_create_kind(body: &ScheduleCreateBody) -> Result<ScheduleCreateKind, String> {
    let has_cron = body.cron.as_deref().map(|c| !c.trim().is_empty()).unwrap_or(false);
    let has_once = body.in_seconds.is_some() || body.at_unix.is_some();
    match (has_cron, has_once) {
        (true, false) => Ok(ScheduleCreateKind::Recurring {
            cron: body.cron.clone().unwrap(),
        }),
        (false, true) => Ok(ScheduleCreateKind::OneShot {
            in_seconds: body.in_seconds,
            at_unix: body.at_unix,
        }),
        (true, true) => Err("provide either a cron (recurring) or a one-time time, not both".to_string()),
        (false, false) => Err("provide a cron (recurring) or inSeconds/atUnix (one-time)".to_string()),
    }
}

/// `POST /api/schedules/:id/enabled` body (TASK-196).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetEnabledBody {
    enabled: bool,
}

/// `POST /api/tasks/:id/answer` — answer the task's pending `AskUserQuestion`
/// (TASK-122) by translating the structured selection to picker keystrokes and
/// writing them to the live agent's PTY. `409` if there is no pending question
/// or no running agent; `400` on a selection that doesn't match the questions.
async fn answer_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let parsed: AnswerBody = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    };
    let state = app.state::<AppState>();
    crate::concierge::note_concierge_activity(state.inner(), &task_id);

    // The pending questions carry the shapes needed to translate the answer.
    let pending = match state.pending_questions.lock() {
        Ok(g) => g.get(&task_id).cloned(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let Some(pending) = pending else {
        return (StatusCode::CONFLICT, "no pending question for task").into_response();
    };
    let chunks = match crate::session::question::questions_to_keystrokes(&pending.questions, &parsed.answers) {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };

    // Resolve the live agent (same lock discipline as reply_handler).
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

    // Write each chunk as a separate PTY read with a short gap so Enter registers
    // as a distinct submit (mirrors the reply-handler paste/Enter split).
    for (i, chunk) in chunks.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
        if let Err(e) = crate::agent::write_to_session(state.inner(), &agent_id, chunk) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
    }

    // Answered — clear the card so the next poll drops it.
    let _ = state.pending_questions.lock().map(|mut m| m.remove(&task_id));
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

/// `POST /api/orchestrator/{repoId}` — ensure a live per-repo orchestrator
/// session (create or resume). Idempotent. Returns the session key the client
/// uses to address the orchestrator via the existing task-keyed
/// `/api/tasks/:id/session` + `/reply` routes.
async fn orchestrator_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    Path(repo_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let state = app.state::<AppState>();
    match crate::concierge::ensure_orchestrator(state.inner(), &repo_id) {
        Ok(()) => Json(json!({ "id": format!("orchestrator:{repo_id}") })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// `GET /api/repos/:id/schedules` — list a repo's schedules (recurring + one-shot),
/// each with `nextRunAt`, `enabled`, `oneShot`. Reuses the store CRUD via the shared
/// core (TASK-196).
async fn list_schedules_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    Path(repo_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let state = app.state::<AppState>();
    match crate::schedule_commands::list_schedules_core(state.inner(), &repo_id) {
        Ok(schedules) => no_store(Json(schedules).into_response()),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// `POST /api/repos/:id/schedules` — create a recurring (cron) or one-time
/// (inSeconds/atUnix) schedule. Validation / fire-time math stays in the cores
/// (TASK-196); `400` on an ambiguous mode, `422` on a validation failure.
async fn create_schedule_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    Path(repo_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let parsed: ScheduleCreateBody = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    };
    let kind = match schedule_create_kind(&parsed) {
        Ok(k) => k,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let state = app.state::<AppState>();
    let created = match kind {
        ScheduleCreateKind::Recurring { cron } => crate::schedule_commands::create_schedule_core(
            state.inner(), repo_id, parsed.name, parsed.prompt, cron,
            parsed.agent, parsed.model, parsed.base_branch, parsed.skip_repo_prompt,
        ),
        ScheduleCreateKind::OneShot { in_seconds, at_unix } => {
            crate::schedule_commands::create_one_shot_core(
                state.inner(), repo_id, parsed.name, parsed.prompt, in_seconds, at_unix,
                parsed.agent, parsed.model, parsed.base_branch, parsed.skip_repo_prompt,
            )
        }
    };
    match created {
        Ok(s) => no_store(Json(s).into_response()),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e).into_response(),
    }
}

/// `POST /api/schedules/:id/enabled` — arm/disarm a schedule (recurring recomputes
/// next-run; one-shots keep their absolute fire time). Returns the updated schedule
/// (TASK-196).
async fn set_schedule_enabled_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let parsed: SetEnabledBody = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    };
    let state = app.state::<AppState>();
    match crate::schedule_commands::set_schedule_enabled_core(state.inner(), id, parsed.enabled) {
        Ok(s) => no_store(Json(s).into_response()),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e).into_response(),
    }
}

/// `DELETE /api/schedules/:id` — remove a schedule (idempotent at the store level).
/// TASK-196.
async fn delete_schedule_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let state = app.state::<AppState>();
    match crate::schedule_commands::delete_schedule_core(state.inner(), &id) {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
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
        .route("/api/agents", get(agents_handler))
        .route("/api/agents/{name}/models", get(agent_models_handler))
        .route("/api/repos/{id}/branches", get(repo_branches_handler))
        .route("/api/tasks", post(create_task_handler))
        .route("/api/concierge", post(concierge_handler))
        .route("/api/orchestrator/{repoId}", post(orchestrator_handler))
        .route("/api/tasks/{id}/session", get(session_handler))
        .route("/api/tasks/{id}/reply", post(reply_handler))
        .route("/api/tasks/{id}/answer", post(answer_handler))
        .route("/api/repos/{id}/schedules", get(list_schedules_handler).post(create_schedule_handler))
        .route("/api/schedules/{id}/enabled", post(set_schedule_enabled_handler))
        .route("/api/schedules/{id}", delete(delete_schedule_handler))
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
            agent: None,
            model: None,
            base_branch: None,
            auto_approve: None,
        };
        let (args, prompt) = launch_args_from(body);
        assert_eq!(args.repo_id, "r1");
        assert_eq!(args.title, "Do thing");
        assert_eq!(args.ticket_key.as_deref(), Some("TASK-99"));
        assert_eq!(args.agent, None);
        assert_eq!(args.base_branch, None);
        assert_eq!(args.model, None);
        assert!(args.after_merge_of.is_empty());
        assert_eq!(prompt.as_deref(), Some("go"));
    }

    // TASK-199: the minimal body (only the always-present fields) must decode with
    // every optional field absent, and map to the exact pre-TASK-199 `None`s so the
    // default-launch behavior is preserved.
    #[test]
    fn minimal_body_omits_optional_fields_and_defaults_to_none() {
        let json = r#"{"repoId":"r1","title":"Do thing"}"#;
        let body: CreateTaskBody = serde_json::from_str(json).unwrap();
        assert_eq!(body.agent, None);
        assert_eq!(body.model, None);
        assert_eq!(body.base_branch, None);
        assert_eq!(body.auto_approve, None);
        let (args, prompt) = launch_args_from(body);
        assert_eq!(args.agent, None);
        assert_eq!(args.model, None);
        assert_eq!(args.base_branch, None);
        assert_eq!(args.auto_approve, None);
        assert!(args.after_merge_of.is_empty());
        assert!(!args.in_place);
        assert_eq!(prompt, None);
    }

    // TASK-199: each optional field, when present, is honored and threaded into
    // LaunchArgs (replacing the hard-coded `None`s).
    #[test]
    fn optional_fields_are_honored_when_present() {
        let json = r#"{
            "repoId":"r1","title":"Do thing",
            "agent":"opencode","model":"gpt-5","baseBranch":"develop","autoApprove":false
        }"#;
        let body: CreateTaskBody = serde_json::from_str(json).unwrap();
        let (args, _prompt) = launch_args_from(body);
        assert_eq!(args.agent.as_deref(), Some("opencode"));
        assert_eq!(args.model.as_deref(), Some("gpt-5"));
        assert_eq!(args.base_branch.as_deref(), Some("develop"));
        assert_eq!(args.auto_approve, Some(false));
    }

    // TASK-199: `autoApprove: true` is a distinct third state from absent (`None`)
    // and `false`, so the TASK-135 tri-state round-trips faithfully.
    #[test]
    fn auto_approve_true_is_honored() {
        let json = r#"{"repoId":"r1","title":"t","autoApprove":true}"#;
        let body: CreateTaskBody = serde_json::from_str(json).unwrap();
        let (args, _) = launch_args_from(body);
        assert_eq!(args.auto_approve, Some(true));
    }

    #[test]
    fn answer_body_parses_mixed_answers() {
        let body = r#"{"answers":[{"optionIndices":[1]},{"custom":"tabs"}]}"#;
        let parsed: AnswerBody = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.answers.len(), 2);
        assert_eq!(
            parsed.answers[0],
            crate::session::question::Answer::Options { option_indices: vec![1] }
        );
        assert_eq!(
            parsed.answers[1],
            crate::session::question::Answer::Custom { custom: "tabs".into() }
        );
    }

    // ── TASK-196: mobile schedules endpoints ───────────────────────────────

    #[test]
    fn schedule_create_kind_recurring_when_cron_present() {
        let body: ScheduleCreateBody = serde_json::from_value(serde_json::json!({
            "name": "weekly scan", "prompt": "/security-scan", "cron": "0 7 * * 1"
        })).unwrap();
        assert!(matches!(
            schedule_create_kind(&body),
            Ok(ScheduleCreateKind::Recurring { cron }) if cron == "0 7 * * 1"
        ));
    }

    #[test]
    fn schedule_create_kind_one_shot_when_in_seconds_present() {
        let body: ScheduleCreateBody = serde_json::from_value(serde_json::json!({
            "name": "later", "prompt": "/foo", "inSeconds": 3600
        })).unwrap();
        assert!(matches!(
            schedule_create_kind(&body),
            Ok(ScheduleCreateKind::OneShot { in_seconds: Some(3600), at_unix: None })
        ));
    }

    #[test]
    fn schedule_create_kind_one_shot_when_at_unix_present() {
        let body: ScheduleCreateBody = serde_json::from_value(serde_json::json!({
            "name": "at", "prompt": "/foo", "atUnix": 4102444800_i64
        })).unwrap();
        assert!(matches!(
            schedule_create_kind(&body),
            Ok(ScheduleCreateKind::OneShot { in_seconds: None, at_unix: Some(_) })
        ));
    }

    #[test]
    fn schedule_create_kind_rejects_both_cron_and_one_shot() {
        let body: ScheduleCreateBody = serde_json::from_value(serde_json::json!({
            "name": "x", "prompt": "/foo", "cron": "0 7 * * 1", "inSeconds": 60
        })).unwrap();
        assert!(schedule_create_kind(&body).is_err());
    }

    #[test]
    fn schedule_create_kind_rejects_neither() {
        let body: ScheduleCreateBody = serde_json::from_value(serde_json::json!({
            "name": "x", "prompt": "/foo"
        })).unwrap();
        assert!(schedule_create_kind(&body).is_err());
    }

    #[test]
    fn schedule_create_kind_treats_blank_cron_as_absent() {
        // A whitespace-only cron with no one-shot fields is "neither", not recurring.
        let body: ScheduleCreateBody = serde_json::from_value(serde_json::json!({
            "name": "x", "prompt": "/foo", "cron": "   "
        })).unwrap();
        assert!(schedule_create_kind(&body).is_err());
    }

    #[test]
    fn set_enabled_body_parses_camel_case() {
        let b: SetEnabledBody = serde_json::from_value(serde_json::json!({ "enabled": false })).unwrap();
        assert!(!b.enabled);
    }
}
