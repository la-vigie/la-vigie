import { describe, it, expect, vi } from "vitest";

// Mock ../api so getSoundUrl (called by player) never touches IPC in tests.
vi.mock("../api", () => ({ readSoundBytes: vi.fn() }));

import { SoundPlayer } from "./player";

function makePlayer(now: { t: number }) {
  const played: string[] = [];
  const player = new SoundPlayer({
    cooldownMs: 1500,
    now: () => now.t,
    play: (url) => played.push(url),
  });
  return { player, played };
}

describe("SoundPlayer", () => {
  it("plays the palette file URL for a sound id", async () => {
    const now = { t: 0 };
    const { player, played } = makePlayer(now);
    await player.playSound("jobs-done", []);
    expect(played).toEqual(["/sounds/jobs-done.mp3"]);
  });

  it("suppresses a second sound within the cooldown window", async () => {
    const now = { t: 0 };
    const { player, played } = makePlayer(now);
    await player.playSound("jobs-done", []);
    now.t = 500;
    await player.playSound("error", []); // within 1500ms → dropped
    expect(played).toEqual(["/sounds/jobs-done.mp3"]);
  });

  it("plays again after the cooldown elapses", async () => {
    const now = { t: 0 };
    const { player, played } = makePlayer(now);
    await player.playSound("jobs-done", []);
    now.t = 1600;
    await player.playSound("error", []);
    expect(played).toEqual(["/sounds/jobs-done.mp3", "/sounds/error.mp3"]);
  });

  it("ignores an unknown sound id without throwing or playing", async () => {
    const now = { t: 0 };
    const { player, played } = makePlayer(now);
    // Unknown id: cooldown fires (synchronous gate), URL resolves to null, nothing plays.
    await player.playSound("nope", []);
    expect(played).toEqual([]);
    // After cooldown elapses, a known id plays normally.
    now.t = 1600;
    await player.playSound("jobs-done", []);
    expect(played).toEqual(["/sounds/jobs-done.mp3"]);
  });
});
