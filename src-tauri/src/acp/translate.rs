//! Pure translation: ACP `SessionUpdate` -> `AcpEvent` (the frontend Channel
//! event), plus ACP semantics -> `StatusEvent` mapping. No I/O, no Tauri —
//! unit-tested against wire JSON captured from a live `claude-agent-acp`
//! 0.59.0 session (see `fixtures/`) plus a handful of hand-authored fixtures
//! for variants that session never exercised
//! (`tool_call`/`tool_call_update`/`plan`/`current_mode_update` — the probed
//! prompt was a trivial "reply pong", so no tool call, plan, or mode change
//! ever occurred; those fixtures are schema-accurate JSON built from the
//! `agent-client-protocol-schema` field names/tags rather than captured live).
//! The `available_commands_update` fixture keeps the captured wire *shape* but
//! its command list is replaced with generic placeholders — the live catalog
//! is this machine's private slash-command/skill inventory, which must not
//! enter the publishable tree (the `leak-scan` gate).

use agent_client_protocol::schema::v1::{
    ContentBlock, PlanEntryPriority, PlanEntryStatus, SessionConfigKind, SessionConfigOption,
    SessionConfigOptionCategory, SessionUpdate, StopReason, ToolCallStatus, ToolKind,
};

use crate::agent::status::StatusEvent;

use super::events::{AcpEvent, CostEvent, MessageRole, PlanEntryEvent, RateLimitEvent};

/// Map an ACP `SessionUpdate` notification to zero or more frontend events.
/// `replay` marks updates forwarded from resumed session history (see the
/// design doc's resume section) rather than a live turn; it is threaded
/// through onto `MessageChunk`/`ThoughtChunk` only — the other variants carry
/// no live/replay distinction in the frozen contract.
///
/// `AvailableCommandsUpdate` (the full slash-command/skill catalog — tens of
/// KB per the probe transcript) and `ConfigOptionUpdate`/`SessionInfoUpdate`
/// (no frontend surface yet; model/mode selection and session titling live on
/// the PTY-side UI already) are deliberately dropped (empty `vec![]`) rather
/// than forwarded — there is no timeline UI to render them yet, and forwarding
/// unbounded catalogs over the Channel on every turn would be wasteful. The
/// match is `_ => vec![]` for the same reason plus forward-compat: the crate's
/// `SessionUpdate` is `#[non_exhaustive]` (it also hides feature-gated
/// variants like `PlanUpdate`/`PlanRemoved` we don't enable), so a wildcard
/// arm is required and doubles as the "drop unknown variants" policy.
pub fn translate_update(update: &SessionUpdate, replay: bool) -> Vec<AcpEvent> {
    match update {
        SessionUpdate::UserMessageChunk(chunk) => text_of(&chunk.content)
            .map(|text| {
                vec![AcpEvent::MessageChunk {
                    role: MessageRole::User,
                    text,
                    message_id: chunk.message_id.as_ref().map(|m| m.0.to_string()),
                    replay,
                }]
            })
            .unwrap_or_default(),
        SessionUpdate::AgentMessageChunk(chunk) => text_of(&chunk.content)
            .map(|text| {
                vec![AcpEvent::MessageChunk {
                    role: MessageRole::Assistant,
                    text,
                    message_id: chunk.message_id.as_ref().map(|m| m.0.to_string()),
                    replay,
                }]
            })
            .unwrap_or_default(),
        SessionUpdate::AgentThoughtChunk(chunk) => text_of(&chunk.content)
            .map(|text| {
                vec![AcpEvent::ThoughtChunk {
                    text,
                    message_id: chunk.message_id.as_ref().map(|m| m.0.to_string()),
                    replay,
                }]
            })
            .unwrap_or_default(),
        SessionUpdate::ToolCall(tc) => vec![AcpEvent::ToolCall {
            id: tc.tool_call_id.0.to_string(),
            kind: tool_kind_label(&tc.kind).to_string(),
            title: tc.title.clone(),
            status: tool_status_label(&tc.status).to_string(),
            content: tc.content.iter().filter_map(|c| serde_json::to_value(c).ok()).collect(),
            raw_input: tc.raw_input.clone(),
        }],
        SessionUpdate::ToolCallUpdate(u) => vec![AcpEvent::ToolCallUpdate {
            id: u.tool_call_id.0.to_string(),
            kind: u.fields.kind.as_ref().map(|k| tool_kind_label(k).to_string()),
            title: u.fields.title.clone(),
            status: u.fields.status.as_ref().map(|s| tool_status_label(s).to_string()),
            content: u
                .fields
                .content
                .as_ref()
                .map(|items| items.iter().filter_map(|c| serde_json::to_value(c).ok()).collect()),
        }],
        SessionUpdate::Plan(plan) => vec![AcpEvent::Plan {
            entries: plan
                .entries
                .iter()
                .map(|e| PlanEntryEvent {
                    content: e.content.clone(),
                    priority: plan_priority_label(&e.priority).to_string(),
                    status: plan_status_label(&e.status).to_string(),
                })
                .collect(),
        }],
        SessionUpdate::CurrentModeUpdate(m) => {
            vec![AcpEvent::ModeChanged { mode_id: m.current_mode_id.0.to_string() }]
        }
        // A mid-session model switch, as the `Model`-category selector's
        // `current_value`. Nothing when the agent exposes no model selector.
        SessionUpdate::ConfigOptionUpdate(c) => current_model_of(&c.config_options)
            .map(|model_id| vec![AcpEvent::ModelSelected { model_id }])
            .unwrap_or_default(),
        SessionUpdate::UsageUpdate(u) => vec![AcpEvent::Usage {
            used: u.used,
            size: u.size,
            cost: u.cost.as_ref().map(|c| CostEvent { amount: c.amount, currency: c.currency.clone() }),
            rate_limit: rate_limit_of(u.meta.as_ref()),
        }],
        _ => vec![],
    }
}

/// The currently-selected model id from a set of session config options — the
/// `Model`-category `Select` option's `current_value`. The `category` is
/// UX-only and optional, so fall back to a select whose id/name contains
/// "model". `None` when no model selector is present (attribution stays
/// provider-only).
pub fn current_model_of(options: &[SessionConfigOption]) -> Option<String> {
    let select_current = |opt: &SessionConfigOption| match &opt.kind {
        SessionConfigKind::Select(sel) => Some(sel.current_value.0.to_string()),
        _ => None,
    };
    if let Some(model) = options
        .iter()
        .find(|o| matches!(o.category, Some(SessionConfigOptionCategory::Model)))
        .and_then(select_current)
    {
        return Some(model);
    }
    options
        .iter()
        .find(|o| {
            matches!(o.kind, SessionConfigKind::Select(_))
                && (o.id.0.to_lowercase().contains("model") || o.name.to_lowercase().contains("model"))
        })
        .and_then(select_current)
}

/// Best-effort rate-limit snapshot from a `UsageUpdate._meta` blob. `_meta` is
/// agent-defined; only Claude's `_claude/rateLimit` key is understood — any
/// other/absent shape yields `None` rather than a fabricated status. All fields
/// but `status` are optional.
pub fn rate_limit_of(meta: Option<&serde_json::Map<String, serde_json::Value>>) -> Option<RateLimitEvent> {
    let rl = meta?.get("_claude/rateLimit")?;
    let status = rl.get("status")?.as_str()?.to_string();
    Some(RateLimitEvent {
        status,
        reset_at: rl.get("resetsAt").and_then(|v| v.as_i64()),
        limit_type: rl.get("rateLimitType").and_then(|v| v.as_str()).map(str::to_string),
        using_overage: rl.get("isUsingOverage").and_then(|v| v.as_bool()),
    })
}

/// Extract plain text from a `ContentBlock`. Only `Text` carries something the
/// current bubble/timeline model can render; `Image`/`Audio`/resource
/// variants yield `None` (the caller then emits nothing for that chunk) —
/// there is no image/audio rendering surface yet.
fn text_of(block: &ContentBlock) -> Option<String> {
    match block {
        ContentBlock::Text(t) => Some(t.text.clone()),
        _ => None,
    }
}

fn tool_kind_label(kind: &ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "read",
        ToolKind::Edit => "edit",
        ToolKind::Delete => "delete",
        ToolKind::Move => "move",
        ToolKind::Search => "search",
        ToolKind::Execute => "execute",
        ToolKind::Think => "think",
        ToolKind::Fetch => "fetch",
        ToolKind::SwitchMode => "switch_mode",
        _ => "other",
    }
}

fn tool_status_label(status: &ToolCallStatus) -> &'static str {
    match status {
        ToolCallStatus::Pending => "pending",
        ToolCallStatus::InProgress => "in_progress",
        ToolCallStatus::Completed => "completed",
        ToolCallStatus::Failed => "failed",
        // `ToolCallStatus` is `#[non_exhaustive]`; a future variant falls
        // back to the default rather than failing to compile.
        _ => "pending",
    }
}

fn plan_priority_label(priority: &PlanEntryPriority) -> &'static str {
    match priority {
        PlanEntryPriority::High => "high",
        PlanEntryPriority::Medium => "medium",
        PlanEntryPriority::Low => "low",
        // `PlanEntryPriority` is `#[non_exhaustive]`.
        _ => "medium",
    }
}

fn plan_status_label(status: &PlanEntryStatus) -> &'static str {
    match status {
        PlanEntryStatus::Pending => "pending",
        PlanEntryStatus::InProgress => "in_progress",
        PlanEntryStatus::Completed => "completed",
        // `PlanEntryStatus` is `#[non_exhaustive]`.
        _ => "pending",
    }
}

/// The label carried on `AcpEvent::TurnEnded.stop_reason`. Matches ACP's own
/// wire spelling (`StopReason` serializes `snake_case`) so the frontend and
/// the raw JSONL log (`acp/log.rs`) agree on vocabulary.
pub fn stop_reason_label(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::MaxTurnRequests => "max_turn_requests",
        StopReason::Refusal => "refusal",
        StopReason::Cancelled => "cancelled",
        // `StopReason` is `#[non_exhaustive]`.
        _ => "end_turn",
    }
}

/// An ACP-specific semantic moment, mapped onto the shared cross-provider
/// `StatusEvent` vocabulary (`crate::agent::status`) via `map_status` — the
/// same sink/state-machine the PTY/hooks path feeds, so an ACP session's pill
/// behaves identically to a hook-driven Claude PTY session. Deliberately
/// *not* a new status vocabulary: `AcpPhase` only labels *when* an ACP driver
/// calls into the existing machinery, per the design doc's mapping table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpPhase {
    /// `initialize` succeeded and a session was created (fresh `session/new`
    /// or resumed via `session/load`/`session/resume`).
    SessionCreated,
    /// A prompt was sent, or the first update chunk of a turn arrived.
    TurnStarted,
    /// The turn completed (`session/prompt` returned a `stopReason`).
    TurnCompleted,
    /// The agent sent `session/request_permission`.
    PermissionRequested,
    /// The pending permission request was answered.
    PermissionAnswered,
    /// The turn failed, or a protocol-level error occurred.
    TurnFailed,
    /// The agent process exited or the connection closed.
    ConnectionClosed,
}

/// Project an `AcpPhase` onto the shared `StatusEvent` vocabulary (see the
/// design doc's ACP-semantic -> `StatusEvent` table).
pub fn map_status(phase: AcpPhase) -> StatusEvent {
    match phase {
        AcpPhase::SessionCreated => StatusEvent::Spawned,
        AcpPhase::TurnStarted => StatusEvent::Working,
        AcpPhase::TurnCompleted => StatusEvent::Idle,
        AcpPhase::PermissionRequested => StatusEvent::NeedsAttention,
        AcpPhase::PermissionAnswered => StatusEvent::Working,
        AcpPhase::TurnFailed => StatusEvent::Failed,
        AcpPhase::ConnectionClosed => StatusEvent::Exited,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::SessionNotification;

    fn load_fixture(name: &str) -> SessionNotification {
        let raw = match name {
            "agent_message_chunk" => include_str!("fixtures/agent_message_chunk.json"),
            "available_commands_update" => include_str!("fixtures/available_commands_update.json"),
            "usage_update_with_cost" => include_str!("fixtures/usage_update_with_cost.json"),
            "usage_update_with_ratelimit_meta" => {
                include_str!("fixtures/usage_update_with_ratelimit_meta.json")
            }
            "session_info_update" => include_str!("fixtures/session_info_update.json"),
            "tool_call" => include_str!("fixtures/tool_call.json"),
            "tool_call_update" => include_str!("fixtures/tool_call_update.json"),
            "plan" => include_str!("fixtures/plan.json"),
            "current_mode_update" => include_str!("fixtures/current_mode_update.json"),
            other => panic!("unknown fixture: {other}"),
        };
        serde_json::from_str(raw).unwrap_or_else(|e| panic!("fixture {name} failed to deserialize: {e}"))
    }

    // ── real wire data (captured live from claude-agent-acp 0.59.0) ─────────

    #[test]
    fn agent_message_chunk_fixture_decodes_and_translates() {
        let notif = load_fixture("agent_message_chunk");
        assert_eq!(notif.session_id.0.to_string(), "ea603afd-d858-423b-ac83-8940cf0e279c");
        let events = translate_update(&notif.update, false);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AcpEvent::MessageChunk { role, text, message_id, replay } => {
                assert_eq!(*role, MessageRole::Assistant);
                assert_eq!(text, "p");
                assert_eq!(message_id.as_deref(), Some("msg_011Cd84wR79NT1WixKA9nHyN"));
                assert!(!replay);
            }
            other => panic!("expected MessageChunk, got {other:?}"),
        }
    }

    #[test]
    fn replay_flag_is_threaded_through_message_chunks() {
        let notif = load_fixture("agent_message_chunk");
        let events = translate_update(&notif.update, true);
        match &events[0] {
            AcpEvent::MessageChunk { replay, .. } => assert!(replay),
            other => panic!("expected MessageChunk, got {other:?}"),
        }
    }

    #[test]
    fn available_commands_update_fixture_decodes_but_translates_to_nothing() {
        // The slash-command catalog must deserialize cleanly (proves the
        // crate's serde matches the wire shape, incl. both `input` variants —
        // null and `{hint}`) but is deliberately dropped — no timeline UI to
        // render it yet. The command list here is generic placeholders, not the
        // live private catalog (see the module doc / the `leak-scan` gate).
        let notif = load_fixture("available_commands_update");
        assert!(translate_update(&notif.update, false).is_empty());
    }

    #[test]
    fn usage_update_with_cost_fixture_translates_to_usage_event() {
        let notif = load_fixture("usage_update_with_cost");
        let events = translate_update(&notif.update, false);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AcpEvent::Usage { used, size, cost, rate_limit } => {
                assert_eq!(*used, 23956);
                assert_eq!(*size, 1_000_000);
                let cost = cost.as_ref().expect("cost present");
                assert!((cost.amount - 0.095058).abs() < f64::EPSILON);
                assert_eq!(cost.currency, "USD");
                assert!(rate_limit.is_none(), "no _meta in this fixture");
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn usage_update_with_ratelimit_meta_parses_rate_limit_and_cost_none() {
        // Carries an agent-defined `_meta._claude/rateLimit` blob and no cost:
        // cost stays None (never fabricated), and the rate-limit snapshot is
        // parsed best-effort from the open `serde_json::Map`.
        let notif = load_fixture("usage_update_with_ratelimit_meta");
        let events = translate_update(&notif.update, false);
        match &events[0] {
            AcpEvent::Usage { used, cost, rate_limit, .. } => {
                assert_eq!(*used, 23956);
                assert!(cost.is_none());
                let rl = rate_limit.as_ref().expect("rate limit parsed from _meta");
                assert_eq!(rl.status, "allowed");
                assert_eq!(rl.reset_at, Some(1_784_312_400));
                assert_eq!(rl.limit_type.as_deref(), Some("five_hour"));
                assert_eq!(rl.using_overage, Some(false));
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn rate_limit_of_returns_none_for_absent_or_foreign_meta() {
        assert!(rate_limit_of(None).is_none());
        let mut foreign = serde_json::Map::new();
        foreign.insert("_someOther/thing".into(), serde_json::json!({"status": "ok"}));
        assert!(rate_limit_of(Some(&foreign)).is_none());
    }

    #[test]
    fn config_option_update_with_model_category_yields_model_selected() {
        use agent_client_protocol::schema::v1::{
            ConfigOptionUpdate, SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption,
        };
        let mut model_opt = SessionConfigOption::select(
            "model",
            "Model",
            "claude-opus-4",
            vec![
                SessionConfigSelectOption::new("claude-opus-4", "Opus"),
                SessionConfigSelectOption::new("claude-sonnet-4", "Sonnet"),
            ],
        );
        model_opt.category = Some(SessionConfigOptionCategory::Model);
        let update = SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(vec![model_opt]));
        let events = translate_update(&update, false);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AcpEvent::ModelSelected { model_id } => assert_eq!(model_id, "claude-opus-4"),
            other => panic!("expected ModelSelected, got {other:?}"),
        }
    }

    #[test]
    fn current_model_of_prefers_category_then_falls_back_to_id() {
        use agent_client_protocol::schema::v1::{
            SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption,
        };
        // Category wins even when another select mentions "model" in its id.
        let mut categorized = SessionConfigOption::select(
            "picker",
            "Pick",
            "the-model",
            vec![SessionConfigSelectOption::new("the-model", "M")],
        );
        categorized.category = Some(SessionConfigOptionCategory::Model);
        let decoy = SessionConfigOption::select(
            "model-flavor",
            "Flavor",
            "spicy",
            vec![SessionConfigSelectOption::new("spicy", "Spicy")],
        );
        assert_eq!(
            current_model_of(&[decoy.clone(), categorized]).as_deref(),
            Some("the-model")
        );
        // With no category anywhere, fall back to the id-matching select.
        assert_eq!(current_model_of(&[decoy]).as_deref(), Some("spicy"));
        // No model selector at all → None (provider-only attribution).
        let boolean = SessionConfigOption::boolean("thinking", "Think", true);
        assert_eq!(current_model_of(&[boolean]), None);
    }

    #[test]
    fn session_info_update_fixture_decodes_but_translates_to_nothing() {
        // No frontend surface yet for session titling; still must decode.
        let notif = load_fixture("session_info_update");
        assert!(translate_update(&notif.update, false).is_empty());
    }

    // ── hand-authored, schema-accurate fixtures (see module doc) ────────────

    #[test]
    fn tool_call_fixture_decodes_and_translates() {
        let notif = load_fixture("tool_call");
        let events = translate_update(&notif.update, false);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AcpEvent::ToolCall { id, kind, title, status, raw_input, .. } => {
                assert_eq!(id, "call_1");
                assert_eq!(kind, "read");
                assert_eq!(title, "Read src/lib.rs");
                assert_eq!(status, "pending");
                assert_eq!(raw_input.as_ref().unwrap()["path"], "src/lib.rs");
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn tool_call_update_fixture_decodes_and_translates() {
        let notif = load_fixture("tool_call_update");
        let events = translate_update(&notif.update, false);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AcpEvent::ToolCallUpdate { id, status, kind, title, content } => {
                assert_eq!(id, "call_1");
                assert_eq!(status.as_deref(), Some("completed"));
                assert_eq!(kind, &None);
                assert_eq!(title, &None);
                assert_eq!(content.as_ref().unwrap().len(), 1);
            }
            other => panic!("expected ToolCallUpdate, got {other:?}"),
        }
    }

    #[test]
    fn plan_fixture_decodes_and_translates() {
        let notif = load_fixture("plan");
        let events = translate_update(&notif.update, false);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AcpEvent::Plan { entries } => {
                assert_eq!(entries.len(), 3);
                assert_eq!(entries[0].priority, "high");
                assert_eq!(entries[0].status, "completed");
                assert_eq!(entries[1].status, "in_progress");
                assert_eq!(entries[2].status, "pending");
            }
            other => panic!("expected Plan, got {other:?}"),
        }
    }

    #[test]
    fn current_mode_update_fixture_decodes_and_translates() {
        let notif = load_fixture("current_mode_update");
        let events = translate_update(&notif.update, false);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AcpEvent::ModeChanged { mode_id } => assert_eq!(mode_id, "bypassPermissions"),
            other => panic!("expected ModeChanged, got {other:?}"),
        }
    }

    // ── camelCase-over-IPC spot checks (one per variant translate_update emits) ─

    #[test]
    fn tool_call_event_serializes_camel_case() {
        let notif = load_fixture("tool_call");
        let events = translate_update(&notif.update, false);
        let value = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(value["type"], "toolCall");
        assert_eq!(value["rawInput"]["path"], "src/lib.rs");
    }

    #[test]
    fn tool_call_update_event_serializes_camel_case() {
        let notif = load_fixture("tool_call_update");
        let events = translate_update(&notif.update, false);
        let value = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(value["type"], "toolCallUpdate");
    }

    #[test]
    fn plan_event_serializes_camel_case() {
        let notif = load_fixture("plan");
        let events = translate_update(&notif.update, false);
        let value = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(value["type"], "plan");
        assert_eq!(value["entries"][0]["priority"], "high");
    }

    #[test]
    fn mode_changed_event_serializes_camel_case() {
        let notif = load_fixture("current_mode_update");
        let events = translate_update(&notif.update, false);
        let value = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(value["type"], "modeChanged");
        assert_eq!(value["modeId"], "bypassPermissions");
    }

    #[test]
    fn usage_event_serializes_camel_case() {
        let notif = load_fixture("usage_update_with_cost");
        let events = translate_update(&notif.update, false);
        let value = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(value["type"], "usage");
        assert_eq!(value["cost"]["currency"], "USD");
    }

    // ── status mapping (design doc table) ───────────────────────────────────

    #[test]
    fn status_mapping_matches_design_doc_table() {
        assert_eq!(map_status(AcpPhase::SessionCreated), StatusEvent::Spawned);
        assert_eq!(map_status(AcpPhase::TurnStarted), StatusEvent::Working);
        assert_eq!(map_status(AcpPhase::TurnCompleted), StatusEvent::Idle);
        assert_eq!(map_status(AcpPhase::PermissionRequested), StatusEvent::NeedsAttention);
        assert_eq!(map_status(AcpPhase::PermissionAnswered), StatusEvent::Working);
        assert_eq!(map_status(AcpPhase::TurnFailed), StatusEvent::Failed);
        assert_eq!(map_status(AcpPhase::ConnectionClosed), StatusEvent::Exited);
    }

    #[test]
    fn stop_reason_labels_match_wire_spelling() {
        assert_eq!(stop_reason_label(&StopReason::EndTurn), "end_turn");
        assert_eq!(stop_reason_label(&StopReason::MaxTokens), "max_tokens");
        assert_eq!(stop_reason_label(&StopReason::MaxTurnRequests), "max_turn_requests");
        assert_eq!(stop_reason_label(&StopReason::Refusal), "refusal");
        assert_eq!(stop_reason_label(&StopReason::Cancelled), "cancelled");
    }
}
