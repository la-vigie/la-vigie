//! Pure agent-status state machine: the normalized run-state vocabulary, the
//! cross-provider event vocabulary, the transition function (with a terminal
//! guard), and the projection onto the persisted `TaskStatus`. No Tauri, no I/O
//! — this is the unit-tested core that all provider adapters feed.

use crate::store::TaskStatus;

/// Normalized agent run-state. Hook-capable agents (Claude) occupy the refined
/// states (`Working`/`Idle`/`NeedsAttention`/`Error`); lifecycle-only agents sit
/// at `Running`. `Starting`/`Exited` are the lifecycle endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunState {
    Starting,
    Running,
    Working,
    Idle,
    NeedsAttention,
    Error,
    Exited,
}

/// Cross-provider status event. A per-provider adapter maps its own mechanism
/// (Claude hooks, process liveness, future Antigravity/Codex signals) onto this
/// vocabulary; `apply_event` is the single place that turns events into states.
/// `SubagentStarted`/`SubagentStopped` track in-flight *background* work (AC2-85):
/// they adjust a per-agent counter rather than the main run-state, so the pill
/// stays active while a backgrounded subagent runs past the main loop's `Stop`.
///
/// This gate works for subagents only because the signal is *balanced*: the
/// `SubagentStart` increment is matched by a `SubagentStop` decrement, both real
/// Claude hooks. Background **shell** commands (`run_in_background` Bash) are
/// deliberately *not* gated here (AC2-101): they have an increment-side signal
/// (`PreToolUse`/`PostToolUse`, fired at dispatch) but **no Claude hook fires when
/// the detached command later finishes**, so there is nothing to decrement against.
/// Gating on the increment alone would pin the pill on `Working` forever; a
/// completion signal would have to be polled (process-liveness / `BashOutput`),
/// which violates the hook-driven, out-of-band status rule. Revisit only if Claude
/// Code ships a background-shell-completion hook — map it as a `SubagentStopped`-
/// style decrement and the existing machinery covers the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusEvent {
    Spawned,
    Working,
    NeedsAttention,
    Idle,
    Failed,
    Exited,
    SubagentStarted,
    SubagentStopped,
}

/// Per-agent status bookkeeping: the main loop's run-state plus a count of
/// in-flight background subagents. The *displayed* state is derived from both
/// via `display_state`, so a `Stop` (main → `Idle`) while a background subagent
/// is still running keeps the pill on `Working`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentState {
    pub run: AgentRunState,
    pub bg_count: u32,
}

impl Default for AgentState {
    fn default() -> Self {
        Self { run: AgentRunState::Starting, bg_count: 0 }
    }
}

/// Apply one *main-loop* event to the current run-state. `Exited` is terminal
/// (guarded by the caller). Subagent events never reach here — they adjust the
/// background counter in `apply_event` instead — but the match stays exhaustive
/// by leaving the run-state untouched for them.
pub fn transition(current: AgentRunState, event: StatusEvent) -> AgentRunState {
    use AgentRunState as S;
    use StatusEvent as E;

    if current == S::Exited {
        return S::Exited; // terminal guard
    }
    match event {
        E::Spawned => S::Running,
        E::Working => S::Working,
        E::NeedsAttention => S::NeedsAttention,
        E::Idle => S::Idle,
        E::Failed => S::Error,
        E::Exited => S::Exited,
        // Background-subagent events carry no main-loop meaning.
        E::SubagentStarted | E::SubagentStopped => current,
    }
}

/// Derive the *displayed* run-state from the main run-state and the count of
/// in-flight background subagents (AC2-85). While background work is in flight,
/// an otherwise-`Idle`/`Running` main loop reads `Working`; `NeedsAttention`,
/// `Error`, `Working`, `Starting`, and `Exited` always win over the gate (an
/// actionable or terminal state must not be masked by background activity).
pub fn display_state(main: AgentRunState, bg_count: u32) -> AgentRunState {
    use AgentRunState as S;
    match main {
        S::Idle | S::Running if bg_count > 0 => S::Working,
        other => other,
    }
}

/// Project a run-state onto the persisted `TaskStatus`. `Exited` yields `None`
/// (no write — leaves the last meaningful status so the restored dot shows where
/// the agent ended). `Done` is never produced here; it is reserved for the
/// explicit Finish flow.
pub fn to_task_status(state: AgentRunState) -> Option<TaskStatus> {
    use AgentRunState as S;
    match state {
        S::Working => Some(TaskStatus::Working),
        S::NeedsAttention => Some(TaskStatus::NeedsAttention),
        S::Idle | S::Starting | S::Running => Some(TaskStatus::Idle),
        S::Error => Some(TaskStatus::Error),
        S::Exited => None,
    }
}

/// Apply an event to the per-agent state map, returning the new **displayed**
/// run-state only when it changes. A fresh agent defaults to `Starting` (no
/// background work), so its first real event always emits. Subagent events
/// adjust the background counter; main-loop events advance the run-state.
/// Unchanged displayed states (e.g. a `Stop` suppressed while a subagent runs,
/// or a repeated `Working` within a turn) and post-`Exited` stragglers return
/// `None`, so callers emit/persist only on genuine transitions.
pub fn apply_event(
    states: &mut std::collections::HashMap<String, AgentState>,
    agent_id: &str,
    event: StatusEvent,
) -> Option<AgentRunState> {
    let current = states.get(agent_id).copied().unwrap_or_default();

    // Terminal guard: once the main loop has exited, drop straggler hooks.
    if current.run == AgentRunState::Exited {
        return None;
    }

    let old_display = display_state(current.run, current.bg_count);

    let next = match event {
        StatusEvent::SubagentStarted => {
            AgentState { run: current.run, bg_count: current.bg_count + 1 }
        }
        StatusEvent::SubagentStopped => {
            AgentState { run: current.run, bg_count: current.bg_count.saturating_sub(1) }
        }
        // Exiting clears any outstanding background count — the process is gone.
        StatusEvent::Exited => AgentState { run: AgentRunState::Exited, bg_count: 0 },
        other => AgentState { run: transition(current.run, other), bg_count: current.bg_count },
    };

    let new_display = display_state(next.run, next.bg_count);
    states.insert(agent_id.to_string(), next);

    if new_display == old_display {
        return None;
    }
    Some(new_display)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn apply_event_emits_first_state_for_new_agent() {
        let mut states = HashMap::new();
        assert_eq!(apply_event(&mut states, "a1", StatusEvent::Working), Some(AgentRunState::Working));
        assert_eq!(states.get("a1").map(|s| s.run), Some(AgentRunState::Working));
    }

    #[test]
    fn apply_event_suppresses_unchanged_state() {
        let mut states = HashMap::new();
        apply_event(&mut states, "a1", StatusEvent::Working);
        // Second Working event is redundant — no change, no emit.
        assert_eq!(apply_event(&mut states, "a1", StatusEvent::Working), None);
    }

    #[test]
    fn apply_event_emits_on_real_transition() {
        let mut states = HashMap::new();
        apply_event(&mut states, "a1", StatusEvent::NeedsAttention);
        // AC2-47: a Working event after NeedsAttention is a real change → emitted.
        assert_eq!(apply_event(&mut states, "a1", StatusEvent::Working), Some(AgentRunState::Working));
    }

    #[test]
    fn apply_event_ignores_events_after_exit() {
        let mut states = HashMap::new();
        apply_event(&mut states, "a1", StatusEvent::Exited);
        assert_eq!(apply_event(&mut states, "a1", StatusEvent::Working), None);
        assert_eq!(states.get("a1").map(|s| s.run), Some(AgentRunState::Exited));
    }

    #[test]
    fn working_event_yields_working() {
        assert_eq!(transition(AgentRunState::Idle, StatusEvent::Working), AgentRunState::Working);
    }

    #[test]
    fn needs_attention_to_working_is_reachable() {
        // AC2-47: after a permission prompt, a Working event (PreToolUse/UserPromptSubmit)
        // must move the agent back to Working — not stay stuck on NeedsAttention.
        assert_eq!(
            transition(AgentRunState::NeedsAttention, StatusEvent::Working),
            AgentRunState::Working
        );
    }

    #[test]
    fn spawned_yields_running() {
        assert_eq!(transition(AgentRunState::Starting, StatusEvent::Spawned), AgentRunState::Running);
    }

    #[test]
    fn idle_and_failed_and_exited_map_through() {
        assert_eq!(transition(AgentRunState::Working, StatusEvent::Idle), AgentRunState::Idle);
        assert_eq!(transition(AgentRunState::Working, StatusEvent::Failed), AgentRunState::Error);
        assert_eq!(transition(AgentRunState::Working, StatusEvent::Exited), AgentRunState::Exited);
    }

    #[test]
    fn exited_is_terminal_for_every_event() {
        for ev in [
            StatusEvent::Spawned,
            StatusEvent::Working,
            StatusEvent::NeedsAttention,
            StatusEvent::Idle,
            StatusEvent::Failed,
            StatusEvent::Exited,
        ] {
            assert_eq!(transition(AgentRunState::Exited, ev), AgentRunState::Exited);
        }
    }

    #[test]
    fn to_task_status_maps_refined_states() {
        assert_eq!(to_task_status(AgentRunState::Working), Some(TaskStatus::Working));
        assert_eq!(to_task_status(AgentRunState::NeedsAttention), Some(TaskStatus::NeedsAttention));
        assert_eq!(to_task_status(AgentRunState::Idle), Some(TaskStatus::Idle));
        assert_eq!(to_task_status(AgentRunState::Error), Some(TaskStatus::Error));
    }

    #[test]
    fn to_task_status_neutral_alive_is_idle_and_exited_is_none() {
        assert_eq!(to_task_status(AgentRunState::Starting), Some(TaskStatus::Idle));
        assert_eq!(to_task_status(AgentRunState::Running), Some(TaskStatus::Idle));
        assert_eq!(to_task_status(AgentRunState::Exited), None);
    }

    // ── display_state matrix: counter → pill (AC2-85) ─────────────────────────

    #[test]
    fn display_state_idle_with_no_background_is_idle() {
        assert_eq!(display_state(AgentRunState::Idle, 0), AgentRunState::Idle);
    }

    #[test]
    fn display_state_idle_with_background_is_working() {
        assert_eq!(display_state(AgentRunState::Idle, 1), AgentRunState::Working);
    }

    #[test]
    fn display_state_running_with_background_is_working() {
        assert_eq!(display_state(AgentRunState::Running, 2), AgentRunState::Working);
    }

    #[test]
    fn display_state_working_passes_through() {
        assert_eq!(display_state(AgentRunState::Working, 0), AgentRunState::Working);
    }

    #[test]
    fn display_state_needs_attention_wins_over_background() {
        assert_eq!(display_state(AgentRunState::NeedsAttention, 3), AgentRunState::NeedsAttention);
    }

    #[test]
    fn display_state_error_wins_over_background() {
        assert_eq!(display_state(AgentRunState::Error, 1), AgentRunState::Error);
    }

    #[test]
    fn display_state_exited_ignores_background() {
        assert_eq!(display_state(AgentRunState::Exited, 5), AgentRunState::Exited);
    }

    // ── apply_event background gating (AC2-85) ─────────────────────────────────

    #[test]
    fn background_subagent_keeps_pill_working_across_main_idle() {
        let mut states = HashMap::new();
        apply_event(&mut states, "a1", StatusEvent::Working);
        // A background subagent is dispatched — pill already Working, no change.
        assert_eq!(apply_event(&mut states, "a1", StatusEvent::SubagentStarted), None);
        // The main loop ends its turn, but the subagent is still running:
        // the Stop→Idle must be suppressed so the pill stays Working.
        assert_eq!(apply_event(&mut states, "a1", StatusEvent::Idle), None);
        assert_eq!(states.get("a1").map(|s| display_state(s.run, s.bg_count)), Some(AgentRunState::Working));
    }

    #[test]
    fn last_background_subagent_stop_returns_to_idle() {
        let mut states = HashMap::new();
        apply_event(&mut states, "a1", StatusEvent::Working);
        apply_event(&mut states, "a1", StatusEvent::SubagentStarted);
        apply_event(&mut states, "a1", StatusEvent::Idle); // suppressed
        // The subagent completes → count hits 0 → now genuinely idle.
        assert_eq!(
            apply_event(&mut states, "a1", StatusEvent::SubagentStopped),
            Some(AgentRunState::Idle)
        );
    }

    #[test]
    fn multiple_background_subagents_balance_before_idle() {
        let mut states = HashMap::new();
        apply_event(&mut states, "a1", StatusEvent::Working);
        apply_event(&mut states, "a1", StatusEvent::SubagentStarted);
        apply_event(&mut states, "a1", StatusEvent::SubagentStarted);
        apply_event(&mut states, "a1", StatusEvent::Idle); // suppressed, 2 in flight
        // One finishes — still one in flight, stay Working.
        assert_eq!(apply_event(&mut states, "a1", StatusEvent::SubagentStopped), None);
        // The last one finishes — now idle.
        assert_eq!(
            apply_event(&mut states, "a1", StatusEvent::SubagentStopped),
            Some(AgentRunState::Idle)
        );
    }

    #[test]
    fn needs_attention_during_background_work_is_emitted() {
        let mut states = HashMap::new();
        apply_event(&mut states, "a1", StatusEvent::Working);
        apply_event(&mut states, "a1", StatusEvent::SubagentStarted);
        // The main agent needs input while the subagent runs — attention surfaces.
        assert_eq!(
            apply_event(&mut states, "a1", StatusEvent::NeedsAttention),
            Some(AgentRunState::NeedsAttention)
        );
    }

    #[test]
    fn exit_resets_background_count() {
        let mut states = HashMap::new();
        apply_event(&mut states, "a1", StatusEvent::Working);
        apply_event(&mut states, "a1", StatusEvent::SubagentStarted);
        apply_event(&mut states, "a1", StatusEvent::Exited);
        assert_eq!(states.get("a1").map(|s| s.bg_count), Some(0));
        assert_eq!(states.get("a1").map(|s| s.run), Some(AgentRunState::Exited));
    }

    #[test]
    fn stray_subagent_stop_does_not_underflow() {
        let mut states = HashMap::new();
        apply_event(&mut states, "a1", StatusEvent::Working);
        // A SubagentStop with no outstanding start must saturate at 0, not wrap.
        assert_eq!(apply_event(&mut states, "a1", StatusEvent::SubagentStopped), None);
        assert_eq!(states.get("a1").map(|s| s.bg_count), Some(0));
    }
}
