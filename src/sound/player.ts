import { getSoundUrl } from "./source";
import type { CustomSound } from "./types";

interface SoundPlayerOpts {
  cooldownMs?: number;
  now?: () => number;
  play?: (url: string) => void;
}

/**
 * Plays palette + custom sounds with a single global cooldown so a burst of
 * agents finishing near-simultaneously produces one sound, not a wall of them.
 * The clock and the audio sink are injectable for testing; production uses
 * Date.now + HTMLAudioElement. Resolution of id → URL is delegated to
 * getSoundUrl (bundled asset vs cached blob URL).
 */
export class SoundPlayer {
  private readonly cooldownMs: number;
  private readonly now: () => number;
  private readonly play: (url: string) => void;
  private lastPlayedAt = -Infinity;

  constructor(opts: SoundPlayerOpts = {}) {
    this.cooldownMs = opts.cooldownMs ?? 1500;
    this.now = opts.now ?? (() => Date.now());
    this.play =
      opts.play ??
      ((url: string) => {
        void new Audio(url).play().catch(() => {});
      });
  }

  async playSound(soundId: string, custom: CustomSound[]): Promise<void> {
    // Gate on the cooldown synchronously before any async work.
    const t = this.now();
    if (t - this.lastPlayedAt < this.cooldownMs) return;
    this.lastPlayedAt = t;
    const url = await getSoundUrl(soundId, custom);
    if (url) this.play(url);
  }
}
