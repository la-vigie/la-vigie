import { useEffect, useState } from "react";
import { listAgents } from "../../api";
import type { AgentSpec } from "../../store";

interface AgentPickerProps {
  /** Currently selected agent name. */
  value: string;
  /** Called with the new agent name when the user changes the selection. */
  onChange: (name: string) => void;
  /**
   * Pre-loaded list of agents. When supplied the component renders from this
   * list directly. When omitted the component loads agents internally via
   * `listAgents()`.
   */
  agents?: readonly AgentSpec[];
  /** Disable the select element. */
  disabled?: boolean;
  /** Additional CSS class added to the root `<label>` element. */
  className?: string;
  /** Label text shown to the left of the select. Defaults to "Agent". */
  label?: string;
}

/**
 * A reusable agent `<select>` element that renders the available agents.
 *
 * When the `agents` prop is supplied the component is fully controlled by the
 * parent. When it is omitted the component loads the agent list internally via
 * `listAgents()`.
 *
 * The component renders nothing until at least one agent is available — this
 * prevents a React dev-mode warning about a controlled `<select>` with no
 * matching option.
 *
 * CSS classes (kept stable for TASK-76 styling):
 *   .agent-picker            — root <label>
 *   .agent-picker__label     — label span
 *   .agent-picker__select    — the <select> element
 */
export function AgentPicker({
  value,
  onChange,
  agents: agentsProp,
  disabled,
  className,
  label = "Agent",
}: AgentPickerProps) {
  const [internalAgents, setInternalAgents] = useState<AgentSpec[]>([]);

  // When agents are not supplied externally, load them once on mount.
  useEffect(() => {
    if (agentsProp !== undefined) return;
    let live = true;
    listAgents()
      .then((a) => { if (live) setInternalAgents(a); })
      .catch(() => {});
    return () => { live = false; };
  }, [agentsProp]);

  const agents = agentsProp ?? internalAgents;

  // Guard: render nothing until there is at least one agent to display.
  if (agents.length === 0) return null;

  const rootClass = ["agent-picker", className].filter(Boolean).join(" ");

  return (
    <label className={rootClass}>
      <span className="agent-picker__label">{label}</span>
      <select
        className="agent-picker__select"
        aria-label={label}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
      >
        {agents.map((a) => (
          <option key={a.name} value={a.name}>{a.displayName}</option>
        ))}
      </select>
    </label>
  );
}
