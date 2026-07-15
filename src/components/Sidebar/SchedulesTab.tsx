import { useEffect, useState } from "react";
import {
  listSchedules,
  createSchedule,
  createOneShotSchedule,
  updateSchedule,
  deleteSchedule,
  setScheduleEnabled,
  previewNextRun,
  type Schedule,
} from "../../api";

function formatNextRun(ts: number | null): string {
  if (ts === null) return "—";
  return new Date(ts * 1000).toLocaleString();
}

export function SchedulesTab({
  repoId,
  defaultBranch: _defaultBranch,
}: {
  repoId: string;
  defaultBranch: string;
}) {
  const [schedules, setSchedules] = useState<Schedule[]>([]);
  const [name, setName] = useState("");
  const [prompt, setPrompt] = useState("");
  const [cron, setCron] = useState("0 7 * * 1");
  const [preview, setPreview] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [editing, setEditing] = useState<Schedule | null>(null);
  const [scheduleType, setScheduleType] = useState<"recurring" | "once">("recurring");
  const [inHours, setInHours] = useState("3");
  // TASK-181: skip prepending the repo's initial prompt when this schedule fires.
  // Defaults to true — a scheduled prompt is usually self-contained.
  const [skipRepoPrompt, setSkipRepoPrompt] = useState(true);

  const reload = () => {
    listSchedules(repoId)
      .then(setSchedules)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  };

  useEffect(reload, [repoId]);

  // Live next-run preview whenever the cron text changes.
  useEffect(() => {
    let cancelled = false;
    if (!cron.trim()) {
      setPreview(null);
      return;
    }
    previewNextRun(cron)
      .then((ts) => {
        if (!cancelled) setPreview(`Next run: ${formatNextRun(ts)}`);
      })
      .catch(() => {
        if (!cancelled) setPreview("Invalid cron expression");
      });
    return () => {
      cancelled = true;
    };
  }, [cron]);

  const resetForm = () => {
    setName("");
    setPrompt("");
    setEditing(null);
    setScheduleType("recurring");
    setInHours("3");
    setSkipRepoPrompt(true);
  };

  const startEdit = (s: Schedule) => {
    setEditing(s);
    setName(s.name);
    setPrompt(s.prompt);
    setCron(s.cron);
    setScheduleType("recurring");
    setSkipRepoPrompt(s.skipRepoPrompt);
    setError(null);
  };

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      if (editing) {
        await updateSchedule({
          id: editing.id,
          name,
          prompt,
          cron,
          agent: editing.agent,
          model: editing.model,
          baseBranch: editing.baseBranch,
          enabled: editing.enabled,
          skipRepoPrompt,
        });
      } else if (scheduleType === "once") {
        const hours = Number(inHours);
        if (!Number.isFinite(hours) || hours <= 0) {
          throw new Error("Enter a positive number of hours.");
        }
        await createOneShotSchedule({
          repoId, name, prompt, inSeconds: Math.round(hours * 3600), skipRepoPrompt,
        });
      } else {
        await createSchedule({ repoId, name, prompt, cron, skipRepoPrompt });
      }
      resetForm();
      reload();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const toggle = async (s: Schedule) => {
    try {
      await setScheduleEnabled(s.id, !s.enabled);
      reload();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const remove = async (s: Schedule) => {
    try {
      await deleteSchedule(s.id);
      reload();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="schedules">
      <ul className="schedules__list">
        {schedules.length === 0 && (
          <li className="schedules__empty">No schedules yet.</li>
        )}
        {schedules.map((s) => (
          <li key={s.id} className="schedules__item">
            <div className="schedules__meta">
              <span className="schedules__name">{s.name}</span>
              {s.oneShot ? (
                <span className="schedules__cron">
                  {s.enabled ? "One-time" : "One-time · ran"}
                </span>
              ) : (
                <span className="schedules__cron mono">{s.cron}</span>
              )}
              <span className="schedules__next">
                {s.oneShot ? "Fires" : "Next"}: {formatNextRun(s.nextRunAt)}
              </span>
            </div>
            <div className="schedules__actions">
              <label className="schedules__toggle">
                <input
                  type="checkbox"
                  role="switch"
                  checked={s.enabled}
                  onChange={() => toggle(s)}
                  aria-label={`Enable ${s.name}`}
                />
                <span>{s.enabled ? "On" : "Off"}</span>
              </label>
              {!s.oneShot && (
                <button
                  type="button"
                  className="repo-settings__link"
                  aria-label={`Edit ${s.name}`}
                  onClick={() => startEdit(s)}
                >
                  Edit
                </button>
              )}
              <button
                type="button"
                className="repo-settings__link"
                aria-label={`Delete ${s.name}`}
                onClick={() => remove(s)}
              >
                Delete
              </button>
            </div>
          </li>
        ))}
      </ul>

      <div className="repo-settings__divider" />

      <div className="schedules__form">
        <label className="repo-settings__field">
          <span className="repo-settings__label">Schedule name</span>
          <input
            className="field"
            aria-label="Schedule name"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </label>
        <label className="repo-settings__field">
          <span className="repo-settings__label">Prompt</span>
          <textarea
            className="field"
            aria-label="Prompt"
            rows={2}
            placeholder="e.g. /security-scan"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
          />
        </label>
        {!editing && (
          <div className="repo-settings__field" role="radiogroup" aria-label="Schedule type">
            <label>
              <input
                type="radio"
                name="schedule-type"
                checked={scheduleType === "recurring"}
                onChange={() => setScheduleType("recurring")}
                aria-label="Recurring"
              />
              <span>Recurring</span>
            </label>
            <label>
              <input
                type="radio"
                name="schedule-type"
                checked={scheduleType === "once"}
                onChange={() => setScheduleType("once")}
                aria-label="One-time"
              />
              <span>One-time</span>
            </label>
          </div>
        )}
        {scheduleType === "recurring" || editing ? (
          <label className="repo-settings__field">
            <span className="repo-settings__label">Cron</span>
            <input
              className="field mono"
              aria-label="Cron"
              value={cron}
              onChange={(e) => setCron(e.target.value)}
            />
            <span className="repo-settings__hint">
              {preview ?? "Standard 5-field cron, local time (e.g. 0 7 * * 1 = Mondays 07:00)."}
            </span>
          </label>
        ) : (
          <label className="repo-settings__field">
            <span className="repo-settings__label">In hours</span>
            <input
              className="field"
              type="number"
              min="0"
              step="0.5"
              aria-label="In hours"
              value={inHours}
              onChange={(e) => setInHours(e.target.value)}
            />
            <span className="repo-settings__hint">
              {(() => {
                const h = Number(inHours);
                if (!Number.isFinite(h) || h <= 0) return "Enter a positive number of hours.";
                return `Fires at ${new Date(Date.now() + h * 3600 * 1000).toLocaleString()} (once).`;
              })()}
            </span>
          </label>
        )}
        <label className="schedules__skip">
          <input
            type="checkbox"
            checked={skipRepoPrompt}
            onChange={(e) => setSkipRepoPrompt(e.target.checked)}
            aria-label="Skip repository prompt"
          />
          <span>Skip repository prompt</span>
          <span className="repo-settings__hint">
            When on, the repo's initial prompt isn't prepended at fire time — the
            schedule prompt runs on its own.
          </span>
        </label>
        {error && <p className="repo-settings__error">{error}</p>}
        <div className="schedules__form-actions">
          <button
            type="button"
            className="btn btn--primary"
            onClick={save}
            disabled={saving || !name.trim() || !prompt.trim()}
          >
            {editing ? "Save changes" : "Add schedule"}
          </button>
          {editing && (
            <button type="button" className="btn" onClick={resetForm} disabled={saving}>
              Cancel
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
