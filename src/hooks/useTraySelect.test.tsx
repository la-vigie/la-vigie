import { render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Capture the registered tray_select_task callback + hand back a spy unlistener.
let trayCb: ((e: { taskId: string }) => void) | undefined;
const off = vi.fn();
vi.mock("../api", () => ({
  onTraySelectTask: (cb: (e: { taskId: string }) => void) => {
    trayCb = cb;
    return Promise.resolve(off);
  },
}));

const setSelectedTask = vi.fn();

vi.mock("../store", () => ({
  useVigieStore: Object.assign(() => undefined, {
    getState: () => ({ setSelectedTask }),
  }),
}));

import { useTraySelect } from "./useTraySelect";

function Harness() {
  useTraySelect();
  return null;
}

describe("useTraySelect", () => {
  beforeEach(() => {
    trayCb = undefined;
    setSelectedTask.mockClear();
    off.mockClear();
  });

  it("selects the task when a tray_select_task event fires", async () => {
    render(<Harness />);
    await waitFor(() => expect(trayCb).toBeDefined());

    trayCb!({ taskId: "task-42" });

    expect(setSelectedTask).toHaveBeenCalledWith("task-42");
  });

  it("unsubscribes on unmount", async () => {
    const { unmount } = render(<Harness />);
    await waitFor(() => expect(trayCb).toBeDefined());

    unmount();

    await waitFor(() => expect(off).toHaveBeenCalled());
  });
});
