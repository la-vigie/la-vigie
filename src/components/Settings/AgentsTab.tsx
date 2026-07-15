import { useEffect, useRef, useState } from "react";
import { listAgents, upsertCustomAgent, deleteCustomAgent } from "../../api";
import type { AgentSpec, PromptMode } from "../../store";

interface FormState {
  name: string;
  displayName: string;
  binary: string;
  promptMode: PromptMode;
  baseArgsText: string;
  resumeArgsText: string;
  extraArgsText: string;
}

const emptyForm: FormState = {
  name: "",
  displayName: "",
  binary: "",
  promptMode: "stdin",
  baseArgsText: "",
  resumeArgsText: "",
  extraArgsText: "",
};

function formFromSpec(spec: AgentSpec): FormState {
  return {
    name: spec.name,
    displayName: spec.displayName,
    binary: spec.binary,
    promptMode: spec.promptMode,
    baseArgsText: spec.baseArgs.join("\n"),
    resumeArgsText: spec.resumeArgs.join("\n"),
    extraArgsText: spec.extraArgs.join("\n"),
  };
}

function parseArgs(text: string): string[] {
  return text
    .split("\n")
    .map((s) => s.trim())
    .filter(Boolean);
}

export function AgentsTab() {
  const [agents, setAgents] = useState<AgentSpec[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [formOpen, setFormOpen] = useState(false);
  /** Name of the agent being edited; null when adding a new one. */
  const [editingName, setEditingName] = useState<string | null>(null);
  const [form, setForm] = useState<FormState>(emptyForm);

  // Track mount state so refreshAgents never sets state after unmount.
  const mountedRef = useRef(true);
  useEffect(() => {
    // Reset on (re)mount so StrictMode's mount→cleanup→remount cycle doesn't
    // leave the ref latched false while the component is live.
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Load agents on mount with a live flag to avoid state-on-unmount.
  useEffect(() => {
    let live = true;
    listAgents().then((list) => {
      if (live) setAgents(list);
    });
    return () => {
      live = false;
    };
  }, []);

  const refreshAgents = async () => {
    const list = await listAgents();
    if (mountedRef.current) setAgents(list);
  };

  const openAdd = () => {
    setForm(emptyForm);
    setEditingName(null);
    setFormOpen(true);
    setError(null);
  };

  const openEdit = (spec: AgentSpec) => {
    setForm(formFromSpec(spec));
    setEditingName(spec.name);
    setFormOpen(true);
    setError(null);
  };

  const closeForm = () => {
    setFormOpen(false);
    setError(null);
  };

  const submit = async () => {
    const trimmedName = form.name.trim();
    const spec: AgentSpec = {
      name: trimmedName,
      displayName: form.displayName.trim() || trimmedName,
      binary: form.binary.trim(),
      baseArgs: parseArgs(form.baseArgsText),
      resumeArgs: parseArgs(form.resumeArgsText),
      extraArgs: parseArgs(form.extraArgsText),
      promptMode: form.promptMode,
      // The backend re-forces status/builtin; setting them here keeps the TS
      // type satisfied and makes intent clear.
      status: "lifecycle",
      builtin: false,
    };
    try {
      await upsertCustomAgent(spec);
      await refreshAgents();
      closeForm();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const remove = async (name: string) => {
    try {
      await deleteCustomAgent(name);
      await refreshAgents();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const builtins = agents.filter((a) => a.builtin);
  const customs = agents.filter((a) => !a.builtin);

  return (
    <section className="settings__section">
      <div className="settings__section-header">
        <h3 className="settings__section-title">Agents</h3>
        {!formOpen && (
          <button type="button" className="btn" onClick={openAdd}>
            Add agent
          </button>
        )}
      </div>

      {error && <p className="settings__error">{error}</p>}

      {builtins.length > 0 && (
        <div className="settings__agent-group">
          <span className="settings__agent-group-label">Built-in</span>
          <ul className="settings__agent-list">
            {builtins.map((agent) => (
              <li key={agent.name} className="settings__agent-row">
                <div className="settings__agent-info">
                  <span className="settings__agent-name">{agent.displayName}</span>
                  <span className="settings__agent-binary">{agent.binary}</span>
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}

      {customs.length > 0 && (
        <div className="settings__agent-group">
          <span className="settings__agent-group-label">Custom</span>
          <ul className="settings__agent-list">
            {customs.map((agent) => (
              <li key={agent.name} className="settings__agent-row">
                <div className="settings__agent-info">
                  <span className="settings__agent-name">{agent.displayName}</span>
                  <span className="settings__agent-binary">{agent.binary}</span>
                </div>
                <div className="settings__agent-actions">
                  <button
                    type="button"
                    className="btn btn--ghost"
                    onClick={() => openEdit(agent)}
                  >
                    Edit
                  </button>
                  <button
                    type="button"
                    className="btn btn--danger"
                    onClick={() => remove(agent.name)}
                  >
                    Remove
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}

      {formOpen && (
        <div className="settings__form">
          <h4 className="settings__form-title">
            {editingName ? "Edit agent" : "Add agent"}
          </h4>
          <div className="settings__form-fields">
            <label className="settings__field">
              <span className="settings__label">Name</span>
              <input
                className="field"
                aria-label="Name"
                value={form.name}
                onChange={(e) => setForm({ ...form, name: e.target.value })}
                placeholder="my-agent"
                readOnly={editingName !== null}
              />
            </label>

            <label className="settings__field">
              <span className="settings__label">Display name</span>
              <input
                className="field"
                aria-label="Display name"
                value={form.displayName}
                onChange={(e) => setForm({ ...form, displayName: e.target.value })}
                placeholder="My Agent"
              />
            </label>

            <label className="settings__field">
              <span className="settings__label">Binary</span>
              <input
                className="field"
                aria-label="Binary"
                value={form.binary}
                onChange={(e) => setForm({ ...form, binary: e.target.value })}
                placeholder="/usr/local/bin/my-agent"
              />
            </label>

            <label className="settings__field">
              <span className="settings__label">Prompt mode</span>
              <select
                className="field"
                aria-label="Prompt mode"
                value={form.promptMode}
                onChange={(e) =>
                  setForm({ ...form, promptMode: e.target.value as PromptMode })
                }
              >
                <option value="stdin">stdin</option>
                <option value="arg">arg</option>
                <option value="none">none</option>
              </select>
            </label>

            <div className="settings__field">
              <span className="settings__label">Base args</span>
              <textarea
                className="field"
                aria-label="Base args"
                rows={3}
                value={form.baseArgsText}
                onChange={(e) => setForm({ ...form, baseArgsText: e.target.value })}
              />
              <span className="settings__hint">One argument per line</span>
            </div>

            <div className="settings__field">
              <span className="settings__label">Resume args</span>
              <textarea
                className="field"
                aria-label="Resume args"
                rows={3}
                value={form.resumeArgsText}
                onChange={(e) => setForm({ ...form, resumeArgsText: e.target.value })}
              />
              <span className="settings__hint">One argument per line</span>
            </div>

            <div className="settings__field">
              <span className="settings__label">Extra args</span>
              <textarea
                className="field"
                aria-label="Extra args"
                rows={3}
                value={form.extraArgsText}
                onChange={(e) => setForm({ ...form, extraArgsText: e.target.value })}
              />
              <span className="settings__hint">One argument per line</span>
            </div>
          </div>

          <div className="settings__form-footer">
            <button type="button" className="btn btn--ghost" onClick={closeForm}>
              Cancel
            </button>
            <button type="button" className="btn btn--primary" onClick={submit}>
              {editingName ? "Save" : "Add"}
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
