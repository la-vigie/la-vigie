//! Raw JSONL event log for ACP sessions: a debugging/safety-net record of
//! every `AcpEvent` emitted by `drive_connection`, appended as one
//! JSON line per event under `AppState.acp_logs_root`. This complements —
//! never replaces — the ACP agent's own native session storage; per the
//! design doc's Persistence & resume section, agent-native storage stays the
//! source of truth for replay, and this log is the app-side safety net for
//! agents with broken/absent `loadSession` (Cursor-class) plus a debugging
//! record.
//!
//! The file is keyed by **`task_id`** (`{task_id}.jsonl`), not the ephemeral
//! per-launch `agent_id`: the agent id is a fresh UUID every launch
//! and is lost across an app restart, which would orphan the log and defeat
//! its purpose as a resume safety net. A task's id is stable, so
//! `parse_replay_events` can find and re-hydrate the timeline after a restart
//! (the `ResumeSource::EventLog` tier). Records still tag the real `agentId`
//! so successive launches for one task stay distinguishable in the file.
//!
//! Best-effort throughout: any I/O or serialization failure is logged to
//! stderr and otherwise ignored — a full disk or missing directory must never
//! disrupt a live ACP session.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::AcpEvent;

/// The on-disk path of the raw event log for `task_id` under `dir`.
pub fn log_path(dir: &Path, task_id: &str) -> PathBuf {
    dir.join(format!("{task_id}.jsonl"))
}

/// Parse a task's raw JSONL event log back into the `AcpEvent`s to replay for a
/// visual timeline restore (`ResumeSource::EventLog`). Pure and lenient: blank
/// or malformed lines are skipped, never fatal.
///
/// Two transforms make the replay honest rather than a verbatim re-run:
/// - `MessageChunk`/`ThoughtChunk` are re-emitted with `replay: true` so the
///   reducer restores them as history without flipping the "turn running"
///   affordances (`reduceAcpEvent`'s `turnActive` stays put on replay).
/// - `SessionStarted` / `Exit` / `PermissionRequest` are dropped: the fresh
///   `session/new` emits its own `SessionStarted`, the session is not exited,
///   and a resolved-in-history permission prompt must not be resurrected as a
///   live pending request.
pub fn parse_replay_events(jsonl: &str) -> Vec<AcpEvent> {
    jsonl
        .lines()
        .filter_map(|line| {
            if line.trim().is_empty() {
                return None;
            }
            let record: serde_json::Value = serde_json::from_str(line).ok()?;
            let event: AcpEvent = serde_json::from_value(record.get("event")?.clone()).ok()?;
            match event {
                AcpEvent::SessionStarted { .. }
                | AcpEvent::Exit { .. }
                | AcpEvent::PermissionRequest { .. } => None,
                AcpEvent::MessageChunk { role, text, message_id, .. } => {
                    Some(AcpEvent::MessageChunk { role, text, message_id, replay: true })
                }
                AcpEvent::ThoughtChunk { text, message_id, .. } => {
                    Some(AcpEvent::ThoughtChunk { text, message_id, replay: true })
                }
                other => Some(other),
            }
        })
        .collect()
}

/// Build the JSON record for one logged event, without the timestamp (kept as
/// a separate pure step so this stays deterministic and unit-testable;
/// `AcpLog::append` adds `ts` at write time).
pub fn log_record(agent_id: &str, session_id: Option<&str>, event: &AcpEvent) -> serde_json::Value {
    serde_json::json!({
        "agentId": agent_id,
        "sessionId": session_id,
        "event": event,
    })
}

/// A per-session append-only JSONL log with the output file opened once at
/// session start and held for the session's life. ACP streams message content
/// token-by-token (many notifications per turn), so re-running
/// `create_dir_all` + open + close on every event would put hundreds of
/// syscalls per turn on the connection's hot event loop. Opening once and
/// writing a line per event keeps it to a single `write` per event.
///
/// Best-effort throughout: construction failure yields `None` (logging is
/// simply skipped), and a per-line write failure is logged to stderr and
/// swallowed — a full disk must never disrupt a live ACP session.
pub struct AcpLog {
    agent_id: String,
    file: Mutex<File>,
}

impl AcpLog {
    /// Open (creating parent dirs) the task's log `{dir}/{task_id}.jsonl` in
    /// append mode, tagging every record with the launch's `agent_id`. Returns
    /// `None` on any I/O failure — the caller then logs nothing. Append mode
    /// means a resumed task keeps writing to the same file its prior launch(es)
    /// wrote, so the log is a continuous per-task history.
    pub fn create(dir: &Path, task_id: &str, agent_id: &str) -> Option<Self> {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("[acp] log dir {}: {e:#}", dir.display());
            return None;
        }
        let path = log_path(dir, task_id);
        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => Some(Self { agent_id: agent_id.to_string(), file: Mutex::new(file) }),
            Err(e) => {
                eprintln!("[acp] opening log {}: {e:#}", path.display());
                None
            }
        }
    }

    /// Append one JSON line for `event` (tagged with `session_id` once known
    /// and a write-time `ts`). Best-effort — a failure is logged and swallowed.
    pub fn append(&self, session_id: Option<&str>, event: &AcpEvent) {
        let mut record = log_record(&self.agent_id, session_id, event);
        if let serde_json::Value::Object(map) = &mut record {
            map.insert("ts".to_string(), serde_json::Value::String(chrono::Utc::now().to_rfc3339()));
        }
        let line = match serde_json::to_string(&record) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[acp] serializing log record: {e:#}");
                return;
            }
        };
        if let Ok(mut file) = self.file.lock() {
            if let Err(e) = writeln!(file, "{line}") {
                eprintln!("[acp] writing log for agent {}: {e:#}", self.agent_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::events::MessageRole;

    #[test]
    fn log_record_serializes_agent_session_and_event() {
        let event = AcpEvent::TurnEnded { stop_reason: "end_turn".to_string() };
        let record = log_record("agent-1", Some("sess-1"), &event);
        assert_eq!(record["agentId"], "agent-1");
        assert_eq!(record["sessionId"], "sess-1");
        assert_eq!(record["event"]["type"], "turnEnded");
        assert_eq!(record["event"]["stopReason"], "end_turn");
    }

    #[test]
    fn log_record_session_id_none_serializes_null() {
        let event = AcpEvent::Exit { code: None };
        let record = log_record("agent-2", None, &event);
        assert_eq!(record["sessionId"], serde_json::Value::Null);
    }

    #[test]
    fn log_record_omits_ts_field() {
        let event = AcpEvent::MessageChunk {
            role: MessageRole::Assistant,
            text: "hi".to_string(),
            message_id: None,
            replay: false,
        };
        let record = log_record("agent-3", None, &event);
        assert!(record.get("ts").is_none());
    }

    #[test]
    fn log_path_is_keyed_by_task_id() {
        let p = log_path(Path::new("/logs"), "task-42");
        assert_eq!(p, Path::new("/logs/task-42.jsonl"));
    }

    /// Round-trip: a line written by `log_record` (+`ts`) parses back into the
    /// event, so the on-disk format and `parse_replay_events` stay in lockstep.
    #[test]
    fn parse_replay_events_round_trips_a_logged_line() {
        let event = AcpEvent::MessageChunk {
            role: MessageRole::Assistant,
            text: "hello".to_string(),
            message_id: Some("msg_1".to_string()),
            replay: false,
        };
        let line = serde_json::to_string(&log_record("a", Some("s"), &event)).unwrap();
        let replayed = parse_replay_events(&line);
        assert_eq!(replayed.len(), 1);
        match &replayed[0] {
            // The live=false chunk comes back marked replay=true.
            AcpEvent::MessageChunk { text, replay, message_id, .. } => {
                assert_eq!(text, "hello");
                assert!(replay);
                assert_eq!(message_id.as_deref(), Some("msg_1"));
            }
            other => panic!("expected MessageChunk, got {other:?}"),
        }
    }

    #[test]
    fn parse_replay_events_marks_message_and_thought_chunks_replay() {
        let msg = serde_json::to_string(&log_record(
            "a",
            Some("s"),
            &AcpEvent::MessageChunk {
                role: MessageRole::User,
                text: "hi".to_string(),
                message_id: None,
                replay: false,
            },
        ))
        .unwrap();
        let thought = serde_json::to_string(&log_record(
            "a",
            Some("s"),
            &AcpEvent::ThoughtChunk { text: "hmm".to_string(), message_id: None, replay: false },
        ))
        .unwrap();
        let events = parse_replay_events(&format!("{msg}\n{thought}"));
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], AcpEvent::MessageChunk { replay: true, .. }));
        assert!(matches!(&events[1], AcpEvent::ThoughtChunk { replay: true, .. }));
    }

    #[test]
    fn parse_replay_events_drops_stale_lifecycle_and_permission_events() {
        // SessionStarted/Exit/PermissionRequest must not be resurrected: the
        // fresh session emits its own SessionStarted, we are not exited, and a
        // permission resolved in history is not a live pending request.
        let started = serde_json::to_string(&log_record(
            "a",
            Some("s"),
            &AcpEvent::SessionStarted { session_id: "s".to_string(), modes: None, models: None },
        ))
        .unwrap();
        let exit = serde_json::to_string(&log_record("a", Some("s"), &AcpEvent::Exit { code: None }))
            .unwrap();
        let perm = serde_json::to_string(&log_record(
            "a",
            Some("s"),
            &AcpEvent::PermissionRequest {
                request_id: "r".to_string(),
                options: vec![],
                tool_call: serde_json::Value::Null,
            },
        ))
        .unwrap();
        let events = parse_replay_events(&format!("{started}\n{exit}\n{perm}"));
        assert!(events.is_empty(), "expected all dropped, got {events:?}");
    }

    #[test]
    fn parse_replay_events_preserves_tool_calls_verbatim() {
        let tc = serde_json::to_string(&log_record(
            "a",
            Some("s"),
            &AcpEvent::ToolCall {
                id: "call_1".to_string(),
                kind: "read".to_string(),
                title: "Read x".to_string(),
                status: "completed".to_string(),
                content: vec![],
                raw_input: None,
            },
        ))
        .unwrap();
        let events = parse_replay_events(&tc);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AcpEvent::ToolCall { id, .. } if id == "call_1"));
    }

    #[test]
    fn parse_replay_events_skips_blank_and_malformed_lines() {
        let good = serde_json::to_string(&log_record(
            "a",
            Some("s"),
            &AcpEvent::MessageChunk {
                role: MessageRole::Assistant,
                text: "ok".to_string(),
                message_id: None,
                replay: false,
            },
        ))
        .unwrap();
        let input = format!("\n  \n{{not json\n{good}\n{{\"no\":\"event\"}}");
        let events = parse_replay_events(&input);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AcpEvent::MessageChunk { text, .. } if text == "ok"));
    }
}
