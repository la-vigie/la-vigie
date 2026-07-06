import type { RepoSoundOverride, SoundSettings } from "./types";

/**
 * Safely parse a raw JSON string from the store into a `RepoSoundOverride`.
 * Returns `{}` (inherit everything from the app default) when `json` is
 * null/undefined/empty or contains invalid JSON, so a corrupt value in the
 * database never blows up the UI.
 */
export function parseRepoOverride(json: string | null | undefined): RepoSoundOverride {
  if (!json) return {};
  try {
    return JSON.parse(json) as RepoSoundOverride;
  } catch {
    return {};
  }
}

/**
 * Safely parse the app-level sound settings JSON from the store.
 * Returns `undefined` when `json` is null/undefined/empty or invalid,
 * which the caller then treats as "use all defaults" (same as before).
 */
export function parseSoundSettings(
  json: string | null | undefined,
): Partial<SoundSettings> | undefined {
  if (!json) return undefined;
  try {
    return JSON.parse(json) as Partial<SoundSettings>;
  } catch {
    return undefined;
  }
}
