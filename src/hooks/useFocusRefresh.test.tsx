import { renderHook, act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useVigieStore } from "../store";

const { onFocusChangedMock, focusHandlers } = vi.hoisted(() => {
  const handlers: Array<(e: { payload: boolean }) => void> = [];
  return {
    focusHandlers: handlers,
    onFocusChangedMock: vi.fn((h: (e: { payload: boolean }) => void) => {
      handlers.push(h);
      return Promise.resolve(() => {
        const i = handlers.indexOf(h);
        if (i !== -1) handlers.splice(i, 1);
      });
    }),
  };
});

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ onFocusChanged: onFocusChangedMock }),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { useFocusRefresh } from "./useFocusRefresh";

function pushFocus(focused: boolean) {
  for (const h of [...focusHandlers]) h({ payload: focused });
}

describe("useFocusRefresh", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    focusHandlers.length = 0;
    onFocusChangedMock.mockClear();
    useVigieStore.setState({ selectedTaskId: "task-1", reviewNonceByTask: {}, prNonceByTask: {} } as any);
  });
  afterEach(() => vi.useRealTimers());

  it("on window focus: refreshes snapshot and bumps review + pr for the selected task", async () => {
    const snap = vi.spyOn(useVigieStore.getState(), "refreshSnapshot").mockResolvedValue();
    const review = vi.spyOn(useVigieStore.getState(), "bumpReview");
    const pr = vi.spyOn(useVigieStore.getState(), "bumpPr");
    renderHook(() => useFocusRefresh());
    await act(async () => {});

    act(() => pushFocus(true));

    expect(snap).toHaveBeenCalledOnce();
    expect(review).toHaveBeenCalledWith("task-1");
    expect(pr).toHaveBeenCalledWith("task-1");
    snap.mockRestore();
    review.mockRestore();
    pr.mockRestore();
  });

  it("throttles: a second focus within the window is a no-op", async () => {
    const snap = vi.spyOn(useVigieStore.getState(), "refreshSnapshot").mockResolvedValue();
    renderHook(() => useFocusRefresh());
    await act(async () => {});

    act(() => pushFocus(true));
    act(() => pushFocus(true));
    expect(snap).toHaveBeenCalledOnce();

    act(() => { vi.advanceTimersByTime(5000); });
    act(() => pushFocus(true));
    expect(snap).toHaveBeenCalledTimes(2);
    snap.mockRestore();
  });

  it("ignores focus-lost (payload false)", async () => {
    const snap = vi.spyOn(useVigieStore.getState(), "refreshSnapshot").mockResolvedValue();
    renderHook(() => useFocusRefresh());
    await act(async () => {});

    act(() => pushFocus(false));
    expect(snap).not.toHaveBeenCalled();
    snap.mockRestore();
  });
});
