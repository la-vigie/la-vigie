import { describe, expect, it } from "vitest";
import { buildSegments } from "./StatusBanner";
import type { Task } from "../../store";

const task = { id: "t1", prNumber: null } as unknown as Task;

describe("buildSegments", () => {
  it("derives ctx, model, and mode from real console data", () => {
    const keys = buildSegments(task, { model: "Opus", contextRemainingPercent: 73, mode: "auto" }).map((s) => s.key);
    expect(keys).toEqual(["ctx", "model", "mode"]);
  });

  it("omits ctx when context is unknown and model when absent", () => {
    expect(buildSegments(task, {}).map((s) => s.key)).toEqual([]);
  });

  it("omits the mode segment for default mode", () => {
    expect(buildSegments(task, { mode: "default" }).map((s) => s.key)).toEqual([]);
  });

  it("includes the PR segment when present", () => {
    const t = { id: "t1", prNumber: 7 } as unknown as Task;
    expect(buildSegments(t, {}).map((s) => s.key)).toEqual(["pr"]);
  });
});
