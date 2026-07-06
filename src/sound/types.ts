export type SoundEvent = "completed" | "failed" | "awaitingInput";

export interface EventSetting {
  enabled: boolean;
  sound: string; // a SOUND_PALETTE id
}

export interface SoundSettings {
  muted: boolean;
  /**
   * When true, suppress notification *sounds* while the user is in a meeting
   * (mic/camera capturing anywhere on the system). Off by default. Visual OS
   * notifications still fire — this mutes sound only. macOS-only (AC2-105).
   */
  automute: boolean;
  events: Record<SoundEvent, EventSetting>;
}

export interface RepoSoundOverride {
  muted?: boolean;
  /** Per-repo override of the app `automute` default; undefined = inherit. */
  automute?: boolean;
  events?: Partial<Record<SoundEvent, { enabled?: boolean; sound?: string }>>;
}

export const SOUND_PALETTE: { id: string; label: string; file: string }[] = [
  { id: "jobs-done", label: "Jobs done", file: "/sounds/jobs-done.mp3" },
  { id: "ready-work", label: "Ready / attention", file: "/sounds/ready-work.mp3" },
  { id: "error", label: "Error", file: "/sounds/error.mp3" },
];

export const DEFAULT_SOUND_SETTINGS: SoundSettings = {
  muted: false,
  automute: false,
  events: {
    completed: { enabled: true, sound: "jobs-done" },
    failed: { enabled: true, sound: "error" },
    awaitingInput: { enabled: true, sound: "ready-work" },
  },
};

export const SOUND_EVENTS: { key: SoundEvent; label: string }[] = [
  { key: "completed", label: "Completed" },
  { key: "failed", label: "Failed" },
  { key: "awaitingInput", label: "Awaiting input" },
];

export const ALLOWED_SOUND_EXTS = ["mp3", "wav", "ogg", "m4a", "aac", "flac"] as const;

export interface CustomSound {
  id: string;
  label: string;
  ext: string;
}

/** Display label for a sound id (bundled or custom); undefined if unknown. */
export function soundLabel(id: string, custom: CustomSound[]): string | undefined {
  const bundled = SOUND_PALETTE.find((s) => s.id === id);
  if (bundled) return bundled.label;
  return custom.find((c) => c.id === id)?.label;
}

/** Set of all selectable sound ids (bundled palette + current custom library). */
export function knownSoundIds(custom: CustomSound[]): Set<string> {
  return new Set<string>([...SOUND_PALETTE.map((s) => s.id), ...custom.map((c) => c.id)]);
}

// AgentActivity (store) → SoundEvent. "working" has no terminal sound.
export function activityToEvent(activity: string): SoundEvent | undefined {
  switch (activity) {
    case "idle":
      return "completed";
    case "error":
      return "failed";
    case "needs_attention":
      return "awaitingInput";
    default:
      return undefined;
  }
}
