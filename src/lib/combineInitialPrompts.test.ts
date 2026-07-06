import { describe, expect, it } from "vitest";
import { combineInitialPrompts } from "./combineInitialPrompts";

describe("combineInitialPrompts", () => {
  it("joins repo prompt then task prompt with a blank line", () => {
    expect(combineInitialPrompts("repo ctx", "do X")).toBe("repo ctx\n\ndo X");
  });
  it("returns the only non-empty part alone", () => {
    expect(combineInitialPrompts("repo ctx", "")).toBe("repo ctx");
    expect(combineInitialPrompts("  ", "do X")).toBe("do X");
  });
  it("trims each part", () => {
    expect(combineInitialPrompts("  repo  ", "  task  ")).toBe("repo\n\ntask");
  });
  it("returns undefined when both are empty/absent", () => {
    expect(combineInitialPrompts(null, null)).toBeUndefined();
    expect(combineInitialPrompts("", "   ")).toBeUndefined();
    expect(combineInitialPrompts()).toBeUndefined();
  });
});
