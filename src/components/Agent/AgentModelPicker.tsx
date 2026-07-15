import { useState, useRef, useEffect, useLayoutEffect, useCallback } from "react";
import { createPortal } from "react-dom";
import { useAgents, useAgentModels } from "../../hooks/useAgents";
import type { AgentSpec } from "../../store";
import "./AgentModelPicker.css";

interface Props {
  agent: string;
  model: string | null;
  onChange: (agent: string, model: string | null) => void;
}

const advertisesModels = (s: AgentSpec | undefined) =>
  !!(s?.modelsListArgs && s.modelsListArgs.length > 0);

// An agent takes a model if it can enumerate one (list picker) or merely accepts
// a `--model` flag (free-text entry). TASK-209: Claude Code is the latter.
const takesModel = (s: AgentSpec | undefined) => advertisesModels(s) || !!s?.modelArg;

// Advisory quick-pick suggestions for free-text engines, keyed by agent name.
// Non-binding hints only — free-text stays authoritative — so a stale/missing
// entry is harmless. Claude's tier aliases are stable across model releases.
const MODEL_SUGGESTIONS: Record<string, readonly string[]> = {
  claude: ["opus", "sonnet", "haiku"],
};

export function AgentModelPicker({ agent, model, onChange }: Props) {
  const { agents } = useAgents();
  const [open, setOpen] = useState(false);
  const [hovered, setHovered] = useState<string>(agent);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);
  // Fixed-viewport coordinates for the portaled popover (null until measured).
  const [coords, setCoords] = useState<{ left: number; top: number } | null>(null);

  const current = agents.find((a) => a.name === agent);
  const hoveredSpec = agents.find((a) => a.name === hovered);
  const { models } = useAgentModels(advertisesModels(hoveredSpec) ? hovered : undefined);
  // Free-text draft for engines that take `--model` but can't enumerate models.
  // Seeded from the current model when the hovered agent is the current one.
  const [draft, setDraft] = useState("");
  useEffect(() => {
    setDraft(hovered === agent ? (model ?? "") : "");
  }, [hovered, agent, model, open]);

  const triggerLabel = current?.displayName ?? agent;

  // Position the popover relative to the trigger, in viewport (fixed) space so
  // it is never clipped by an ancestor's overflow (e.g. the new-task modal).
  // Flips above the trigger when there isn't room below.
  const reposition = useCallback(() => {
    const trigger = triggerRef.current;
    const pop = popoverRef.current;
    if (!trigger || !pop) return;
    const t = trigger.getBoundingClientRect();
    const p = pop.getBoundingClientRect();
    const margin = 8;
    let top = t.bottom + 4;
    if (top + p.height > window.innerHeight - margin && t.top - p.height - 4 > margin) {
      top = t.top - p.height - 4; // flip up
    }
    const left = Math.max(margin, Math.min(t.left, window.innerWidth - p.width - margin));
    setCoords({ left, top });
  }, []);

  // Measure and place after the popover mounts (pre-paint, so no flash), and
  // keep it anchored on scroll/resize while open.
  useLayoutEffect(() => {
    if (!open) {
      setCoords(null);
      return;
    }
    reposition();
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [open, reposition, agents.length, models.length, hovered]);

  // Close on outside click. The popover is portaled out of the root, so check
  // both the root (trigger) and the popover.
  useEffect(() => {
    if (!open) return;
    function handleMouseDown(e: MouseEvent) {
      const target = e.target as Node;
      if (rootRef.current?.contains(target)) return;
      if (popoverRef.current?.contains(target)) return;
      setOpen(false);
    }
    document.addEventListener("mousedown", handleMouseDown);
    return () => document.removeEventListener("mousedown", handleMouseDown);
  }, [open]);

  function toggleOpen() {
    if (!open) setHovered(agent); // reset stale hover when opening
    setOpen((o) => !o);
  }

  function pickAgent(spec: AgentSpec) {
    if (takesModel(spec)) {
      setHovered(spec.name); // reveal its model pane (list or free-text); await a pick
    } else {
      onChange(spec.name, null);
      setOpen(false);
    }
  }

  function pickModel(id: string) {
    onChange(hovered, id);
    setOpen(false);
  }

  // Commit a free-text model id for the hovered agent. Blank ⇒ unset (default).
  function commitModel(id: string) {
    const trimmed = id.trim();
    onChange(hovered, trimmed === "" ? null : trimmed);
    setOpen(false);
  }

  // Commit the hovered agent with no model (used when the model list is empty).
  function pickDefault() {
    onChange(hovered, null);
    setOpen(false);
  }

  return (
    <div className="amp" ref={rootRef}>
      <button type="button" className="amp__trigger" data-testid="amp-trigger" ref={triggerRef} onClick={toggleOpen}>
        {triggerLabel}
        {model && <span className="amp__model"> · {model}</span>}
        <span className="amp__caret" aria-hidden>▾</span>
      </button>
      {open && createPortal(
        <div
          className="amp__popover"
          role="menu"
          ref={popoverRef}
          style={{
            left: coords?.left ?? 0,
            top: coords?.top ?? 0,
            visibility: coords ? "visible" : "hidden",
          }}
        >
          <ul className="amp__agents">
            {agents.map((a) => (
              <li
                key={a.name}
                className={"amp__agent-row" + (a.name === hovered ? " is-hovered" : "")}
                onMouseEnter={() => setHovered(a.name)}
                onClick={() => pickAgent(a)}
              >
                {a.displayName}
                {takesModel(a) && <span className="amp__chevron" aria-hidden>›</span>}
                {a.name === agent && <span className="amp__check" aria-hidden>✓</span>}
              </li>
            ))}
          </ul>
          {advertisesModels(hoveredSpec) && (
            <ul className="amp__models">
              <li className="amp__models-head">MODEL</li>
              {models.map((id) => (
                <li
                  key={id}
                  className={"amp__model-row" + (id === model ? " is-selected" : "")}
                  onClick={() => pickModel(id)}
                >
                  {id}
                  {id === model && <span className="amp__check" aria-hidden>✓</span>}
                </li>
              ))}
              {models.length === 0 && (
                <li
                  className={"amp__model-row" + (hovered === agent && model === null ? " is-selected" : "")}
                  onClick={pickDefault}
                >
                  Default model
                  {hovered === agent && model === null && <span className="amp__check" aria-hidden>✓</span>}
                </li>
              )}
            </ul>
          )}
          {!advertisesModels(hoveredSpec) && takesModel(hoveredSpec) && (
            <div className="amp__models amp__freetext">
              <div className="amp__models-head">MODEL</div>
              <input
                className="amp__model-input"
                data-testid="amp-model-input"
                value={draft}
                placeholder="model id (e.g. opus)"
                autoFocus
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    commitModel(draft);
                  }
                }}
              />
              {(MODEL_SUGGESTIONS[hovered] ?? []).length > 0 && (
                <div className="amp__chips">
                  {(MODEL_SUGGESTIONS[hovered] ?? []).map((s) => (
                    <button
                      key={s}
                      type="button"
                      className={"amp__chip" + (s === model ? " is-selected" : "")}
                      onClick={() => commitModel(s)}
                    >
                      {s}
                    </button>
                  ))}
                </div>
              )}
              <button type="button" className="amp__model-set" onClick={() => commitModel(draft)}>
                Set model
              </button>
              <div
                className={"amp__model-row amp__default-row" + (hovered === agent && model === null ? " is-selected" : "")}
                onClick={() => commitModel("")}
              >
                Default model
                {hovered === agent && model === null && <span className="amp__check" aria-hidden>✓</span>}
              </div>
            </div>
          )}
        </div>,
        document.body,
      )}
    </div>
  );
}
