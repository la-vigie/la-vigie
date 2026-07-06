import { describe, it, expect } from "vitest";
import { resolveSound } from "./resolve";
import { DEFAULT_SOUND_SETTINGS, knownSoundIds, type RepoSoundOverride } from "./types";

const app = DEFAULT_SOUND_SETTINGS;

describe("resolveSound", () => {
  it("plays the app default sound when no override", () => {
    expect(resolveSound(app, undefined, "completed")).toEqual({
      play: true,
      sound: "jobs-done",
    });
  });

  it("app master mute silences everything", () => {
    expect(resolveSound({ ...app, muted: true }, undefined, "failed")).toEqual({
      play: false,
    });
  });

  it("app per-event disable silences that event only", () => {
    const a = { ...app, events: { ...app.events, completed: { enabled: false, sound: "jobs-done" } } };
    expect(resolveSound(a, undefined, "completed")).toEqual({ play: false });
    expect(resolveSound(a, undefined, "failed")).toEqual({ play: true, sound: "error" });
  });

  it("repo force-mute overrides an unmuted app default", () => {
    const repo: RepoSoundOverride = { muted: true };
    expect(resolveSound(app, repo, "completed")).toEqual({ play: false });
  });

  it("repo inherits sound but overrides one event's sound", () => {
    const repo: RepoSoundOverride = { events: { failed: { sound: "ready-work" } } };
    expect(resolveSound(app, repo, "failed")).toEqual({ play: true, sound: "ready-work" });
    // unrelated event still inherits
    expect(resolveSound(app, repo, "completed")).toEqual({ play: true, sound: "jobs-done" });
  });

  it("repo enabled:false force-off while app is on", () => {
    const repo: RepoSoundOverride = { events: { completed: { enabled: false } } };
    expect(resolveSound(app, repo, "completed")).toEqual({ play: false });
  });

  it("repo muted:false force-on while app is muted", () => {
    const repo: RepoSoundOverride = { muted: false };
    expect(resolveSound({ ...app, muted: true }, repo, "completed")).toEqual({
      play: true,
      sound: "jobs-done",
    });
  });

  describe("automute (in a meeting)", () => {
    const muting = { ...app, automute: true };

    it("suppresses the sound but still notifies when automute is on and in a meeting", () => {
      // play:true (notification fires) but no sound — sound-only mute.
      expect(resolveSound(muting, undefined, "completed", undefined, true)).toEqual({
        play: true,
      });
    });

    it("plays normally when automute is on but not in a meeting", () => {
      expect(resolveSound(muting, undefined, "completed", undefined, false)).toEqual({
        play: true,
        sound: "jobs-done",
      });
    });

    it("plays in a meeting when automute is off (default)", () => {
      expect(resolveSound(app, undefined, "completed", undefined, true)).toEqual({
        play: true,
        sound: "jobs-done",
      });
    });

    it("repo can opt into automute while the app default is off", () => {
      const repo: RepoSoundOverride = { automute: true };
      expect(resolveSound(app, repo, "completed", undefined, true)).toEqual({ play: true });
    });

    it("repo can opt out of automute while the app default is on", () => {
      const repo: RepoSoundOverride = { automute: false };
      expect(resolveSound(muting, repo, "completed", undefined, true)).toEqual({
        play: true,
        sound: "jobs-done",
      });
    });

    it("master mute still wins over automute (no notification either)", () => {
      expect(resolveSound({ ...muting, muted: true }, undefined, "completed", undefined, true)).toEqual({
        play: false,
      });
    });
  });
});

it("falls back to the event default when the chosen id is unknown", () => {
  const app = {
    muted: false,
    automute: false,
    events: {
      completed: { enabled: true, sound: "custom:gone" },
      failed: { enabled: true, sound: "error" },
      awaitingInput: { enabled: true, sound: "ready-work" },
    },
  };
  const valid = knownSoundIds([]); // only bundled ids
  const res = resolveSound(app, undefined, "completed", valid);
  expect(res).toEqual({ play: true, sound: "jobs-done" });
});

it("keeps a valid custom id", () => {
  const app = {
    muted: false,
    automute: false,
    events: {
      completed: { enabled: true, sound: "custom:abc" },
      failed: { enabled: true, sound: "error" },
      awaitingInput: { enabled: true, sound: "ready-work" },
    },
  };
  const valid = knownSoundIds([{ id: "custom:abc", label: "Ding", ext: "mp3" }]);
  const res = resolveSound(app, undefined, "completed", valid);
  expect(res).toEqual({ play: true, sound: "custom:abc" });
});
