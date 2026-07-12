import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { updateRepo, listRepoBranches, setRepoDefaultModel } from "../../api";
import { useVigieStore } from "../../store";
import type { Repo } from "../../store";
import { SOUND_PALETTE, SOUND_EVENTS, DEFAULT_SOUND_SETTINGS, soundLabel } from "../../sound/types";
import type { SoundEvent, RepoSoundOverride } from "../../sound/types";
import { parseRepoOverride } from "../../sound/safe-parse";
import { AgentModelPicker } from "../Agent/AgentModelPicker";

interface RepoSettingsModalProps {
  repo: Repo;
  onClose: () => void;
}

export function RepoSettingsModal({ repo, onClose }: RepoSettingsModalProps) {
  const refresh = useVigieStore((state) => state.refresh);
  const worktreesRoot = useVigieStore((state) => state.worktreesRoot);
  const removeRepo = useVigieStore((state) => state.removeRepo);
  const appSound = useVigieStore((state) => state.soundSettings);
  const customSounds = useVigieStore((s) => s.customSounds);
  const taskCount = useVigieStore(
    (state) => state.tasks.filter((t) => t.repoId === repo.id).length,
  );
  const [confirmingRemove, setConfirmingRemove] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [name, setName] = useState(repo.name);
  const [defaultBranch, setDefaultBranch] = useState(repo.defaultBranch);
  const [branches, setBranches] = useState<string[]>([repo.defaultBranch]);
  const [worktreeRoot, setWorktreeRoot] = useState<string | null>(
    repo.worktreeRoot ?? null,
  );
  const [setupCommand, setSetupCommand] = useState(repo.setupCommand ?? "");
  const [defaultAgent, setDefaultAgent] = useState(repo.defaultAgent ?? "claude");
  const [defaultModel, setDefaultModel] = useState<string | null>(repo.defaultModel ?? null);
  const [autoStartAgent, setAutoStartAgent] = useState(repo.autoStartAgent ?? false);
  const [initialPrompt, setInitialPrompt] = useState(repo.initialPrompt ?? "");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [override, setOverride] = useState<RepoSoundOverride>(
    parseRepoOverride(repo.soundSettings),
  );
  const [fetchRemoteBase, setFetchRemoteBase] = useState<boolean | null>(
    repo.fetchRemoteBase ?? null,
  );
  const [autoApprove, setAutoApprove] = useState<boolean | null>(
    repo.autoApprove ?? null,
  );

  // Populate the base-branch dropdown from the repo's local branches. Merge the
  // stored value in (it may be a branch that no longer exists locally) so it's
  // never silently dropped, and degrade quietly to just that value on error.
  useEffect(() => {
    let cancelled = false;
    listRepoBranches(repo.id)
      .then((list) => {
        if (cancelled) return;
        setBranches(
          list.includes(repo.defaultBranch) ? list : [repo.defaultBranch, ...list],
        );
      })
      .catch(() => {
        /* keep the [defaultBranch] fallback */
      });
    return () => {
      cancelled = true;
    };
  }, [repo.id, repo.defaultBranch]);

  // ── Sound override helpers ────────────────────────────────────────────────────

  const muteValue =
    override.muted === undefined ? "inherit" : override.muted ? "off" : "on";

  const setMute = (v: string) =>
    setOverride((o) => {
      const next = { ...o };
      if (v === "inherit") delete next.muted;
      else next.muted = v === "off";
      return next;
    });

  const automuteValue =
    override.automute === undefined ? "inherit" : override.automute ? "on" : "off";

  const setAutomute = (v: string) =>
    setOverride((o) => {
      const next = { ...o };
      if (v === "inherit") delete next.automute;
      else next.automute = v === "on";
      return next;
    });

  const getEnableValue = (key: SoundEvent): string => {
    const ev = override.events?.[key];
    if (ev?.enabled === undefined) return "inherit";
    return ev.enabled ? "on" : "off";
  };

  const getSoundValue = (key: SoundEvent): string =>
    override.events?.[key]?.sound ?? "inherit";

  const appSoundLabel = (key: SoundEvent): string => {
    const soundId = appSound?.events?.[key]?.sound ?? DEFAULT_SOUND_SETTINGS.events[key].sound;
    return SOUND_PALETTE.find((p) => p.id === soundId)?.label ?? soundId;
  };

  const setEventField = (key: SoundEvent, field: "enabled" | "sound", value: string) => {
    setOverride((o) => {
      const events: RepoSoundOverride["events"] = { ...(o.events ?? {}) };
      const eventObj: { enabled?: boolean; sound?: string } = { ...(events[key] ?? {}) };

      if (value === "inherit") {
        delete eventObj[field];
      } else if (field === "enabled") {
        eventObj.enabled = value === "on";
      } else {
        eventObj.sound = value;
      }

      if (Object.keys(eventObj).length === 0) {
        delete events[key];
      } else {
        events[key] = eventObj;
      }

      const next = { ...o };
      if (Object.keys(events).length === 0) {
        delete next.events;
      } else {
        next.events = events;
      }
      return next;
    });
  };

  // ─────────────────────────────────────────────────────────────────────────────

  const chooseFolder = async () => {
    const picked = await open({ directory: true });
    if (typeof picked === "string") setWorktreeRoot(picked);
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const save = async () => {
    const trimmedName = name.trim();
    const trimmedBranch = defaultBranch.trim();
    if (!trimmedName) {
      setError("Name cannot be empty.");
      return;
    }
    if (!trimmedBranch) {
      setError("Default base branch cannot be empty.");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const hasOverride =
        override.muted !== undefined ||
        override.automute !== undefined ||
        (override.events !== undefined && Object.keys(override.events).length > 0);
      await updateRepo(
        repo.id,
        trimmedName,
        trimmedBranch,
        worktreeRoot,
        setupCommand.trim() || null,
        autoStartAgent,
        initialPrompt.trim() || null,
        hasOverride ? JSON.stringify(override) : null,
        fetchRemoteBase,
        defaultAgent,
        autoApprove,
      );
      await setRepoDefaultModel(repo.id, defaultModel);
      await refresh();
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setSaving(false);
    }
  };

  const remove = async () => {
    setRemoving(true);
    setError(null);
    try {
      await removeRepo(repo.id);
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setRemoving(false);
    }
  };

  const taskLabel = `${taskCount} task${taskCount === 1 ? "" : "s"}`;
  const defaultWorktreePath = `${worktreesRoot}/${repo.id}`;

  return (
    <div className="repo-settings__backdrop" role="presentation" onClick={onClose}>
      <div
        className="repo-settings"
        role="dialog"
        aria-label={`Settings for ${repo.name}`}
        onClick={(e) => e.stopPropagation()}
      >
        <header className="repo-settings__header">
          <span className="repo-settings__icon" aria-hidden="true">
            <svg viewBox="0 0 24 24" width="16" height="16" fill="none">
              <path
                d="M2 12s3.5-6.5 10-6.5S22 12 22 12s-3.5 6.5-10 6.5S2 12 2 12Z"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
              <circle cx="12" cy="12" r="2.5" stroke="currentColor" strokeWidth="1.8" />
            </svg>
          </span>
          <div className="repo-settings__heading">
            <h2 className="repo-settings__title">Repository settings</h2>
            <span className="repo-settings__subtitle">{repo.name}</span>
          </div>
          <button
            type="button"
            className="icon-btn repo-settings__close"
            aria-label="Close"
            onClick={onClose}
          >
            ✕
          </button>
        </header>

        <div className="repo-settings__body">
          <div className="repo-settings__row">
            <label className="repo-settings__field">
              <span className="repo-settings__label">Name</span>
              <input
                className="field"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </label>

            <label className="repo-settings__field">
              <span className="repo-settings__label">Default base branch</span>
              <select
                className="field repo-settings__select"
                aria-label="Default base branch"
                value={defaultBranch}
                onChange={(e) => setDefaultBranch(e.target.value)}
              >
                {branches.map((b) => (
                  <option key={b} value={b}>
                    {b}
                  </option>
                ))}
              </select>
            </label>
          </div>

          <div className="repo-settings__field">
            <span className="repo-settings__label">Details</span>
            <div className="repo-settings__details">
              <div className="repo-settings__detail-row">
                <span className="repo-settings__detail-key">Path</span>
                <span className="repo-settings__detail-val mono">{repo.path}</span>
              </div>
              <div className="repo-settings__detail-row">
                <span className="repo-settings__detail-key">Remote URL</span>
                <span className="repo-settings__detail-val mono">
                  {repo.remoteUrl ?? "none"}
                </span>
              </div>
              <div className="repo-settings__detail-row">
                <span className="repo-settings__detail-key">Worktrees</span>
                <div className="repo-settings__detail-val repo-settings__worktrees">
                  <span className="repo-settings__worktree-path">
                    {!worktreeRoot && <span className="badge badge--accent">Default</span>}
                    <span className="mono repo-settings__worktree-loc">
                      {worktreeRoot ?? defaultWorktreePath}
                    </span>
                  </span>
                  <span className="repo-settings__worktree-actions">
                    <button
                      type="button"
                      className="repo-settings__link"
                      onClick={chooseFolder}
                    >
                      Choose location…
                    </button>
                    {worktreeRoot && (
                      <button
                        type="button"
                        className="repo-settings__link"
                        onClick={() => setWorktreeRoot(null)}
                      >
                        Clear
                      </button>
                    )}
                  </span>
                </div>
              </div>
            </div>
          </div>

          <label className="repo-settings__field">
            <span className="repo-settings__label">Setup command</span>
            <input
              className="field"
              aria-label="Setup command"
              placeholder="e.g. npm install"
              value={setupCommand}
              onChange={(e) => setSetupCommand(e.target.value)}
            />
            <span className="repo-settings__hint">
              Runs in each new task's worktree via your interactive shell. Leave empty to
              use <code>.vigie/setup.sh</code> if present.
            </span>
          </label>

          <div className="repo-settings__divider" />

          <div className="repo-settings__field">
            <span className="repo-settings__label">Default agent</span>
            <AgentModelPicker
              agent={defaultAgent}
              model={defaultModel}
              onChange={(a, m) => { setDefaultAgent(a); setDefaultModel(m); }}
            />
            <span className="repo-settings__hint">
              Seeds the agent (and model) for new tasks in this repo.
            </span>
          </div>

          <div className="repo-settings__row">
            <label className="repo-settings__field">
              <span className="repo-settings__label">Fetch remote base</span>
              <select
                className="field"
                aria-label="Fetch remote base"
                value={fetchRemoteBase === null ? "inherit" : fetchRemoteBase ? "on" : "off"}
                onChange={(e) =>
                  setFetchRemoteBase(
                    e.target.value === "inherit" ? null : e.target.value === "on",
                  )
                }
              >
                <option value="inherit">Use app default</option>
                <option value="on">On</option>
                <option value="off">Off</option>
              </select>
            </label>
          </div>

          <div className="repo-settings__row">
            <label className="repo-settings__field">
              <span className="repo-settings__label">Auto-approve agent actions</span>
              <select
                className="field"
                aria-label="Auto-approve agent actions"
                value={autoApprove === null ? "inherit" : autoApprove ? "on" : "off"}
                onChange={(e) =>
                  setAutoApprove(
                    e.target.value === "inherit" ? null : e.target.value === "on",
                  )
                }
              >
                <option value="inherit">Use default (on)</option>
                <option value="on">On</option>
                <option value="off">Off</option>
              </select>
              <span className="repo-settings__hint">
                Auto-approves agent actions for engines that support it (e.g. Mistral Vibe).
              </span>
            </label>
          </div>

          <div className="repo-settings__field repo-settings__autostart">
            <label className="repo-settings__toggle">
              <input
                type="checkbox"
                role="switch"
                className="repo-settings__switch"
                checked={autoStartAgent}
                onChange={(e) => setAutoStartAgent(e.target.checked)}
                aria-label="Auto-start agent on task creation"
              />
              <span className="repo-settings__switch-track" aria-hidden="true">
                <span className="repo-settings__switch-thumb" />
              </span>
              <span className="repo-settings__toggle-text">
                <span className="repo-settings__toggle-title">
                  Auto-start agent on task creation
                </span>
                <span className="repo-settings__hint">
                  Spawns a Claude session as soon as a new task is created in this repo.
                </span>
              </span>
            </label>

            {autoStartAgent && (
              <label className="repo-settings__field repo-settings__nested">
                <span className="repo-settings__label">Initial prompt</span>
                <textarea
                  className="field"
                  aria-label="Repository initial prompt"
                  rows={3}
                  value={initialPrompt}
                  onChange={(e) => setInitialPrompt(e.target.value)}
                />
                <span className="repo-settings__hint">
                  Prepended to each new task's prompt when auto-start is on.
                </span>
              </label>
            )}
          </div>

          <div className="repo-settings__divider" />

          <div className="repo-settings__field">
            <span className="repo-settings__label">Sound notifications</span>

            <div className="repo-settings__sound-row">
              <span className="repo-settings__sound-col-label">Mute</span>
              <select
                className="field repo-settings__select"
                aria-label="Repo sound mute"
                value={muteValue}
                onChange={(e) => setMute(e.target.value)}
              >
                <option value="inherit">Inherit</option>
                <option value="on">Unmuted</option>
                <option value="off">Muted</option>
              </select>
            </div>

            <div className="repo-settings__sound-row">
              <span className="repo-settings__sound-col-label">Mute in meetings</span>
              <select
                className="field repo-settings__select"
                aria-label="Repo mute in meetings"
                value={automuteValue}
                onChange={(e) => setAutomute(e.target.value)}
              >
                <option value="inherit">Inherit</option>
                <option value="on">On</option>
                <option value="off">Off</option>
              </select>
            </div>

            {SOUND_EVENTS.map(({ key, label }) => (
              <div key={key} className="repo-settings__sound-event">
                <span className="repo-settings__sound-event-label">{label}</span>
                <select
                  className="field repo-settings__select"
                  aria-label={`${label} enable`}
                  value={getEnableValue(key)}
                  onChange={(e) => setEventField(key, "enabled", e.target.value)}
                >
                  <option value="inherit">Inherit</option>
                  <option value="on">On</option>
                  <option value="off">Off</option>
                </select>
                <select
                  className="field repo-settings__select"
                  aria-label={`${label} sound`}
                  value={getSoundValue(key)}
                  onChange={(e) => setEventField(key, "sound", e.target.value)}
                >
                  <option value="inherit">Inherit ({appSoundLabel(key)})</option>
                  {getSoundValue(key) !== "inherit" &&
                    soundLabel(getSoundValue(key), customSounds) === undefined && (
                      <option value={getSoundValue(key)}>(missing sound)</option>
                    )}
                  <optgroup label="Bundled">
                    {SOUND_PALETTE.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.label}
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
              </div>
            ))}
          </div>

          {error && <p className="repo-settings__error">{error}</p>}
        </div>

        {confirmingRemove ? (
          <footer className="repo-settings__footer repo-settings__footer--confirm">
            <p className="repo-settings__danger-text">
              Remove <strong>{repo.name}</strong> and its {taskLabel}? This detaches the
              repository from La Vigie and removes the worktrees it created. Your original
              checkout at <code>{repo.path}</code> is left untouched.
            </p>
            <div className="repo-settings__footer-right">
              <button
                type="button"
                className="btn btn--ghost"
                onClick={() => setConfirmingRemove(false)}
                disabled={removing}
              >
                Cancel
              </button>
              <button
                type="button"
                className="btn btn--danger"
                onClick={remove}
                disabled={removing}
              >
                Remove
              </button>
            </div>
          </footer>
        ) : (
          <footer className="repo-settings__footer">
            <button
              type="button"
              className="repo-settings__remove"
              onClick={() => setConfirmingRemove(true)}
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" aria-hidden="true">
                <path
                  d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2m2 0v12a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V7"
                  stroke="currentColor"
                  strokeWidth="1.8"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
              Remove repository
            </button>
            <div className="repo-settings__footer-right">
              <button type="button" className="btn btn--ghost" onClick={onClose}>
                Cancel
              </button>
              <button
                type="button"
                className="btn btn--primary"
                onClick={save}
                disabled={saving}
              >
                Save
              </button>
            </div>
          </footer>
        )}
      </div>
    </div>
  );
}
