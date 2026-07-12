//! Tauri-managed application state: the task store, the root directory
//! under which task worktrees are created, and the kind-agnostic registry of
//! running PTY sessions.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::agent::SessionHandle;
use crate::store::{SetupStatus, TaskStore};

/// In-memory state for a task's background setup job: the live accumulated
/// log (status is also persisted to the DB), and the job handle so it can be
/// aborted if the task is deleted mid-setup.
pub struct SetupState {
    pub status: SetupStatus,
    pub log: String,
    /// Exit code of the finished setup (None while running or on spawn error).
    pub exit_code: Option<i32>,
    pub handle: Option<tauri::async_runtime::JoinHandle<()>>,
}

/// Originating context for a per-agent MCP token: resolves the caller's
/// task/repo so `start_task` can default `repo` correctly.
pub struct AgentLaunchContext {
    pub task_id: String,
    pub repo_id: String,
}

/// An issued MCP bearer token's scope tier (TASK-111). `Agent` is the per-agent,
/// repo-scoped token minted at agent spawn (TASK-89). `Concierge` is the
/// broad-scope, cross-repo *read* token for the mobile concierge — minted by
/// TASK-112; this ticket only defines and enforces the tier.
pub enum McpToken {
    Agent(AgentLaunchContext),
    Concierge,
}

pub struct AppState {
    pub store: Mutex<TaskStore>,
    pub worktrees_root: PathBuf,
    /// Directory holding imported custom notification sound files.
    pub sounds_root: PathBuf,
    /// Neutral working directory for the rootless concierge session (TASK-112).
    /// A stable dir so `claude --continue`'s cwd-scoped history persists.
    pub concierge_root: PathBuf,
    pub sessions: Mutex<HashMap<String, SessionHandle>>,
    /// Port of the local HookBridge HTTP server (bound at startup, ephemeral).
    pub hook_port: u16,
    /// Per-agent status bookkeeping (run-state + in-flight background count) for
    /// the status state machine (keyed by agent_id).
    pub agent_states: Mutex<HashMap<String, crate::agent::status::AgentState>>,
    /// agent_id → task_id, populated at spawn so hook-driven status can be
    /// persisted to the owning task.
    pub agent_tasks: Mutex<HashMap<String, String>>,
    /// task_id → background setup job state (live log + status + abort handle).
    pub setups: Mutex<HashMap<String, SetupState>>,
    /// Port of the local MCP server (bound at startup, ephemeral).
    pub mcp_port: u16,
    /// An issued MCP bearer token (per-agent `Agent` tier or broad `Concierge`
    /// tier) → its scope. Agent tokens are inserted at agent spawn and removed
    /// at stop; the token is the auth + context carrier.
    pub mcp_tokens: Mutex<HashMap<String, McpToken>>,
    /// Tailnet remote-control server state (TASK-86). `None` active ⇒ off.
    pub remote: crate::remote::RemoteSlot,
    /// task_id → filesystem path of that task's Claude transcript (TASK-108),
    /// captured from hook payloads. Keyed by task so the latest hook reflects the
    /// current file (resume overwrites); retained after the agent stops so the
    /// conversation stays readable post-stop.
    pub transcripts: Mutex<HashMap<String, String>>,
    /// Serializes the concierge create path (TASK-112) so concurrent
    /// `POST /api/concierge` calls cannot both pass the liveness check and
    /// stack processes. Synchronous critical section — never held across `.await`.
    pub concierge_spawn: std::sync::Mutex<()>,
    /// TASK-144: last time we fetched `origin/<base>` for a task's Diff tab,
    /// keyed by `"<repo_id>:<base_branch>"`. Throttles the background base fetch
    /// so frequent re-renders don't spawn a fetch each time. Never held across `.await`.
    pub base_fetch_at: Mutex<HashMap<String, std::time::Instant>>,
}
