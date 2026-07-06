import { describe, it, expect, vi, beforeEach } from "vitest";

const { stopSession, apiDeleteTask, invokeMock } = vi.hoisted(() => ({
  stopSession: vi.fn().mockResolvedValue(undefined),
  apiDeleteTask: vi.fn().mockResolvedValue(undefined),
  invokeMock: vi.fn().mockResolvedValue({
    repos: [], tasks: [], worktreesRoot: null, soundSettings: null, fetchRemoteBase: true,
  }),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("../api", () => ({
  deleteTask: (...a: unknown[]) => apiDeleteTask(...a),
  stopSession: (...a: unknown[]) => stopSession(...a),
  removeRepo: vi.fn(),
  setSoundSettings: vi.fn(),
  setFetchRemoteBase: vi.fn(),
  listCustomSounds: vi.fn().mockResolvedValue([]),
  enableRemote: vi.fn(),
  disableRemote: vi.fn(),
  remoteStatus: vi.fn(),
}));

import { useVigieStore } from "./index";

describe("store.deleteTask", () => {
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

  it("stops sessions, clears them, deletes with the branch flag, deselects, refreshes", async () => {
    await useVigieStore.getState().deleteTask("t1", true);
    expect(stopSession).toHaveBeenCalledWith("be-1");
    expect(apiDeleteTask).toHaveBeenCalledWith("t1", true);
    expect(useVigieStore.getState().selectedTaskId).toBeNull();
    expect(useVigieStore.getState().sessionsByTask.t1).toBeUndefined();
    expect(invokeMock).toHaveBeenCalledWith("list_state");
  });

  it("keeps selection when deleting a non-selected task", async () => {
    useVigieStore.setState({ selectedTaskId: "other" });
    await useVigieStore.getState().deleteTask("t1", false);
    expect(apiDeleteTask).toHaveBeenCalledWith("t1", false);
    expect(useVigieStore.getState().selectedTaskId).toBe("other");
  });
});
