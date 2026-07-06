import { describe, it, expect } from "vitest";
import { parseRepoOverride, parseSoundSettings } from "./safe-parse";

describe("parseRepoOverride", () => {
  it("parses valid JSON and returns the object", () => {
    expect(parseRepoOverride('{"muted":true}')).toEqual({ muted: true });
  });

  it("returns {} for null", () => {
    expect(parseRepoOverride(null)).toEqual({});
  });

  it("returns {} for undefined", () => {
    expect(parseRepoOverride(undefined)).toEqual({});
  });

  it("returns {} for invalid JSON", () => {
    expect(parseRepoOverride("not-valid-json!!!")).toEqual({});
  });
});

describe("parseSoundSettings", () => {
  it("parses valid JSON and returns the object", () => {
    const input = JSON.stringify({ muted: true, events: { completed: { enabled: false, sound: "error" } } });
    expect(parseSoundSettings(input)).toEqual({
      muted: true,
      events: { completed: { enabled: false, sound: "error" } },
    });
  });

  it("returns undefined for null", () => {
    expect(parseSoundSettings(null)).toBeUndefined();
  });

  it("returns undefined for undefined", () => {
    expect(parseSoundSettings(undefined)).toBeUndefined();
  });

  it("returns undefined for invalid/garbage JSON", () => {
    expect(parseSoundSettings("not-valid-json!!!")).toBeUndefined();
  });
});
