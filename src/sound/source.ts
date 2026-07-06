import { readSoundBytes } from "../api";
import { SOUND_PALETTE, type CustomSound } from "./types";

const MIME: Record<string, string> = {
  mp3: "audio/mpeg",
  wav: "audio/wav",
  ogg: "audio/ogg",
  m4a: "audio/mp4",
  aac: "audio/aac",
  flac: "audio/flac",
};

// Custom-sound id → object URL. Built once per id, reused for the session.
const blobCache = new Map<string, string>();

/** Test seam: drop cached blob URLs. */
export function resetSoundUrlCache(): void {
  for (const url of blobCache.values()) URL.revokeObjectURL(url);
  blobCache.clear();
}

/**
 * Resolve a sound id to a playable URL. Bundled ids map to their static
 * same-origin asset; custom ids are fetched over IPC once and cached as a
 * blob: URL. Returns null when the id matches neither (e.g. a deleted custom).
 */
export async function getSoundUrl(
  id: string,
  custom: CustomSound[],
): Promise<string | null> {
  const bundled = SOUND_PALETTE.find((s) => s.id === id);
  if (bundled) return bundled.file;

  const entry = custom.find((c) => c.id === id);
  if (!entry) return null;

  const cached = blobCache.get(id);
  if (cached) return cached;

  // Degrade to null rather than rejecting: the file may have been deleted from
  // disk while the registry still lists it. Not cached so a later retry can
  // succeed if the file is restored.
  let bytes: number[];
  try {
    bytes = await readSoundBytes(id);
  } catch {
    return null;
  }
  const blob = new Blob([new Uint8Array(bytes)], {
    type: MIME[entry.ext] ?? "audio/mpeg",
  });
  const url = URL.createObjectURL(blob);
  blobCache.set(id, url);
  return url;
}

/**
 * Load a URL into an Audio element and resolve true once it has enough data to
 * play, false on decode/format error. Used to validate a freshly imported file.
 */
export function decodeTest(url: string): Promise<boolean> {
  return new Promise((resolve) => {
    const audio = new Audio();
    const done = (ok: boolean) => {
      audio.oncanplaythrough = null;
      audio.onloadeddata = null;
      audio.onerror = null;
      resolve(ok);
    };
    audio.onloadeddata = () => done(true);
    audio.oncanplaythrough = () => done(true);
    audio.onerror = () => done(false);
    audio.src = url;
    audio.load();
  });
}
