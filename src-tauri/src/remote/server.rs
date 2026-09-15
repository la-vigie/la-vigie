//! Remote axum server: GET / (static page), GET /api/state, POST /api/tasks.
//! Auth + Host checks read `RemoteState` per request. The write action reuses
//! the shared launch core + `task_launched` event, so the desktop
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

use crate::remote::{auth, session, webauthn};
use crate::state::AppState;
use crate::store::StoredCredential;
use webauthn_rs::prelude::{Passkey, PublicKeyCredential, RegisterPublicKeyCredential};

const INDEX_HTML: &str = include_str!("index.html");

/// `POST /api/tasks` body (camelCase from the phone client). The `agent`, `model`,
/// `base_branch`, and `auto_approve` fields are all optional — omitting
/// any of them preserves the minimal default-launch behavior exactly (each maps to
/// `None`).
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
/// body fields; when absent they stay `None`, which `resolve_launch`
/// resolves to the minimal default path (repo/global agent, repo
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
            // Placeholder default — the remote-control API doesn't expose
            // in-place launches.
            in_place: false,
            branch_name: None,
        },
        body.prompt,
    )
}

/// Reject unless: remote is active, the Host matches the MagicDNS allowlist, and
/// the request carries EITHER the bearer token (QR-pairing path) OR a valid
/// durable passkey session cookie. The Host anti-DNS-rebinding check
/// runs on every path regardless of which credential is presented.
fn authorize(app: &tauri::AppHandle, headers: &HeaderMap) -> Result<(), StatusCode> {
    let state = app.state::<AppState>();
    let mut remote = state.remote.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let active = remote.active.as_mut().ok_or(StatusCode::UNAUTHORIZED)?;

    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    if !auth::host_allowed(host, &active.magic_dns) {
        return Err(StatusCode::FORBIDDEN);
    }

    // Bearer token (QR pairing / first-pair / fallback).
    let presented = headers.get(header::AUTHORIZATION).and_then(|h| h.to_str().ok());
    if let Some(token) = auth::parse_bearer(presented) {
        if auth::constant_time_eq(token.as_bytes(), active.token.as_bytes()) {
            return Ok(());
        }
    }

    // Durable passkey session cookie: validate and roll the idle window.
    let cookie = headers.get(header::COOKIE).and_then(|h| h.to_str().ok());
    if let Some(sid) = session::parse_session_cookie(cookie) {
        if active
            .sessions
            .validate_and_roll(&sid, session::now_ms(), session::SESSION_TTL_MS)
            .is_some()
        {
            return Ok(());
        }
    }

    Err(StatusCode::UNAUTHORIZED)
}

/// A fresh high-entropy opaque id (two v4 UUIDs ⇒ 256 bits) for ceremony
/// correlation and session ids. Mirrors `commands::mint_token`.
fn new_id() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// The active remote's MagicDNS host, or `None` when remote is off.
fn current_magic_dns(app: &tauri::AppHandle) -> Option<String> {
    let state = app.state::<AppState>();
    let remote = state.remote.lock().ok()?;
    remote.active.as_ref().map(|a| a.magic_dns.clone())
}

/// Load every stored passkey (id + deserialized credential). A corrupt row is a
/// hard error — better to surface it than silently drop a credential.
fn load_passkeys(state: &AppState) -> Result<Vec<(String, Passkey)>, String> {
    let creds = {
        let store = state.store.lock().map_err(|e| format!("{e:#}"))?;
        store.list_credentials().map_err(|e| format!("{e:#}"))?
    };
    creds
        .into_iter()
        .map(|c| {
            serde_json::from_str::<Passkey>(&c.passkey_json)
                .map(|pk| (c.id.clone(), pk))
                .map_err(|e| format!("corrupt stored credential {}: {e}", c.id))
        })
        .collect()
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
        Ok(snapshot) => {
            let live_task_ids = live_agent_task_ids(state.inner());
            let mut value = match serde_json::to_value(snapshot) {
                Ok(value) => value,
                Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
            };
            if let Some(object) = value.as_object_mut() {
                object.insert("liveTaskIds".into(), json!(live_task_ids));
            }
            no_store(Json(value).into_response())
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// Snapshot the task ids whose mapped agent still has a live PTY. Kept separate
/// from the durable task status: an idle agent can still be running at its prompt.
fn live_agent_task_ids(state: &AppState) -> Vec<String> {
    let agent_tasks = state.agent_tasks.lock().map(|g| g.clone()).unwrap_or_default();
    let live = state.sessions.lock().map(|g| g.keys().cloned().collect::<std::collections::HashSet<_>>()).unwrap_or_default();
    let mut task_ids: Vec<String> = agent_tasks
        .into_iter()
        .filter_map(|(agent_id, task_id)| live.contains(&agent_id).then_some(task_id))
        .collect();
    task_ids.sort();
    task_ids.dedup();
    task_ids
}

/// `GET /api/agents` — the selectable agents (built-ins + custom), so the mobile
/// New-task form can offer an agent picker. Reuses the desktop
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
/// desktop `list_agent_models` core.
async fn agent_models_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    Path(agent_name): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let state = app.state::<AppState>();
    match crate::agent_commands::agent_models(state.inner(), &agent_name).await {
        Ok(models) => no_store(Json(models).into_response()),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// `GET /api/repos/{id}/branches` — the repo's branches for the base-branch picker
/// (default = repo default). Reuses the desktop `list_repo_branches` core.
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

    // The running desktop frontend's `useTaskLaunch` hook starts
    // the agent on this event (prompt rides the event, not the task row).
    use tauri::Emitter as _;
    let _ = app.emit(
        "task_launched",
        json!({ "taskId": task.id, "initialPrompt": prompt }),
    );

    no_store(Json(json!({ "taskId": task.id, "branch": task.branch })).into_response())
}

/// `POST /api/tasks/:id/resume` — ask the desktop frontend to mount its normal
/// terminal surface and resume the task's configured agent. PTY ownership stays
/// on the existing frontend path, preserving TerminalHost keep-alive behavior.
async fn resume_task_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let state = app.state::<AppState>();
    if live_agent_task_ids(state.inner()).iter().any(|id| id == &task_id) {
        return (StatusCode::CONFLICT, "agent is already running").into_response();
    }

    let resumable = {
        let store = match state.store.lock() {
            Ok(store) => store,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        };
        let task = match store.get_task(&task_id) {
            Ok(Some(task)) => task,
            Ok(None) => return (StatusCode::NOT_FOUND, "task not found").into_response(),
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        };
        if matches!(task.status, crate::store::TaskStatus::Done | crate::store::TaskStatus::Pending) {
            return (StatusCode::CONFLICT, "task cannot be resumed in its current state").into_response();
        }
        let repo = match store.get_repo(&task.repo_id) {
            Ok(Some(repo)) => repo,
            Ok(None) => return (StatusCode::NOT_FOUND, "repository not found").into_response(),
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        };
        let custom = match store.list_custom_agents() {
            Ok(custom) => custom,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        };
        !crate::agent::spec::resolve_for_task(task.agent.as_deref(), repo.default_agent.as_deref(), &custom)
            .resume_args
            .is_empty()
    };
    if !resumable {
        return (StatusCode::UNPROCESSABLE_ENTITY, "configured agent does not support resume").into_response();
    }

    use tauri::Emitter as _;
    if let Err(e) = app.emit("task_launched", json!({ "taskId": task_id, "resume": true })) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response();
    }
    StatusCode::ACCEPTED.into_response()
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
    crate::concierge::note_session_activity(state.inner(), &task_id);
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
    crate::concierge::note_session_activity(state.inner(), &task_id);

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

/// `POST /api/repos/:id/schedules` body. A non-empty `cron` ⇒ recurring;
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
    // Skip prepending the repo's initial prompt when this schedule fires.
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

/// `POST /api/schedules/:id/enabled` body.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetEnabledBody {
    enabled: bool,
}

/// `POST /api/tasks/:id/answer` — answer the task's pending `AskUserQuestion`
/// by translating the structured selection to picker keystrokes and
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
    crate::concierge::note_session_activity(state.inner(), &task_id);

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
/// core.
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
/// (inSeconds/atUnix) schedule. Validation / fire-time math stays in the cores;
/// `400` on an ambiguous mode, `422` on a validation failure.
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
/// next-run; one-shots keep their absolute fire time). Returns the updated schedule.
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

// ===========================================================================
// WebAuthn passkey auth endpoints.
//
// register/{begin,finish}       require an existing session (Bearer or cookie) —
//                               you register a passkey from an already-paired
//                               session (bootstrapped via QR).
// authenticate/{begin,finish}   Host-checked only — this IS the login, so it
//                               cannot require prior auth; a successful assertion
//                               mints the durable session cookie.
// logout / credentials CRUD     session management + revocation.
//
// All of these keep the Host anti-DNS-rebinding check (via `authorize`/`host_ok`).
// ===========================================================================

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisterFinishBody {
    ceremony_id: String,
    credential: RegisterPublicKeyCredential,
    #[serde(default)]
    label: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthenticateFinishBody {
    ceremony_id: String,
    credential: PublicKeyCredential,
}

/// Normalize a user-supplied passkey label: trimmed, capped, non-empty default.
fn clean_label(label: Option<String>) -> String {
    label
        .map(|s| s.trim().chars().take(64).collect::<String>())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Passkey".to_string())
}

/// `POST /api/auth/register/begin` — start registering a new passkey. Authed:
/// only an already-paired session may enroll a credential.
async fn register_begin_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    headers: HeaderMap,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let Some(magic_dns) = current_magic_dns(&app) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let state = app.state::<AppState>();
    let exclude = match load_passkeys(state.inner()) {
        Ok(pks) => pks.iter().map(|(_, pk)| pk.cred_id().clone()).collect(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let (ccr, reg_state) = match webauthn::start_registration(&magic_dns, exclude) {
        Ok(x) => x,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let ceremony_id = new_id();
    {
        let mut remote = match state.remote.lock() {
            Ok(r) => r,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        let Some(active) = remote.active.as_mut() else {
            return StatusCode::UNAUTHORIZED.into_response();
        };
        active.reg_states.insert(ceremony_id.clone(), reg_state);
    }
    no_store(Json(json!({ "ceremonyId": ceremony_id, "options": ccr })).into_response())
}

/// `POST /api/auth/register/finish` — complete registration, persist the passkey.
async fn register_finish_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let parsed: RegisterFinishBody = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    };
    let state = app.state::<AppState>();

    // Take the pending ceremony state (one-shot) + the host, under one lock.
    let (magic_dns, reg_state) = {
        let mut remote = match state.remote.lock() {
            Ok(r) => r,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        let Some(active) = remote.active.as_mut() else {
            return StatusCode::UNAUTHORIZED.into_response();
        };
        (active.magic_dns.clone(), active.reg_states.remove(&parsed.ceremony_id))
    };
    let Some(reg_state) = reg_state else {
        return (StatusCode::BAD_REQUEST, "unknown or expired registration ceremony").into_response();
    };

    let passkey = match webauthn::finish_registration(&magic_dns, &parsed.credential, &reg_state) {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let id = webauthn::cred_id_b64(passkey.cred_id());
    let passkey_json = match serde_json::to_string(&passkey) {
        Ok(j) => j,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    };
    let record = StoredCredential {
        id,
        passkey_json,
        label: clean_label(parsed.label),
        created_at: (session::now_ms() / 1000) as i64,
    };
    {
        let store = match state.store.lock() {
            Ok(s) => s,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        if let Err(e) = store.insert_credential(&record) {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response();
        }
    }
    no_store(Json(json!({ "ok": true, "id": record.id, "label": record.label })).into_response())
}

/// `POST /api/auth/authenticate/begin` — Host-only (this is the login). Returns a
/// challenge over the registered passkeys.
async fn authenticate_begin_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    headers: HeaderMap,
) -> Response {
    if !host_ok(&app, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(magic_dns) = current_magic_dns(&app) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let state = app.state::<AppState>();
    let passkeys = match load_passkeys(state.inner()) {
        Ok(pks) => pks,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    if passkeys.is_empty() {
        return (StatusCode::CONFLICT, "no passkeys registered").into_response();
    }
    let pks: Vec<Passkey> = passkeys.into_iter().map(|(_, pk)| pk).collect();
    let (rcr, auth_state) = match webauthn::start_authentication(&magic_dns, &pks) {
        Ok(x) => x,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let ceremony_id = new_id();
    {
        let mut remote = match state.remote.lock() {
            Ok(r) => r,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        let Some(active) = remote.active.as_mut() else {
            return StatusCode::UNAUTHORIZED.into_response();
        };
        active.auth_states.insert(ceremony_id.clone(), auth_state);
    }
    no_store(Json(json!({ "ceremonyId": ceremony_id, "options": rcr })).into_response())
}

/// `POST /api/auth/authenticate/finish` — Host-only. Verifies the assertion, bumps
/// the stored signature counter if needed, mints a durable session, sets the cookie.
async fn authenticate_finish_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !host_ok(&app, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let parsed: AuthenticateFinishBody = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    };
    let state = app.state::<AppState>();

    let (magic_dns, auth_state) = {
        let mut remote = match state.remote.lock() {
            Ok(r) => r,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        let Some(active) = remote.active.as_mut() else {
            return StatusCode::UNAUTHORIZED.into_response();
        };
        (active.magic_dns.clone(), active.auth_states.remove(&parsed.ceremony_id))
    };
    let Some(auth_state) = auth_state else {
        return (StatusCode::BAD_REQUEST, "unknown or expired authentication ceremony").into_response();
    };

    let result = match webauthn::finish_authentication(&magic_dns, &parsed.credential, &auth_state) {
        Ok(r) => r,
        Err(e) => return (StatusCode::UNAUTHORIZED, e).into_response(),
    };
    let cred_id = webauthn::cred_id_b64(result.cred_id());

    // Persist a bumped signature counter (best-effort; a stale counter is not fatal
    // and `finish_passkey_authentication` already rejected any regression).
    if let Ok(store) = state.store.lock() {
        if let Ok(creds) = store.list_credentials() {
            if let Some(c) = creds.iter().find(|c| c.id == cred_id) {
                if let Ok(mut pk) = serde_json::from_str::<Passkey>(&c.passkey_json) {
                    if let Some(true) = pk.update_credential(&result) {
                        if let Ok(js) = serde_json::to_string(&pk) {
                            let _ = store.update_credential_passkey(&cred_id, &js);
                        }
                    }
                }
            }
        }
    }

    // Mint the durable session bound to the authenticating credential.
    let sid = new_id();
    {
        let mut remote = match state.remote.lock() {
            Ok(r) => r,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        let Some(active) = remote.active.as_mut() else {
            return StatusCode::UNAUTHORIZED.into_response();
        };
        active.sessions.insert(
            sid.clone(),
            cred_id,
            session::now_ms(),
            session::SESSION_TTL_MS,
        );
    }
    let cookie = session::build_session_cookie(&sid, session::SESSION_COOKIE_MAX_AGE_SECS);
    let mut resp = Json(json!({ "ok": true })).into_response();
    if let Ok(v) = cookie.parse() {
        resp.headers_mut().insert(header::SET_COOKIE, v);
    }
    no_store(resp)
}

/// `POST /api/auth/logout` — Host-only. Drops the caller's session and clears the
/// cookie. Idempotent: succeeds even without a live session.
async fn logout_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    headers: HeaderMap,
) -> Response {
    if !host_ok(&app, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let cookie = headers.get(header::COOKIE).and_then(|h| h.to_str().ok());
    if let Some(sid) = session::parse_session_cookie(cookie) {
        let state = app.state::<AppState>();
        if let Ok(mut remote) = state.remote.lock() {
            if let Some(active) = remote.active.as_mut() {
                active.sessions.remove(&sid);
            }
        }
        drop(state);
    }
    let mut resp = Json(json!({ "ok": true })).into_response();
    if let Ok(v) = session::build_clearing_cookie().parse() {
        resp.headers_mut().insert(header::SET_COOKIE, v);
    }
    no_store(resp)
}

/// `GET /api/auth/credentials` — list registered passkeys (metadata only; the
/// passkey blob never leaves the server). Authed.
async fn list_credentials_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    headers: HeaderMap,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let state = app.state::<AppState>();
    let creds = {
        let store = match state.store.lock() {
            Ok(s) => s,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        match store.list_credentials() {
            Ok(c) => c,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        }
    };
    let view: Vec<_> = creds
        .into_iter()
        .map(|c| json!({ "id": c.id, "label": c.label, "createdAt": c.created_at }))
        .collect();
    no_store(Json(json!({ "credentials": view })).into_response())
}

/// `DELETE /api/auth/credentials/{id}` — revoke a passkey and drop every session it
/// minted (lost-device kill switch). Authed. `404` if the id is unknown.
async fn delete_credential_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(code) = authorize(&app, &headers) {
        return code.into_response();
    }
    let state = app.state::<AppState>();
    let deleted = {
        let store = match state.store.lock() {
            Ok(s) => s,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        match store.delete_credential(&id) {
            Ok(d) => d,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        }
    };
    if !deleted {
        return StatusCode::NOT_FOUND.into_response();
    }
    // Revoke live sessions minted by the deleted credential.
    if let Ok(mut remote) = state.remote.lock() {
        if let Some(active) = remote.active.as_mut() {
            active.sessions.remove_by_credential(&id);
        }
    }
    no_store(Json(json!({ "ok": true })).into_response())
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
        .route("/api/tasks/{id}/resume", post(resume_task_handler))
        .route("/api/concierge", post(concierge_handler))
        .route("/api/orchestrator/{repoId}", post(orchestrator_handler))
        .route("/api/tasks/{id}/session", get(session_handler))
        .route("/api/tasks/{id}/reply", post(reply_handler))
        .route("/api/tasks/{id}/answer", post(answer_handler))
        .route("/api/repos/{id}/schedules", get(list_schedules_handler).post(create_schedule_handler))
        .route("/api/schedules/{id}/enabled", post(set_schedule_enabled_handler))
        .route("/api/schedules/{id}", delete(delete_schedule_handler))
        // Passkey/WebAuthn auth + session management.
        .route("/api/auth/register/begin", post(register_begin_handler))
        .route("/api/auth/register/finish", post(register_finish_handler))
        .route("/api/auth/authenticate/begin", post(authenticate_begin_handler))
        .route("/api/auth/authenticate/finish", post(authenticate_finish_handler))
        .route("/api/auth/logout", post(logout_handler))
        .route(
            "/api/auth/credentials",
            get(list_credentials_handler),
        )
        .route("/api/auth/credentials/{id}", delete(delete_credential_handler))
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

    // The minimal body (only the always-present fields) must decode with
    // every optional field absent, and map each to `None`, preserving the
    // default-launch behavior.
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

    // Each optional field, when present, is honored and threaded into LaunchArgs.
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

    // `autoApprove: true` is a distinct third state from absent (`None`)
    // and `false`, so the tri-state round-trips faithfully.
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

    // ── Mobile schedules endpoints ─────────────────────────────────────────

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
