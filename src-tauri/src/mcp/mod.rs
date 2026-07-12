//! In-process MCP server (TASK-89): exposes `start_task`, `finish_task`, and
//! `list_repos` to spawned Claude agents over a loopback HTTP JSON-RPC endpoint,
//! so an agent can self-dispatch (and self-/cross-tear-down) a La Vigie task.
//!
//! Two layers, mirroring `hooks/`:
//!   * pure (`route`, `*_response`, schemas): JSON-RPC parsing/dispatch +
//!     result formatting — unit-tested here, no I/O.
//!   * async glue (the axum handler + `start_mcp_server`): auth, the launch
//!     side effects, and event emission — verified via the app, not unit-tested.

use std::path::Path;

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
    /// Per-agent, repo-scoped token (TASK-89). `task_id` is the caller's own task
    /// (the default target for `finish_task`); `repo_id` is the default target
    /// for `start_task`.
    Agent {
        task_id: String,
        repo_id: String,
    },
    /// Broad-scope concierge token: cross-repo reads (TASK-111).
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
    /// Optional La Vigie task id to queue this task behind (start-on-merge,
    /// TASK-90). When set, the created task is `Pending` until that task is
    /// merged through La Vigie.
    pub after_merge_of: Option<String>,
}

/// Parsed `finish_task` tool arguments. `task_id` defaults to the caller's own
/// task (Agent tier); `mode` defaults to `"keep"`; `force` defaults to `false`.
#[derive(Debug, Default, PartialEq)]
pub struct FinishTaskArgs {
    pub task_id: Option<String>,
    pub mode: Option<String>,
    pub force: bool,
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
    /// TASK-90: true when the task was QUEUED (Pending) behind another task's
    /// landing rather than started now. Then branch/worktree_path are empty and
    /// the blocking_* fields describe the dependency it waits on.
    pub queued: bool,
    pub blocking_task_id: Option<String>,
    pub blocking_title: Option<String>,
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
    FinishTask { id: Value, args: FinishTaskArgs },
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
                        "ticketKey": { "type": "string", "description": "Provider ticket id (e.g. TASK-99); links the task and seeds its branch name." },
                        "repo": { "type": "string", "description": "Target repo id. Defaults to the calling agent's repo. Use list_repos to discover ids." },
                        "agent": { "type": "string", "description": "Optional agent name to launch (defaults to the repo/global default)." },
                        "prompt": { "type": "string", "description": "Optional initial prompt for the new agent. Combined with the repo's configured prompt." },
                        "afterMergeOf": { "type": "string", "description": "Queue this task to auto-start when the given La Vigie task id is merged through La Vigie (start-on-merge)." }
                    }
                }
            },
            {
                "name": "finish_task",
                "description": "Finish (tear down) a La Vigie task: stop its agent, remove the git worktree, and delete the task. \
Defaults to the calling agent's own task; pass `taskId` to target another (requires a concierge-scope token). \
`mode` is keep (default: leave the branch), discard (delete the branch), or merge (squash-merge the PR, then delete the branch). \
Refuses a task with uncommitted or unmerged work unless `force` is true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "taskId": { "type": "string", "description": "Task id to finish. Defaults to the calling agent's own task. Finishing another task requires a concierge-scope token." },
                        "mode": { "type": "string", "enum": ["keep", "discard", "merge"], "description": "keep (leave branch, default), discard (delete branch), or merge (squash-merge the PR then delete branch)." },
                        "force": { "type": "boolean", "description": "Tear down even with uncommitted or unmerged work. Defaults to false." }
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
        after_merge_of: opt_str(arguments, "afterMergeOf"),
    }
}

fn parse_finish_task_args(arguments: &Value) -> FinishTaskArgs {
    FinishTaskArgs {
        task_id: opt_str(arguments, "taskId"),
        mode: opt_str(arguments, "mode"),
        force: arguments.get("force").and_then(|f| f.as_bool()).unwrap_or(false),
    }
}

/// Error returned when a normal Agent-tier token tries to finish a task other
/// than its own — a cross-task teardown needs the concierge tier (TASK-111).
const FINISH_SCOPE_REQUIRED: &str =
    "Finishing another agent's task requires a concierge-scope token.";

/// Pure scope decision for `finish_task`: resolve the effective target task id
/// from the caller's context and the optional `taskId` arg, enforcing the tier
/// rule. An Agent token defaults to (and may only finish) its own task; a
/// Concierge token has no own task, so it must name one explicitly but may name
/// any.
fn resolve_finish_target(ctx: &CallContext, task_id_arg: Option<&str>) -> Result<String, String> {
    let arg = task_id_arg.map(|s| s.trim()).filter(|s| !s.is_empty());
    match ctx {
        CallContext::Agent { task_id: own, .. } => match arg {
            None => Ok(own.clone()),
            Some(t) if t == own => Ok(own.clone()),
            Some(_) => Err(FINISH_SCOPE_REQUIRED.to_string()),
        },
        CallContext::Concierge => match arg {
            Some(t) => Ok(t.to_string()),
            None => Err("finish_task from a concierge token requires an explicit taskId.".to_string()),
        },
    }
}

/// Pure validation of the finish `mode` (defaults to `keep`). Mirrors
/// `commands::finish_task`'s accepted set.
fn validate_finish_mode(mode: Option<&str>) -> Result<String, String> {
    match mode.map(|m| m.trim()).filter(|m| !m.is_empty()).unwrap_or("keep") {
        m @ ("keep" | "discard" | "merge") => Ok(m.to_string()),
        other => Err(format!("invalid finish mode: {other}")),
    }
}

/// Whether this finish tears down the caller's OWN task — an Agent token whose
/// task is the target. Such a teardown stops the caller's own PTY (the agent is
/// this request's HTTP client), which can cancel the request future mid-teardown,
/// so the destructive phase must be detached rather than awaited. A concierge
/// finishing another task is never self (the caller isn't the agent being stopped).
fn is_self_finish(ctx: &CallContext, target: &str) -> bool {
    matches!(ctx, CallContext::Agent { task_id, .. } if task_id == target)
}

/// Route a single JSON-RPC request. Pure: no I/O. Notifications (no `id`) yield
/// `Routed::None`. `initialize`, `tools/list`, and error cases yield a ready
/// `Respond`; a tool call yields its per-tool variant (`StartTask`, `FinishTask`,
/// `ListRepos`, …) for the async handler to execute.
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
                "finish_task" => Routed::FinishTask { id, args: parse_finish_task_args(&arguments) },
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
        Ok(task) if task.queued => {
            let dep = task.blocking_task_id.as_deref().unwrap_or("(unknown)");
            let title = task
                .blocking_title
                .as_deref()
                .map(|t| format!(" ({t})"))
                .unwrap_or_default();
            success(
                id,
                tool_text(
                    format!(
                        "Queued task {} (Pending) — will auto-start when task {dep}{title} lands. \
No worktree/branch yet; it unblocks automatically when that task's PR merges (detected at \
finish/teardown), or immediately via /finished with promote=true for a no-PR landing.",
                        task.id
                    ),
                    false,
                ),
            )
        }
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

/// Format the `finish_task` tool result. `Done` ⇒ confirmation text; a refusal
/// (`Unsafe`), an unknown task, or a scope/mode/gh error ⇒ `isError` text so the
/// calling model sees the failure rather than a silent success.
pub fn finish_task_response(id: Value, result: Result<crate::teardown::TeardownOutcome, String>) -> Value {
    use crate::teardown::TeardownOutcome;
    match result {
        Ok(TeardownOutcome::Done) => success(
            id,
            tool_text("Task finished: agent stopped, worktree removed, task deleted.".to_string(), false),
        ),
        Ok(TeardownOutcome::UnknownTask) => success(id, tool_text("task not found".to_string(), true)),
        Ok(TeardownOutcome::Unsafe(reason)) => success(
            id,
            tool_text(format!("refusing to finish: {reason} — pass force:true to override"), true),
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

/// Emit `task_launched` so the frontend's `useTaskLaunch` hook starts the agent
/// on the existing path. Shared by `do_start_task` (MCP self-dispatch) and the
/// TASK-90 merge-time promote path so both fire the identical event/payload.
pub(crate) fn emit_task_launched(app: &tauri::AppHandle, task_id: String, initial_prompt: Option<String>) {
    use tauri::Emitter as _;
    let _ = app.emit("task_launched", TaskLaunchedPayload { task_id, initial_prompt });
}

/// Tauri event payload announcing a task the frontend didn't initiate itself —
/// e.g. a pending/queued task created via MCP `start_task(afterMergeOf:…)`
/// that has no worktree/agent yet, so `task_launched` never fires for it. TASK-90.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TaskCreatedPayload {
    task_id: String,
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
    let StartTaskArgs { title, ticket_key, repo, agent, prompt, after_merge_of } = args;
    let CallContext::Agent { repo_id: fallback_repo, .. } = ctx else {
        return Err(
            "start_task is not available to the concierge token (acting is gated; see TASK-113)."
                .to_string(),
        );
    };
    let repo_id = resolve_repo_id(repo, fallback_repo);
    let blocking_id = after_merge_of.clone();

    let launch_args = crate::launch::LaunchArgs {
        repo_id,
        title: title.unwrap_or_default(),
        base_branch: None,
        ticket_key,
        agent,
        model: None,
        auto_approve: None,
        after_merge_of,
        prompt: prompt.clone(),
    };

    let task = {
        let state = app.state::<AppState>();
        crate::commands::launch_and_kickoff_setup(state.inner(), app, launch_args).await?
    };

    if task.status == crate::store::TaskStatus::Pending {
        // Queued behind another task's merge — no worktree/agent yet, so no
        // task_launched event. The landed-gated trigger promotes it when its
        // dependency lands (PR merged), from any finish surface (GUI
        // finish_task or the /finished teardown). TASK-90. Emit task_created so
        // the frontend refreshes and the queued task is visible in the sidebar
        // right away instead of only appearing on a later, unrelated refresh.
        use tauri::Emitter as _;
        let _ = app.emit("task_created", TaskCreatedPayload { task_id: task.id.clone() });

        // TASK-90: look up the blocking task's title so the MCP response can
        // name it (brief store lock, no await inside the guard).
        let blocking_title = blocking_id.as_deref().and_then(|dep| {
            let state = app.state::<AppState>();
            let store = state.store.lock().ok()?;
            store.get_task(dep).ok().flatten().map(|t| t.title)
        });

        return Ok(CreatedTask {
            id: task.id,
            branch: task.branch,
            worktree_path: task.worktree_path,
            queued: true,
            blocking_task_id: blocking_id,
            blocking_title,
        });
    }

    // The prompt is delivered when the frontend starts the agent (not persisted
    // on the task row), so it rides on the event rather than LaunchArgs.
    crate::mcp::emit_task_launched(app, task.id.clone(), prompt);

    Ok(CreatedTask {
        id: task.id,
        branch: task.branch,
        worktree_path: task.worktree_path,
        queued: false,
        blocking_task_id: None,
        blocking_title: None,
    })
}

/// Execute `finish_task`: enforce the scope tier, then compose the TASK-139
/// teardown core with the keep/discard/merge branch/PR semantics.
///
///   * scope — an Agent token may only finish its own task; another task needs
///     the concierge tier (`resolve_finish_target`).
///   * merge — squash-merge the PR *before* teardown (gh runs in the worktree,
///     which teardown removes); the safety gate then passes via `pr_merged`.
///   * teardown — stops the PTY, removes the worktree, and deletes the row,
///     behind the safety gate (refuse unsafe unless `force`). A self-finish (an
///     agent tearing down its own task) detaches the destructive phase so
///     stopping the caller's PTY can't cancel it mid-teardown; a cross-task
///     finish awaits it for an exact outcome.
///   * discard/merge — best-effort delete the branch once teardown is `Done`.
async fn do_finish_task(
    app: &tauri::AppHandle,
    ctx: &CallContext,
    args: FinishTaskArgs,
) -> Result<crate::teardown::TeardownOutcome, String> {
    use crate::teardown::TeardownOutcome;
    let FinishTaskArgs { task_id, mode, force } = args;
    let target = resolve_finish_target(ctx, task_id.as_deref())?;
    let mode = validate_finish_mode(mode.as_deref())?;
    let state = app.state::<AppState>();

    // Capture the task's worktree/branch/repo before teardown deletes the row —
    // needed by merge (PR) and by discard/merge (branch delete). An unknown task
    // short-circuits identically to the teardown core's own UnknownTask.
    let (worktree_path, branch, repo_path) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let task = match store.get_task(&target).map_err(|e| format!("{e:#}"))? {
            Some(t) => t,
            None => return Ok(TeardownOutcome::UnknownTask),
        };
        let repo_path = store
            .get_repo(&task.repo_id)
            .map_err(|e| format!("{e:#}"))?
            .map(|r| r.path);
        (task.worktree_path, task.branch, repo_path)
    };

    // merge mode: squash-merge the PR before teardown removes the worktree.
    // Gate the dirty-tree check BEFORE the irreversible merge (unless forced): the
    // safety gate otherwise runs inside teardown_task, so a dirty worktree would
    // merge the PR and *then* refuse — leaving a merged PR the caller can't finish
    // (a force retry would re-merge an already-merged PR and error). The unmerged-
    // commits half of the gate is precisely what the merge resolves, so a dirty
    // check is the only pre-merge safety needed.
    if mode == "merge" {
        if !force {
            let dirty = crate::git::working_tree_dirty(Path::new(&worktree_path))
                .await
                .map_err(|e| format!("{e:#}"))?;
            if dirty {
                return Ok(TeardownOutcome::Unsafe(
                    "worktree has uncommitted or untracked changes".to_string(),
                ));
            }
        }
        let pr = crate::github::pr_status(Path::new(&worktree_path), &branch)
            .await
            .map_err(|e| format!("{e:#}"))?;
        let pr_number = pr.ok_or_else(|| "no PR found for this task".to_string())?.number;
        crate::github::merge_pr(Path::new(&worktree_path), pr_number)
            .await
            .map_err(|e| format!("{e:#}"))?;
    }

    // For merge we already gated (dirty) and merged, so force teardown to skip the
    // now-redundant gate and dodge a pr_status "is it MERGED yet" race. Other modes
    // use the caller's force verbatim.
    let effective_force = force || mode == "merge";
    let delete_branch_after = mode == "discard" || mode == "merge";

    // Self-finish (an Agent tearing down its OWN task) stops the caller's PTY — the
    // agent IS this MCP request's HTTP client, so that drops the connection and can
    // cancel this handler future mid-teardown, between worktree removal and the
    // DB-row delete + `task_removed` emit (stranding the task in the UI). Mirror the
    // HookBridge self-teardown (TASK-139): detach the destructive phase so it runs to
    // completion regardless, and report Done optimistically. A cross-task finish (a
    // concierge tearing down someone else's task) has no such hazard — the caller
    // isn't the agent being stopped — so it awaits for an exact outcome.
    if is_self_finish(ctx, &target) {
        match crate::teardown::prepare_teardown(state.inner(), &target, effective_force).await? {
            crate::teardown::TeardownStep::Early(outcome) => Ok(outcome),
            crate::teardown::TeardownStep::Ready(plan) => {
                let app2 = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app2.state::<AppState>();
                    if let Err(e) = crate::teardown::perform_teardown(&state, &app2, &plan).await {
                        eprintln!("[mcp] self finish_task teardown of {} failed: {e}", plan.task_id);
                        return;
                    }
                    if delete_branch_after {
                        if let Some(repo_path) = repo_path {
                            let _ = crate::git::delete_branch(Path::new(&repo_path), &branch, true).await;
                        }
                    }
                });
                Ok(TeardownOutcome::Done)
            }
        }
    } else {
        let outcome =
            crate::teardown::teardown_task(state.inner(), app, &target, effective_force, false).await?;
        // discard/merge: drop the branch once the task is actually gone (best-effort).
        if matches!(outcome, TeardownOutcome::Done) && delete_branch_after {
            if let Some(repo_path) = repo_path {
                let _ = crate::git::delete_branch(Path::new(&repo_path), &branch, true).await;
            }
        }
        Ok(outcome)
    }
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
        Routed::FinishTask { id, args } => {
            // Scope is enforced inside do_finish_task: an Agent token may finish its
            // own task (no concierge needed), so we can't gate on is_concierge here.
            let result = do_finish_task(&app, &ctx, args).await;
            Json(finish_task_response(id, result)).into_response()
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
    fn tools_list_advertises_both_tools() {
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
    }

    #[test]
    fn start_task_schema_advertises_after_merge_of() {
        let result = tools_list_result();
        let start = result["tools"]
            .as_array().unwrap()
            .iter()
            .find(|t| t["name"] == "start_task").unwrap();
        let props = &start["inputSchema"]["properties"];
        assert!(props.get("afterMergeOf").is_some(), "afterMergeOf must be advertised");
        assert_eq!(props["afterMergeOf"]["type"], "string");
    }

    #[test]
    fn parse_start_task_args_reads_after_merge_of() {
        let args = serde_json::json!({ "title": "Follow-up", "afterMergeOf": "task-abc" });
        let parsed = parse_start_task_args(&args);
        assert_eq!(parsed.after_merge_of.as_deref(), Some("task-abc"));
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
            "params":{"name":"start_task","arguments":{"title":"Do thing","ticketKey":"TASK-99","repo":"r1"}}
        });
        let Routed::StartTask { id, args } = route(&req) else { panic!("expected StartTask") };
        assert_eq!(id, json!(4));
        assert_eq!(args.title.as_deref(), Some("Do thing"));
        assert_eq!(args.ticket_key.as_deref(), Some("TASK-99"));
        assert_eq!(args.repo.as_deref(), Some("r1"));
        assert_eq!(args.agent, None);
        assert_eq!(args.prompt, None);
    }

    #[test]
    fn tools_call_start_task_parses_prompt() {
        let req = json!({
            "jsonrpc":"2.0","id":7,"method":"tools/call",
            "params":{"name":"start_task","arguments":{"ticketKey":"TASK-99","prompt":"do the thing"}}
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
            Ok(CreatedTask {
                id: "t1".into(),
                branch: "task-99".into(),
                worktree_path: "/wt/task-99".into(),
                queued: false,
                blocking_task_id: None,
                blocking_title: None,
            }),
        );
        assert_eq!(resp["id"], 7);
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("t1"));
        assert!(text.contains("task-99"));
        assert!(text.contains("Started task"));
    }

    // TASK-90: a queued (Pending, afterMergeOf) dispatch must not read as
    // "Started task  on branch  (worktree )." — it must say it's queued, and
    // name the dependency it's waiting on.
    #[test]
    fn start_task_response_queued_is_informative_not_started() {
        let resp = start_task_response(
            json!(21),
            Ok(CreatedTask {
                id: "t2".into(),
                branch: "".into(),
                worktree_path: "".into(),
                queued: true,
                blocking_task_id: Some("dep-1".into()),
                blocking_title: Some("blocking3".into()),
            }),
        );
        assert_eq!(resp["id"], 21);
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Queued"), "expected Queued in: {text}");
        assert!(text.contains("Pending"), "expected Pending in: {text}");
        assert!(text.contains("dep-1"), "expected blocking task id in: {text}");
        assert!(text.contains("blocking3"), "expected blocking task title in: {text}");
        assert!(!text.contains("Started task"), "must not read as Started: {text}");
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
            branch: "task-99".into(),
            base_branch: "main".into(),
            status: TaskStatus::Working,
            created_at: 1,
            updated_at: 2,
            pr_number: Some(7),
            pr_url: None,
            ticket_key: Some("TASK-99".into()),
            agent: None,
            model: None,
            setup_status: None,
            hidden: false,
            pending_prompt: None,
            auto_approve: None,
        };
        let s = task_summary(&t);
        assert_eq!(s.id, "t1");
        assert_eq!(s.repo_id, "r1");
        assert_eq!(s.status, "working");
        assert_eq!(s.ticket_key.as_deref(), Some("TASK-99"));
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

    // ── TASK-140: finish_task ──────────────────────────────────────────────

    #[test]
    fn tools_list_advertises_finish_task() {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}});
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        let tools = resp["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"finish_task"));
        let finish = tools.iter().find(|t| t["name"] == "finish_task").unwrap();
        let props = &finish["inputSchema"]["properties"];
        assert!(props["taskId"].is_object());
        assert!(props["mode"].is_object());
        assert!(props["force"].is_object());
        // All args are optional (defaults: own task / keep / false).
        assert!(finish["inputSchema"].get("required").is_none());
    }

    #[test]
    fn tools_call_finish_task_routes_with_parsed_args() {
        let req = json!({
            "jsonrpc":"2.0","id":21,"method":"tools/call",
            "params":{"name":"finish_task","arguments":{"taskId":"t9","mode":"merge","force":true}}
        });
        let Routed::FinishTask { id, args } = route(&req) else { panic!("expected FinishTask") };
        assert_eq!(id, json!(21));
        assert_eq!(args.task_id.as_deref(), Some("t9"));
        assert_eq!(args.mode.as_deref(), Some("merge"));
        assert!(args.force);
    }

    #[test]
    fn tools_call_finish_task_defaults_to_empty_args() {
        let req = json!({
            "jsonrpc":"2.0","id":22,"method":"tools/call",
            "params":{"name":"finish_task","arguments":{}}
        });
        let Routed::FinishTask { args, .. } = route(&req) else { panic!("expected FinishTask") };
        assert_eq!(args, FinishTaskArgs::default());
        assert_eq!(args.task_id, None);
        assert_eq!(args.mode, None);
        assert!(!args.force);
    }

    fn agent_ctx(task_id: &str) -> CallContext {
        CallContext::Agent { task_id: task_id.to_string(), repo_id: "r1".into() }
    }

    #[test]
    fn resolve_finish_target_agent_defaults_to_own_task() {
        assert_eq!(resolve_finish_target(&agent_ctx("t1"), None).unwrap(), "t1");
    }

    #[test]
    fn resolve_finish_target_agent_may_finish_own_task_explicitly() {
        assert_eq!(resolve_finish_target(&agent_ctx("t1"), Some("  t1  ")).unwrap(), "t1");
    }

    #[test]
    fn resolve_finish_target_agent_rejects_other_task() {
        let err = resolve_finish_target(&agent_ctx("t1"), Some("t2")).unwrap_err();
        assert_eq!(err, FINISH_SCOPE_REQUIRED);
    }

    #[test]
    fn resolve_finish_target_concierge_requires_explicit_task() {
        assert_eq!(resolve_finish_target(&CallContext::Concierge, Some("t2")).unwrap(), "t2");
        let err = resolve_finish_target(&CallContext::Concierge, None).unwrap_err();
        assert!(err.contains("requires an explicit taskId"));
        // A blank/whitespace taskId is treated as absent.
        assert!(resolve_finish_target(&CallContext::Concierge, Some("   ")).is_err());
    }

    #[test]
    fn validate_finish_mode_defaults_and_accepts_known_modes() {
        assert_eq!(validate_finish_mode(None).unwrap(), "keep");
        assert_eq!(validate_finish_mode(Some("keep")).unwrap(), "keep");
        assert_eq!(validate_finish_mode(Some("discard")).unwrap(), "discard");
        assert_eq!(validate_finish_mode(Some("merge")).unwrap(), "merge");
    }

    #[test]
    fn validate_finish_mode_rejects_unknown() {
        assert!(validate_finish_mode(Some("nuke")).unwrap_err().contains("invalid finish mode"));
    }

    #[test]
    fn is_self_finish_only_for_agent_finishing_own_task() {
        // Agent tearing down its own task → self (detach the destructive phase).
        assert!(is_self_finish(&agent_ctx("t1"), "t1"));
        // Agent targeting another task is not self (and is rejected upstream anyway).
        assert!(!is_self_finish(&agent_ctx("t1"), "t2"));
        // A concierge is never the agent being stopped, so never self.
        assert!(!is_self_finish(&CallContext::Concierge, "t1"));
    }

    #[test]
    fn finish_task_response_done_is_non_error_text() {
        let resp = finish_task_response(json!(23), Ok(crate::teardown::TeardownOutcome::Done));
        assert_eq!(resp["id"], 23);
        assert_eq!(resp["result"]["isError"], false);
        assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("finished"));
    }

    #[test]
    fn finish_task_response_unknown_task_is_error_text() {
        let resp = finish_task_response(json!(24), Ok(crate::teardown::TeardownOutcome::UnknownTask));
        assert_eq!(resp["result"]["isError"], true);
        assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("not found"));
    }

    #[test]
    fn finish_task_response_unsafe_is_error_text_with_reason() {
        let resp = finish_task_response(
            json!(25),
            Ok(crate::teardown::TeardownOutcome::Unsafe("worktree has uncommitted changes".into())),
        );
        assert_eq!(resp["result"]["isError"], true);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("uncommitted"));
        assert!(text.contains("force"));
    }

    #[test]
    fn finish_task_response_scope_error_is_error_text() {
        let resp = finish_task_response(json!(26), Err(FINISH_SCOPE_REQUIRED.to_string()));
        assert_eq!(resp["result"]["isError"], true);
        assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("concierge"));
    }
}
