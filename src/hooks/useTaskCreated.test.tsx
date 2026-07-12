import { render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Capture the registered task_created callback so the test can fire it.
let createdCb: ((e: { taskId: string }) => void) | undefined;
vi.mock("../api", () => ({
  onTaskCreated: (cb: (e: { taskId: string }) => void) => {
    createdCb = cb;
    return Promise.resolve(() => {});
  },
}));

const refresh = vi.fn().mockResolvedValue(undefined);

vi.mock("../store", () => ({
  useVigieStore: Object.assign((selector: (s: unknown) => unknown) => selector({ refresh }), {
    getState: () => ({ refresh }),
  }),
}));

import { useTaskCreated } from "./useTaskCreated";

function Harness() {
  useTaskCreated();
  return null;
}

describe("useTaskCreated", () => {
  beforeEach(() => {
    createdCb = undefined;
    refresh.mockClear();
  });

  it("refreshes the store when a task_created event fires", async () => {
    render(<Harness />);
    await waitFor(() => expect(createdCb).toBeDefined());

    await createdCb!({ taskId: "task-pending-1" });

    await waitFor(() => expect(refresh).toHaveBeenCalled());
  });
});
