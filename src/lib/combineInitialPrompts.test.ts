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

  // TASK-160: skipping the repo-level prompt is expressed by passing null for the
  // repo arg. skip-on → task prompt only; skip-off → repo prompt still prepended.
  describe("skipping the repo prompt (TASK-160)", () => {
    it("skip-on: a null repo arg yields the task prompt alone", () => {
      expect(combineInitialPrompts(null, "do X")).toBe("do X");
    });
    it("skip-on with an empty task prompt yields undefined (bare agent)", () => {
      expect(combineInitialPrompts(null, "")).toBeUndefined();
    });
    it("skip-off: the repo prompt is still prepended", () => {
      expect(combineInitialPrompts("repo ctx", "do X")).toBe("repo ctx\n\ndo X");
    });
  });
});
