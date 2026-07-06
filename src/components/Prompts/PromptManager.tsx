import { useEffect, useState } from "react";
import { useVigieStore } from "../../store";
import type { Prompt } from "../../api";

interface PromptRowProps {
  p: Prompt;
  index: number;
  total: number;
  editPrompt: (id: string, label: string, body: string) => Promise<void>;
  removePrompt: (id: string) => Promise<void>;
  movePrompt: (id: string, direction: "up" | "down") => Promise<void>;
}

function PromptRow({ p, index, total, editPrompt, removePrompt, movePrompt }: PromptRowProps) {
  const [label, setLabel] = useState(p.label);
  const [body, setBody] = useState(p.body);

  useEffect(() => { setLabel(p.label); }, [p.label]);
  useEffect(() => { setBody(p.body); }, [p.body]);

  const persistIfChanged = () => {
    if (label !== p.label || body !== p.body) {
      void editPrompt(p.id, label, body);
    }
  };

  return (
    <li className="prompt-manager__row">
      <input
        className="field"
        aria-label={`Prompt ${index + 1} label`}
        value={label}
        onChange={(e) => setLabel(e.target.value)}
        onBlur={persistIfChanged}
      />
      <textarea
        className="field"
        aria-label={`Prompt ${index + 1} body`}
        rows={2}
        value={body}
        onChange={(e) => setBody(e.target.value)}
        onBlur={persistIfChanged}
      />
      <div className="prompt-manager__row-actions">
        <button type="button" aria-label={`Move ${p.label} up`} disabled={index === 0}
          onClick={() => void movePrompt(p.id, "up")}>↑</button>
        <button type="button" aria-label={`Move ${p.label} down`} disabled={index === total - 1}
          onClick={() => void movePrompt(p.id, "down")}>↓</button>
        <button type="button" aria-label={`Delete ${p.label}`}
          onClick={() => void removePrompt(p.id)}>Delete</button>
      </div>
    </li>
  );
}

export function PromptManager() {
  const prompts = useVigieStore((s) => s.prompts);
  const addPrompt = useVigieStore((s) => s.addPrompt);
  const editPrompt = useVigieStore((s) => s.editPrompt);
  const removePrompt = useVigieStore((s) => s.removePrompt);
  const movePrompt = useVigieStore((s) => s.movePrompt);

  const [newLabel, setNewLabel] = useState("");
  const [newBody, setNewBody] = useState("");

  const add = () => {
    if (!newLabel.trim() || !newBody.trim()) return;
    void addPrompt(newLabel.trim(), newBody.trim());
    setNewLabel("");
    setNewBody("");
  };

  return (
    <section className="settings__section">
      <div className="settings__section-header">
        <h3 className="settings__section-title">Prompts</h3>
      </div>
      <span className="settings__hint">
        Saved prompts you can insert from the "Library" picker when starting a task or during an agent run.
      </span>

      <ul className="prompt-manager__list">
        {prompts.map((p, i) => (
          <PromptRow
            key={p.id}
            p={p}
            index={i}
            total={prompts.length}
            editPrompt={editPrompt}
            removePrompt={removePrompt}
            movePrompt={movePrompt}
          />
        ))}
      </ul>

      <div className="prompt-manager__add">
        <input
          className="field"
          aria-label="New prompt label"
          placeholder="Label (e.g. No brainstorm, go)"
          value={newLabel}
          onChange={(e) => setNewLabel(e.target.value)}
        />
        <textarea
          className="field"
          aria-label="New prompt body"
          placeholder="Prompt text inserted into the agent"
          rows={2}
          value={newBody}
          onChange={(e) => setNewBody(e.target.value)}
        />
        <button type="button" className="btn" onClick={add}>Add prompt</button>
      </div>
    </section>
  );
}
