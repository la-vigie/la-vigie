import { renderHook, act } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useVigieStore, AGENT_TAB } from "../store";
import { DEFAULT_SOUND_SETTINGS } from "../sound/types";

// ---- mock @tauri-apps/api/event ----
type ListenHandler = (event: { payload: unknown }) => void;
const { listenHandlers, listenMock } = vi.hoisted(() => {
  const handlers: ListenHandler[] = [];
  return {
    listenHandlers: handlers,
    listenMock: vi.fn((_event: string, handler: ListenHandler) => {
      handlers.push(handler);
      return Promise.resolve(() => {
        // remove this handler on unlisten
        const idx = handlers.indexOf(handler);
        if (idx !== -1) handlers.splice(idx, 1);
      });
    }),
  };
});

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

// ---- mock @tauri-apps/api/core (needed by store) ----
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

// ---- mock @tauri-apps/plugin-notification ----
const { sendNotificationMock, isPermissionGrantedMock, onActionMock } = vi.hoisted(() => ({
  sendNotificationMock: vi.fn(),
  isPermissionGrantedMock: vi.fn().mockResolvedValue(true),
  onActionMock: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: isPermissionGrantedMock,
  requestPermission: vi.fn(),
  sendNotification: sendNotificationMock,
  onAction: onActionMock,
}));

// ---- mock @tauri-apps/api/window ----
const { setFocusMock, unminimizeMock, onFocusChangedMock, isFocusedMock } = vi.hoisted(() => ({
  setFocusMock: vi.fn().mockResolvedValue(undefined),
  unminimizeMock: vi.fn().mockResolvedValue(undefined),
  onFocusChangedMock: vi.fn().mockResolvedValue(() => {}),
  isFocusedMock: vi.fn().mockResolvedValue(false),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    setFocus: setFocusMock,
    unminimize: unminimizeMock,
    onFocusChanged: onFocusChangedMock,
    isFocused: isFocusedMock,
  }),
}));

// ---- mock ../sound/player ----
const { playSoundSpy } = vi.hoisted(() => ({ playSoundSpy: vi.fn() }));

vi.mock("../sound/player", () => ({
  // Must be a regular function (not arrow) so `new SoundPlayer()` works.
  SoundPlayer: vi.fn(function () {
    return { playSound: playSoundSpy };
  }),
}));

// Helper to push an event to all registered handlers
function pushAgentStatusEvent(payload: { agentId: string; status: string }) {
  for (const h of listenHandlers) {
    h({ payload });
  }
}

import { invoke } from "@tauri-apps/api/core";
import { useAgentStatus } from "./useAgentStatus";

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

/** Make the native meeting probe (`is_meeting_active`) resolve to `active`. */
function setMeetingActive(active: boolean) {
  invokeMock.mockImplementation((cmd: string) =>
    Promise.resolve(cmd === "is_meeting_active" ? active : undefined),
  );
}

describe("useAgentStatus", () => {
  beforeEach(() => {
    useVigieStore.setState({
      repos: [],
      tasks: [
        {
          id: "task-1",
          repoId: "repo-1",
          title: "My Important Task",
          worktreePath: "/tmp/wt/1",
          branch: "my-task",
          baseBranch: "main",
          status: "idle",
          createdAt: 1,
          updatedAt: 1,
        },
      ],
      selectedTaskId: null,
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "agent-1" }],
      },
      activeTabByTask: { "task-1": AGENT_TAB },
    });
    listenHandlers.length = 0;
    listenMock.mockClear();
    sendNotificationMock.mockClear();
    isPermissionGrantedMock.mockResolvedValue(true);
    playSoundSpy.mockClear();
    invokeMock.mockReset();
  });

  it("subscribes to agent_status on mount and unsubscribes on unmount", async () => {
    const { unmount } = renderHook(() => useAgentStatus());

    // Give the async mount time to set up
    await act(async () => {});

    expect(listenMock).toHaveBeenCalledWith("agent_status", expect.any(Function));
    expect(listenHandlers).toHaveLength(1);

    unmount();
    // After unmount, handler removed
    await act(async () => {});
    expect(listenHandlers).toHaveLength(0);
  });

  it("updates store activity when a 'working' event arrives", async () => {
    renderHook(() => useAgentStatus());
    await act(async () => {});

    await act(async () => {
      pushAgentStatusEvent({ agentId: "agent-1", status: "working" });
    });

    const session = useVigieStore.getState().sessionsByTask["task-1"]?.find((s) => s.localId === AGENT_TAB);
    expect(session?.activity).toBe("working");
  });

  it("calls sendNotification when a 'needs_attention' event arrives", async () => {
    renderHook(() => useAgentStatus());
    await act(async () => {});

    await act(async () => {
      pushAgentStatusEvent({ agentId: "agent-1", status: "needs_attention" });
    });

    // Give the async notification call time to resolve
    await act(async () => {});

    const session = useVigieStore.getState().sessionsByTask["task-1"]?.find((s) => s.localId === AGENT_TAB);
    expect(session?.activity).toBe("needs_attention");
    expect(sendNotificationMock).toHaveBeenCalledWith({
      id: expect.any(Number),
      title: "My Important Task",
      body: "Awaiting input",
    });
  });

  it("does not call sendNotification for 'working' status", async () => {
    renderHook(() => useAgentStatus());
    await act(async () => {});

    await act(async () => {
      pushAgentStatusEvent({ agentId: "agent-1", status: "working" });
    });
    await act(async () => {});

    expect(sendNotificationMock).not.toHaveBeenCalled();
  });

  it("tears down the listener if unmounted before the subscription resolves", async () => {
    const unlistenSpy = vi.fn();
    let resolveListen: (fn: () => void) => void = () => {};
    listenMock.mockImplementationOnce(
      () => new Promise<() => void>((res) => (resolveListen = res)),
    );

    const { unmount } = renderHook(() => useAgentStatus());
    // Unmount before the listen() promise resolves.
    unmount();

    // Resolving now: cleanup already ran (cancelled=true), so the hook must
    // immediately invoke the unlisten fn rather than leaking the subscription.
    await act(async () => {
      resolveListen(unlistenSpy);
    });

    expect(unlistenSpy).toHaveBeenCalledTimes(1);
  });

  // ---- Sound integration tests ----

  /** Seed the store so a given agentId maps to a repo (optionally with a sound override). */
  function seedStoreWithAgent({
    agentId,
    repoSoundSettings,
  }: {
    agentId: string;
    repoSoundSettings: string | null;
  }) {
    useVigieStore.setState({
      repos: [
        {
          id: "r1",
          name: "Test Repo",
          path: "/tmp/repo",
          defaultBranch: "main",
          soundSettings: repoSoundSettings,
        },
      ],
      tasks: [
        {
          id: "t1",
          repoId: "r1",
          title: "Test Task",
          worktreePath: "/tmp/wt/t1",
          branch: "test-task",
          baseBranch: "main",
          status: "idle",
          createdAt: 1,
          updatedAt: 1,
        },
      ],
      sessionsByTask: {
        t1: [
          {
            localId: AGENT_TAB,
            kind: "agent",
            status: "running",
            title: "Claude",
            backendId: agentId,
          },
        ],
      },
      soundSettings: DEFAULT_SOUND_SETTINGS,
      selectedTaskId: null,
      activeTabByTask: { t1: AGENT_TAB },
    });
  }

  it("plays the completed sound when an agent goes idle", async () => {
    seedStoreWithAgent({ agentId: "agent-1", repoSoundSettings: null });
    renderHook(() => useAgentStatus());
    await act(async () => {});

    await act(async () => {
      pushAgentStatusEvent({ agentId: "agent-1", status: "idle" });
    });

    expect(playSoundSpy).toHaveBeenCalledWith("jobs-done", []);
  });

  it("does not play when the resolved event is muted by the repo", async () => {
    seedStoreWithAgent({ agentId: "agent-1", repoSoundSettings: '{"muted":true}' });
    renderHook(() => useAgentStatus());
    await act(async () => {});

    await act(async () => {
      pushAgentStatusEvent({ agentId: "agent-1", status: "idle" });
    });

    expect(playSoundSpy).not.toHaveBeenCalled();
  });

  it("fires a rich notification on completion using the resolved repo", async () => {
    seedStoreWithAgent({ agentId: "agent-1", repoSoundSettings: null });
    renderHook(() => useAgentStatus());
    await act(async () => {});

    await act(async () => {
      pushAgentStatusEvent({ agentId: "agent-1", status: "idle" });
    });
    await act(async () => {});

    expect(sendNotificationMock).toHaveBeenCalledWith({
      id: expect.any(Number),
      title: "Test Task",
      body: "Completed — Test Repo/test-task",
    });
  });

  it("does not notify for 'working' status", async () => {
    seedStoreWithAgent({ agentId: "agent-1", repoSoundSettings: null });
    renderHook(() => useAgentStatus());
    await act(async () => {});

    await act(async () => {
      pushAgentStatusEvent({ agentId: "agent-1", status: "working" });
    });
    await act(async () => {});

    expect(sendNotificationMock).not.toHaveBeenCalled();
  });

  it("does not notify when the event is muted by the repo", async () => {
    seedStoreWithAgent({ agentId: "agent-1", repoSoundSettings: '{"muted":true}' });
    renderHook(() => useAgentStatus());
    await act(async () => {});

    await act(async () => {
      pushAgentStatusEvent({ agentId: "agent-1", status: "idle" });
    });
    await act(async () => {});

    expect(sendNotificationMock).not.toHaveBeenCalled();
  });

  it("suppresses the notification for the focused + selected task", async () => {
    seedStoreWithAgent({ agentId: "agent-1", repoSoundSettings: null });
    useVigieStore.setState({ selectedTaskId: "t1" });
    isFocusedMock.mockResolvedValue(true);

    renderHook(() => useAgentStatus());
    await act(async () => {});

    await act(async () => {
      pushAgentStatusEvent({ agentId: "agent-1", status: "idle" });
    });
    await act(async () => {});

    expect(sendNotificationMock).not.toHaveBeenCalled();
    // Sound is unaffected by focus.
    expect(playSoundSpy).toHaveBeenCalledWith("jobs-done", []);
  });

  // ---- Automute (TASK-105) ----

  it("with automute on and in a meeting, suppresses the sound but still notifies", async () => {
    seedStoreWithAgent({ agentId: "agent-1", repoSoundSettings: null });
    useVigieStore.setState({ soundSettings: { ...DEFAULT_SOUND_SETTINGS, automute: true } });
    setMeetingActive(true);

    renderHook(() => useAgentStatus());
    await act(async () => {});

    await act(async () => {
      pushAgentStatusEvent({ agentId: "agent-1", status: "idle" });
    });
    await act(async () => {});

    expect(playSoundSpy).not.toHaveBeenCalled();
    expect(sendNotificationMock).toHaveBeenCalled();
  });

  it("with automute on but no meeting, plays the sound normally", async () => {
    seedStoreWithAgent({ agentId: "agent-1", repoSoundSettings: null });
    useVigieStore.setState({ soundSettings: { ...DEFAULT_SOUND_SETTINGS, automute: true } });
    setMeetingActive(false);

    renderHook(() => useAgentStatus());
    await act(async () => {});

    await act(async () => {
      pushAgentStatusEvent({ agentId: "agent-1", status: "idle" });
    });
    await act(async () => {});

    expect(playSoundSpy).toHaveBeenCalledWith("jobs-done", []);
  });

  it("does not probe for a meeting when automute is off", async () => {
    seedStoreWithAgent({ agentId: "agent-1", repoSoundSettings: null });
    setMeetingActive(true); // would mute, but automute is off so we must not probe

    renderHook(() => useAgentStatus());
    await act(async () => {});

    await act(async () => {
      pushAgentStatusEvent({ agentId: "agent-1", status: "idle" });
    });
    await act(async () => {});

    expect(invokeMock).not.toHaveBeenCalledWith("is_meeting_active");
    expect(playSoundSpy).toHaveBeenCalledWith("jobs-done", []);
  });

  it("still notifies for a background task while the window is focused", async () => {
    seedStoreWithAgent({ agentId: "agent-1", repoSoundSettings: null });
    useVigieStore.setState({ selectedTaskId: "some-other-task" });
    isFocusedMock.mockResolvedValue(true);

    renderHook(() => useAgentStatus());
    await act(async () => {});

    await act(async () => {
      pushAgentStatusEvent({ agentId: "agent-1", status: "idle" });
    });
    await act(async () => {});

    expect(sendNotificationMock).toHaveBeenCalled();
  });
});
