import { DEFAULT_SOUND_SETTINGS } from "./types";
import type { SoundSettings, RepoSoundOverride, SoundEvent } from "./types";

/**
 * Resolve whether a sound should play and which one, per-knob: repo value wins
 * when explicitly set (incl. `false`), else the app default. When `validIds` is
 * supplied and the chosen sound is not among them (e.g. a deleted custom sound),
 * fall back to the event's default bundled sound so playback is never silent.
 *
 * `inMeeting` (AC2-105): when automute is on for this repo/app and the user is
 * in a meeting, suppress the *sound only* — return `{ play: true }` with no
 * `sound`, so the caller still fires the OS notification but stays silent.
 */
export function resolveSound(
  app: SoundSettings,
  repo: RepoSoundOverride | undefined,
  event: SoundEvent,
  validIds?: Set<string>,
  inMeeting = false,
): { play: boolean; sound?: string } {
  const mutedEff = repo?.muted ?? app.muted;
  if (mutedEff) return { play: false };

  const appEvent = app.events[event];
  const repoEvent = repo?.events?.[event];

  const enabledEff = repoEvent?.enabled ?? appEvent.enabled;
  if (!enabledEff) return { play: false };

  // Automute: notify but stay silent while in a meeting.
  const automuteEff = repo?.automute ?? app.automute;
  if (automuteEff && inMeeting) return { play: true };

  let soundEff = repoEvent?.sound ?? appEvent.sound;
  if (validIds && !validIds.has(soundEff)) {
    soundEff = DEFAULT_SOUND_SETTINGS.events[event].sound;
  }
  return { play: true, sound: soundEff };
}
