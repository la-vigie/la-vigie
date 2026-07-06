import { renderHook, act } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { onSetupOutputMock, onSetupStatusMock } = vi.hoisted(() => ({
  onSetupOutputMock: vi.fn(),
  onSetupStatusMock: vi.fn(),
}));

vi.mock("../api", () => ({
  onSetupOutput: onSetupOutputMock,
  onSetupStatus: onSetupStatusMock,
}));

import { useSetupStatus } from "./useSetupStatus";
import { useVigieStore } from "../store";

describe("useSetupStatus", () => {
  beforeEach(() => {
    useVigieStore.setState({ setupByTask: {} });
    onSetupOutputMock.mockResolvedValue(() => {});
    onSetupStatusMock.mockResolvedValue(() => {});
  });

  it("routes output and status events into the store", async () => {
    renderHook(() => useSetupStatus());
    // Wait for the async subscriptions to register.
    await act(async () => { await Promise.resolve(); });

    const outputCb = onSetupOutputMock.mock.calls[0][0];
    const statusCb = onSetupStatusMock.mock.calls[0][0];

    act(() => outputCb({ taskId: "t1", data: "hello\n" }));
    act(() => statusCb({ taskId: "t1", status: "failed", exitCode: 2 }));

    const entry = useVigieStore.getState().setupByTask["t1"];
    expect(entry.log).toBe("hello\n");
    expect(entry.status).toBe("failed");
  });
});
