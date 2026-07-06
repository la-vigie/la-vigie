import { useEffect, useRef, useState } from "react";
import { useVigieStore } from "../../store";
import "./PromptPicker.css";

export interface PromptPickerProps {
  onSelect: (body: string) => void;
  onManage: () => void;
}

export function PromptPicker({ onSelect, onManage }: PromptPickerProps) {
  const prompts = useVigieStore((s) => s.prompts);
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  // While open: close on an outside click (no full-screen overlay, so the
  // surrounding modal/terminal stays interactive), and on Escape. The Escape
  // listener is on `document` in the capture phase with stopPropagation so it
  // closes only the dropdown and never bubbles to a parent's window keydown
  // (e.g. NewTaskForm's Escape-closes-the-modal handler).
  useEffect(() => {
    if (!open) return;
    const onMouseDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown, true);
    };
  }, [open]);

  return (
    <div className="prompt-picker" ref={rootRef}>
      <button
        type="button"
        className="prompt-picker__button"
        aria-label="Prompt library"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        ▾ Library
      </button>
      {open && (
        <div className="prompt-picker__menu" role="menu">
          {prompts.map((p) => (
            <button
              key={p.id}
              type="button"
              role="menuitem"
              className="prompt-picker__item"
              title={p.body}
              // Keep focus on the target field (e.g. the new-task textarea) so
              // selecting doesn't blur it. Selection fires on click (so it stays
              // keyboard-accessible).
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => {
                setOpen(false);
                onSelect(p.body);
              }}
            >
              {p.label}
            </button>
          ))}
          {prompts.length > 0 && <div className="prompt-picker__divider" />}
          <button
            type="button"
            role="menuitem"
            className="prompt-picker__item prompt-picker__item--manage"
            onClick={() => {
              setOpen(false);
              onManage();
            }}
          >
            ✎ Manage prompts…
          </button>
        </div>
      )}
    </div>
  );
}
