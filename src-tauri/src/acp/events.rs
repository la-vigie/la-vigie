//! The frontend-facing event vocabulary for the ACP backend. Mirrors the
//! frozen IPC contract in
//! `docs/superpowers/specs/2026-07-17-acp-backend-engine-design.md` — a
//! `Channel<AcpEvent>` streams these to the frontend, same shape as the PTY
//! path's byte stream, but structured. All payloads are JSON-friendly (plain
//! `String`/`serde_json::Value`) rather than re-exporting ACP's own types, so
//! this contract doesn't silently shift when the ACP crate does.
//!
//! No I/O here — construction is either pure (`translate.rs`, from a
//! `SessionUpdate`) or done by the connection driver (`acp/mod.rs`) for the
//! variants that don't originate from a `session/update` notification
//! (`SessionStarted`, `PermissionRequest`, `TurnEnded`, `Error`, `Exit`).

use serde::{Deserialize, Serialize};

/// Who produced a streamed message chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageRole {
    User,
    Assistant,
}

/// One entry in an agent's execution plan, projected from ACP's
/// `PlanEntry` (`priority`/`status` carried as their wire strings — see
/// `translate::plan_priority_label`/`plan_status_label`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntryEvent {
    pub content: String,
    pub priority: String,
    pub status: String,
}

/// Cumulative session cost, projected from ACP's `Cost`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostEvent {
    pub amount: f64,
    pub currency: String,
}

/// A best-effort rate-limit / quota snapshot parsed from the agent-defined
/// `UsageUpdate._meta` blob, which is reserved for agent extensions and carries
/// no schema guarantees — only Claude's `_claude/rateLimit` key is understood;
/// anything else yields `None` rather than a fabricated status. All fields but
/// `status` are optional since a provider may omit them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitEvent {
    /// e.g. "allowed" / "rejected" (agent-defined vocabulary).
    pub status: String,
    /// Unix seconds at which the window resets, if reported.
    pub reset_at: Option<i64>,
    /// e.g. "five_hour" (agent-defined).
    pub limit_type: Option<String>,
    /// Whether the session is currently drawing on overage capacity.
    pub using_overage: Option<bool>,
}

/// The tagged event enum streamed to the frontend over `Channel<AcpEvent>`.
/// `#[serde(tag = "type", rename_all = "camelCase")]` matches every other
/// camelCase-over-IPC contract in this codebase (`AgentSpec`, `Task`, …).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AcpEvent {
    /// A session was created (fresh `session/new`, or resumed via
    /// `session/load`/`session/resume`). `modes`/`models` are the raw
    /// `SessionModeState`/model `SessionConfigOption` JSON — open-ended
    /// enough (nested option lists) that re-typing them here would just
    /// duplicate the ACP schema; the frontend reads them structurally.
    SessionStarted {
        session_id: String,
        modes: Option<serde_json::Value>,
        models: Option<serde_json::Value>,
    },
    /// A chunk of a user or agent message being streamed
    /// (`UserMessageChunk`/`AgentMessageChunk`). `replay` marks a chunk
    /// forwarded from resumed session history rather than a live turn.
    MessageChunk {
        role: MessageRole,
        text: String,
        message_id: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        replay: bool,
    },
    /// A chunk of the agent's internal reasoning (`AgentThoughtChunk`).
    ThoughtChunk {
        text: String,
        message_id: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        replay: bool,
    },
    /// A new tool call was initiated. `content`/`raw_input` stay as
    /// `serde_json::Value` — `ToolCallContent` (Content/Diff/Terminal) is
    /// open-ended and best left for the future timeline UI to interpret.
    ToolCall {
        id: String,
        kind: String,
        title: String,
        status: String,
        content: Vec<serde_json::Value>,
        raw_input: Option<serde_json::Value>,
    },
    /// An update to a previously-announced tool call. Every field but `id`
    /// is optional — only changed fields are present, mirroring ACP's own
    /// `ToolCallUpdateFields`.
    ToolCallUpdate {
        id: String,
        kind: Option<String>,
        title: Option<String>,
        status: Option<String>,
        content: Option<Vec<serde_json::Value>>,
    },
    /// The agent's execution plan for a complex task.
    Plan { entries: Vec<PlanEntryEvent> },
    /// The agent needs authorization before a sensitive tool call
    /// (`session/request_permission`). Constructed by the connection driver —
    /// `session/request_permission` is a request the agent sends, not a
    /// `session/update` notification, so `translate_update` never produces
    /// this variant.
    PermissionRequest {
        request_id: String,
        options: Vec<PermissionOptionEvent>,
        tool_call: serde_json::Value,
    },
    /// The session's active mode changed (`CurrentModeUpdate`), e.g. entering
    /// `bypassPermissions` after an auto-approve toggle.
    ModeChanged { mode_id: String },
    /// The session's active model, from the `Model`-category
    /// `SessionConfigOption`'s `current_value` (at session start and on
    /// `ConfigOptionUpdate`). `UsageUpdate` carries no model, so this is what
    /// attributes usage/cost to one. `model_id` is the agent-defined value id
    /// (e.g. "claude-opus-4"); an agent with no model selector never emits this.
    ModelSelected { model_id: String },
    /// Context-window and cost update. `used`/`size` are a context-window gauge
    /// ("tokens currently in context"), NOT cumulative billed tokens — ACP
    /// exposes no cumulative token count. `cost` is the cumulative session cost
    /// (monotonic) when the agent reports it, else `None` (never computed).
    /// `rate_limit` is the best-effort quota snapshot from `_meta`.
    Usage {
        used: u64,
        size: u64,
        cost: Option<CostEvent>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rate_limit: Option<RateLimitEvent>,
    },
    /// A prompt turn ended (`session/prompt`'s result, not a `session/update`
    /// notification — constructed by the connection driver).
    TurnEnded { stop_reason: String },
    /// A protocol or turn-level error.
    Error { message: String },
    /// The agent process/connection exited.
    Exit { code: Option<i32> },
}

/// One option offered on a `PermissionRequest` (mirrors ACP's
/// `PermissionOption`, kept JSON-friendly rather than importing the ACP type).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOptionEvent {
    pub option_id: String,
    pub name: String,
    pub kind: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_chunk_serializes_camel_case_and_omits_replay_when_false() {
        let event = AcpEvent::MessageChunk {
            role: MessageRole::Assistant,
            text: "pong".to_string(),
            message_id: Some("msg_1".to_string()),
            replay: false,
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["type"], "messageChunk");
        assert_eq!(value["role"], "assistant");
        assert_eq!(value["text"], "pong");
        assert_eq!(value["messageId"], "msg_1");
        assert!(value.get("replay").is_none(), "replay=false should be omitted, got {value}");
    }

    #[test]
    fn message_chunk_serializes_replay_when_true() {
        let event = AcpEvent::MessageChunk {
            role: MessageRole::User,
            text: "hi".to_string(),
            message_id: None,
            replay: true,
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["role"], "user");
        assert_eq!(value["replay"], true);
    }

    #[test]
    fn session_started_serializes_camel_case() {
        let event = AcpEvent::SessionStarted {
            session_id: "s1".to_string(),
            modes: None,
            models: None,
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["type"], "sessionStarted");
        assert_eq!(value["sessionId"], "s1");
    }

    #[test]
    fn turn_ended_serializes_camel_case() {
        let event = AcpEvent::TurnEnded { stop_reason: "end_turn".to_string() };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["type"], "turnEnded");
        assert_eq!(value["stopReason"], "end_turn");
    }
}
