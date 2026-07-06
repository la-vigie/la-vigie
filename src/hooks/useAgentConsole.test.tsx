import { renderHook, act } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useVigieStore } from "../store";

// ---- mock ../api ----
type ConsoleCallback = (e: { agentId: string; model?: string; contextRemainingPercent?: number; mode?: string }) => void;

const { capturedCallbacks, onAgentConsoleMock } = vi.hoisted(() => {
  const callbacks: ConsoleCallback[] = [];
  return {
    capturedCallbacks: callbacks,
    onAgentConsoleMock: vi.fn((cb: ConsoleCallback) => {
      callbacks.push(cb);
      return Promise.resolve(() => {
        const idx = callbacks.indexOf(cb);
        if (idx !== -1) callbacks.splice(idx, 1);
      });
    }),
  };
});

vi.mock("../api", () => ({
  onAgentConsole: onAgentConsoleMock,
}));

// ---- mock @tauri-apps/api/core (needed by store) ----
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { useAgentConsole } from "./useAgentConsole";

describe("useAgentConsole", () => {
  beforeEach(() => {
    useVigieStore.setState({ consoleByAgentId: {} });
    capturedCallbacks.length = 0;
    onAgentConsoleMock.mockClear();
  });

  it("subscribes on mount and unsubscribes on unmount", async () => {
    const { unmount } = renderHook(() => useAgentConsole());
    await act(async () => {});

    expect(onAgentConsoleMock).toHaveBeenCalledTimes(1);
    expect(capturedCallbacks).toHaveLength(1);

    unmount();
    await act(async () => {});
    expect(capturedCallbacks).toHaveLength(0);
  });

  it("updates consoleByAgentId when an agent_console event arrives", async () => {
    renderHook(() => useAgentConsole());
    await act(async () => {});

    await act(async () => {
      for (const cb of capturedCallbacks) {
        cb({ agentId: "b1", model: "Opus", contextRemainingPercent: 88 });
      }
    });

    expect(useVigieStore.getState().consoleByAgentId["b1"]).toEqual({
      model: "Opus",
      contextRemainingPercent: 88,
    });
  });

  it("merges partial updates without overwriting absent keys", async () => {
    renderHook(() => useAgentConsole());
    await act(async () => {});

    await act(async () => {
      for (const cb of capturedCallbacks) {
        cb({ agentId: "b1", model: "Opus", contextRemainingPercent: 88 });
        cb({ agentId: "b1", mode: "auto" });
      }
    });

    expect(useVigieStore.getState().consoleByAgentId["b1"]).toEqual({
      model: "Opus",
      contextRemainingPercent: 88,
      mode: "auto",
    });
  });

  it("tears down the listener if unmounted before the subscription resolves", async () => {
    const unlistenSpy = vi.fn();
    let resolveListen: (fn: () => void) => void = () => {};
    onAgentConsoleMock.mockImplementationOnce(
      () => new Promise<() => void>((res) => (resolveListen = res)),
    );

    const { unmount } = renderHook(() => useAgentConsole());
    unmount();

    await act(async () => {
      resolveListen(unlistenSpy);
    });

    expect(unlistenSpy).toHaveBeenCalledTimes(1);
  });
});
