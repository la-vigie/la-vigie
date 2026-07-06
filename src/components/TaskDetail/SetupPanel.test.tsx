import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { getSetupStateMock } = vi.hoisted(() => ({ getSetupStateMock: vi.fn() }));
vi.mock("../../api", () => ({ getSetupState: getSetupStateMock }));

import { SetupPanel } from "./SetupPanel";
import { useVigieStore } from "../../store";

describe("SetupPanel", () => {
  beforeEach(() => {
    useVigieStore.setState({ setupByTask: {} });
    getSetupStateMock.mockResolvedValue({ status: null, log: "", exitCode: null });
  });

  it("renders nothing when there is no setup state", async () => {
    const { container } = render(<SetupPanel taskId="t1" />);
    await waitFor(() => expect(getSetupStateMock).toHaveBeenCalledWith("t1"));
    expect(container.firstChild).toBeNull();
  });

  it("shows a failed strip with the exit code, log hidden until toggled", async () => {
    useVigieStore.setState({ setupByTask: { t1: { status: "failed", log: "boom\n", exitCode: 1 } } });
    render(<SetupPanel taskId="t1" />);

    // The strip and exit code show immediately; the log is collapsed.
    expect(screen.getByText(/setup failed \(exit 1\)/i)).toBeInTheDocument();
    expect(screen.queryByText(/boom/)).not.toBeInTheDocument();

    // Expanding the log reveals the output.
    await userEvent.click(screen.getByRole("button", { name: "log" }));
    expect(screen.getByText(/boom/)).toBeInTheDocument();
  });

  it("auto-hides once setup succeeds", async () => {
    useVigieStore.setState({ setupByTask: { t1: { status: "succeeded", log: "done\n", exitCode: 0 } } });
    const { container } = render(<SetupPanel taskId="t1" />);
    await waitFor(() => expect(getSetupStateMock).toHaveBeenCalled());
    expect(container.firstChild).toBeNull();
  });

  it("dismisses the strip when ✕ is clicked", async () => {
    useVigieStore.setState({ setupByTask: { t1: { status: "failed", log: "boom\n", exitCode: 2 } } });
    const { container } = render(<SetupPanel taskId="t1" />);

    expect(screen.getByText(/setup failed/i)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Dismiss setup status" }));

    expect(container.firstChild).toBeNull();
    expect(useVigieStore.getState().setupByTask["t1"].dismissed).toBe(true);
  });

  it("hydrates from get_setup_state on mount", async () => {
    getSetupStateMock.mockResolvedValue({ status: "running", log: "installing\n", exitCode: null });
    render(<SetupPanel taskId="t2" />);
    await waitFor(() =>
      expect(useVigieStore.getState().setupByTask["t2"]).toMatchObject({
        status: "running",
        log: "installing\n",
      }),
    );
  });

  it("keeps a strip dismissed across a re-hydrate", async () => {
    useVigieStore.setState({ setupByTask: { t3: { status: "failed", log: "x\n", exitCode: 1, dismissed: true } } });
    getSetupStateMock.mockResolvedValue({ status: "failed", log: "x\n", exitCode: 1 });
    const { container } = render(<SetupPanel taskId="t3" />);
    await waitFor(() => expect(getSetupStateMock).toHaveBeenCalledWith("t3"));
    expect(container.firstChild).toBeNull();
    expect(useVigieStore.getState().setupByTask["t3"].dismissed).toBe(true);
  });
});
