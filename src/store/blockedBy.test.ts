import { describe, it, expect, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { useVigieStore } from "./index";

describe("store ingestion — blockedBy (TASK-177)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("attaches each task's blockedBy list from the snapshot map", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_state") {
        return Promise.resolve({
          repos: [],
          tasks: [
            { id: "waiter", repoId: "r1", title: "W", worktreePath: "", branch: "", baseBranch: "main", status: "pending", createdAt: 0, updatedAt: 0 },
            { id: "solo", repoId: "r1", title: "S", worktreePath: "/w", branch: "b", baseBranch: "main", status: "idle", createdAt: 0, updatedAt: 0 },
          ],
          worktreesRoot: "/wt",
          blockedBy: { waiter: [{ taskId: "b1", title: "Build UI", status: "idle" }] },
        });
      }
      return Promise.resolve([]);
    });

    await useVigieStore.getState().refresh();
    const tasks = useVigieStore.getState().tasks;
    const waiter = tasks.find((t) => t.id === "waiter");
    const solo = tasks.find((t) => t.id === "solo");
    expect(waiter?.blockedBy).toEqual([{ taskId: "b1", title: "Build UI", status: "idle" }]);
    expect(solo?.blockedBy).toEqual([]);
  });
});
