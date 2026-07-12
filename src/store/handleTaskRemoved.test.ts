import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn().mockResolvedValue({
    repos: [], tasks: [], worktreesRoot: null, soundSettings: null, fetchRemoteBase: true,
  }),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("../api", () => ({
  removeRepo: vi.fn(),
  deleteTask: vi.fn(),
  stopSession: vi.fn(),
  setSoundSettings: vi.fn(),
  setFetchRemoteBase: vi.fn(),
  listCustomSounds: vi.fn().mockResolvedValue([]),
  enableRemote: vi.fn(),
  disableRemote: vi.fn(),
  remoteStatus: vi.fn(),
}));

import { useVigieStore } from "./index";

describe("store.handleTaskRemoved (TASK-139)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeMock.mockResolvedValue({
      repos: [], tasks: [], worktreesRoot: null, soundSettings: null, fetchRemoteBase: true,
    });
    useVigieStore.setState({
      selectedTaskId: "t1",
      sessionsByTask: { t1: [{ localId: "agent", kind: "agent", status: "running", title: "Claude", backendId: "be-1" }] as never },
    });
  });

  it("clears the task's session state, deselects it, and refreshes", async () => {
    await useVigieStore.getState().handleTaskRemoved("t1");
    expect(useVigieStore.getState().selectedTaskId).toBeNull();
    expect(useVigieStore.getState().sessionsByTask.t1).toBeUndefined();
    expect(invokeMock).toHaveBeenCalledWith("list_state");
  });

  it("keeps selection when the removed task is not selected", async () => {
    useVigieStore.setState({ selectedTaskId: "other" });
    await useVigieStore.getState().handleTaskRemoved("t1");
    expect(useVigieStore.getState().selectedTaskId).toBe("other");
    expect(invokeMock).toHaveBeenCalledWith("list_state");
  });
});
