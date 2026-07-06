import { describe, it, expect, beforeEach } from "vitest";
import { shellEscapePath, buildDropPayload, isWithinRect, resolveActiveBackendId } from "./fileDrop";
import { useVigieStore, AGENT_TAB } from "../../store";

describe("shellEscapePath", () => {
  it("backslash-escapes spaces and shell metacharacters", () => {
    expect(shellEscapePath("/a/b c.png")).toBe("/a/b\\ c.png");
    expect(shellEscapePath("/x/(y)&z.png")).toBe("/x/\\(y\\)\\&z.png");
  });
  it("leaves plain ascii and unicode letters untouched", () => {
    expect(shellEscapePath("/Users/me/café.png")).toBe("/Users/me/café.png");
  });
});

describe("buildDropPayload", () => {
  it("wraps shell-escaped, space-joined paths in bracketed paste", () => {
    expect(buildDropPayload(["/a/b c.png", "/d/e.png"])).toBe(
      "\x1b[200~/a/b\\ c.png /d/e.png\x1b[201~",
    );
  });
});

describe("isWithinRect", () => {
  const rect = { left: 10, top: 10, right: 110, bottom: 60 } as DOMRect;
  it("is true for a physical point inside after dpr conversion", () => {
    // dpr 2 → physical (100,40) maps to css (50,20), inside the rect
    expect(isWithinRect({ x: 100, y: 40 }, rect, 2)).toBe(true);
  });
  it("is false for a point outside after dpr conversion", () => {
    // dpr 2 → physical (240,40) maps to css (120,20), right of the rect
    expect(isWithinRect({ x: 240, y: 40 }, rect, 2)).toBe(false);
  });
});

describe("resolveActiveBackendId", () => {
  beforeEach(() => {
    useVigieStore.setState({
      selectedTaskId: "t1",
      activeTabByTask: { t1: AGENT_TAB },
      sessionsByTask: { t1: [{ localId: AGENT_TAB, kind: "agent", status: "running", backendId: "be-1", title: "Claude" }] },
    });
  });
  it("returns the active session's backendId", () => {
    expect(resolveActiveBackendId(useVigieStore.getState())).toBe("be-1");
  });
  it("returns undefined when no task is selected", () => {
    useVigieStore.setState({ selectedTaskId: null });
    expect(resolveActiveBackendId(useVigieStore.getState())).toBeUndefined();
  });
});
