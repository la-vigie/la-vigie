import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { TaskDetail } from "./TaskDetail";
import { useVigieStore, orchestratorSurfaceId } from "../../store";
import * as agentHooks from "../../hooks/useAgents";

vi.mock("../../hooks/useAgents");

const { invokeMock, stopSessionMock } = vi.hoisted(() => {
  const invokeMock = vi.fn();
  const stopSessionMock = vi.fn().mockResolvedValue(undefined);
  return { invokeMock, stopSessionMock };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  Channel: class {
    onmessage: ((event: unknown) => void) | null = null;
  },
}));

vi.mock("../../api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api")>();
  return {
    ...actual,
    stopSession: stopSessionMock,
  };
});

vi.mock("../../hooks/useTerminalFileDrop", () => ({
  useTerminalFileDrop: vi.fn().mockReturnValue(false),
}));

vi.mock("../Terminal/TerminalHost", () => ({
  TerminalHost: () => <div data-testid="terminal-host" />,
}));

afterEach(cleanup);

describe("TaskDetail orchestrator view", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    stopSessionMock.mockClear();
    (agentHooks.useAgents as ReturnType<typeof vi.fn>).mockReturnValue({
      agents: [],
      loading: false,
      error: null,
    });
  });

  it("shows the orchestrator header for the selected repo and Stop tears the session down", async () => {
    useVigieStore.setState({
      selectedTaskId: null,
      selectedOrchestratorRepoId: "r1",
      repos: [{ id: "r1", name: "acme", path: "/x", defaultBranch: "main" } as never],
      tasks: [],
      sessionsByTask: {
        [orchestratorSurfaceId("r1")]: [
          { localId: "agent", kind: "orchestrator", status: "running", title: "Orchestrator", backendId: "agent-1" },
        ],
      },
      activeTabByTask: { [orchestratorSurfaceId("r1")]: "agent" },
    });

    render(<TaskDetail />);

    expect(screen.getByText(/orchestrator · acme/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /stop/i }));

    expect(stopSessionMock).toHaveBeenCalledWith("agent-1");
  });

  it('shows an "Open orchestrator" placeholder when the repo has no live session', () => {
    useVigieStore.setState({
      selectedTaskId: null,
      selectedOrchestratorRepoId: "r1",
      repos: [{ id: "r1", name: "acme", path: "/x", defaultBranch: "main" } as never],
      tasks: [],
      sessionsByTask: {},
      activeTabByTask: {},
    });

    render(<TaskDetail />);

    expect(screen.getByText(/orchestrator · acme/i)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^stop$/i })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /open orchestrator/i })).toBeInTheDocument();
  });
});
