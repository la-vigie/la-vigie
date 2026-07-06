import { describe, expect, it } from "vitest";
import { runStateView } from "./runState";

describe("runStateView lifecycle", () => {
  it("running lifecycle agent reads neutral Running regardless of activity", () => {
    expect(runStateView("running", undefined, true)).toEqual({ label: "Running", modifier: "running" });
    // even a stray activity does not turn into needs-input for a lifecycle agent
    expect(runStateView("running", "needs_attention", true)).toEqual({ label: "Running", modifier: "running" });
  });

  it("starting lifecycle still shows Starting…", () => {
    expect(runStateView("starting", undefined, true)).toEqual({ label: "Starting…", modifier: "starting" });
  });

  it("exited returns null", () => {
    expect(runStateView("exited", undefined, true)).toBeNull();
  });

  it("non-lifecycle (Claude) behavior is unchanged", () => {
    expect(runStateView("running", "working")).toEqual({ label: "Working…", modifier: "working" });
    expect(runStateView("running", undefined)).toEqual({ label: "Ready", modifier: "idle" });
  });
});

describe("runStateView", () => {
  it("returns null for an exited agent (no pill)", () => {
    expect(runStateView("exited")).toBeNull();
  });

  it("shows a Starting… state while spawning", () => {
    expect(runStateView("starting")).toEqual({
      label: "Starting…",
      modifier: "starting",
    });
  });

  it("shows a Working… state for a running agent that is working", () => {
    expect(runStateView("running", "working")).toEqual({
      label: "Working…",
      modifier: "working",
    });
  });

  it("reads Ready (not Working) for a freshly running agent with no activity yet", () => {
    // No hook has fired yet — the agent is alive but idle at its prompt, so it
    // must not read Working until an explicit "working" signal.
    expect(runStateView("running")).toEqual({
      label: "Ready",
      modifier: "idle",
    });
  });

  it("shows a quiet Ready state when the agent is idle (awaiting you)", () => {
    expect(runStateView("running", "idle")).toEqual({
      label: "Ready",
      modifier: "idle",
    });
  });

  it("shows Needs input when the agent needs attention", () => {
    expect(runStateView("running", "needs_attention")).toEqual({
      label: "Needs input",
      modifier: "attention",
    });
  });

  it("shows Error when the agent reported an error", () => {
    expect(runStateView("running", "error")).toEqual({
      label: "Error",
      modifier: "error",
    });
  });
});
