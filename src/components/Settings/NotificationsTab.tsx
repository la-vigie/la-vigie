import { useState } from "react";
import { deleteCustomSound } from "../../api";
import { useVigieStore } from "../../store";
import { SOUND_PALETTE, SOUND_EVENTS, soundLabel, type SoundEvent } from "../../sound/types";
import { SoundPlayer } from "../../sound/player";
import { pickAndImportSound } from "../../sound/import";

// cooldownMs: 0 so repeated preview clicks aren't swallowed by the debounce.
const previewPlayer = new SoundPlayer({ cooldownMs: 0 });

export function NotificationsTab() {
  const soundSettings = useVigieStore((s) => s.soundSettings);
  const setSoundSettings = useVigieStore((s) => s.setSoundSettings);
  const customSounds = useVigieStore((s) => s.customSounds);
  const refreshCustomSounds = useVigieStore((s) => s.refreshCustomSounds);
  const [soundError, setSoundError] = useState<string | null>(null);

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

  return (
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
  );
}
