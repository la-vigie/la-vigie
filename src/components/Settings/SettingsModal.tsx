import { useEffect, useRef, useState } from "react";
import { QRCodeSVG } from "qrcode.react";
import { listAgents, upsertCustomAgent, deleteCustomAgent, deleteCustomSound, listRemoteSessions, stopSession, type RemoteSession } from "../../api";
import { useVigieStore } from "../../store";
import type { AgentSpec, PromptMode } from "../../store";
import { SOUND_PALETTE, SOUND_EVENTS, soundLabel, type SoundEvent } from "../../sound/types";
import { SoundPlayer } from "../../sound/player";
import { pickAndImportSound } from "../../sound/import";
import { PromptManager } from "../Prompts/PromptManager";
import "./SettingsModal.css";

// cooldownMs: 0 so repeated preview clicks aren't swallowed by the debounce.
const previewPlayer = new SoundPlayer({ cooldownMs: 0 });

interface SettingsModalProps {
  onClose: () => void;
}

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

export function SettingsModal({ onClose }: SettingsModalProps) {
  const soundSettings = useVigieStore((s) => s.soundSettings);
  const setSoundSettings = useVigieStore((s) => s.setSoundSettings);
  const customSounds = useVigieStore((s) => s.customSounds);
  const refreshCustomSounds = useVigieStore((s) => s.refreshCustomSounds);
  const [soundError, setSoundError] = useState<string | null>(null);
  const fetchRemoteBase = useVigieStore((s) => s.fetchRemoteBase);
  const setFetchRemoteBase = useVigieStore((s) => s.setFetchRemoteBase);
  const remote = useVigieStore((s) => s.remote);
  const refreshRemote = useVigieStore((s) => s.refreshRemote);
  const enableRemoteControl = useVigieStore((s) => s.enableRemoteControl);
  const disableRemoteControl = useVigieStore((s) => s.disableRemoteControl);
  const [agents, setAgents] = useState<AgentSpec[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [formOpen, setFormOpen] = useState(false);
  /** Name of the agent being edited; null when adding a new one. */
  const [editingName, setEditingName] = useState<string | null>(null);
  const [form, setForm] = useState<FormState>(emptyForm);

  const [remoteSessions, setRemoteSessions] = useState<RemoteSession[]>([]);

  useEffect(() => {
    if (!remote.active) {
      setRemoteSessions([]);
      return;
    }
    let alive = true;
    const refresh = () => {
      void listRemoteSessions()
        .then((s) => { if (alive) setRemoteSessions(Array.isArray(s) ? s : []); })
        .catch(() => {});
    };
    refresh();
    const t = setInterval(refresh, 5000);
    return () => { alive = false; clearInterval(t); };
  }, [remote.active]);

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

  useEffect(() => { void refreshRemote(); }, [refreshRemote]);

  // Escape to close
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

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

  const setSoundEvent = (
    key: SoundEvent,
    patch: Partial<{ enabled: boolean; sound: string }>,
  ) => {
    const cur = useVigieStore.getState().soundSettings;
    setSoundSettings({
      ...cur,
      events: {
        ...cur.events,
        [key]: { ...cur.events[key], ...patch },
      },
    });
  };

  const addSound = async () => {
    setSoundError(null);
    try {
      const added = await pickAndImportSound();
      if (added) await refreshCustomSounds();
    } catch (err) {
      setSoundError(err instanceof Error ? err.message : String(err));
    }
  };

  const removeSound = async (id: string) => {
    setSoundError(null);
    try {
      await deleteCustomSound(id);
      await refreshCustomSounds();
    } catch (err) {
      setSoundError(err instanceof Error ? err.message : String(err));
    }
  };

  const builtins = agents.filter((a) => a.builtin);
  const customs = agents.filter((a) => !a.builtin);

  return (
    <div className="settings__backdrop" role="presentation" onClick={onClose}>
      <div
        className="settings"
        role="dialog"
        aria-label="Settings"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="settings__header">
          <h2 className="settings__title">Settings</h2>
          <button
            type="button"
            className="icon-btn settings__close"
            aria-label="Close"
            onClick={onClose}
          >
            ✕
          </button>
        </header>

        <div className="settings__body">
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

          <section className="settings__section">
            <div className="settings__section-header">
              <h3 className="settings__section-title">Remote access</h3>
            </div>
            <p className="settings__remote-hint">
              Drive La Vigie from another device on your tailnet. Off by default;
              reachable only over Tailscale (never the public internet).
            </p>
            {remote.active ? (
              <div className="settings__remote-active">
                <p>Remote is <strong>active</strong>.</p>
                <label className="settings__label">Pairing token</label>
                <code className="settings__remote-token">{remote.token}</code>
                {remote.url && (
                  <p className="settings__remote-hint">Open <code>{remote.url}</code> on your phone.</p>
                )}
                {remote.url && remote.token && (
                  <div className="settings__remote-qr">
                    <QRCodeSVG
                      value={`${remote.url}#token=${remote.token}`}
                      size={176}
                      marginSize={2}
                      aria-label="Pairing QR code"
                    />
                    <p className="settings__remote-hint">
                      Scan with your phone camera to open the remote client already paired — no token
                      typing. The token is in the link fragment, so it never reaches the server or its
                      logs. <strong>Anyone who can see this screen (or a screenshot of it) can pair</strong>;
                      keep it private and disable remote when you're done.
                    </p>
                  </div>
                )}
                <p className="settings__remote-hint">
                  {remote.sleepInhibited ? (
                    <>System sleep is <strong>prevented</strong> while remote is active, so the host
                    stays reachable. Works on AC power only — on battery (especially with the lid
                    closed) the Mac may still sleep.</>
                  ) : (
                    <>⚠️ Couldn’t prevent system sleep — the host may become unreachable if the Mac
                    goes to sleep while idle.</>
                  )}
                </p>
                <div className="settings__remote-sessions">
                  <label className="settings__label">Remote sessions</label>
                  {remoteSessions.length === 0 ? (
                    <p className="settings__remote-hint">No remote sessions running.</p>
                  ) : (
                    <ul className="settings__remote-session-list">
                      {remoteSessions.map((s) => (
                        <li key={s.id} className="settings__remote-session">
                          <span>{s.kind} · idle {Math.floor(s.idleSecs / 60)}m</span>
                          <button
                            type="button"
                            className="btn btn--danger"
                            onClick={() =>
                              void stopSession(s.id).then(() =>
                                setRemoteSessions((cur) => cur.filter((x) => x.id !== s.id)),
                              )
                            }
                          >
                            Stop
                          </button>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
                <button type="button" className="btn btn--danger" onClick={() => void disableRemoteControl()}>
                  Disable remote
                </button>
              </div>
            ) : (
              <button type="button" className="btn btn--primary" onClick={() => void enableRemoteControl()}>
                Enable remote
              </button>
            )}
          </section>

          <section className="settings__section">
            <div className="settings__section-header">
              <h3 className="settings__section-title">Alerts (sound + notifications)</h3>
            </div>

            <label className="settings__master">
              <input
                type="checkbox"
                role="switch"
                checked={!soundSettings.muted}
                aria-label="Enable alerts"
                onChange={(e) => {
                  const cur = useVigieStore.getState().soundSettings;
                  setSoundSettings({ ...cur, muted: !e.target.checked });
                }}
              />
              <span>Play sounds and show notifications</span>
            </label>

            <label className="settings__master">
              <input
                type="checkbox"
                role="switch"
                checked={soundSettings.automute}
                aria-label="Mute sounds while in a meeting"
                onChange={(e) => {
                  const cur = useVigieStore.getState().soundSettings;
                  setSoundSettings({ ...cur, automute: e.target.checked });
                }}
              />
              <span>
                Mute sounds while in a meeting
                <span className="settings__hint">
                  Silences alert sounds when your mic or camera is in use (macOS). Notifications
                  still appear.
                </span>
              </span>
            </label>

            {SOUND_EVENTS.map(({ key, label }) => {
              const ev = soundSettings.events[key];
              return (
                <div className="settings__row" key={key}>
                  <input
                    type="checkbox"
                    checked={ev.enabled}
                    aria-label={`Enable ${label} alerts`}
                    onChange={(e) => setSoundEvent(key, { enabled: e.target.checked })}
                  />
                  <span className="settings__row-label">{label}</span>
                  <select
                    className="field"
                    aria-label={`${label} sound`}
                    value={ev.sound}
                    onChange={(e) => setSoundEvent(key, { sound: e.target.value })}
                  >
                    {soundLabel(ev.sound, customSounds) === undefined && (
                      <option value={ev.sound}>(missing sound)</option>
                    )}
                    <optgroup label="Bundled">
                      {SOUND_PALETTE.map((s) => (
                        <option key={s.id} value={s.id}>
                          {s.label}
                        </option>
                      ))}
                    </optgroup>
                    {customSounds.length > 0 && (
                      <optgroup label="Custom">
                        {customSounds.map((s) => (
                          <option key={s.id} value={s.id}>
                            {s.label}
                          </option>
                        ))}
                      </optgroup>
                    )}
                  </select>
                  <button
                    type="button"
                    className="settings__preview"
                    aria-label={`Preview ${label} sound`}
                    onClick={() => void previewPlayer.playSound(ev.sound, customSounds)}
                  >
                    ▶
                  </button>
                </div>
              );
            })}

            <div className="settings__custom-sounds">
              <div className="settings__section-header">
                <span className="settings__agent-group-label">Custom sounds</span>
                <button type="button" className="btn" onClick={addSound}>
                  Add sound…
                </button>
              </div>
              {soundError && <p className="settings__error">{soundError}</p>}
              {customSounds.length > 0 && (
                <ul className="settings__agent-list">
                  {customSounds.map((s) => (
                    <li key={s.id} className="settings__agent-row">
                      <span className="settings__agent-name">{s.label}</span>
                      <div className="settings__agent-actions">
                        <button
                          type="button"
                          className="settings__preview"
                          aria-label={`Preview ${s.label}`}
                          onClick={() => void previewPlayer.playSound(s.id, customSounds)}
                        >
                          ▶
                        </button>
                        <button
                          type="button"
                          className="btn btn--danger"
                          onClick={() => removeSound(s.id)}
                        >
                          Remove
                        </button>
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </section>

          <section className="settings__section">
            <div className="settings__section-header">
              <h3 className="settings__section-title">New worktrees</h3>
            </div>
            <label className="settings__master">
              <input
                type="checkbox"
                role="switch"
                checked={fetchRemoteBase}
                aria-label="Base new worktrees on the latest remote base branch"
                onChange={(e) => setFetchRemoteBase(e.target.checked)}
              />
              <span>Base new worktrees on the latest remote base branch (git fetch first)</span>
            </label>
          </section>

          <PromptManager />
        </div>
      </div>
    </div>
  );
}
