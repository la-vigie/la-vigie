//! Agent-session core (AC2-108/AC2-111): parse Claude Code's JSONL transcript
//! into chat-shaped messages, frame a reply for the PTY, resolve a task to its
//! live agent, and provide the shared `read_session` service consumed by both
//! the remote HTTP handler and the MCP `get_task_activity` tool. Glue (handlers,
//! file I/O, PTY writes) lives in `server.rs`/`agent` and is verified live.

use serde::Serialize;

/// A chat-shaped item distilled from one transcript content block. Kept loose on
/// purpose — the real mobile UI lands with AC2-107. Serialized camelCase; `None`
/// fields are omitted.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    /// "assistant" | "user" | "tool".
    pub role: String,
    /// Prose for assistant/user messages; `None` for tool items.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Tool name for tool items; `None` otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// ISO timestamp from the record, if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<String>,
}

/// Parse transcript `bytes` starting at byte offset `since`. Returns the
/// chat-shaped messages found in the *complete* lines from `since` onward, plus
/// the new cursor: the offset just past the last complete line consumed. A
/// partial trailing line (file caught mid-write) is NOT consumed, so the next
/// poll re-reads it once whole. `since` past EOF clamps to EOF and yields nothing.
pub fn parse_transcript(bytes: &[u8], since: usize) -> (Vec<SessionMessage>, usize) {
    let start = since.min(bytes.len());
    let region = &bytes[start..];

    // Consume only up to (and including) the last newline; anything after it is a
    // partial line that must not advance the cursor.
    let Some(last_nl) = region.iter().rposition(|&b| b == b'\n') else {
        return (Vec::new(), start);
    };
    let new_cursor = start + last_nl + 1;

    let mut out = Vec::new();
    for line in region[..=last_nl].split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(line) {
            messages_from_record(&v, &mut out);
        }
    }
    (out, new_cursor)
}

/// Distill one transcript record into zero or more chat-shaped messages.
/// - `assistant`: each `text` block → an assistant message; each `tool_use`
///   block → a tool item (the tool name). `thinking` blocks are skipped.
/// - `user`: only *string* content is a real typed message. Array content is
///   either tool_result output or injected command expansions — skipped (the
///   assistant's `tool_use` already represents tool activity).
/// - every other record type (system, attachment, mode, summary, …) is skipped.
fn messages_from_record(v: &serde_json::Value, out: &mut Vec<SessionMessage>) {
    let ts = v.get("timestamp").and_then(|t| t.as_str()).map(str::to_string);
    match v.get("type").and_then(|t| t.as_str()) {
        Some("assistant") => {
            let Some(blocks) = v.pointer("/message/content").and_then(|c| c.as_array()) else {
                return;
            };
            for b in blocks {
                match b.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                            if !t.trim().is_empty() {
                                out.push(SessionMessage {
                                    role: "assistant".into(),
                                    text: Some(t.to_string()),
                                    tool: None,
                                    ts: ts.clone(),
                                });
                            }
                        }
                    }
                    Some("tool_use") => {
                        let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                        out.push(SessionMessage {
                            role: "tool".into(),
                            text: None,
                            tool: Some(name.to_string()),
                            ts: ts.clone(),
                        });
                    }
                    _ => {}
                }
            }
        }
        Some("user") => {
            if let Some(s) = v.pointer("/message/content").and_then(|c| c.as_str()) {
                if !s.trim().is_empty() {
                    out.push(SessionMessage {
                        role: "user".into(),
                        text: Some(s.to_string()),
                        tool: None,
                        ts,
                    });
                }
            }
        }
        _ => {}
    }
}

use crate::state::AppState;

/// Result of a task-session read: the chat-shaped messages from `since`
/// onward plus the new byte cursor to pass to the next poll.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRead {
    pub messages: Vec<SessionMessage>,
    pub cursor: usize,
}

/// Shared task-session read service (AC2-111): resolve a task's captured
/// transcript path, read it, and parse from byte offset `since`. Consumed by
/// the remote HTTP `GET /api/tasks/:id/session` handler AND the MCP
/// `get_task_activity` tool. An absent transcript (no hook yet) is NOT an
/// error — it yields an empty read so callers keep polling. Glue (locks +
/// file IO), so it is verified live, not unit-tested; the parsing core
/// (`parse_transcript`) is the tested part.
pub fn read_session(state: &AppState, task_id: &str, since: usize) -> Result<SessionRead, String> {
    let path = {
        let map = state.transcripts.lock().map_err(|e| format!("{e:#}"))?;
        map.get(task_id).cloned()
    };
    let Some(path) = path else {
        return Ok(SessionRead { messages: Vec::new(), cursor: 0 });
    };
    let bytes = std::fs::read(&path).map_err(|e| format!("{e:#}"))?;
    let (messages, cursor) = parse_transcript(&bytes, since);
    Ok(SessionRead { messages, cursor })
}

use std::collections::{HashMap, HashSet};

/// Frame a reply as a bracketed paste so a multi-line reply stays a single
/// message in the agent's TUI. The caller writes this, then a separate "\r" to
/// submit it.
pub fn bracketed_paste(text: &str) -> String {
    format!("\x1b[200~{text}\x1b[201~")
}

/// Find the live agent for `task_id`: the agent mapped to it whose PTY session is
/// still registered in `live_sessions`. Stale entries (a stopped or un-stopped
/// resumed agent no longer in `sessions`) are ignored. `None` when none is live.
pub fn resolve_live_agent(
    agent_tasks: &HashMap<String, String>,
    live_sessions: &HashSet<String>,
    task_id: &str,
) -> Option<String> {
    agent_tasks
        .iter()
        .find(|(agent_id, t)| t.as_str() == task_id && live_sessions.contains(agent_id.as_str()))
        .map(|(agent_id, _)| agent_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = include_bytes!("fixtures/transcript_sample.jsonl");

    #[test]
    fn parses_sample_into_chat_messages_and_skips_noise() {
        let (msgs, cursor) = parse_transcript(SAMPLE, 0);
        // system line, thinking block, and the tool_result user line are all skipped.
        assert_eq!(
            msgs.iter().map(|m| (m.role.as_str(), m.text.as_deref(), m.tool.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                ("user", Some("Add the reply endpoint"), None),
                ("assistant", Some("On it."), None),
                ("tool", None, Some("Read")),
                ("assistant", Some("Done."), None),
            ]
        );
        // Sample ends with a newline, so the cursor sits at EOF.
        assert_eq!(cursor, SAMPLE.len());
        // Timestamps carried through.
        assert_eq!(msgs[0].ts.as_deref(), Some("2026-06-28T10:00:01Z"));
    }

    #[test]
    fn partial_trailing_line_is_not_consumed_and_cursor_stops_before_it() {
        let data = b"{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n{\"type\":\"user\",\"messa";
        let (msgs, cursor) = parse_transcript(data, 0);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text.as_deref(), Some("hi"));
        // Cursor stops just past the first line's newline, not into the partial line.
        let first_line_len = b"{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n".len();
        assert_eq!(cursor, first_line_len);
    }

    #[test]
    fn no_complete_line_yields_nothing_and_returns_since() {
        let data = b"{\"partial\":";
        let (msgs, cursor) = parse_transcript(data, 0);
        assert!(msgs.is_empty());
        assert_eq!(cursor, 0);
    }

    #[test]
    fn re_reading_from_the_returned_cursor_yields_no_duplicates() {
        // Poll once from the start, then poll again from the cursor it returned:
        // the second poll must see nothing new and leave the cursor unchanged.
        let (first, cursor) = parse_transcript(SAMPLE, 0);
        assert_eq!(first.len(), 4);
        let (delta, cursor2) = parse_transcript(SAMPLE, cursor);
        assert!(delta.is_empty());
        assert_eq!(cursor2, cursor);
    }

    #[test]
    fn since_past_eof_is_safe() {
        let (msgs, cursor) = parse_transcript(SAMPLE, SAMPLE.len() + 999);
        assert!(msgs.is_empty());
        assert_eq!(cursor, SAMPLE.len());
    }

    #[test]
    fn unparseable_line_is_skipped_not_fatal() {
        let data = b"not json at all\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"ok\"}}\n";
        let (msgs, _) = parse_transcript(data, 0);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text.as_deref(), Some("ok"));
    }

    #[test]
    fn bracketed_paste_wraps_in_paste_escapes() {
        assert_eq!(bracketed_paste("hi\nthere"), "\x1b[200~hi\nthere\x1b[201~");
    }

    #[test]
    fn resolve_live_agent_picks_the_agent_with_a_live_session() {
        use std::collections::{HashMap, HashSet};
        let mut agent_tasks = HashMap::new();
        agent_tasks.insert("agentA".to_string(), "task1".to_string());
        agent_tasks.insert("agentB".to_string(), "task2".to_string());
        let live: HashSet<String> = ["agentA".to_string()].into_iter().collect();
        assert_eq!(resolve_live_agent(&agent_tasks, &live, "task1"), Some("agentA".to_string()));
    }

    #[test]
    fn resolve_live_agent_ignores_stale_stopped_entries() {
        use std::collections::{HashMap, HashSet};
        let mut agent_tasks = HashMap::new();
        // An old resumed agent for task1 that is no longer in `sessions`.
        agent_tasks.insert("stale".to_string(), "task1".to_string());
        let live: HashSet<String> = HashSet::new();
        assert_eq!(resolve_live_agent(&agent_tasks, &live, "task1"), None);
    }

    #[test]
    fn resolve_live_agent_none_for_unknown_task() {
        use std::collections::{HashMap, HashSet};
        let agent_tasks: HashMap<String, String> = HashMap::new();
        let live: HashSet<String> = HashSet::new();
        assert_eq!(resolve_live_agent(&agent_tasks, &live, "nope"), None);
    }
}
