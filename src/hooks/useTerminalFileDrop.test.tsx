// src/hooks/useTerminalFileDrop.test.tsx
import { renderHook, act } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useVigieStore, AGENT_TAB } from "../store";
import type { WebviewDropPayload } from "../api";

// ---- mock the api.ts wrappers the hook uses ----
const { dropHandlers, onWebviewFileDropMock, writeSessionMock } = vi.hoisted(() => {
  const handlers: Array<(p: unknown) => void> = [];
  return {
    dropHandlers: handlers,
    onWebviewFileDropMock: vi.fn((h: (p: unknown) => void) => {
      handlers.push(h);
      return Promise.resolve(() => {
        const i = handlers.indexOf(h);
        if (i !== -1) handlers.splice(i, 1);
      });
    }),
    writeSessionMock: vi.fn().mockResolvedValue(undefined),
  };
});

vi.mock("../api", () => ({
  onWebviewFileDrop: onWebviewFileDropMock,
  writeSession: writeSessionMock,
}));
// store imports @tauri-apps/api/core
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { useTerminalFileDrop } from "./useTerminalFileDrop";

function emit(payload: WebviewDropPayload) {
  for (const h of dropHandlers) h(payload);
}

function paneRef() {
  const el = document.createElement("div");
  el.getBoundingClientRect = () =>
    ({ left: 0, top: 0, right: 100, bottom: 100, width: 100, height: 100, x: 0, y: 0, toJSON: () => {} }) as DOMRect;
  return { current: el };
}

describe("useTerminalFileDrop", () => {
  beforeEach(() => {
    dropHandlers.length = 0;
    writeSessionMock.mockClear();
    window.devicePixelRatio = 1;
    useVigieStore.setState({
      selectedTaskId: "t1",
      activeTabByTask: { t1: AGENT_TAB },
      sessionsByTask: { t1: [{ localId: AGENT_TAB, kind: "agent", status: "running", backendId: "be-1", title: "Claude" }] },
    });
  });

  it("sets drag-over true while hovering inside the pane and false on leave", async () => {
    const { result } = renderHook(() => useTerminalFileDrop(paneRef()));
    await Promise.resolve(); // let onWebviewFileDrop resolve
    act(() => emit({ type: "over", position: { x: 50, y: 50 } }));
    expect(result.current).toBe(true);
    act(() => emit({ type: "leave" }));
    expect(result.current).toBe(false);
  });

  it("forwards a bracketed-paste payload to the active backend on drop inside the pane", async () => {
    renderHook(() => useTerminalFileDrop(paneRef()));
    await Promise.resolve();
    act(() => emit({ type: "drop", position: { x: 50, y: 50 }, paths: ["/a/b c.png"] }));
    expect(writeSessionMock).toHaveBeenCalledWith("be-1", "\x1b[200~/a/b\\ c.png\x1b[201~");
  });

  it("does not forward when the drop is outside the pane", async () => {
    renderHook(() => useTerminalFileDrop(paneRef()));
    await Promise.resolve();
    act(() => emit({ type: "drop", position: { x: 500, y: 500 }, paths: ["/a/b.png"] }));
    expect(writeSessionMock).not.toHaveBeenCalled();
  });

  it("does not forward when the active session has no backend (agent not running)", async () => {
    useVigieStore.setState({
      selectedTaskId: "t1",
      activeTabByTask: { t1: AGENT_TAB },
      sessionsByTask: { t1: [{ localId: AGENT_TAB, kind: "agent", status: "starting", title: "Claude" }] },
    });
    renderHook(() => useTerminalFileDrop(paneRef()));
    await Promise.resolve();
    act(() => emit({ type: "drop", position: { x: 50, y: 50 }, paths: ["/a/b.png"] }));
    expect(writeSessionMock).not.toHaveBeenCalled();
  });
});
