//! Task classifier for auto-routing.
//!
//! Thin async glue: shell out to a headless model CLI (`claude -p`, resolved via
//! `claude_path`) with a self-contained prompt asking it to classify a task's
//! difficulty and kind as strict JSON, then decode the output with the pure
//! `routing::parse_classification` parser. On-ethos with the rest of La Vigie —
//! we invoke a CLI (like `gh`/`git`), never store an API key.
//!
//! Every failure path (binary missing, timeout, non-zero exit, unparseable
//! output) collapses to `None`: the classifier can slow task creation by at most
//! one bounded round-trip and can never block or fail it. Not unit-tested (needs
//! a live `claude` binary); the parsing it feeds IS unit-tested in `routing`.
//!
//! v1 limitation (deliberate): the classifier is pinned to the `claude` CLI. On
//! a machine without `claude` installed, classification always yields `None`, so
//! rules that key on `difficulty`/`kind` never match and routing degrades to the
//! policy's `fallback` (or the static default) — no crash, but such rules are
//! inert. A configurable classifier command (or classifying via the repo's own
//! default engine) is a follow-up; title-substring rules work regardless.

use std::time::Duration;

use tokio::process::Command;

use crate::agent::routing::{parse_classification, TaskClassification};

/// Hard ceiling on the classifier round-trip. This call is on the task-creation
/// critical path (routing must pick the engine before the task launches), so it
/// blocks the create up to this long when a policy's rules need classification —
/// a deliberate cost of the classifier-driven design, bounded tightly here.
/// Past the deadline we abandon the classification and route without it (rules
/// needing a signal simply don't match) rather than hang task creation. The tiny
/// classify prompt normally returns in a few seconds; this is the hung-CLI cap.
const CLASSIFY_TIMEOUT: Duration = Duration::from_secs(20);

/// Classify a task from its title (and optional launch prompt). Returns `None`
/// on any error so the caller degrades to non-classifier routing.
pub async fn classify_task(title: &str, prompt: Option<&str>) -> Option<TaskClassification> {
    let instruction = build_prompt(title, prompt);
    let bin = crate::claude_path::find_binary("claude");

    let run = Command::new(&bin)
        .arg("-p")
        .arg(&instruction)
        .kill_on_drop(true)
        .output();

    let output = match tokio::time::timeout(CLASSIFY_TIMEOUT, run).await {
        Ok(Ok(o)) => o,
        // Timed out, or the process failed to spawn/run.
        _ => return None,
    };
    if !output.status.success() {
        return None;
    }
    parse_classification(&String::from_utf8_lossy(&output.stdout))
}

/// Build the self-contained classification prompt. Kept pure + unit-tested so the
/// contract (strict-JSON ask, both signals embedded) can't silently drift.
fn build_prompt(title: &str, prompt: Option<&str>) -> String {
    let extra = match prompt {
        Some(p) if !p.trim().is_empty() => format!("\n\nInitial instructions:\n{}", p.trim()),
        _ => String::new(),
    };
    format!(
        "You are a routing classifier for a software task. Classify the task below.\n\
         Respond with ONLY a single JSON object and nothing else, in exactly this shape:\n\
         {{\"difficulty\": \"easy|medium|hard\", \"kind\": \"refactor|greenfield|debug|ui|docs|other\"}}\n\
         - difficulty: how hard the change is to implement correctly.\n\
         - kind: the dominant nature of the work.\n\
         Do not explain. Do not use tools. Output only the JSON.\n\n\
         Task title:\n{title}{extra}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_embeds_title_and_demands_strict_json() {
        let p = build_prompt("Fix the login crash", None);
        assert!(p.contains("Fix the login crash"));
        assert!(p.contains("\"difficulty\""));
        assert!(p.contains("\"kind\""));
        assert!(p.contains("only the JSON"));
        assert!(!p.contains("Initial instructions"));
    }

    #[test]
    fn prompt_includes_launch_prompt_when_present() {
        let p = build_prompt("t", Some("  refactor the parser  "));
        assert!(p.contains("Initial instructions:"));
        assert!(p.contains("refactor the parser"));
    }
}
