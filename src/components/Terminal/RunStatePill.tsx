import type { AgentActivity, AgentStatus } from "../../store";
import { runStateView } from "./runState";
import "./RunStatePill.css";

export interface RunStatePillProps {
  status: AgentStatus;
  activity?: AgentActivity;
  lifecycle?: boolean;
  onStop: () => void;
}

/**
 * The live run-state pill: floats at the top-right of the Claude pane body and
 * *is* the agent's status. Hover/focus turns the whole pill into a Stop control.
 * Renders nothing when there is no live agent. Status comes from the store
 * (out-of-band hook activity) — never from terminal output.
 */
export function RunStatePill({ status, activity, lifecycle, onStop }: RunStatePillProps) {
  const view = runStateView(status, activity, lifecycle);
  if (!view) return null;

  return (
    <button
      type="button"
      className={`run-pill run-pill--${view.modifier}`}
      onClick={onStop}
      aria-label={`Stop agent — ${view.label}`}
    >
      <span className="run-pill__status">
        <span className="run-pill__dot" aria-hidden />
        {view.label}
      </span>
      <span className="run-pill__stop" aria-hidden>
        <span className="run-pill__stop-glyph">◼</span> Stop
      </span>
    </button>
  );
}
