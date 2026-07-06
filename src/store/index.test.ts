import { beforeEach, describe, expect, it, vi } from "vitest";
import { useVigieStore, AGENT_TAB } from "./index";
import type { Repo, Task } from "./index";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

const sampleRepo: Repo = {
  id: "repo-1",
  name: "my-repo",
  path: "/tmp/my-repo",
  defaultBranch: "main",
};

const sampleTask: Task = {
  id: "task-1",
  repoId: "repo-1",
  title: "Do the thing",
  worktreePath: "/tmp/my-repo-worktrees/task-1",
  branch: "do-the-thing",
  baseBranch: "main",
  status: "idle",
  createdAt: 1_700_000_000,
  updatedAt: 1_700_000_000,
};

describe("useVigieStore", () => {
  beforeEach(() => {
    useVigieStore.setState({
      repos: [],
      tasks: [],
      selectedTaskId: null,
      sessionsByTask: {},
      activeTabByTask: {},
      attentionByTask: {},
    });
    invokeMock.mockReset();
  });

  it("has empty repos, empty tasks, and no selected task initially", () => {
    const state = useVigieStore.getState();

    expect(state.repos).toEqual([]);
    expect(state.tasks).toEqual([]);
    expect(state.selectedTaskId).toBeNull();
  });

  it("updates selectedTaskId when setSelectedTask is called", () => {
    useVigieStore.getState().setSelectedTask("task-1");

    expect(useVigieStore.getState().selectedTaskId).toBe("task-1");
  });

  it("setRepos and setTasks replace the corresponding slices", () => {
    useVigieStore.getState().setRepos([sampleRepo]);
    useVigieStore.getState().setTasks([sampleTask]);

    expect(useVigieStore.getState().repos).toEqual([sampleRepo]);
    expect(useVigieStore.getState().tasks).toEqual([sampleTask]);
  });

  it("refresh() populates repos and tasks from list_state", async () => {
    invokeMock.mockResolvedValueOnce({
      repos: [sampleRepo],
      tasks: [sampleTask],
    });

    await useVigieStore.getState().refresh();

    expect(invokeMock).toHaveBeenCalledWith("list_state");
    expect(useVigieStore.getState().repos).toEqual([sampleRepo]);
    expect(useVigieStore.getState().tasks).toEqual([sampleTask]);
  });

  describe("removeRepo (AC2-69)", () => {
    const repo2: Repo = { ...sampleRepo, id: "repo-2", name: "other" };
    const task2: Task = { ...sampleTask, id: "task-2", repoId: "repo-2" };

    it("invokes remove_repo then refreshes from list_state", async () => {
      useVigieStore.setState({ repos: [sampleRepo, repo2], tasks: [sampleTask, task2] });
      // 1st call: remove_repo (void). 2nd call: refresh -> list_state snapshot.
      invokeMock
        .mockResolvedValueOnce(undefined)
        .mockResolvedValueOnce({ repos: [repo2], tasks: [task2] });

      await useVigieStore.getState().removeRepo("repo-1");

      expect(invokeMock).toHaveBeenNthCalledWith(1, "remove_repo", { repoId: "repo-1" });
      expect(invokeMock).toHaveBeenNthCalledWith(2, "list_state");
      expect(useVigieStore.getState().repos).toEqual([repo2]);
      expect(useVigieStore.getState().tasks).toEqual([task2]);
    });

    it("clears the selection when the selected task belonged to the removed repo", async () => {
      useVigieStore.setState({ repos: [sampleRepo], tasks: [sampleTask], selectedTaskId: "task-1" });
      invokeMock
        .mockResolvedValueOnce(undefined)
        .mockResolvedValueOnce({ repos: [], tasks: [] });

      await useVigieStore.getState().removeRepo("repo-1");

      expect(useVigieStore.getState().selectedTaskId).toBeNull();
    });

    it("keeps the selection when it belongs to another repo", async () => {
      useVigieStore.setState({ repos: [sampleRepo, repo2], tasks: [sampleTask, task2], selectedTaskId: "task-2" });
      invokeMock
        .mockResolvedValueOnce(undefined)
        .mockResolvedValueOnce({ repos: [repo2], tasks: [task2] });

      await useVigieStore.getState().removeRepo("repo-1");

      expect(useVigieStore.getState().selectedTaskId).toBe("task-2");
    });

    it("drops sessions for the removed repo's tasks", async () => {
      useVigieStore.setState({ repos: [sampleRepo], tasks: [sampleTask] });
      useVigieStore.getState().startAgentSession("task-1", false);
      invokeMock
        .mockResolvedValueOnce(undefined)
        .mockResolvedValueOnce({ repos: [], tasks: [] });

      await useVigieStore.getState().removeRepo("repo-1");

      expect(useVigieStore.getState().sessionsByTask["task-1"]).toBeUndefined();
    });
  });

  it("starts with empty sessionsByTask", () => {
    expect(useVigieStore.getState().sessionsByTask).toEqual({});
  });

  it("startAgentSession creates a starting agent session with the given resume flag", () => {
    useVigieStore.getState().startAgentSession("task-1", true);

    const session = useVigieStore.getState().sessionsByTask["task-1"]?.find((s) => s.localId === AGENT_TAB);
    expect(session).toMatchObject({
      localId: AGENT_TAB,
      kind: "agent",
      status: "starting",
      resume: true,
    });
  });

  it("setSessionInfo merges partial info into the existing session", () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    useVigieStore.getState().setSessionInfo("task-1", AGENT_TAB, { backendId: "agent-1", status: "running" });

    const session = useVigieStore.getState().sessionsByTask["task-1"]?.find((s) => s.localId === AGENT_TAB);
    expect(session).toMatchObject({
      status: "running",
      resume: false,
      backendId: "agent-1",
    });
  });

  it("removeAgentSession removes the agent session for the task", () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    useVigieStore.getState().removeAgentSession("task-1");

    expect(useVigieStore.getState().sessionsByTask["task-1"]).toEqual([]);
  });

  it("setSessionActivity sets activity on the session whose backendId matches", () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    useVigieStore.getState().setSessionInfo("task-1", AGENT_TAB, { backendId: "agent-1", status: "running" });

    useVigieStore.getState().setSessionActivity("agent-1", "working");

    const session = useVigieStore.getState().sessionsByTask["task-1"]?.find((s) => s.localId === AGENT_TAB);
    expect(session?.activity).toBe("working");
  });

  it("setSessionActivity is a no-op when no session has a matching backendId", () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    useVigieStore.getState().setSessionInfo("task-1", AGENT_TAB, { backendId: "agent-1", status: "running" });

    // "agent-X" doesn't match any session
    useVigieStore.getState().setSessionActivity("agent-X", "error");

    const session = useVigieStore.getState().sessionsByTask["task-1"]?.find((s) => s.localId === AGENT_TAB);
    expect(session?.activity).toBeUndefined();
  });

  describe("attention cue (AC2-33)", () => {
    beforeEach(() => {
      useVigieStore.setState({ attentionByTask: {} });
      useVigieStore.getState().startAgentSession("task-1", false);
      useVigieStore.getState().setSessionInfo("task-1", AGENT_TAB, { backendId: "agent-1", status: "running" });
    });

    it.each(["needs_attention", "idle", "error"] as const)(
      "flags a non-selected task when its agent becomes %s",
      (activity) => {
        useVigieStore.getState().setSelectedTask("other-task");
        useVigieStore.getState().setSessionActivity("agent-1", activity);
        expect(useVigieStore.getState().attentionByTask["task-1"]).toBe(true);
      },
    );

    it("does not flag on 'working' activity", () => {
      useVigieStore.getState().setSelectedTask("other-task");
      useVigieStore.getState().setSessionActivity("agent-1", "working");
      expect(useVigieStore.getState().attentionByTask["task-1"]).toBeFalsy();
    });

    it("does not flag the currently-selected task for its own activity", () => {
      useVigieStore.getState().setSelectedTask("task-1");
      useVigieStore.getState().setSessionActivity("agent-1", "needs_attention");
      expect(useVigieStore.getState().attentionByTask["task-1"]).toBeFalsy();
    });

    it("clears a task's attention cue when it is selected", () => {
      useVigieStore.getState().setSelectedTask("other-task");
      useVigieStore.getState().setSessionActivity("agent-1", "needs_attention");
      expect(useVigieStore.getState().attentionByTask["task-1"]).toBe(true);

      useVigieStore.getState().setSelectedTask("task-1");
      expect(useVigieStore.getState().attentionByTask["task-1"]).toBeFalsy();
    });

    it("setSelectedTask(null) preserves other tasks' attention flags", () => {
      useVigieStore.getState().setSelectedTask("other-task");
      useVigieStore.getState().setSessionActivity("agent-1", "idle");
      expect(useVigieStore.getState().attentionByTask["task-1"]).toBe(true);

      useVigieStore.getState().setSelectedTask(null);
      expect(useVigieStore.getState().attentionByTask["task-1"]).toBe(true);
    });

    it("clearTaskSessions also clears the task's attention flag", () => {
      useVigieStore.getState().setSelectedTask("other-task");
      useVigieStore.getState().setSessionActivity("agent-1", "idle");
      expect(useVigieStore.getState().attentionByTask["task-1"]).toBe(true);

      useVigieStore.getState().clearTaskSessions("task-1");
      expect(useVigieStore.getState().attentionByTask["task-1"]).toBeUndefined();
    });
  });

  describe("sidebar collapse + width (AC2-17)", () => {
    beforeEach(() => {
      localStorage.clear();
      // Reset sidebar slice to defaults (override any module-load state)
      useVigieStore.setState({
        sidebarCollapsed: false,
        sidebarWidth: 260,
      } as any);
    });

    it("setSidebarCollapsed(true) updates state and writes localStorage", () => {
      useVigieStore.getState().setSidebarCollapsed(true);
      expect(useVigieStore.getState().sidebarCollapsed).toBe(true);
      expect(localStorage.getItem("vigie.sidebarCollapsed")).toBe("true");
    });

    it("setSidebarCollapsed(false) updates state and writes false to localStorage", () => {
      useVigieStore.getState().setSidebarCollapsed(true);
      useVigieStore.getState().setSidebarCollapsed(false);
      expect(useVigieStore.getState().sidebarCollapsed).toBe(false);
      expect(localStorage.getItem("vigie.sidebarCollapsed")).toBe("false");
    });

    it("setSidebarWidth updates state and writes localStorage", () => {
      useVigieStore.getState().setSidebarWidth(320);
      expect(useVigieStore.getState().sidebarWidth).toBe(320);
      expect(localStorage.getItem("vigie.sidebarWidth")).toBe("320");
    });

  });

  // AC2-53: exercise the module-load initializer directly (the tests above
  // only cover the setter). Set localStorage, reset the module registry, then
  // re-import the store so its initial-state factory re-runs against the
  // freshly stored values.
  describe("sidebar init from localStorage at module load (AC2-53)", () => {
    beforeEach(() => {
      localStorage.clear();
      vi.resetModules();
    });

    const freshStore = async () =>
      (await import("./index")).useVigieStore.getState();

    it("initializes sidebarCollapsed=true when localStorage is \"true\"", async () => {
      localStorage.setItem("vigie.sidebarCollapsed", "true");
      expect((await freshStore()).sidebarCollapsed).toBe(true);
    });

    it("initializes sidebarCollapsed=false when localStorage is unset", async () => {
      expect((await freshStore()).sidebarCollapsed).toBe(false);
    });

    it("initializes sidebarCollapsed=false for any non-\"true\" value", async () => {
      localStorage.setItem("vigie.sidebarCollapsed", "1");
      expect((await freshStore()).sidebarCollapsed).toBe(false);
    });

    it("initializes sidebarWidth from a stored numeric value", async () => {
      localStorage.setItem("vigie.sidebarWidth", "400");
      expect((await freshStore()).sidebarWidth).toBe(400);
    });

    it("falls back to default width 260 when unset", async () => {
      expect((await freshStore()).sidebarWidth).toBe(260);
    });

    it("falls back to default width 260 when stored value is non-numeric", async () => {
      localStorage.setItem("vigie.sidebarWidth", "not-a-number");
      expect((await freshStore()).sidebarWidth).toBe(260);
    });
  });
});
