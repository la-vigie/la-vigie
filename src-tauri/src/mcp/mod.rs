//! In-process MCP server (TASK-89): exposes `start_task`, `finish_task`,
//! `list_repos`, the control-plane read tools, and the schedule-management
//! tools (`create_schedule` / `list_schedules` / `update_schedule` /
//! `set_schedule_enabled` / `delete_schedule`, TASK-178) to spawned Claude
//! agents over a loopback HTTP JSON-RPC endpoint, so an agent can
//! self-dispatch, self-tear-down, and manage recurring schedules.
//!
//! Two layers, mirroring `hooks/`:
//!   * pure (`route`, `*_response`, schemas): JSON-RPC parsing/dispatch +
//!     result formatting — unit-tested here, no I/O.
//!   * async glue (the axum handler + `start_mcp_server`): auth, the launch
//!     side effects, and event emission — verified via the app, not unit-tested.

pub(crate) mod authz;

use std::path::Path;

use serde_json::{json, Value};

use axum::extract::State as AxumState;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use tauri::Manager as _;

use crate::mcp::authz::{AuthorizedContext, AuthzError, Capability, ResolutionStrategy};
use crate::state::AppState;

/// MCP protocol version this server speaks. Echoed in `initialize`.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

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
    /// Per-repo orchestrator: broad act+read confined to `repo_id` (TASK-180).
    Orchestrator {
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
        crate::state::McpToken::Orchestrator { repo_id } => {
            CallContext::Orchestrator {
                repo_id: repo_id.clone(),
            }
        }
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
    /// Optional model override for the launched agent (TASK-223). Flows to the
    /// created task's `model`; the spawn then passes `--model <id>` for engines
    /// with a `model_arg` (TASK-209). Omitted ⇒ the engine's own default.
    pub model: Option<String>,
    /// La Vigie task ids to queue this task behind (start-on-merge, TASK-90/177).
    /// Accepts a single id or an array; the created task stays `Pending` until
    /// all of them are merged through La Vigie.
    pub after_merge_of: Vec<String>,
}

/// Parsed `finish_task` tool arguments. `task_id` defaults to the caller's own
/// task (Agent tier); `mode` defaults to `"keep"`; `force` defaults to `false`.
#[derive(Debug, Default, PartialEq)]
pub struct FinishTaskArgs {
    pub task_id: Option<String>,
    pub mode: Option<String>,
    pub force: bool,
}

/// Parsed `create_schedule` arguments (validated downstream by
/// `schedule::validate_schedule_fields`; `repo` scoped by the `authorize_call`
/// choke-point (TASK-180)).
#[derive(Debug, Default, PartialEq)]
pub struct CreateScheduleArgs {
    pub repo: Option<String>,
    pub name: Option<String>,
    pub prompt: Option<String>,
    pub cron: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub base_branch: Option<String>,
}

/// Parsed `update_schedule` arguments. `schedule_id` and `enabled` are required
/// (a full-field replace); the rest are validated downstream.
#[derive(Debug, Default, PartialEq)]
pub struct UpdateScheduleArgs {
    pub schedule_id: String,
    pub name: Option<String>,
    pub prompt: Option<String>,
    pub cron: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub base_branch: Option<String>,
    pub enabled: bool,
}

/// Parsed `schedule_task` tool arguments. Identity of the deferred launch is a
/// title + prompt in the caller's repo (override via `repo`); the fire time is
/// a relative delay (`in_seconds`, derived from `inSeconds`/`inHours`) and/or an
/// absolute unix time (`at_unix`). TASK-179.
#[derive(Debug, Default, PartialEq)]
pub struct ScheduleTaskArgs {
    pub title: Option<String>,
    pub prompt: Option<String>,
    pub repo: Option<String>,
    pub agent: Option<String>,
    /// Optional model override for the deferred launch (TASK-223); mirrors
    /// `create_schedule`'s `model`. Omitted ⇒ the engine's own default.
    pub model: Option<String>,
    pub in_seconds: Option<i64>,
    pub at_unix: Option<i64>,
}

/// The one-shot schedule created by `schedule_task`, summarized for the result.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledOnce {
    pub id: String,
    pub fire_at: i64,
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

/// One blocker a queued task waits on: its task id and (if resolvable) title.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockingRef {
    pub task_id: String,
    pub title: Option<String>,
}

/// The task created by `start_task`, summarized for the tool result.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatedTask {
    pub id: String,
    pub branch: String,
    pub worktree_path: String,
    /// TASK-90/177: true when the task was QUEUED (Pending) behind other tasks'
    /// landings rather than started now. Then branch/worktree_path are empty and
    /// `blocking` lists every task it waits on.
    pub queued: bool,
    pub blocking: Vec<BlockingRef>,
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
    /// `queue_dependency` (TASK-164): start_task with a *required* non-empty
    /// dependency list. Dispatched via the same `do_start_task` path, so the
    /// created task is always `Pending`.
    QueueDependency { id: Value, args: StartTaskArgs },
    FinishTask { id: Value, args: FinishTaskArgs },
    ScheduleTask { id: Value, args: ScheduleTaskArgs },
    ListRepos { id: Value },
    ListTasks { id: Value },
    TaskStatus { id: Value, task_id: String },
    GetTaskActivity { id: Value, task_id: String, since: usize },
    CreateSchedule { id: Value, args: CreateScheduleArgs },
    ListSchedules { id: Value, repo: Option<String> },
    UpdateSchedule { id: Value, args: UpdateScheduleArgs },
    SetScheduleEnabled { id: Value, schedule_id: String, enabled: bool },
    DeleteSchedule { id: Value, schedule_id: String },
    SendTaskMessage { id: Value, task_id: String, message: String },
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
Defaults to the calling agent's repo; a `repo` arg must match the calling token's repo (cross-repo is denied). \
Provide a `title` and/or a `ticketKey` (the task's branch derives from the ticket key).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "Human title for the task." },
                        "ticketKey": { "type": "string", "description": "Provider ticket id (e.g. TASK-99); links the task and seeds its branch name." },
                        "repo": { "type": "string", "description": "Target repo id. Defaults to the calling agent's repo. Must match the calling token's repo (cross-repo is denied). Use list_repos to discover ids." },
                        "agent": { "type": "string", "description": "Optional agent name to launch (defaults to the repo/global default)." },
                        "prompt": { "type": "string", "description": "Optional initial prompt for the new agent. Combined with the repo's configured prompt." },
                        "model": { "type": "string", "description": "Optional model override for the launched agent (e.g. opus, sonnet). Omitted ⇒ the engine's default." },
                        "afterMergeOf": { "type": ["string", "array"], "items": { "type": "string" }, "description": "Queue this task to auto-start only after ALL of the given La Vigie task ids are merged through La Vigie (start-on-merge). Accepts a single id or an array of ids." }
                    }
                }
            },
            {
                "name": "queue_dependency",
                "description": "Create a NEW La Vigie task QUEUED behind one or more existing tasks — a clearer, dependency-first alternative to start_task's afterMergeOf. \
The new task stays Pending (no worktree/agent yet) and auto-starts only once ALL of the tasks in `dependsOn` are merged through La Vigie. \
`dependsOn` is required (one La Vigie task id or an array). Otherwise identical to start_task: defaults to the calling agent's repo (a `repo` arg must match; cross-repo denied), and takes an optional `title`/`ticketKey`/`prompt`/`agent`.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "dependsOn": { "type": ["string", "array"], "items": { "type": "string" }, "description": "Required. La Vigie task id(s) this task waits on; it auto-starts only after ALL of them are MERGED through La Vigie. Accepts a single id or an array of ids." },
                        "title": { "type": "string", "description": "Human title for the task." },
                        "ticketKey": { "type": "string", "description": "Provider ticket id (e.g. TASK-99); links the task and seeds its branch name." },
                        "repo": { "type": "string", "description": "Target repo id. Defaults to the calling agent's repo. Must match the calling token's repo (cross-repo is denied). Use list_repos to discover ids." },
                        "agent": { "type": "string", "description": "Optional agent name to launch once released (defaults to the repo/global default)." },
                        "prompt": { "type": "string", "description": "Optional initial prompt for the new agent. Combined with the repo's configured prompt." },
                        "model": { "type": "string", "description": "Optional model override for the launched agent (e.g. opus, sonnet). Omitted ⇒ the engine's default." }
                    },
                    "required": ["dependsOn"]
                }
            },
            {
                "name": "finish_task",
                "description": "Finish (tear down) a La Vigie task: stop its agent, remove the git worktree, and delete the task. \
Defaults to the calling agent's own task; pass `taskId` to target another, which requires an orchestrator-scope token for that task's repo (cross-repo is denied). \
`mode` is keep (default: leave the branch), discard (delete the branch), or merge (squash-merge the PR, then delete the branch). \
Refuses a task with uncommitted or unmerged work unless `force` is true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "taskId": { "type": "string", "description": "Task id to finish. Defaults to the calling agent's own task. Finishing another task requires an orchestrator-scope token for that task's repo." },
                        "mode": { "type": "string", "enum": ["keep", "discard", "merge"], "description": "keep (leave branch, default), discard (delete branch), or merge (squash-merge the PR then delete branch)." },
                        "force": { "type": "boolean", "description": "Tear down even with uncommitted or unmerged work. Defaults to false." }
                    }
                }
            },
            {
                "name": "schedule_task",
                "description": "Schedule a La Vigie task to launch ONCE at a future time, then retire (a one-shot deferred launch). \
Defaults to the calling agent's repo; a `repo` arg must match the calling token's repo (cross-repo is denied). Give a `title` and/or `prompt`. \
Set `inHours` (e.g. 3 = in three hours) or `inSeconds` for a relative delay, or `atUnix` for an absolute unix time. \
Useful to defer a launch until Claude quota resets. Does NOT create a worktree now — La Vigie launches it when due.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "Human title for the deferred task." },
                        "prompt": { "type": "string", "description": "Initial prompt for the launched agent. Combined with the repo's configured prompt." },
                        "repo": { "type": "string", "description": "Target repo id. Defaults to the calling agent's repo. Use list_repos to discover ids." },
                        "agent": { "type": "string", "description": "Optional agent name to launch (defaults to the repo/global default)." },
                        "model": { "type": "string", "description": "Optional model override for the launched agent (e.g. opus, sonnet). Omitted ⇒ the engine's default." },
                        "inHours": { "type": "number", "description": "Relative delay in hours (fractional allowed). Resolved to an absolute fire time at creation." },
                        "inSeconds": { "type": "integer", "description": "Relative delay in seconds. Takes precedence over inHours if both are given." },
                        "atUnix": { "type": "integer", "description": "Absolute fire time as a unix timestamp (seconds). Wins over inHours/inSeconds." }
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
                "description": "List La Vigie tasks with their current status. Available to the concierge (across all repos) and to a repo-scoped orchestrator (its own repo only).",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "task_status",
                "description": "Get one task's current status and metadata by id. Available to the concierge (all repos) and to a repo-scoped orchestrator (its own repo).",
                "inputSchema": {
                    "type": "object",
                    "properties": { "taskId": { "type": "string", "description": "The task id." } },
                    "required": ["taskId"]
                }
            },
            {
                "name": "get_task_activity",
                "description": "Read a task's recent agent conversation/activity (chat-shaped messages). Poll incrementally by passing the returned `cursor` as `since`. Available to the concierge (all repos) and to a repo-scoped orchestrator (its own repo).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "taskId": { "type": "string", "description": "The task id." },
                        "since": { "type": "integer", "description": "Byte offset cursor from a prior call; omit or 0 to read from the start." }
                    },
                    "required": ["taskId"]
                }
            },
            {
                "name": "send_task_message",
                "description": "Send a message to another task's running agent — 'stir' it to keep it moving (unblock, redirect, or nudge a waiting agent). The message is delivered to the agent's input and submitted, as if typed. Repo-scoped: you may only message a task in your own repo. Requires a live agent — if the task has no running agent this errors (start or resume it first). Read the agent's reply afterward via get_task_activity.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "taskId": { "type": "string", "description": "The target task id (must be in your repo)." },
                        "message": { "type": "string", "description": "The message to deliver to the task's agent. Submitted as if typed." }
                    },
                    "required": ["taskId", "message"]
                }
            },
            {
                "name": "create_schedule",
                "description": "Create a recurring schedule that launches a task on a cron. \
Defaults to the calling agent's repo; a `repoId` arg must match the calling token's repo (cross-repo is denied). \
`prompt` is the initial prompt the launched agent receives (typically a repo skill like `/security-scan`). \
`cron` is standard 5/6-field cron in the app's local time. Returns the created schedule (incl. `nextRunAt`).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "repoId": { "type": "string", "description": "Target repo id. Defaults to the calling agent's repo. Must match the calling token's repo (cross-repo is denied)." },
                        "name": { "type": "string", "description": "Human name for the schedule." },
                        "prompt": { "type": "string", "description": "Initial prompt for the launched agent (e.g. /security-scan)." },
                        "cron": { "type": "string", "description": "Standard 5/6-field cron expression, local time (e.g. `0 2 * * 1`)." },
                        "agent": { "type": "string", "description": "Optional agent name (defaults to the repo/global default)." },
                        "model": { "type": "string", "description": "Optional model override." },
                        "baseBranch": { "type": "string", "description": "Optional base branch for the launched task's worktree." }
                    },
                    "required": ["name", "prompt", "cron"]
                }
            },
            {
                "name": "list_schedules",
                "description": "List the recurring schedules for a repo. Defaults to the calling agent's repo; a `repoId` arg must match the calling token's repo (cross-repo is denied).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "repoId": { "type": "string", "description": "Repo id. Defaults to the calling agent's repo. Must match the calling token's repo (cross-repo is denied)." }
                    }
                }
            },
            {
                "name": "update_schedule",
                "description": "Update all fields of an existing schedule (full replace). Requires `scheduleId` and `enabled`. \
Confined to the calling token's repo (cross-repo denied); the schedule's repo must match. Returns the updated schedule.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "scheduleId": { "type": "string", "description": "The schedule id to update." },
                        "name": { "type": "string", "description": "Human name for the schedule." },
                        "prompt": { "type": "string", "description": "Initial prompt for the launched agent." },
                        "cron": { "type": "string", "description": "Standard 5/6-field cron expression, local time." },
                        "agent": { "type": "string", "description": "Optional agent name." },
                        "model": { "type": "string", "description": "Optional model override." },
                        "baseBranch": { "type": "string", "description": "Optional base branch." },
                        "enabled": { "type": "boolean", "description": "Whether the schedule is active (disabled ⇒ never fires)." }
                    },
                    "required": ["scheduleId", "name", "prompt", "cron", "enabled"]
                }
            },
            {
                "name": "set_schedule_enabled",
                "description": "Enable or disable a schedule without editing its other fields. Requires `scheduleId` and `enabled`. \
Confined to the calling token's repo (cross-repo denied); the schedule's repo must match. Returns the updated schedule.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "scheduleId": { "type": "string", "description": "The schedule id." },
                        "enabled": { "type": "boolean", "description": "Whether the schedule is active." }
                    },
                    "required": ["scheduleId", "enabled"]
                }
            },
            {
                "name": "delete_schedule",
                "description": "Delete a schedule by id. Confined to the calling token's repo (cross-repo denied); the schedule's repo must match.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "scheduleId": { "type": "string", "description": "The schedule id to delete." }
                    },
                    "required": ["scheduleId"]
                }
            }
        ]
    })
}

fn opt_str(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|s| s.to_string())
}

/// Parse a string-or-array-of-strings argument into a `Vec<String>`. A bare
/// string yields a one-element vec (back-compat); an array yields its string
/// elements; anything else (absent, wrong type) yields empty.
fn str_or_vec(v: &Value, key: &str) -> Vec<String> {
    match v.get(key) {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Extract a required non-empty `taskId` string argument.
fn require_task_id(arguments: &Value) -> Result<String, String> {
    match opt_str(arguments, "taskId") {
        Some(s) if !s.trim().is_empty() => Ok(s),
        _ => Err("missing required argument: taskId".to_string()),
    }
}

/// Extract a required non-empty `message` string argument.
fn require_message(arguments: &Value) -> Result<String, String> {
    match opt_str(arguments, "message") {
        Some(s) if !s.trim().is_empty() => Ok(s),
        _ => Err("missing required argument: message".to_string()),
    }
}

fn parse_start_task_args(arguments: &Value) -> StartTaskArgs {
    StartTaskArgs {
        title: opt_str(arguments, "title"),
        ticket_key: opt_str(arguments, "ticketKey"),
        repo: opt_str(arguments, "repo"),
        agent: opt_str(arguments, "agent"),
        prompt: opt_str(arguments, "prompt"),
        model: opt_str(arguments, "model"),
        after_merge_of: str_or_vec(arguments, "afterMergeOf"),
    }
}

/// Parse `queue_dependency` arguments (TASK-164). Identical to `start_task`'s
/// fields except the dependency list is read from the clearer `dependsOn` key and
/// is REQUIRED: an empty/missing list is a usage error, so the caller can't
/// silently launch-now by forgetting it. Returns `StartTaskArgs` (with the deps
/// in `after_merge_of`) so it reuses the exact `do_start_task` path.
fn parse_queue_dependency_args(arguments: &Value) -> Result<StartTaskArgs, String> {
    let after_merge_of = str_or_vec(arguments, "dependsOn");
    if after_merge_of.is_empty() {
        return Err(
            "missing required argument: dependsOn (one or more La Vigie task ids to queue behind)"
                .to_string(),
        );
    }
    Ok(StartTaskArgs {
        title: opt_str(arguments, "title"),
        ticket_key: opt_str(arguments, "ticketKey"),
        repo: opt_str(arguments, "repo"),
        agent: opt_str(arguments, "agent"),
        prompt: opt_str(arguments, "prompt"),
        model: opt_str(arguments, "model"),
        after_merge_of,
    })
}

fn parse_finish_task_args(arguments: &Value) -> FinishTaskArgs {
    FinishTaskArgs {
        task_id: opt_str(arguments, "taskId"),
        mode: opt_str(arguments, "mode"),
        force: arguments.get("force").and_then(|f| f.as_bool()).unwrap_or(false),
    }
}

/// Extract a required non-empty `scheduleId` string argument.
fn require_schedule_id(arguments: &Value) -> Result<String, String> {
    match opt_str(arguments, "scheduleId") {
        Some(s) if !s.trim().is_empty() => Ok(s),
        _ => Err("missing required argument: scheduleId".to_string()),
    }
}

/// Extract a required boolean argument.
fn require_bool(arguments: &Value, key: &str) -> Result<bool, String> {
    arguments
        .get(key)
        .and_then(|v| v.as_bool())
        .ok_or_else(|| format!("missing required argument: {key}"))
}

fn parse_create_schedule_args(arguments: &Value) -> CreateScheduleArgs {
    CreateScheduleArgs {
        repo: opt_str(arguments, "repoId"),
        name: opt_str(arguments, "name"),
        prompt: opt_str(arguments, "prompt"),
        cron: opt_str(arguments, "cron"),
        agent: opt_str(arguments, "agent"),
        model: opt_str(arguments, "model"),
        base_branch: opt_str(arguments, "baseBranch"),
    }
}

fn parse_update_schedule_args(arguments: &Value) -> Result<UpdateScheduleArgs, String> {
    let schedule_id = require_schedule_id(arguments)?;
    let enabled = require_bool(arguments, "enabled")?;
    Ok(UpdateScheduleArgs {
        schedule_id,
        name: opt_str(arguments, "name"),
        prompt: opt_str(arguments, "prompt"),
        cron: opt_str(arguments, "cron"),
        agent: opt_str(arguments, "agent"),
        model: opt_str(arguments, "model"),
        base_branch: opt_str(arguments, "baseBranch"),
        enabled,
    })
}

fn parse_schedule_task_args(arguments: &Value) -> ScheduleTaskArgs {
    // inSeconds wins over inHours; both fold into a single in_seconds offset.
    let in_seconds = arguments
        .get("inSeconds")
        .and_then(|v| v.as_i64())
        .or_else(|| {
            arguments
                .get("inHours")
                .and_then(|v| v.as_f64())
                .map(|h| (h * 3600.0).round() as i64)
        });
    ScheduleTaskArgs {
        title: opt_str(arguments, "title"),
        prompt: opt_str(arguments, "prompt"),
        repo: opt_str(arguments, "repo"),
        agent: opt_str(arguments, "agent"),
        model: opt_str(arguments, "model"),
        in_seconds,
        at_unix: arguments.get("atUnix").and_then(|v| v.as_i64()),
    }
}

/// Error returned when a normal Agent-tier token tries to finish a task other
/// than its own — a cross-task teardown needs an orchestrator-scope token for
/// that task's repo (TASK-180; the legacy concierge is read-only).
const FINISH_SCOPE_REQUIRED: &str =
    "Finishing another agent's task requires an orchestrator-scope token for that task's repo.";

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
        // Orchestrator has no own task, so it must name one explicitly; repo
        // scoping is enforced by the choke-point (TASK-180 A3/B2).
        CallContext::Orchestrator { .. } => match arg {
            Some(t) => Ok(t.to_string()),
            None => {
                Err("finish_task from an orchestrator token requires an explicit taskId.".to_string())
            }
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
                "queue_dependency" => match parse_queue_dependency_args(&arguments) {
                    Ok(args) => Routed::QueueDependency { id, args },
                    Err(e) => Routed::Respond(rpc_error(id, -32602, &e)),
                },
                "finish_task" => Routed::FinishTask { id, args: parse_finish_task_args(&arguments) },
                "schedule_task" => Routed::ScheduleTask { id, args: parse_schedule_task_args(&arguments) },
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
                "create_schedule" => Routed::CreateSchedule { id, args: parse_create_schedule_args(&arguments) },
                "list_schedules" => Routed::ListSchedules { id, repo: opt_str(&arguments, "repoId") },
                "update_schedule" => match parse_update_schedule_args(&arguments) {
                    Ok(args) => Routed::UpdateSchedule { id, args },
                    Err(e) => Routed::Respond(rpc_error(id, -32602, &e)),
                },
                "set_schedule_enabled" => match require_schedule_id(&arguments) {
                    Ok(schedule_id) => match require_bool(&arguments, "enabled") {
                        Ok(enabled) => Routed::SetScheduleEnabled { id, schedule_id, enabled },
                        Err(e) => Routed::Respond(rpc_error(id, -32602, &e)),
                    },
                    Err(e) => Routed::Respond(rpc_error(id, -32602, &e)),
                },
                "delete_schedule" => match require_schedule_id(&arguments) {
                    Ok(schedule_id) => Routed::DeleteSchedule { id, schedule_id },
                    Err(e) => Routed::Respond(rpc_error(id, -32602, &e)),
                },
                "send_task_message" => match require_task_id(&arguments) {
                    Ok(task_id) => match require_message(&arguments) {
                        Ok(message) => Routed::SendTaskMessage { id, task_id, message },
                        Err(e) => Routed::Respond(rpc_error(id, -32602, &e)),
                    },
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
            let names = if task.blocking.is_empty() {
                "(unknown)".to_string()
            } else {
                task.blocking
                    .iter()
                    .map(|b| match &b.title {
                        Some(t) => format!("{} ({t})", b.task_id),
                        None => b.task_id.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            success(
                id,
                tool_text(
                    format!(
                        "Queued task {} (Pending) — will auto-start once all of these land: {names}. \
No worktree/branch yet; it unblocks automatically when each blocker's PR merges (detected at \
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

/// Format the `schedule_task` result: confirmation with the fire time (as a
/// unix ts the caller can render) and schedule id, or an `isError` text.
pub fn schedule_task_response(id: Value, result: Result<ScheduledOnce, String>) -> Value {
    match result {
        Ok(s) => success(
            id,
            tool_text(
                format!(
                    "Scheduled a one-shot launch (schedule {}) to fire at unix {} — retires after it runs.",
                    s.id, s.fire_at
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

/// Format a single-`Schedule` tool result (create/update/set-enabled): the
/// schedule as pretty JSON text (camelCase via serde) or `isError` text.
pub fn schedule_response(id: Value, result: Result<crate::store::Schedule, String>) -> Value {
    match result {
        Ok(s) => {
            let body = serde_json::to_string_pretty(&s).unwrap_or_else(|_| "{}".to_string());
            success(id, tool_text(body, false))
        }
        Err(e) => success(id, tool_text(e, true)),
    }
}

/// Format the `list_schedules` tool result (schedules as pretty JSON text).
pub fn list_schedules_response(id: Value, result: Result<Vec<crate::store::Schedule>, String>) -> Value {
    match result {
        Ok(list) => {
            let body = serde_json::to_string_pretty(&list).unwrap_or_else(|_| "[]".to_string());
            success(id, tool_text(body, false))
        }
        Err(e) => success(id, tool_text(e, true)),
    }
}

/// Format the `delete_schedule` tool result (confirmation text or `isError`).
pub fn delete_schedule_response(id: Value, result: Result<String, String>) -> Value {
    match result {
        Ok(sid) => success(id, tool_text(format!("Schedule {sid} deleted."), false)),
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

/// Format the `send_task_message` tool result (success text; failure ⇒ isError).
pub fn send_task_message_response(id: Value, result: Result<String, String>) -> Value {
    match result {
        Ok(msg) => success(id, tool_text(msg, false)),
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
    /// TASK-181: when `true`, the frontend skips prepending the repo's initial
    /// prompt (TASK-160's `combineInitialPrompts(null, …)` path). Only the
    /// scheduler sets this; manual/self-dispatch/promote paths pass `false`.
    skip_repo_prompt: bool,
}

/// Emit `task_launched` so the frontend's `useTaskLaunch` hook starts the agent
/// on the existing path. Shared by `do_start_task` (MCP self-dispatch), the
/// TASK-90 merge-time promote path, and the TASK-173 scheduler so all fire the
/// identical event/payload. `skip_repo_prompt` (TASK-181) is `true` only for
/// scheduled runs configured to skip the repo prompt.
pub(crate) fn emit_task_launched(
    app: &tauri::AppHandle,
    task_id: String,
    initial_prompt: Option<String>,
    skip_repo_prompt: bool,
) {
    use tauri::Emitter as _;
    let _ = app.emit(
        "task_launched",
        TaskLaunchedPayload { task_id, initial_prompt, skip_repo_prompt },
    );
}

/// Tauri event payload announcing a task the frontend didn't initiate itself —
/// e.g. a pending/queued task created via MCP `start_task(afterMergeOf:…)`
/// that has no worktree/agent yet, so `task_launched` never fires for it. TASK-90.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TaskCreatedPayload {
    task_id: String,
}

/// The single deny-by-default authorization choke-point (TASK-180). Resolve the
/// call's *target repo* from storage per `strategy` — never trusting a
/// caller-supplied `args_repo` for id-addressed resources — then run the pure
/// `authz::decide` policy against the caller's tier. Handlers receive an
/// `AuthorizedContext` (the resolved repo), never the raw `CallContext`, so they
/// cannot act outside the resolved repo.
///
/// The store is locked only to read the target row and is dropped before the
/// (synchronous) policy check returns — honoring the "never hold the store lock
/// across an await" invariant.
async fn authorize_call(
    app: &tauri::AppHandle,
    ctx: &CallContext,
    cap: Capability,
    strategy: ResolutionStrategy,
    args_repo: Option<&str>,
    id: Option<&str>,
) -> Result<AuthorizedContext, AuthzError> {
    let state = app.state::<AppState>();
    // Resolve the target repo from storage per strategy (never trust args_repo
    // for id-addressed resources). Lock briefly; drop the guard before returning.
    let (target_repo, caller_task_id) = {
        let store = state
            .store
            .lock()
            .map_err(|e| AuthzError::Denied(format!("{e:#}")))?;
        let repo = match strategy {
            ResolutionStrategy::FromTaskId => {
                let id = id.ok_or_else(|| AuthzError::NotFound("task id".into()))?;
                store
                    .get_task(id)
                    .ok()
                    .flatten()
                    .ok_or_else(|| AuthzError::NotFound(format!("task {id}")))?
                    .repo_id
            }
            ResolutionStrategy::FromScheduleId => {
                let id = id.ok_or_else(|| AuthzError::NotFound("schedule id".into()))?;
                store
                    .get_schedule(id)
                    .ok()
                    .flatten()
                    .ok_or_else(|| AuthzError::NotFound(format!("schedule {id}")))?
                    .repo_id
            }
            ResolutionStrategy::RepoArg | ResolutionStrategy::CallerRepo => {
                // For create/list/self ops the target is the caller's own repo
                // unless an explicit (non-empty, trimmed) repo arg is given
                // (validated against the token scope by `decide`).
                match (args_repo.map(|r| r.trim()).filter(|r| !r.is_empty()), ctx) {
                    (Some(r), _) => r.to_string(),
                    (None, CallContext::Agent { repo_id, .. })
                    | (None, CallContext::Orchestrator { repo_id }) => repo_id.clone(),
                    (None, CallContext::Concierge) => {
                        return Err(AuthzError::Denied(
                            "acting requires an explicit repoId".into(),
                        ))
                    }
                }
            }
        };
        let caller = match ctx {
            CallContext::Agent { task_id, .. } => Some(task_id.clone()),
            _ => None,
        };
        (repo, caller)
    }; // store guard dropped here — before any await downstream
    authz::decide(&authz::principal_of(ctx), cap, &target_repo)?;
    Ok(AuthorizedContext {
        repo_id: target_repo,
        caller_task_id,
    })
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
    let StartTaskArgs { title, ticket_key, repo, agent, prompt, model, after_merge_of } = args;
    // Deny-by-default choke-point (TASK-180): resolve + authorize the target repo
    // (own repo unless an explicit repo arg is given; cross-repo is denied; the
    // concierge cannot act). Replaces the old inline tier gate + `resolve_repo_id`.
    let authz = authorize_call(
        app,
        ctx,
        Capability::StartTask,
        ResolutionStrategy::RepoArg,
        repo.as_deref(),
        None,
    )
    .await
    .map_err(|e| e.into_message())?;
    let repo_id = authz.repo_id;

    let launch_args = crate::launch::LaunchArgs {
        repo_id,
        title: title.unwrap_or_default(),
        base_branch: None,
        ticket_key,
        agent,
        // TASK-223: model override from the tool arg (normalized — trimmed,
        // empty ⇒ None — by launch::resolve_launch); omitted ⇒ engine default.
        model,
        auto_approve: None,
        after_merge_of,
        prompt: prompt.clone(),
        // TASK-163: placeholder default — the MCP start_task tool doesn't expose
        // in-place launches.
        in_place: false,
        branch_name: None,
    };

    let task = {
        let state = app.state::<AppState>();
        crate::commands::launch_and_kickoff_setup(state.inner(), app, launch_args).await?
    };

    if task.status == crate::store::TaskStatus::Pending {
        // Queued behind other tasks' merges — no worktree/agent yet, so no
        // task_launched event. The landed-gated trigger promotes it once every
        // blocker lands, from any finish surface. TASK-90/177. Emit task_created
        // so the queued task shows in the sidebar right away.
        use tauri::Emitter as _;
        let _ = app.emit("task_created", TaskCreatedPayload { task_id: task.id.clone() });

        // Resolve the ACTUAL queued edges (live blockers only — dangling ones
        // were filtered out in launch_task) and name them for the response.
        let blocking = {
            let state = app.state::<AppState>();
            let store = state.store.lock().map_err(|e| e.to_string())?;
            store
                .blockers_of(&task.id)
                .unwrap_or_default()
                .into_iter()
                .map(|dep_id| {
                    let title = store.get_task(&dep_id).ok().flatten().map(|t| t.title);
                    BlockingRef { task_id: dep_id, title }
                })
                .collect()
        };

        return Ok(CreatedTask {
            id: task.id,
            branch: task.branch,
            worktree_path: task.worktree_path,
            queued: true,
            blocking,
        });
    }

    // The prompt is delivered when the frontend starts the agent (not persisted
    // on the task row), so it rides on the event rather than LaunchArgs.
    // TASK-181: MCP self-dispatch keeps the repo-prompt combine (skip = false).
    crate::mcp::emit_task_launched(app, task.id.clone(), prompt, false);

    Ok(CreatedTask {
        id: task.id,
        branch: task.branch,
        worktree_path: task.worktree_path,
        queued: false,
        blocking: vec![],
    })
}

/// Execute `schedule_task`: resolve the repo (override → caller's context) and
/// insert a one-shot schedule row that fires once at the resolved time. Agent
/// tier only — like `start_task`, the concierge token cannot act. No launch or
/// worktree now; the poller fires it when due. TASK-179.
async fn do_schedule_task(
    app: &tauri::AppHandle,
    ctx: &CallContext,
    args: ScheduleTaskArgs,
) -> Result<ScheduledOnce, String> {
    let ScheduleTaskArgs { title, prompt, repo, agent, model, in_seconds, at_unix } = args;
    // Deny-by-default choke-point (TASK-180): same repo resolution/authorization
    // as start_task — own repo by default, cross-repo denied, concierge cannot act.
    let authz = authorize_call(
        app,
        ctx,
        Capability::ScheduleTask,
        ResolutionStrategy::RepoArg,
        repo.as_deref(),
        None,
    )
    .await
    .map_err(|e| e.into_message())?;
    let repo_id = authz.repo_id;
    let name = title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "Deferred launch".to_string());
    let prompt = prompt.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    let now = crate::schedule::now_secs_pub();
    let fire_at = crate::schedule::resolve_fire_at(now, in_seconds, at_unix)?;

    let schedule = crate::store::Schedule {
        id: uuid::Uuid::new_v4().to_string(),
        repo_id,
        name,
        prompt: prompt.unwrap_or_default(),
        cron: String::new(),
        agent: agent.map(|a| a.trim().to_string()).filter(|a| !a.is_empty()),
        // TASK-223: model override (trimmed; empty ⇒ None), mirroring `agent`
        // above and `create_schedule`. Omitted ⇒ engine default at fire time.
        model: model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty()),
        base_branch: None,
        enabled: true,
        one_shot: true,
        // TASK-181: MCP-created schedules default to skipping the repo prompt.
        skip_repo_prompt: true,
        next_run_at: Some(fire_at),
        last_run_at: None,
        created_at: now,
        updated_at: now,
    };

    let state = app.state::<AppState>();
    {
        let store = state.store.lock().map_err(|e| format!("{e}"))?;
        store.insert_schedule(&schedule).map_err(|e| format!("{e:#}"))?;
    }
    Ok(ScheduledOnce { id: schedule.id, fire_at })
}

/// Execute `finish_task`: enforce the scope tier, then compose the TASK-139
/// teardown core with the keep/discard/merge branch/PR semantics.
///
///   * scope — an Agent token may only finish its own task
///     (`resolve_finish_target`); an Orchestrator may finish any task **in its
///     own repo** and the read-only Concierge is denied outright — both enforced
///     by the `authorize_call(FinishTask, FromTaskId)` choke-point, which
///     resolves the target's repo from storage (never caller input). TASK-180 B2.
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
    let (worktree_path, branch, repo_path, in_place) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let task = match store.get_task(&target).map_err(|e| format!("{e:#}"))? {
            Some(t) => t,
            None => return Ok(TeardownOutcome::UnknownTask),
        };
        let repo_path = store
            .get_repo(&task.repo_id)
            .map_err(|e| format!("{e:#}"))?
            .map(|r| r.path);
        (task.worktree_path, task.branch, repo_path, task.in_place)
    };

    // Deny-by-default choke-point (TASK-180 B2): resolve the target task's repo
    // from storage and run the tier policy before any destructive step. An
    // Agent/Orchestrator may only finish a task in its own repo; the read-only
    // Concierge is denied. (An unknown task already short-circuited above as
    // `UnknownTask`, preserving idempotent-finish semantics.) The store lock is
    // taken briefly and dropped inside `authorize_call` — never held across the
    // teardown awaits below.
    authorize_call(
        app,
        ctx,
        Capability::FinishTask,
        ResolutionStrategy::FromTaskId,
        None,
        Some(&target),
    )
    .await
    .map_err(|e| e.into_message())?;

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
    // Never delete the branch for an in-place task (TASK-163): it's the
    // checkout's current branch, not a task-owned throwaway. Worktree removal
    // itself routes through `teardown::prepare_teardown`/`teardown_task`, which
    // already carry `in_place` via `TeardownPlan`.
    let delete_branch_after = !in_place && (mode == "discard" || mode == "merge");

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

/// Execute `send_task_message`: authorize (own-repo task only, concierge denied),
/// find the task's live agent, and deliver `message` as a bracketed paste + Enter
/// — mirroring the remote reply path (TASK-108). Unauthorized / no-live-agent /
/// write failures return as an isError tool result.
async fn do_send_task_message(
    app: &tauri::AppHandle,
    ctx: &CallContext,
    task_id: String,
    message: String,
) -> Result<String, String> {
    // Deny-by-default choke-point (TASK-180): resolve the target task's repo from
    // storage and run the tier policy. An Agent/Orchestrator may only message a
    // task in its own repo; the read-only concierge is denied. The store guard is
    // taken briefly and dropped inside authorize_call — never held across an await.
    authorize_call(
        app,
        ctx,
        Capability::StirTask,
        ResolutionStrategy::FromTaskId,
        None,
        Some(&task_id),
    )
    .await
    .map_err(|e| e.into_message())?;

    let state = app.state::<AppState>();

    // Resolve the task's live agent. Snapshot each PTY map SEPARATELY (clone / drop
    // the guard) — never hold two PTY locks at once — then resolve against the owned
    // snapshots (mirrors remote::server::reply_handler).
    let agent_id = {
        let agent_tasks = state
            .agent_tasks
            .lock()
            .map_err(|e| format!("{e:#}"))?
            .clone();
        let live: std::collections::HashSet<String> = state
            .sessions
            .lock()
            .map_err(|e| format!("{e:#}"))?
            .keys()
            .cloned()
            .collect();
        crate::session::resolve_live_agent(&agent_tasks, &live, &task_id)
    };
    let Some(agent_id) = agent_id else {
        return Err(format!(
            "no running agent for task {task_id} — start or resume it first"
        ));
    };

    // Deliver as a bracketed paste, then submit with Enter. The Enter MUST be a
    // SEPARATE PTY read from the paste (a short gap forces a distinct read), or
    // Claude's TUI consumes the `\r` as the paste terminator and the message sits
    // unsubmitted — see remote::server::reply_handler for the full rationale.
    let paste = crate::session::bracketed_paste(&message);
    crate::agent::write_to_session(state.inner(), &agent_id, &paste)?;
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    crate::agent::write_to_session(state.inner(), &agent_id, "\r")?;

    Ok(format!("delivered to task {task_id}"))
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

/// Read one repo's tasks as `TaskSummary` — the repo-filtered read served to the
/// Agent/Orchestrator tiers (the concierge read stays cross-repo via `list_tasks`).
fn list_tasks_in_repo(app: &tauri::AppHandle, repo_id: &str) -> Result<Vec<TaskSummary>, String> {
    Ok(list_tasks(app)?
        .into_iter()
        .filter(|t| t.repo_id == repo_id)
        .collect())
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

/// Execute `create_schedule`: resolve+scope the repo, validate fields, compute
/// the first fire, and insert. Returns the created schedule.
/// Lock the store, surfacing a poisoned-lock error with full context (the
/// REVIEW.md error-mapping invariant). Centralizes the lock boilerplate the
/// schedule glue shares so it lives in exactly one place.
fn lock_store(state: &AppState) -> Result<std::sync::MutexGuard<'_, crate::store::TaskStore>, String> {
    state.store.lock().map_err(|e| format!("{e:#}"))
}

async fn do_create_schedule(
    app: &tauri::AppHandle,
    ctx: &CallContext,
    args: CreateScheduleArgs,
) -> Result<crate::store::Schedule, String> {
    // Deny-by-default choke-point (TASK-180): resolve+authorize the target repo.
    // An Agent/Orchestrator manages only its own repo's schedules; a cross-repo
    // arg is denied and the read-only Concierge is denied outright.
    let repo_id = authorize_call(
        app,
        ctx,
        Capability::ManageSchedule,
        ResolutionStrategy::RepoArg,
        args.repo.as_deref(),
        None,
    )
    .await
    .map_err(|e| e.into_message())?
    .repo_id;
    let fields = crate::schedule::validate_schedule_fields(
        args.name.as_deref().unwrap_or(""),
        args.prompt.as_deref().unwrap_or(""),
        args.cron.as_deref().unwrap_or(""),
        args.agent,
        args.model,
        args.base_branch,
    )?;
    let now = crate::schedule::now_secs_pub();
    let next_run_at = crate::schedule::next_run_after(&fields.cron, chrono::Local::now())?;
    let schedule = crate::store::Schedule {
        id: uuid::Uuid::new_v4().to_string(),
        repo_id,
        name: fields.name,
        prompt: fields.prompt,
        cron: fields.cron,
        agent: fields.agent,
        model: fields.model,
        base_branch: fields.base_branch,
        enabled: true,
        one_shot: false,
        // TASK-181: MCP-created schedules default to skipping the repo prompt.
        skip_repo_prompt: true,
        next_run_at,
        last_run_at: None,
        created_at: now,
        updated_at: now,
    };
    let state = app.state::<AppState>();
    let store = lock_store(&state)?;
    store.insert_schedule(&schedule).map_err(|e| format!("{e:#}"))?;
    Ok(schedule)
}

/// Execute `list_schedules`: resolve+scope the repo, return its schedules.
async fn do_list_schedules(
    app: &tauri::AppHandle,
    ctx: &CallContext,
    repo_arg: Option<String>,
) -> Result<Vec<crate::store::Schedule>, String> {
    // Deny-by-default choke-point (TASK-180): only the caller's own repo; the
    // read-only Concierge is denied (revoking the TASK-178 any-repo grant).
    let repo_id = authorize_call(
        app,
        ctx,
        Capability::ManageSchedule,
        ResolutionStrategy::RepoArg,
        repo_arg.as_deref(),
        None,
    )
    .await
    .map_err(|e| e.into_message())?
    .repo_id;
    let state = app.state::<AppState>();
    let store = lock_store(&state)?;
    store.list_schedules(&repo_id).map_err(|e| format!("{e:#}"))
}

/// Execute `update_schedule`: authorize against the stored schedule's repo,
/// validate the new fields, recompute the next fire, and full-replace.
async fn do_update_schedule(
    app: &tauri::AppHandle,
    ctx: &CallContext,
    args: UpdateScheduleArgs,
) -> Result<crate::store::Schedule, String> {
    // Deny-by-default choke-point (TASK-180): resolve the target repo from the
    // stored schedule row (never caller input); an Agent/Orchestrator may touch
    // only its own repo, the read-only Concierge is denied.
    authorize_call(
        app,
        ctx,
        Capability::ManageSchedule,
        ResolutionStrategy::FromScheduleId,
        None,
        Some(&args.schedule_id),
    )
    .await
    .map_err(|e| e.into_message())?;
    let fields = crate::schedule::validate_schedule_fields(
        args.name.as_deref().unwrap_or(""),
        args.prompt.as_deref().unwrap_or(""),
        args.cron.as_deref().unwrap_or(""),
        args.agent,
        args.model,
        args.base_branch,
    )?;
    let next_run_at = if args.enabled {
        crate::schedule::next_run_after(&fields.cron, chrono::Local::now())?
    } else {
        None
    };
    let now = crate::schedule::now_secs_pub();
    let state = app.state::<AppState>();
    let store = lock_store(&state)?;
    // TASK-181: MCP update preserves the stored skip-repo-prompt flag (no MCP arg).
    let current = store
        .get_schedule(&args.schedule_id)
        .map_err(|e| format!("{e:#}"))?
        .ok_or_else(|| "schedule not found".to_string())?;
    store
        .update_schedule_fields(
            &args.schedule_id, &fields.name, &fields.prompt, &fields.cron,
            fields.agent.as_deref(), fields.model.as_deref(), fields.base_branch.as_deref(),
            args.enabled, current.skip_repo_prompt, next_run_at, now,
        )
        .map_err(|e| format!("{e:#}"))?;
    store
        .get_schedule(&args.schedule_id)
        .map_err(|e| format!("{e:#}"))?
        .ok_or_else(|| "schedule not found after update".to_string())
}

/// Execute `set_schedule_enabled`: authorize, then toggle enabled (recomputing
/// the next fire when enabling), leaving the other fields untouched.
async fn do_set_schedule_enabled(
    app: &tauri::AppHandle,
    ctx: &CallContext,
    schedule_id: &str,
    enabled: bool,
) -> Result<crate::store::Schedule, String> {
    // Deny-by-default choke-point (TASK-180): repo resolved from the schedule row.
    authorize_call(
        app,
        ctx,
        Capability::ManageSchedule,
        ResolutionStrategy::FromScheduleId,
        None,
        Some(schedule_id),
    )
    .await
    .map_err(|e| e.into_message())?;
    let state = app.state::<AppState>();
    let store = lock_store(&state)?;
    let current = store
        .get_schedule(schedule_id)
        .map_err(|e| format!("{e:#}"))?
        .ok_or_else(|| "schedule not found".to_string())?;
    let next_run_at = if enabled {
        crate::schedule::next_run_after(&current.cron, chrono::Local::now())?
    } else {
        None
    };
    let now = crate::schedule::now_secs_pub();
    store
        .update_schedule_fields(
            schedule_id, &current.name, &current.prompt, &current.cron,
            current.agent.as_deref(), current.model.as_deref(), current.base_branch.as_deref(),
            enabled, current.skip_repo_prompt, next_run_at, now,
        )
        .map_err(|e| format!("{e:#}"))?;
    store
        .get_schedule(schedule_id)
        .map_err(|e| format!("{e:#}"))?
        .ok_or_else(|| "schedule not found after update".to_string())
}

/// Execute `delete_schedule`: authorize against the stored schedule's repo,
/// then delete. Returns the id for the confirmation message.
async fn do_delete_schedule(
    app: &tauri::AppHandle,
    ctx: &CallContext,
    schedule_id: &str,
) -> Result<String, String> {
    // Deny-by-default choke-point (TASK-180): repo resolved from the schedule row.
    authorize_call(
        app,
        ctx,
        Capability::ManageSchedule,
        ResolutionStrategy::FromScheduleId,
        None,
        Some(schedule_id),
    )
    .await
    .map_err(|e| e.into_message())?;
    let state = app.state::<AppState>();
    let store = lock_store(&state)?;
    store
        .get_schedule(schedule_id)
        .map_err(|e| format!("{e:#}"))?
        .ok_or_else(|| "schedule not found".to_string())?;
    store.delete_schedule(schedule_id).map_err(|e| format!("{e:#}"))?;
    Ok(schedule_id.to_string())
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
        Routed::QueueDependency { id, args } => {
            // queue_dependency (TASK-164) is start_task with a guaranteed-non-empty
            // dependency list — same handler, same authz (Capability::StartTask),
            // same response formatter (the created task is always Pending/queued).
            let result = do_start_task(&app, &ctx, args).await;
            Json(start_task_response(id, result)).into_response()
        }
        Routed::FinishTask { id, args } => {
            // Scope is enforced inside do_finish_task: an Agent token may finish its
            // own task (no concierge needed), so we can't gate on is_concierge here.
            let result = do_finish_task(&app, &ctx, args).await;
            Json(finish_task_response(id, result)).into_response()
        }
        Routed::ScheduleTask { id, args } => {
            let result = do_schedule_task(&app, &ctx, args).await;
            Json(schedule_task_response(id, result)).into_response()
        }
        Routed::ListRepos { id } => {
            let result = list_repos(&app);
            Json(list_repos_response(id, result)).into_response()
        }
        Routed::ListTasks { id } => {
            // Control-plane read via the choke-point (TASK-180). The legacy global
            // concierge keeps its cross-repo read (return all tasks); an
            // Agent/Orchestrator token is repo-filtered to its own repo.
            let result = match &ctx {
                CallContext::Concierge => list_tasks(&app),
                _ => match authorize_call(
                    &app,
                    &ctx,
                    Capability::ReadControlPlane,
                    ResolutionStrategy::CallerRepo,
                    None,
                    None,
                )
                .await
                {
                    Ok(authz) => list_tasks_in_repo(&app, &authz.repo_id),
                    Err(e) => Err(e.into_message()),
                },
            };
            Json(list_tasks_response(id, result)).into_response()
        }
        Routed::TaskStatus { id, task_id } => {
            // Resolve the task's repo from storage, then authorize: the concierge
            // may read any repo; an Agent/Orchestrator only its own (TASK-180).
            let result = match authorize_call(
                &app,
                &ctx,
                Capability::ReadControlPlane,
                ResolutionStrategy::FromTaskId,
                None,
                Some(&task_id),
            )
            .await
            {
                Ok(_) => task_status(&app, &task_id),
                Err(e) => Err(e.into_message()),
            };
            Json(task_status_response(id, result)).into_response()
        }
        Routed::GetTaskActivity { id, task_id, since } => {
            let result = match authorize_call(
                &app,
                &ctx,
                Capability::ReadControlPlane,
                ResolutionStrategy::FromTaskId,
                None,
                Some(&task_id),
            )
            .await
            {
                Ok(_) => get_task_activity(&app, &task_id, since),
                Err(e) => Err(e.into_message()),
            };
            Json(get_task_activity_response(id, result)).into_response()
        }
        Routed::CreateSchedule { id, args } => {
            let result = do_create_schedule(&app, &ctx, args).await;
            Json(schedule_response(id, result)).into_response()
        }
        Routed::ListSchedules { id, repo } => {
            let result = do_list_schedules(&app, &ctx, repo).await;
            Json(list_schedules_response(id, result)).into_response()
        }
        Routed::UpdateSchedule { id, args } => {
            let result = do_update_schedule(&app, &ctx, args).await;
            Json(schedule_response(id, result)).into_response()
        }
        Routed::SetScheduleEnabled { id, schedule_id, enabled } => {
            let result = do_set_schedule_enabled(&app, &ctx, &schedule_id, enabled).await;
            Json(schedule_response(id, result)).into_response()
        }
        Routed::DeleteSchedule { id, schedule_id } => {
            let result = do_delete_schedule(&app, &ctx, &schedule_id).await;
            Json(delete_schedule_response(id, result)).into_response()
        }
        Routed::SendTaskMessage { id, task_id, message } => {
            // Scope is enforced inside do_send_task_message via the TASK-180
            // choke-point: an Agent/Orchestrator may message a task in its own
            // repo; the concierge is denied. So we can't gate on tier here.
            let result = do_send_task_message(&app, &ctx, task_id, message).await;
            Json(send_task_message_response(id, result)).into_response()
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
        assert_eq!(props["afterMergeOf"]["type"], json!(["string", "array"]));
    }

    #[test]
    fn parse_start_task_args_reads_after_merge_of() {
        let args = serde_json::json!({ "title": "Follow-up", "afterMergeOf": "task-abc" });
        let parsed = parse_start_task_args(&args);
        assert_eq!(parsed.after_merge_of, vec!["task-abc".to_string()]);
    }

    #[test]
    fn start_task_parses_after_merge_of_string() {
        let req = json!({
            "jsonrpc":"2.0","id":7,"method":"tools/call",
            "params":{"name":"start_task","arguments":{"title":"x","afterMergeOf":"TASK-7"}}
        });
        let Routed::StartTask { args, .. } = route(&req) else { panic!("expected StartTask") };
        assert_eq!(args.after_merge_of, vec!["TASK-7".to_string()]);
    }

    #[test]
    fn start_task_parses_after_merge_of_array() {
        let req = json!({
            "jsonrpc":"2.0","id":7,"method":"tools/call",
            "params":{"name":"start_task","arguments":{"title":"x","afterMergeOf":["TASK-7","TASK-8"]}}
        });
        let Routed::StartTask { args, .. } = route(&req) else { panic!("expected StartTask") };
        assert_eq!(args.after_merge_of, vec!["TASK-7".to_string(), "TASK-8".to_string()]);
    }

    #[test]
    fn start_task_after_merge_of_absent_is_empty() {
        let req = json!({
            "jsonrpc":"2.0","id":7,"method":"tools/call",
            "params":{"name":"start_task","arguments":{"title":"x"}}
        });
        let Routed::StartTask { args, .. } = route(&req) else { panic!("expected StartTask") };
        assert!(args.after_merge_of.is_empty());
    }

    // --- queue_dependency (TASK-164) ---

    #[test]
    fn tools_list_advertises_queue_dependency_with_required_depends_on() {
        let result = tools_list_result();
        let tool = result["tools"]
            .as_array().unwrap()
            .iter()
            .find(|t| t["name"] == "queue_dependency")
            .expect("queue_dependency must be advertised");
        let props = &tool["inputSchema"]["properties"];
        assert_eq!(props["dependsOn"]["type"], json!(["string", "array"]));
        // dependsOn is required; the other fields mirror start_task and are optional.
        assert_eq!(tool["inputSchema"]["required"], json!(["dependsOn"]));
        assert!(props["title"].is_object());
        assert!(props["ticketKey"].is_object());
        assert!(props["prompt"].is_object());
    }

    #[test]
    fn queue_dependency_parses_depends_on_string() {
        let req = json!({
            "jsonrpc":"2.0","id":20,"method":"tools/call",
            "params":{"name":"queue_dependency","arguments":{"title":"x","dependsOn":"TASK-7"}}
        });
        let Routed::QueueDependency { id, args } = route(&req) else {
            panic!("expected QueueDependency")
        };
        assert_eq!(id, json!(20));
        assert_eq!(args.after_merge_of, vec!["TASK-7".to_string()]);
        assert_eq!(args.title.as_deref(), Some("x"));
    }

    #[test]
    fn queue_dependency_parses_depends_on_array_and_fields() {
        let req = json!({
            "jsonrpc":"2.0","id":21,"method":"tools/call",
            "params":{"name":"queue_dependency","arguments":{
                "dependsOn":["TASK-7","TASK-8"],"ticketKey":"TASK-99","repo":"r1",
                "agent":"claude","prompt":"go"
            }}
        });
        let Routed::QueueDependency { args, .. } = route(&req) else {
            panic!("expected QueueDependency")
        };
        assert_eq!(args.after_merge_of, vec!["TASK-7".to_string(), "TASK-8".to_string()]);
        assert_eq!(args.ticket_key.as_deref(), Some("TASK-99"));
        assert_eq!(args.repo.as_deref(), Some("r1"));
        assert_eq!(args.agent.as_deref(), Some("claude"));
        assert_eq!(args.prompt.as_deref(), Some("go"));
    }

    #[test]
    fn queue_dependency_missing_depends_on_is_invalid_params() {
        let req = json!({
            "jsonrpc":"2.0","id":22,"method":"tools/call",
            "params":{"name":"queue_dependency","arguments":{"title":"x"}}
        });
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        assert_eq!(resp["error"]["code"], -32602);
        assert!(resp["error"]["message"].as_str().unwrap().contains("dependsOn"));
    }

    #[test]
    fn queue_dependency_empty_array_depends_on_is_invalid_params() {
        // An empty list would otherwise resolve to Immediate — a silent launch-now.
        let req = json!({
            "jsonrpc":"2.0","id":23,"method":"tools/call",
            "params":{"name":"queue_dependency","arguments":{"title":"x","dependsOn":[]}}
        });
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        assert_eq!(resp["error"]["code"], -32602);
    }

    #[test]
    fn queue_dependency_ignores_after_merge_of_key() {
        // The dependency list is read from `dependsOn`, not start_task's
        // `afterMergeOf`; using the old key alone is a usage error.
        let req = json!({
            "jsonrpc":"2.0","id":24,"method":"tools/call",
            "params":{"name":"queue_dependency","arguments":{"title":"x","afterMergeOf":"TASK-7"}}
        });
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        assert_eq!(resp["error"]["code"], -32602);
    }

    // --- TASK-223: optional `model` on the task-creation tools ---

    #[test]
    fn parse_start_task_args_reads_model() {
        let parsed = parse_start_task_args(&json!({ "title": "x", "model": "opus" }));
        assert_eq!(parsed.model.as_deref(), Some("opus"));
        // Omitted ⇒ None (engine default).
        assert_eq!(parse_start_task_args(&json!({ "title": "x" })).model, None);
    }

    #[test]
    fn start_task_routes_model_into_launch_args() {
        // Route-level wiring: a passed `model` reaches StartTaskArgs, which
        // do_start_task threads verbatim into LaunchArgs.model (TASK-223).
        let req = json!({
            "jsonrpc":"2.0","id":30,"method":"tools/call",
            "params":{"name":"start_task","arguments":{"title":"x","model":"sonnet"}}
        });
        let Routed::StartTask { args, .. } = route(&req) else { panic!("expected StartTask") };
        assert_eq!(args.model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn queue_dependency_routes_model() {
        let req = json!({
            "jsonrpc":"2.0","id":31,"method":"tools/call",
            "params":{"name":"queue_dependency","arguments":{
                "title":"x","dependsOn":"TASK-7","model":"haiku"
            }}
        });
        let Routed::QueueDependency { args, .. } = route(&req) else {
            panic!("expected QueueDependency")
        };
        assert_eq!(args.model.as_deref(), Some("haiku"));
    }

    #[test]
    fn task_creation_tools_advertise_model() {
        let result = tools_list_result();
        let tools = result["tools"].as_array().unwrap();
        for name in ["start_task", "queue_dependency", "schedule_task"] {
            let tool = tools
                .iter()
                .find(|t| t["name"] == name)
                .unwrap_or_else(|| panic!("{name} must be advertised"));
            assert!(
                tool["inputSchema"]["properties"]["model"].is_object(),
                "{name} must advertise the optional `model` property"
            );
        }
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
                blocking: vec![],
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
                blocking: vec![
                    BlockingRef { task_id: "dep-1".into(), title: Some("blocking3".into()) },
                    BlockingRef { task_id: "dep-2".into(), title: None },
                ],
            }),
        );
        assert_eq!(resp["id"], 21);
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Queued"), "expected Queued in: {text}");
        assert!(text.contains("Pending"), "expected Pending in: {text}");
        assert!(text.contains("dep-1"), "expected first blocking id in: {text}");
        assert!(text.contains("blocking3"), "expected first blocking title in: {text}");
        assert!(text.contains("dep-2"), "expected second blocking id in: {text}");
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
            in_place: false,
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

    #[test]
    fn orchestrator_token_maps_to_repo_scoped_context() {
        let tok = crate::state::McpToken::Orchestrator { repo_id: "r1".into() };
        match context_from_token(&tok) {
            CallContext::Orchestrator { repo_id } => assert_eq!(repo_id, "r1"),
            other => panic!("expected Orchestrator, got {other:?}"),
        }
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
    fn resolve_finish_target_orchestrator_requires_explicit_task() {
        // An orchestrator has no own task, so — like a concierge — it must name a
        // target explicitly. Repo scoping (own-repo only) is then enforced by the
        // `authorize_call(FinishTask, FromTaskId)` choke-point in `do_finish_task`
        // (TASK-180 B2), which resolves the task's repo from storage.
        let orch = CallContext::Orchestrator { repo_id: "r1".into() };
        assert_eq!(resolve_finish_target(&orch, Some("  t9  ")).unwrap(), "t9");
        let err = resolve_finish_target(&orch, None).unwrap_err();
        assert!(err.contains("requires an explicit taskId"));
        // A blank/whitespace taskId is treated as absent.
        assert!(resolve_finish_target(&orch, Some("   ")).is_err());
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
        assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("orchestrator-scope token"));
    }

    // ── TASK-178: schedule tools — pure parse + authz ──────────────────────

    #[test]
    fn parse_create_schedule_args_reads_all_fields() {
        let a = serde_json::json!({
            "repoId": "r1", "name": "nightly", "prompt": "/security-scan",
            "cron": "0 2 * * *", "agent": "claude", "model": "opus", "baseBranch": "main"
        });
        let p = parse_create_schedule_args(&a);
        assert_eq!(p.repo.as_deref(), Some("r1"));
        assert_eq!(p.name.as_deref(), Some("nightly"));
        assert_eq!(p.prompt.as_deref(), Some("/security-scan"));
        assert_eq!(p.cron.as_deref(), Some("0 2 * * *"));
        assert_eq!(p.agent.as_deref(), Some("claude"));
        assert_eq!(p.model.as_deref(), Some("opus"));
        assert_eq!(p.base_branch.as_deref(), Some("main"));
    }

    #[test]
    fn parse_update_schedule_args_requires_schedule_id_and_enabled() {
        let ok = parse_update_schedule_args(&serde_json::json!({
            "scheduleId": "s1", "name": "n", "prompt": "p", "cron": "0 2 * * *", "enabled": false
        }))
        .expect("valid");
        assert_eq!(ok.schedule_id, "s1");
        assert!(!ok.enabled);
        assert_eq!(ok.name.as_deref(), Some("n"));

        assert!(parse_update_schedule_args(&serde_json::json!({"enabled": true}))
            .unwrap_err()
            .contains("scheduleId"));
        assert!(parse_update_schedule_args(&serde_json::json!({"scheduleId": "s1"}))
            .unwrap_err()
            .contains("enabled"));
    }

    fn sample_schedule() -> crate::store::Schedule {
        crate::store::Schedule {
            id: "s1".into(),
            repo_id: "r1".into(),
            name: "nightly".into(),
            prompt: "/security-scan".into(),
            cron: "0 2 * * *".into(),
            agent: None,
            model: None,
            base_branch: None,
            enabled: true,
            one_shot: false,
            skip_repo_prompt: true,
            next_run_at: Some(1_800_000_000),
            last_run_at: None,
            created_at: 1,
            updated_at: 2,
        }
    }

    #[test]
    fn tools_list_advertises_all_schedule_tools() {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}});
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        let names: Vec<&str> = resp["result"]["tools"].as_array().unwrap()
            .iter().map(|t| t["name"].as_str().unwrap()).collect();
        for n in ["create_schedule", "list_schedules", "update_schedule", "set_schedule_enabled", "delete_schedule"] {
            assert!(names.contains(&n), "missing tool: {n}");
        }
        let tools = resp["result"]["tools"].as_array().unwrap();
        let create = tools.iter().find(|t| t["name"] == "create_schedule").unwrap();
        let props = &create["inputSchema"]["properties"];
        assert!(props["repoId"].is_object());
        assert!(props["cron"].is_object());
        let del = tools.iter().find(|t| t["name"] == "delete_schedule").unwrap();
        assert_eq!(del["inputSchema"]["required"][0], "scheduleId");
    }

    #[test]
    fn tools_call_create_schedule_routes_with_parsed_args() {
        let req = json!({"jsonrpc":"2.0","id":30,"method":"tools/call","params":{
            "name":"create_schedule","arguments":{"repoId":"r1","name":"n","prompt":"p","cron":"0 2 * * *"}}});
        let Routed::CreateSchedule { id, args } = route(&req) else { panic!("expected CreateSchedule") };
        assert_eq!(id, json!(30));
        assert_eq!(args.repo.as_deref(), Some("r1"));
        assert_eq!(args.cron.as_deref(), Some("0 2 * * *"));
    }

    #[test]
    fn tools_call_list_schedules_routes_with_optional_repo() {
        let req = json!({"jsonrpc":"2.0","id":31,"method":"tools/call","params":{
            "name":"list_schedules","arguments":{"repoId":"r1"}}});
        let Routed::ListSchedules { id, repo } = route(&req) else { panic!("expected ListSchedules") };
        assert_eq!(id, json!(31));
        assert_eq!(repo.as_deref(), Some("r1"));
    }

    #[test]
    fn tools_call_update_schedule_missing_required_is_invalid_params() {
        let req = json!({"jsonrpc":"2.0","id":32,"method":"tools/call","params":{
            "name":"update_schedule","arguments":{"scheduleId":"s1"}}}); // no enabled
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        assert_eq!(resp["error"]["code"], -32602);
    }

    #[test]
    fn tools_call_set_schedule_enabled_routes() {
        let req = json!({"jsonrpc":"2.0","id":33,"method":"tools/call","params":{
            "name":"set_schedule_enabled","arguments":{"scheduleId":"s1","enabled":false}}});
        let Routed::SetScheduleEnabled { id, schedule_id, enabled } = route(&req) else { panic!("expected SetScheduleEnabled") };
        assert_eq!(id, json!(33));
        assert_eq!(schedule_id, "s1");
        assert!(!enabled);
    }

    #[test]
    fn tools_call_set_schedule_enabled_missing_enabled_is_invalid_params() {
        let req = json!({"jsonrpc":"2.0","id":34,"method":"tools/call","params":{
            "name":"set_schedule_enabled","arguments":{"scheduleId":"s1"}}});
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        assert_eq!(resp["error"]["code"], -32602);
    }

    #[test]
    fn tools_call_delete_schedule_routes_and_requires_id() {
        let ok = json!({"jsonrpc":"2.0","id":35,"method":"tools/call","params":{
            "name":"delete_schedule","arguments":{"scheduleId":"s1"}}});
        let Routed::DeleteSchedule { id, schedule_id } = route(&ok) else { panic!("expected DeleteSchedule") };
        assert_eq!(id, json!(35));
        assert_eq!(schedule_id, "s1");

        let bad = json!({"jsonrpc":"2.0","id":36,"method":"tools/call","params":{
            "name":"delete_schedule","arguments":{}}});
        let Routed::Respond(resp) = route(&bad) else { panic!("expected Respond") };
        assert_eq!(resp["error"]["code"], -32602);
    }

    #[test]
    fn schedule_response_ok_serializes_camelcase_next_run_at() {
        let resp = schedule_response(json!(40), Ok(sample_schedule()));
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("nextRunAt"));
        assert!(text.contains("\"repoId\": \"r1\""));
    }

    #[test]
    fn schedule_response_err_is_error_text() {
        let resp = schedule_response(json!(41), Err("schedule not found".into()));
        assert_eq!(resp["result"]["isError"], true);
        assert!(resp["result"]["content"][0]["text"].as_str().unwrap().contains("not found"));
    }

    #[test]
    fn list_schedules_response_ok_and_err() {
        let ok = list_schedules_response(json!(42), Ok(vec![sample_schedule()]));
        assert_eq!(ok["result"]["isError"], false);
        assert!(ok["result"]["content"][0]["text"].as_str().unwrap().contains("nextRunAt"));
        let err = list_schedules_response(json!(43), Err("boom".into()));
        assert_eq!(err["result"]["isError"], true);
    }

    #[test]
    fn delete_schedule_response_ok_and_err() {
        let ok = delete_schedule_response(json!(44), Ok("s1".into()));
        assert_eq!(ok["result"]["isError"], false);
        assert!(ok["result"]["content"][0]["text"].as_str().unwrap().contains("s1"));
        let err = delete_schedule_response(json!(45), Err("not authorized: ManageSchedule".to_string()));
        assert_eq!(err["result"]["isError"], true);
        assert!(err["result"]["content"][0]["text"].as_str().unwrap().contains("not authorized"));
    }

    // ── TASK-179: schedule_task ────────────────────────────────────────────

    #[test]
    fn tools_list_advertises_schedule_task() {
        let result = tools_list_result();
        let tool = result["tools"]
            .as_array().unwrap()
            .iter()
            .find(|t| t["name"] == "schedule_task").unwrap();
        let props = &tool["inputSchema"]["properties"];
        assert!(props["inHours"].is_object());
        assert!(props["atUnix"].is_object());
        assert!(props["prompt"].is_object());
    }

    #[test]
    fn route_dispatches_schedule_task() {
        let req = json!({
            "jsonrpc":"2.0","id":7,"method":"tools/call",
            "params":{"name":"schedule_task","arguments":{"title":"Later","inHours":3}}
        });
        let Routed::ScheduleTask { args, .. } = route(&req) else { panic!("expected ScheduleTask") };
        assert_eq!(args.title.as_deref(), Some("Later"));
        assert_eq!(args.in_seconds, Some(10_800)); // 3h → 10800s
    }

    #[test]
    fn parse_schedule_task_in_seconds_wins_over_in_hours() {
        let args = parse_schedule_task_args(&json!({"inSeconds": 42, "inHours": 3}));
        assert_eq!(args.in_seconds, Some(42));
    }

    #[test]
    fn parse_schedule_task_reads_at_unix() {
        let args = parse_schedule_task_args(&json!({"atUnix": 2_000_000_000}));
        assert_eq!(args.at_unix, Some(2_000_000_000));
        assert_eq!(args.in_seconds, None);
    }

    #[test]
    fn parse_schedule_task_reads_model() {
        // TASK-223: model flows to the deferred launch's schedule row.
        let args = parse_schedule_task_args(&json!({"title": "Later", "model": "opus"}));
        assert_eq!(args.model.as_deref(), Some("opus"));
        assert_eq!(parse_schedule_task_args(&json!({"title": "Later"})).model, None);
    }

    // ── TASK-190: send_task_message ────────────────────────────────────────

    #[test]
    fn tools_list_advertises_send_task_message() {
        let result = tools_list_result();
        let tool = result["tools"]
            .as_array().unwrap()
            .iter()
            .find(|t| t["name"] == "send_task_message").unwrap();
        let props = &tool["inputSchema"]["properties"];
        assert!(props["taskId"].is_object());
        assert!(props["message"].is_object());
        let required: Vec<&str> = tool["inputSchema"]["required"]
            .as_array().unwrap()
            .iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required.contains(&"taskId"));
        assert!(required.contains(&"message"));
    }

    #[test]
    fn route_dispatches_send_task_message() {
        let req = json!({
            "jsonrpc":"2.0","id":8,"method":"tools/call",
            "params":{"name":"send_task_message","arguments":{"taskId":"t1","message":"keep going"}}
        });
        let Routed::SendTaskMessage { task_id, message, .. } = route(&req)
            else { panic!("expected SendTaskMessage") };
        assert_eq!(task_id, "t1");
        assert_eq!(message, "keep going");
    }

    #[test]
    fn route_send_task_message_missing_task_id_is_invalid_params() {
        let req = json!({
            "jsonrpc":"2.0","id":9,"method":"tools/call",
            "params":{"name":"send_task_message","arguments":{"message":"hi"}}
        });
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        assert_eq!(resp["error"]["code"], -32602);
        assert!(resp["error"]["message"].as_str().unwrap().contains("taskId"));
    }

    #[test]
    fn route_send_task_message_missing_message_is_invalid_params() {
        let req = json!({
            "jsonrpc":"2.0","id":10,"method":"tools/call",
            "params":{"name":"send_task_message","arguments":{"taskId":"t1"}}
        });
        let Routed::Respond(resp) = route(&req) else { panic!("expected Respond") };
        assert_eq!(resp["error"]["code"], -32602);
        assert!(resp["error"]["message"].as_str().unwrap().contains("message"));
    }

    #[test]
    fn send_task_message_response_formats_success_and_error() {
        let ok = send_task_message_response(json!(11), Ok("delivered to task t1".into()));
        assert_eq!(ok["result"]["isError"], false);
        assert!(ok["result"]["content"][0]["text"].as_str().unwrap().contains("delivered"));
        let err = send_task_message_response(
            json!(12),
            Err("no running agent for task t1 — start or resume it first".into()),
        );
        assert_eq!(err["result"]["isError"], true);
        assert!(err["result"]["content"][0]["text"].as_str().unwrap().contains("no running agent"));
    }
}
