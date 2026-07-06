import { describe, expect, it } from "vitest";
import { formatNotification } from "./format";
import type { Task, Repo } from "../store";

const baseTask: Task = {
  id: "t1",
  repoId: "r1",
  title: "Fix login flow",
  worktreePath: "/tmp/wt/t1",
  branch: "ac2-87",
  baseBranch: "main",
  status: "idle",
  createdAt: 1,
  updatedAt: 1,
  ticketKey: "AC2-87",
};
const repo: Repo = { id: "r1", name: "agents-desktop", path: "/r", defaultBranch: "main" };

describe("formatNotification", () => {
  it("titled + keyed task: 'KEY · title' and 'label — repo/branch'", () => {
    expect(formatNotification(baseTask, repo, "awaitingInput")).toEqual({
      title: "AC2-87 · Fix login flow",
      body: "Awaiting input — agents-desktop/ac2-87",
    });
  });

  it("no ticket key: title only, completed label", () => {
    expect(formatNotification({ ...baseTask, ticketKey: null }, repo, "completed")).toEqual({
      title: "Fix login flow",
      body: "Completed — agents-desktop/ac2-87",
    });
  });

  it("key-only task (empty title): no ' · ' duplication", () => {
    expect(formatNotification({ ...baseTask, title: "" }, repo, "failed")).toEqual({
      title: "AC2-87",
      body: "Failed — agents-desktop/ac2-87",
    });
  });

  it("unknown repo: body is the bare state label", () => {
    expect(formatNotification({ ...baseTask, ticketKey: null }, undefined, "completed")).toEqual({
      title: "Fix login flow",
      body: "Completed",
    });
  });
});
