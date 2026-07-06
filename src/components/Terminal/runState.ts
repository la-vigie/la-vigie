import type { AgentActivity, AgentStatus } from "../../store";

export interface RunStateView {
  /** Visible label, e.g. "Working…", "Ready", "Needs input", "Starting…". */
  label: string;
  /** CSS modifier + dot-color key: "starting" | "working" | "idle" | "attention" | "error". */
  modifier: string;
}

/**
 * Maps the live agent's PTY status + hook-reported activity to the pill view.
 * Returns null when there is no live agent (status "exited") — the pill renders nothing.
 */
export function runStateView(
  status: AgentStatus,
  activity?: AgentActivity,
  lifecycle?: boolean,
): RunStateView | null {
  if (status === "exited") return null;
  if (status === "starting") {
    return { label: "Starting…", modifier: "starting" };
  }
  // Lifecycle agents have no hook activity; show a neutral, honest Running.
  if (lifecycle) {
    return { label: "Running", modifier: "running" };
  }
  // status === "running": branch on the hook-reported activity. Only an explicit
  // "working" signal (fired on UserPromptSubmit/PreToolUse) shows the Working…
  // state; until then the agent is alive but idle at its prompt, so a fresh run
  // with no activity yet (undefined) reads Ready.
  switch (activity) {
    case "working":
      return { label: "Working…", modifier: "working" };
    case "needs_attention":
      return { label: "Needs input", modifier: "attention" };
    case "error":
      return { label: "Error", modifier: "error" };
    case "idle":
    default:
      return { label: "Ready", modifier: "idle" };
  }
}
