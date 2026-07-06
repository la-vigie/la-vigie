import { describe, it, expect } from "vitest";
import { soundLabel, knownSoundIds, type CustomSound } from "./types";

const custom: CustomSound[] = [{ id: "custom:abc", label: "My Ding", ext: "mp3" }];

describe("soundLabel", () => {
  it("returns the bundled label", () => {
    expect(soundLabel("jobs-done", custom)).toBe("Jobs done");
  });
  it("returns the custom label", () => {
    expect(soundLabel("custom:abc", custom)).toBe("My Ding");
  });
  it("returns undefined for an unknown id", () => {
    expect(soundLabel("custom:gone", custom)).toBeUndefined();
  });
});

describe("knownSoundIds", () => {
  it("includes bundled + custom ids", () => {
    const ids = knownSoundIds(custom);
    expect(ids.has("jobs-done")).toBe(true);
    expect(ids.has("custom:abc")).toBe(true);
    expect(ids.has("custom:gone")).toBe(false);
  });
});
