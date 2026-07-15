import { render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Capture the registered task_launched callback so the test can fire it.
let launchedCb: ((e: { taskId: string; initialPrompt?: string | null; skipRepoPrompt?: boolean }) => void) | undefined;
vi.mock("../api", () => ({
  onTaskLaunched: (cb: (e: { taskId: string; initialPrompt?: string | null; skipRepoPrompt?: boolean }) => void) => {
    launchedCb = cb;
    return Promise.resolve(() => {});
  },
}));

const refresh = vi.fn().mockResolvedValue(undefined);
const setSelectedTask = vi.fn();
const startAgentSession = vi.fn();

vi.mock("../store", () => ({
  useVigieStore: Object.assign(
    (selector: (s: unknown) => unknown) =>
      selector({
        refresh,
        setSelectedTask,
        startAgentSession,
        tasks: [{ id: "task-b", repoId: "repo-1" }],
        repos: [{ id: "repo-1", initialPrompt: "/worktree-init" }],
      }),
    {
      getState: () => ({
        refresh,
        setSelectedTask,
        startAgentSession,
        tasks: [{ id: "task-b", repoId: "repo-1" }],
        repos: [{ id: "repo-1", initialPrompt: "/worktree-init" }],
      }),
    },
  ),
}));

import { useTaskLaunch } from "./useTaskLaunch";

function Harness() {
  useTaskLaunch();
  return null;
}

describe("useTaskLaunch", () => {
  beforeEach(() => {
    launchedCb = undefined;
    refresh.mockClear();
    setSelectedTask.mockClear();
    startAgentSession.mockClear();
  });

  it("refreshes, selects, and starts the agent for a launched task", async () => {
    render(<Harness />);
    await waitFor(() => expect(launchedCb).toBeDefined());

    await launchedCb!({ taskId: "task-b" });

    await waitFor(() => expect(refresh).toHaveBeenCalled());
    expect(setSelectedTask).toHaveBeenCalledWith("task-b");
    expect(startAgentSession).toHaveBeenCalledWith(
      "task-b",
      false,
      undefined,
      "/worktree-init",
    );
  });

  it("combines a caller-supplied prompt with the repo prompt", async () => {
    render(<Harness />);
    await waitFor(() => expect(launchedCb).toBeDefined());

    await launchedCb!({ taskId: "task-b", initialPrompt: "do the thing" });

    await waitFor(() => expect(startAgentSession).toHaveBeenCalled());
    expect(startAgentSession).toHaveBeenCalledWith(
      "task-b",
      false,
      undefined,
      "/worktree-init\n\ndo the thing",
    );
  });

  it("skips the repo prompt when skipRepoPrompt is set (TASK-181)", async () => {
    render(<Harness />);
    await waitFor(() => expect(launchedCb).toBeDefined());

    await launchedCb!({ taskId: "task-b", initialPrompt: "do the thing", skipRepoPrompt: true });

    await waitFor(() => expect(startAgentSession).toHaveBeenCalled());
    // Repo prompt "/worktree-init" is dropped; only the schedule prompt survives.
    expect(startAgentSession).toHaveBeenCalledWith("task-b", false, undefined, "do the thing");
  });
});
