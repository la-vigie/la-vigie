import { describe, expect, it } from "vitest";
import { taskName } from "./taskName";
import type { Task } from "../store";

function task(partial: Partial<Task>): Task {
  return {
    id: "t1",
    repoId: "r1",
    title: "",
    worktreePath: "/wt",
    branch: "b",
    baseBranch: "main",
    status: "idle",
    createdAt: 1,
    updatedAt: 1,
    ...partial,
  } as Task;
}

describe("taskName", () => {
  it("uses the title when present", () => {
    expect(taskName(task({ title: "Fix login", ticketKey: "TST-1" }))).toBe("Fix login");
  });

  it("falls back to the ticket key when the title is empty", () => {
    expect(taskName(task({ title: "   ", ticketKey: "TST-1" }))).toBe("TST-1");
  });

  it("returns an empty string when neither is present", () => {
    expect(taskName(task({ title: "", ticketKey: null }))).toBe("");
  });
});
