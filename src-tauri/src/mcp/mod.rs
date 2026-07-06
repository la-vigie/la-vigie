//! In-process MCP server (AC2-89): exposes `start_task` and `list_repos` to
//! spawned Claude agents over a loopback HTTP JSON-RPC endpoint, so an agent can
//! self-dispatch a La Vigie task.
//!
//! Two layers, mirroring `hooks/`:
//!   * pure (`route`, `*_response`, schemas): JSON-RPC parsing/dispatch +
//!     result formatting — unit-tested here, no I/O.
//!   * async glue (the axum handler + `start_mcp_server`): auth, the launch
//!     side effects, and event emission — verified via the app, not unit-tested.

use serde_json::{json, Value};

use axum::extract::State as AxumState;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use tauri::Manager as _;

use crate::state::AppState;

/// MCP protocol version this server speaks. Echoed in `initialize`.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// Error returned when a non-concierge token calls a concierge-only read tool.
const CONCIERGE_REQUIRED: &str = "This tool requires a concierge-scope token.";

/// Resolved scope of an MCP call (from the bearer token's tier).
#[derive(Debug, Clone)]
pub enum CallContext {
    /// Per-agent, repo-scoped token (AC2-89). `task_id` carried for future
    /// audit/chaining; `repo_id` is the default target for `start_task`.
    Agent {
        #[allow(dead_code)]
        task_id: String,
        repo_id: String,
    },
    /// Broad-scope concierge token: cross-repo reads (AC2-111).
    Concierge,
}

impl CallContext {
    #[allow(dead_code)]
    pub fn is_concierge(&self) -> bool {
        matches!(self, CallContext::Concierge)
    }
}

/// Pure mapping from a stored token tier to a call context.
fn context_from_token(tok: &crate::state::McpToken) -> CallContext {
    match tok {
        crate::state::McpToken::Agent(c) => CallContext::Agent {
            task_id: c.task_id.clone(),
            repo_id: c.repo_id.clone(),
        },
        crate::state::McpToken::Concierge => CallContext::Concierge,
    }
}

/// Parsed `start_task` tool arguments (all optional at the wire; identity is
/// enforced downstream by `launch_task`'s `has_task_identity`).
#[derive(Debug, Default, PartialEq)]
pub struct StartTaskArgs {
    pub title: Option<String>,
    pub ticket_key: Option<String>,
    pub repo: Option<String>,
    pub agent: Option<String>,
    /// Optional initial prompt for the new agent. Combined with the repo's
    /// configured prompt (repo prefix + this), mirroring the New-Task form.
    pub prompt: Option<String>,
}

/// One repo as returned by `list_repos`.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RepoSummary {
    pub id: String,
    pub name: String,
    pub default_branch: String,
}

/// One task as returned by `list_tasks` / `task_status`.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    pub id: String,
    pub repo_id: String,
    pub title: String,
    /// Snake-case status string (idle/working/needs_attention/done/error).
    pub status: String,
    pub branch: String,
    pub ticket_key: Option<String>,
    pub pr_number: Option<i64>,
    pub updated_at: i64,
}

/// Pure mapping from a store `Task` to the wire summary.
fn task_summary(t: &crate::store::Task) -> TaskSummary {
    TaskSummary {
        id: t.id.clone(),
        repo_id: t.repo_id.clone(),
        title: t.title.clone(),
        status: t.status.as_str().to_string(),
        branch: t.branch.clone(),
        ticket_key: t.ticket_key.clone(),
        pr_number: t.pr_number,
        updated_at: t.updated_at,
    }
}

/// The task created by `start_task`, summarized for the tool result.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatedTask {
    pub id: String,
    pub branch: String,
    pub worktree_path: String,
}

/// The outcome of routing a JSON-RPC request: an immediate response, nothing
/// (a notification), or a tool invocation the async handler must execute.
#[derive(Debug)]
pub enum Routed {
    /// No response (the request was a notification — no `id`).
    None,
    /// A complete response ready to return (initialize / tools/list / errors).
    Respond(Value),
    StartTask { id: Value, args: StartTaskArgs },
    ListRepos { id: Value },
    ListTasks { id: Value },
    TaskStatus { id: Value, task_id: String },
    GetTaskActivity { id: Value, task_id: String, since: usize },
}

fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// An MCP `tools/call` result body: a single text content block plus the
/// `isError` flag (true ⇒ the model sees a tool failure, not a protocol error).
fn tool_text(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "la-vigie", "version": env!("CARGO_PKG_VERSION") }
    })
}

fn tools_list_result() -> Value {
    json!({
        "tools": [
            {
                "name": "start_task",
                "description": "Create and start a La Vigie task (git worktree + branch + agent). \
Defaults to the calling agent's repo; pass `repo` to target a different one. \
Provide a `title` and/or a `ticketKey` (the task's branch derives from the ticket key).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "Human title for the task." },
                        "ticketKey": { "type": "string", "description": "Provider ticket id (e.g. AC2-99); links the task and seeds its branch name." },
                        "repo": { "type": "string", "description": "Target repo id. Defaults to the calling agent's repo. Use list_repos to discover ids." },
                        "agent": { "type": "string", "description": "Optional agent name to launch (defaults to the repo/global default)." },
                        "prompt": { "type": "string", "description": "Optional initial prompt for the new agent. Combined with the repo's configured prompt." }
                    }
                }
            },
            {
                "name": "list_repos",
                "description": "List the repos registered in La Vigie so you can pick or confirm a target for start_task.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "list_tasks",
                "description": "List La Vigie tasks across all repos with their current status. Requires a concierge-scope token.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "task_status",
                "description": "Get one task's current status and metadata by id. Requires a concierge-scope token.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "taskId": { "type": "string", "description": "The task id." } },
                    "required": ["taskId"]
                }
            },
            {
                "name": "get_task_activity",
                "description": "Read a task's recent agent conversation/activity (chat-shaped messages). Poll incrementally by passing the returned `cursor` as `since`. Requires a concierge-scope token.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "taskId": { "type": "string", "description": "The task id." },
                        "since": { "type": "integer", "description": "Byte offset cursor from a prior call; omit or 0 to read from the start." }
                    },
                    "required": ["taskId"]
                }
            }
        ]
    })
}

fn opt_str(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|s| s.to_string())
}

/// Extract a required non-empty `taskId` string argument.
fn require_task_id(arguments: &Value) -> Result<String, String> {
    match opt_str(arguments, "taskId") {
        Some(s) if !s.trim().is_empty() => Ok(s),
        _ => Err("missing required argument: taskId".to_string()),
    }
}

fn parse_start_task_args(arguments: &Value) -> StartTaskArgs {
    StartTaskArgs {
        title: opt_str(arguments, "title"),
        ticket_key: opt_str(arguments, "ticketKey"),
        repo: opt_str(arguments, "repo"),
        agent: opt_str(arguments, "agent"),
        prompt: opt_str(arguments, "prompt"),
    }
}

/// Route a single JSON-RPC request. Pure: no I/O. Notifications (no `id`) yield
/// `Routed::None`. `initialize`, `tools/list`, and error cases yield a ready
/// `Respond`; tool calls yield a `StartTask`/`ListRepos` for the async handler.
pub fn route(req: &Value) -> Routed {
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = match req.get("id") {
        Some(id) if !id.is_null() => id.clone(),
        _ => return Routed::None, // notification: never respond
    };

    match method {
        "initialize" => Routed::Respond(success(id, initialize_result())),
        "tools/list" => Routed::Respond(success(id, tools_list_result())),
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match name {
                "start_task" => Routed::StartTask { id, args: parse_start_task_args(&arguments) },
                "list_repos" => Routed::ListRepos { id },
                "list_tasks" => Routed::ListTasks { id },
                "task_status" => match require_task_id(&arguments) {
                    Ok(task_id) => Routed::TaskStatus { id, task_id },
                    Err(e) => Routed::Respond(rpc_error(id, -32602, &e)),
                },
                "get_task_activity" => match require_task_id(&arguments) {
                    Ok(task_id) => {
                        let since = arguments.get("since").and_then(|s| s.as_u64()).unwrap_or(0) as usize;
                        Routed::GetTaskActivity { id, task_id, since }
                    }
                    Err(e) => Routed::Respond(rpc_error(id, -32602, &e)),
                },
                other => Routed::Respond(rpc_error(id, -32602, &format!("unknown tool: {other}"))),
            }
        }
        other => Routed::Respond(rpc_error(id, -32601, &format!("method not found: {other}"))),
    }
}

/// Format the `start_task` tool result (success ⇒ confirmation text; error ⇒
/// `isError` text so the calling model sees the failure).
pub fn start_task_response(id: Value, result: Result<CreatedTask, String>) -> Value {
    match result {
        Ok(task) => success(
            id,
            tool_text(
                format!(
                    "Started task {} on branch {} (worktree {}).",
                    task.id, task.branch, task.worktree_path
                ),
                false,
            ),
        ),
        Err(e) => success(id, tool_text(e, true)),
    }
}

/// Format the `list_repos` tool result. The repo list is embedded as pretty
/// JSON text inside the content block (camelCase via `RepoSummary`'s serde).
pub fn list_repos_response(id: Value, result: Result<Vec<RepoSummary>, String>) -> Value {
    match result {
        Ok(repos) => {
            let body = serde_json::to_string_pretty(&repos)
                .unwrap_or_else(|_| "[]".to_string());
            success(id, tool_text(body, false))
        }
        Err(e) => success(id, tool_text(e, true)),
    }
}

/// Format the `list_tasks` tool result (task list as pretty JSON text).
pub fn list_tasks_response(id: Value, result: Result<Vec<TaskSummary>, String>) -> Value {
    match result {
        Ok(tasks) => {
            let body = serde_json::to_string_pretty(&tasks).unwrap_or_else(|_| "[]".to_string());
            success(id, tool_text(body, false))
        }
        Err(e) => success(id, tool_text(e, true)),
    }
}

/// Format the `task_status` tool result (single task as pretty JSON; missing ⇒ isError).
pub fn task_status_response(id: Value, result: Result<TaskSummary, String>) -> Value {
    match result {
        Ok(task) => {
            let body = serde_json::to_string_pretty(&task).unwrap_or_else(|_| "{}".to_string());
            success(id, tool_text(body, false))
        }
        Err(e) => success(id, tool_text(e, true)),
    }
}

/// Format the `get_task_activity` tool result (`{messages, cursor}` as pretty JSON).
pub fn get_task_activity_response(id: Value, result: Result<crate::session::SessionRead, String>) -> Value {
    match result {
        Ok(read) => {
            let body = serde_json::to_string_pretty(&read).unwrap_or_else(|_| "{}".to_string());
            success(id, tool_text(body, false))
        }
        Err(e) => success(id, tool_text(e, true)),
    }
}

/// Tauri event payload announcing a task created via MCP self-dispatch. The
/// frontend's `useTaskLaunch` hook starts the agent on the existing path.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TaskLaunchedPayload {
    task_id: String,
    /// Caller-supplied prompt to deliver to the new agent, if any. Combined with
    /// the repo's prompt by the frontend; not persisted on the task row.
    initial_prompt: Option<String>,
}

/// Resolve the target repo for a launch: an explicit non-empty (trimmed)
/// override wins; otherwise fall back to the caller's originating repo.
fn resolve_repo_id(repo_override: Option<String>, fallback: &str) -> String {
    repo_override
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// Extract and validate the bearer token, returning the originating context.
/// Missing/unknown token ⇒ `None` (the handler replies 401).
fn resolve_token(app: &tauri::AppHandle, headers: &HeaderMap) -> Option<CallContext> {
    let raw = headers.get("authorization")?.to_str().ok()?;
    let token = raw.strip_prefix("Bearer ").unwrap_or(raw).trim();
    let state = app.state::<AppState>();
    let tokens = state.mcp_tokens.lock().ok()?;
    Some(context_from_token(tokens.get(token)?))
}

/// Execute `start_task`: resolve the repo (override → caller's context), run the
/// shared launch+setup path, and emit `task_launched`. Errors are surfaced with
/// `{e:#}` upstream (they already are — `launch_and_kickoff_setup` maps them).
async fn do_start_task(
    app: &tauri::AppHandle,
    ctx: &CallContext,
    args: StartTaskArgs,
) -> Result<CreatedTask, String> {
    let StartTaskArgs { title, ticket_key, repo, agent, prompt } = args;
    let CallContext::Agent { repo_id: fallback_repo, .. } = ctx else {
        return Err(
            "start_task is not available to the concierge token (acting is gated; see AC2-113)."
                .to_string(),
        );
    };
    let repo_id = resolve_repo_id(repo, fallback_repo);

    let launch_args = crate::launch::LaunchArgs {
        repo_id,
        title: title.unwrap_or_default(),
        base_branch: None,
        ticket_key,
        agent,
        model: None,
        after_merge_of: None,
    };

    let task = {
        let state = app.state::<AppState>();
        crate::commands::launch_and_kickoff_setup(state.inner(), app, launch_args).await?
    };

    // The prompt is delivered when the frontend starts the agent (not persisted
    // on the task row), so it rides on the event rather than LaunchArgs.
    use tauri::Emitter as _;
    let _ = app.emit(
        "task_launched",
        TaskLaunchedPayload { task_id: task.id.clone(), initial_prompt: prompt },
    );

    Ok(CreatedTask { id: task.id, branch: task.branch, worktree_path: task.worktree_path })
}

/// Read all repos as `RepoSummary` (brief store lock, no await).
fn list_repos(app: &tauri::AppHandle) -> Result<Vec<RepoSummary>, String> {
    let state = app.state::<AppState>();
    let store = state.store.lock().map_err(|e| format!("{e:#}"))?;
    let repos = store.list_repos().map_err(|e| format!("{e:#}"))?;
    Ok(repos
        .into_iter()
        .map(|r| RepoSummary { id: r.id, name: r.name, default_branch: r.default_branch })
        .collect())
}

/// Read all tasks across repos as `TaskSummary` (brief store lock, no await).
fn list_tasks(app: &tauri::AppHandle) -> Result<Vec<TaskSummary>, String> {
    let state = app.state::<AppState>();
    let store = state.store.lock().map_err(|e| format!("{e:#}"))?;
    let tasks = store.list_tasks().map_err(|e| format!("{e:#}"))?;
    Ok(tasks.iter().map(task_summary).collect())
}

/// Read a single task as `TaskSummary` (brief store lock, no await).
fn task_status(app: &tauri::AppHandle, task_id: &str) -> Result<TaskSummary, String> {
    let state = app.state::<AppState>();
    let store = state.store.lock().map_err(|e| format!("{e:#}"))?;
    match store.get_task(task_id).map_err(|e| format!("{e:#}"))? {
        Some(t) => Ok(task_summary(&t)),
        None => Err(format!("task not found: {task_id}")),
    }
}

/// Read a task's session activity via the shared session-read service.
fn get_task_activity(app: &tauri::AppHandle, task_id: &str, since: usize) -> Result<crate::session::SessionRead, String> {
    let state = app.state::<AppState>();
    crate::session::read_session(state.inner(), task_id, since)
}

/// `POST /mcp` — a single JSON-RPC request. Auth via bearer token; dispatch via
/// the pure `route`; tool calls performed here. Returns `application/json` for
/// responses and `202 Accepted` (empty) for notifications.
async fn mcp_handler(
    AxumState(app): AxumState<tauri::AppHandle>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let Some(ctx) = resolve_token(&app, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            return Json(rpc_error(Value::Null, -32700, "parse error")).into_response();
        }
    };

    match route(&req) {
        Routed::None => StatusCode::ACCEPTED.into_response(),
        Routed::Respond(resp) => Json(resp).into_response(),
        Routed::StartTask { id, args } => {
            let result = do_start_task(&app, &ctx, args).await;
            Json(start_task_response(id, result)).into_response()
        }
        Routed::ListRepos { id } => {
            let result = list_repos(&app);
            Json(list_repos_response(id, result)).into_response()
        }
        Routed::ListTasks { id } => {
            if !ctx.is_concierge() {
                return Json(list_tasks_response(id, Err(CONCIERGE_REQUIRED.to_string())))
                    .into_response();
            }
            let result = list_tasks(&app);
            Json(list_tasks_response(id, result)).into_response()
        }
        Routed::TaskStatus { id, task_id } => {
            if !ctx.is_concierge() {
                return Json(task_status_response(id, Err(CONCIERGE_REQUIRED.to_string())))
                    .into_response();
            }
            let result = task_status(&app, &task_id);
            Json(task_status_response(id, result)).into_response()
        }
        Routed::GetTaskActivity { id, task_id, since } => {
            if !ctx.is_concierge() {
                return Json(get_task_activity_response(id, Err(CONCIERGE_REQUIRED.to_string())))
                    .into_response();
            }
            let result = get_task_activity(&app, &task_id, since);
            Json(get_task_activity_response(id, result)).into_response()
        }
    }
}

/// `GET /mcp` — we do not offer a server→client SSE stream; signal that to
/// streamable-HTTP clients so they proceed with request/response only.
async fn mcp_get_handler() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

/// Start the MCP server. Binds an ephemeral loopback port, serves on the Tauri
/// runtime, and returns the chosen port (stored in `AppState.mcp_port`).
pub async fn start_mcp_server(app: tauri::AppHandle) -> std::io::Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    let router: Router = Router::new()
        .route("/mcp", post(mcp_handler).get(mcp_get_handler))
        .with_state(app);

    tauri::async_runtime::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn initialize_returns_protocol_and_capabilities() {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}});
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(resp["result"]["capabilities"]["tools"].is_object());
        assert_eq!(resp["result"]["serverInfo"]["name"], "la-vigie");
    }

    #[test]
    fn notification_yields_no_response() {
        let req = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        assert!(matches!(route(&req), Routed::None));
    }

    #[test]
    fn tools_list_advertises_both_tools_without_after_merge_of() {
        let req = json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}});
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        let tools = resp["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"start_task"));
        assert!(names.contains(&"list_repos"));
        let start = tools.iter().find(|t| t["name"] == "start_task").unwrap();
        let props = &start["inputSchema"]["properties"];
        assert!(props["title"].is_object());
        assert!(props["ticketKey"].is_object());
        assert!(props["repo"].is_object());
        assert!(props["agent"].is_object());
        // afterMergeOf is deferred to AC2-90 and must NOT be advertised in v1.
        assert!(props.get("afterMergeOf").is_none());
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let req = json!({"jsonrpc":"2.0","id":3,"method":"does/not/exist"});
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[test]
    fn tools_call_start_task_routes_with_parsed_args() {
        let req = json!({
            "jsonrpc":"2.0","id":4,"method":"tools/call",
            "params":{"name":"start_task","arguments":{"title":"Do thing","ticketKey":"AC2-99","repo":"r1"}}
        });
        let Routed::StartTask { id, args } = route(&req) else { panic!("expected StartTask") };
        assert_eq!(id, json!(4));
        assert_eq!(args.title.as_deref(), Some("Do thing"));
        assert_eq!(args.ticket_key.as_deref(), Some("AC2-99"));
        assert_eq!(args.repo.as_deref(), Some("r1"));
        assert_eq!(args.agent, None);
        assert_eq!(args.prompt, None);
    }

    #[test]
    fn tools_call_start_task_parses_prompt() {
        let req = json!({
            "jsonrpc":"2.0","id":7,"method":"tools/call",
            "params":{"name":"start_task","arguments":{"ticketKey":"AC2-99","prompt":"do the thing"}}
        });
        let Routed::StartTask { args, .. } = route(&req) else { panic!("expected StartTask") };
        assert_eq!(args.prompt.as_deref(), Some("do the thing"));
    }

    #[test]
    fn tools_list_start_task_schema_includes_prompt() {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}});
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        let tools = resp["result"]["tools"].as_array().unwrap();
        let start = tools.iter().find(|t| t["name"] == "start_task").unwrap();
        assert!(start["inputSchema"]["properties"]["prompt"].is_object());
    }

    #[test]
    fn tools_call_list_repos_routes() {
        let req = json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_repos","arguments":{}}});
        let Routed::ListRepos { id } = route(&req) else { panic!("expected ListRepos") };
        assert_eq!(id, json!(5));
    }

    #[test]
    fn tools_call_unknown_tool_is_invalid_params() {
        let req = json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"nope","arguments":{}}});
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        assert_eq!(resp["error"]["code"], -32602);
    }

    #[test]
    fn start_task_response_ok_is_non_error_text() {
        let resp = start_task_response(
            json!(7),
            Ok(CreatedTask { id: "t1".into(), branch: "ac2-99".into(), worktree_path: "/wt/ac2-99".into() }),
        );
        assert_eq!(resp["id"], 7);
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("t1"));
        assert!(text.contains("ac2-99"));
    }

    #[test]
    fn start_task_response_err_is_error_text() {
        let resp = start_task_response(json!(8), Err("repo not found: x".into()));
        assert_eq!(resp["result"]["isError"], true);
        assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("repo not found"));
    }

    #[test]
    fn list_repos_response_serializes_summaries_camelcase() {
        let resp = list_repos_response(
            json!(9),
            Ok(vec![RepoSummary { id: "r1".into(), name: "Repo One".into(), default_branch: "main".into() }]),
        );
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        // The repo list is embedded as JSON text; assert it carries the camelCase field.
        assert!(text.contains("defaultBranch"));
        assert!(text.contains("r1"));
    }

    #[test]
    fn resolve_repo_id_uses_override_when_present() {
        assert_eq!(resolve_repo_id(Some("other".to_string()), "caller"), "other");
    }

    #[test]
    fn resolve_repo_id_trims_override() {
        assert_eq!(resolve_repo_id(Some("  other  ".to_string()), "caller"), "other");
    }

    #[test]
    fn resolve_repo_id_falls_back_on_none_or_blank() {
        assert_eq!(resolve_repo_id(None, "caller"), "caller");
        assert_eq!(resolve_repo_id(Some("   ".to_string()), "caller"), "caller");
    }

    #[test]
    fn task_summary_maps_status_to_str_and_camelcase_fields() {
        use crate::store::{Task, TaskStatus};
        let t = Task {
            id: "t1".into(),
            repo_id: "r1".into(),
            title: "Do thing".into(),
            worktree_path: "/wt".into(),
            branch: "ac2-99".into(),
            base_branch: "main".into(),
            status: TaskStatus::Working,
            created_at: 1,
            updated_at: 2,
            pr_number: Some(7),
            pr_url: None,
            ticket_key: Some("AC2-99".into()),
            agent: None,
            model: None,
            setup_status: None,
            hidden: false,
        };
        let s = task_summary(&t);
        assert_eq!(s.id, "t1");
        assert_eq!(s.repo_id, "r1");
        assert_eq!(s.status, "working");
        assert_eq!(s.ticket_key.as_deref(), Some("AC2-99"));
        assert_eq!(s.pr_number, Some(7));
    }

    #[test]
    fn tools_call_list_tasks_routes() {
        let req = json!({"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"list_tasks","arguments":{}}});
        let Routed::ListTasks { id } = route(&req) else { panic!("expected ListTasks") };
        assert_eq!(id, json!(10));
    }

    #[test]
    fn list_tasks_response_serializes_camelcase_and_status() {
        let resp = list_tasks_response(
            json!(11),
            Ok(vec![TaskSummary {
                id: "t1".into(),
                repo_id: "r1".into(),
                title: "T".into(),
                status: "working".into(),
                branch: "b".into(),
                ticket_key: None,
                pr_number: None,
                updated_at: 5,
            }]),
        );
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("repoId"));
        assert!(text.contains("\"status\": \"working\""));
    }

    #[test]
    fn list_tasks_response_error_is_error_text() {
        let resp = list_tasks_response(json!(12), Err("boom".into()));
        assert_eq!(resp["result"]["isError"], true);
        assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("boom"));
    }

    #[test]
    fn tools_list_advertises_list_tasks() {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}});
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        let names: Vec<&str> = resp["result"]["tools"].as_array().unwrap()
            .iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"list_tasks"));
    }

    #[test]
    fn tools_call_task_status_routes_with_task_id() {
        let req = json!({"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"task_status","arguments":{"taskId":"t1"}}});
        let Routed::TaskStatus { id, task_id } = route(&req) else { panic!("expected TaskStatus") };
        assert_eq!(id, json!(13));
        assert_eq!(task_id, "t1");
    }

    #[test]
    fn tools_call_task_status_missing_id_is_invalid_params() {
        let req = json!({"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"task_status","arguments":{}}});
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        assert_eq!(resp["error"]["code"], -32602);
    }

    #[test]
    fn task_status_response_ok_and_not_found() {
        let ok = task_status_response(json!(15), Ok(TaskSummary {
            id: "t1".into(), repo_id: "r1".into(), title: "T".into(), status: "idle".into(),
            branch: "b".into(), ticket_key: None, pr_number: None, updated_at: 1,
        }));
        assert_eq!(ok["result"]["isError"], false);
        assert!(ok["result"]["content"][0]["text"].as_str().unwrap().contains("\"status\": \"idle\""));

        let nf = task_status_response(json!(16), Err("task not found: x".into()));
        assert_eq!(nf["result"]["isError"], true);
        assert!(nf["result"]["content"][0]["text"].as_str().unwrap().contains("not found"));
    }

    #[test]
    fn tools_call_get_task_activity_routes_with_since_default_zero() {
        let req = json!({"jsonrpc":"2.0","id":17,"method":"tools/call","params":{"name":"get_task_activity","arguments":{"taskId":"t1"}}});
        let Routed::GetTaskActivity { id, task_id, since } = route(&req) else { panic!("expected GetTaskActivity") };
        assert_eq!(id, json!(17));
        assert_eq!(task_id, "t1");
        assert_eq!(since, 0);
    }

    #[test]
    fn tools_call_get_task_activity_parses_since() {
        let req = json!({"jsonrpc":"2.0","id":18,"method":"tools/call","params":{"name":"get_task_activity","arguments":{"taskId":"t1","since":42}}});
        let Routed::GetTaskActivity { since, .. } = route(&req) else { panic!("expected GetTaskActivity") };
        assert_eq!(since, 42);
    }

    #[test]
    fn get_task_activity_response_embeds_messages_and_cursor() {
        use crate::session::{SessionMessage, SessionRead};
        let read = SessionRead {
            messages: vec![SessionMessage { role: "user".into(), text: Some("hi".into()), tool: None, ts: None }],
            cursor: 12,
        };
        let resp = get_task_activity_response(json!(19), Ok(read));
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"cursor\": 12"));
        assert!(text.contains("\"role\": \"user\""));
    }

    #[test]
    fn get_task_activity_response_error_is_error_text() {
        let resp = get_task_activity_response(json!(20), Err("io boom".into()));
        assert_eq!(resp["result"]["isError"], true);
        assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("io boom"));
    }

    #[test]
    fn context_from_token_maps_agent_and_concierge_tiers() {
        use crate::state::{AgentLaunchContext, McpToken};
        let agent = McpToken::Agent(AgentLaunchContext {
            task_id: "t1".into(),
            repo_id: "r1".into(),
        });
        match context_from_token(&agent) {
            CallContext::Agent { task_id, repo_id } => {
                assert_eq!(task_id, "t1");
                assert_eq!(repo_id, "r1");
            }
            _ => panic!("expected Agent"),
        }
        assert!(!context_from_token(&agent).is_concierge());

        let concierge = McpToken::Concierge;
        assert!(matches!(context_from_token(&concierge), CallContext::Concierge));
        assert!(context_from_token(&concierge).is_concierge());
    }
}
