import { describe, expect, it } from "vitest";
import { formatNotification } from "./format";
import type { Task, Repo } from "../store";

const baseTask: Task = {
  id: "t1",
  repoId: "r1",
  title: "Fix login flow",
  worktreePath: "/tmp/wt/t1",
  branch: "task-87",
  baseBranch: "main",
  status: "idle",
  createdAt: 1,
  updatedAt: 1,
  ticketKey: "TASK-87",
  inPlace: false,
};
const repo: Repo = { id: "r1", name: "my-repo", path: "/r", defaultBranch: "main", inPlaceDefault: false };

describe("formatNotification", () => {
  it("titled + keyed task: 'KEY · title' and 'label — repo/branch'", () => {
    expect(formatNotification(baseTask, repo, "awaitingInput")).toEqual({
      title: "TASK-87 · Fix login flow",
      body: "Awaiting input — my-repo/task-87",
    });
  });

  it("no ticket key: title only, completed label", () => {
    expect(formatNotification({ ...baseTask, ticketKey: null }, repo, "completed")).toEqual({
      title: "Fix login flow",
      body: "Completed — my-repo/task-87",
    });
  });

  it("key-only task (empty title): no ' · ' duplication", () => {
    expect(formatNotification({ ...baseTask, title: "" }, repo, "failed")).toEqual({
      title: "TASK-87",
      body: "Failed — my-repo/task-87",
    });
  });

  it("unknown repo: body is the bare state label", () => {
    expect(formatNotification({ ...baseTask, ticketKey: null }, undefined, "completed")).toEqual({
      title: "Fix login flow",
      body: "Completed",
    });
  });

  it("failed with a reason: appends the reason after the label", () => {
    expect(formatNotification(baseTask, repo, "failed", "Rate limited by the model API.")).toEqual({
      title: "TASK-87 · Fix login flow",
      body: "Failed — my-repo/task-87: Rate limited by the model API.",
    });
  });

  it("reason with an unknown repo: appended to the bare label", () => {
    expect(formatNotification({ ...baseTask, ticketKey: null }, undefined, "failed", "Billing error.")).toEqual({
      title: "Fix login flow",
      body: "Failed: Billing error.",
    });
  });

  it("collapses whitespace and truncates a very long reason", () => {
    const long = "x".repeat(200);
    const { body } = formatNotification(baseTask, repo, "failed", `line1\n  line2  ${long}`);
    // One line, capped at 120 chars of reason with an ellipsis.
    expect(body).not.toContain("\n");
    const reason = body.split(": ")[1];
    expect(reason.length).toBe(120);
    expect(reason.endsWith("…")).toBe(true);
  });

  it("empty/whitespace reason is ignored (no trailing colon)", () => {
    expect(formatNotification(baseTask, repo, "failed", "   ").body).toBe("Failed — my-repo/task-87");
    expect(formatNotification(baseTask, repo, "failed", null).body).toBe("Failed — my-repo/task-87");
  });
});
