import { beforeEach, describe, expect, it } from "vitest";
import { useVigieStore, type Task } from "./index";

function sampleTask(id: string, title: string): Task {
  return {
    id,
    repoId: "repo-1",
    title,
    worktreePath: `/wt/${id}`,
    branch: `task/${id}`,
    baseBranch: "main",
    status: "idle",
    createdAt: 0,
    updatedAt: 0,
    inPlace: false,
  };
}

describe("setTaskTitle (TASK-40)", () => {
  beforeEach(() => {
    useVigieStore.setState({ tasks: [sampleTask("t1", "old"), sampleTask("t2", "other")] });
  });

  it("patches only the targeted task's title, leaving others untouched", () => {
    useVigieStore.getState().setTaskTitle("t1", "renamed by agent");
    const tasks = useVigieStore.getState().tasks;
    expect(tasks.find((t) => t.id === "t1")?.title).toBe("renamed by agent");
    expect(tasks.find((t) => t.id === "t2")?.title).toBe("other");
  });

  it("is a no-op for an unknown task id", () => {
    useVigieStore.getState().setTaskTitle("nope", "ghost");
    expect(useVigieStore.getState().tasks.map((t) => t.title)).toEqual(["old", "other"]);
  });
});
