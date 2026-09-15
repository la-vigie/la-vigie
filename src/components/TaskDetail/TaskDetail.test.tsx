import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { TaskDetail } from "./TaskDetail";
import { useVigieStore } from "../../store";
import type { Task, VigieState } from "../../store";
import { AGENT_TAB } from "../../store";
import * as agentHooks from "../../hooks/useAgents";
import * as api from "../../api";

vi.mock("../../hooks/useAgents");

const { invokeMock, stopSession, listAgentsMock, setTaskAgentMock, setTaskModelMock, setTaskAutoApproveMock } = vi.hoisted(() => {
  const invokeMock = vi.fn();
  // stopSession delegates to invokeMock so existing assertions on invokeMock("stop_session")
  // continue to work after the api module is mocked at the component boundary.
  const stopSession = vi.fn((sessionId: string) => invokeMock("stop_session", { sessionId }));
  // listAgents, setTaskAgent, setTaskModel, setTaskAutoApprove default to safe values so existing tests don't throw.
  const listAgentsMock = vi.fn().mockResolvedValue([]);
  const setTaskAgentMock = vi.fn().mockResolvedValue(undefined);
  const setTaskModelMock = vi.fn().mockResolvedValue(undefined);
  const setTaskAutoApproveMock = vi.fn().mockResolvedValue(undefined);
  return { invokeMock, stopSession, listAgentsMock, setTaskAgentMock, setTaskModelMock, setTaskAutoApproveMock };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  Channel: class {
    onmessage: ((event: unknown) => void) | null = null;
  },
}));

// Replace stopSession/listAgents/setTaskAgent/setTaskModel/setTaskAutoApprove with spies so TaskDetail.tsx calls them instead of the real api.
vi.mock("../../api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api")>();
  return {
    ...actual,
    stopSession,
    listAgents: listAgentsMock,
    setTaskAgent: setTaskAgentMock,
    setTaskModel: setTaskModelMock,
    setTaskAutoApprove: setTaskAutoApproveMock,
  };
});

// Default safe hook mocks — overridden per-describe where needed.
beforeEach(() => {
  (agentHooks.useAgents as ReturnType<typeof vi.fn>).mockReturnValue({ agents: [], loading: false, error: null });
  (agentHooks.useAgentModels as ReturnType<typeof vi.fn>).mockReturnValue({ models: [], loading: false });
});

vi.mock("../../hooks/useTerminalFileDrop", () => ({
  useTerminalFileDrop: vi.fn().mockReturnValue(false),
}));

vi.mock("../Terminal/TerminalHost", () => ({
  TerminalHost: () => <div data-testid="terminal-host" />,
}));

vi.mock("../Acp/AcpSurface", () => ({
  AcpSurface: ({ taskId }: { taskId: string }) => (
    <div data-testid="acp-surface" data-task-id={taskId} />
  ),
}));

vi.mock("../Terminal/RunStatePill", () => ({
  RunStatePill: ({ onStop }: { onStop: () => void }) => (
    <button type="button" aria-label="Stop agent" onClick={onStop}>
      Stop
    </button>
  ),
}));

vi.mock("../Review/ReviewPanel", async () => {
  const { useState } = await import("react");
  return {
    ReviewPanel: ({
      taskId,
      diffPosition,
      onSetDiffPosition,
      onToggleDiff,
      specMaximized,
      onToggleSpecMaximize,
    }: {
      taskId: string;
      showDiff: boolean;
      onToggleDiff: () => void;
      diffPosition: "right" | "bottom";
      onSetDiffPosition: (p: "right" | "bottom") => void;
      specMaximized: boolean;
      onToggleSpecMaximize: () => void;
    }) => {
      const [open, setOpen] = useState(false);
      return (
        <div data-testid="review-panel" data-task-id={taskId} data-spec-max={String(specMaximized)}>
          <button type="button" aria-label="Diff options" onClick={() => setOpen((o) => !o)}>…</button>
          <button
            type="button"
            aria-label={specMaximized ? "Restore spec and docs dock" : "Maximize spec and docs dock"}
            onClick={onToggleSpecMaximize}
          >
            {specMaximized ? "⤡" : "⤢"}
          </button>
          {open && (
            <div role="menu">
              <button role="menuitem" type="button" onClick={() => { onSetDiffPosition("right"); setOpen(false); }}>Right split {diffPosition === "right" ? "✓" : ""}</button>
              <button role="menuitem" type="button" onClick={() => { onSetDiffPosition("bottom"); setOpen(false); }}>Bottom split {diffPosition === "bottom" ? "✓" : ""}</button>
              <button role="menuitem" type="button" onClick={() => { onToggleDiff(); setOpen(false); }}>Hide diff</button>
            </div>
          )}
        </div>
      );
    },
  };
});

const task: Task = {
  id: "task-1",
  repoId: "repo-1",
  title: "Fix login bug",
  worktreePath: "/tmp/wt/fix-login-bug",
  branch: "fix-login-bug",
  baseBranch: "main",
  status: "idle",
  createdAt: 1,
  updatedAt: 1,
  inPlace: false,
};

describe("TaskDetail", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    localStorage.clear();
    useVigieStore.setState({
      repos: [],
      tasks: [],
      selectedTaskId: null,
      sessionsByTask: {},
      activeTabByTask: {},
      errorByTask: {},
      // startAgentSession awaits the catalog before routing unless
      // it's already loaded; mark it loaded so button-click flows stay
      // synchronous (the catalog is realistically loaded by click time).
      agentsLoaded: true,
    });
  });

  it('shows "Select a task" when nothing is selected', () => {
    render(<TaskDetail />);

    expect(screen.getByText("Select a task")).toBeInTheDocument();
  });

  it("shows the selected task's title, branch, base branch, and worktree path", () => {
    useVigieStore.setState({
      repos: [],
      tasks: [task],
      selectedTaskId: "task-1",
    });

    render(<TaskDetail />);

    expect(screen.getByText("Fix login bug")).toBeInTheDocument();
    expect(screen.getByText("fix-login-bug")).toBeInTheDocument();
    expect(screen.getByText("main")).toBeInTheDocument();
    expect(screen.getByText("/tmp/wt/fix-login-bug")).toBeInTheDocument();
  });

  it("shows Start agent and Resume when there is no agent session", () => {
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    render(<TaskDetail />);

    // Start agent + Resume live in the in-pane placeholder.
    expect(screen.getByText("Start agent")).toBeInTheDocument();
    expect(screen.getByText("Resume")).toBeInTheDocument();
    expect(screen.queryByText("Stop")).not.toBeInTheDocument();
  });

  it("sets the task auto-approve override", async () => {
    const setTaskAutoApprove = vi.mocked(api.setTaskAutoApprove);
    setTaskAutoApprove.mockResolvedValue();
    // The onChange handler chains .then(refresh); refresh() calls list_state via
    // invoke, so give it a valid snapshot shape (avoids an unhandled rejection
    // from destructuring an undefined snapshot).
    invokeMock.mockResolvedValue({ repos: [], tasks: [] });
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    render(<TaskDetail />);

    fireEvent.change(screen.getByLabelText("Auto-approve for this task"), {
      target: { value: "off" },
    });

    await waitFor(() =>
      expect(setTaskAutoApprove).toHaveBeenCalledWith(expect.any(String), false),
    );
  });

  it("shows Start agent and Resume when the agent session has exited", () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "exited", title: "Claude", backendId: "a1" }],
      },
      activeTabByTask: { "task-1": AGENT_TAB },
    });

    render(<TaskDetail />);

    expect(screen.getByText("Start agent")).toBeInTheDocument();
    expect(screen.getByText("Resume")).toBeInTheDocument();
  });

  it("clicking Start agent starts a non-resuming session for the task", () => {
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    render(<TaskDetail />);
    // Start agent lives only in the in-pane placeholder.
    fireEvent.click(screen.getByRole("button", { name: "Start agent" }));

    const sessions = useVigieStore.getState().sessionsByTask["task-1"];
    expect(sessions).toHaveLength(1);
    expect(sessions[0]).toMatchObject({
      localId: AGENT_TAB,
      kind: "agent",
      status: "starting",
      resume: false,
    });
  });

  it("clicking Resume starts a resuming session for the task", async () => {
    // Provide a claude spec with resumeArgs (via the shared useAgents hook) so Resume is enabled.
    (agentHooks.useAgents as ReturnType<typeof vi.fn>).mockReturnValue({
      agents: [
        { name: "claude", displayName: "Claude Code", binary: "claude", baseArgs: [], resumeArgs: ["--continue"], extraArgs: [], promptMode: "arg", status: "claudeHooks", builtin: true },
      ],
      loading: false,
      error: null,
    });
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    render(<TaskDetail />);
    // Resume is enabled because the claude spec (with resumeArgs) is available.
    await waitFor(() => expect(screen.getByRole("button", { name: /resume/i })).not.toBeDisabled());
    fireEvent.click(screen.getByText("Resume"));

    const sessions = useVigieStore.getState().sessionsByTask["task-1"];
    expect(sessions).toHaveLength(1);
    expect(sessions[0]).toMatchObject({
      localId: AGENT_TAB,
      kind: "agent",
      status: "starting",
      resume: true,
    });
  });

  // ACP engines have no `resumeArgs` (that's a PTY concept); their
  // Resume affordance is gated on the task's stored `acpSessionId` instead.
  const acpAgent = {
    name: "claude-acp",
    displayName: "Claude (ACP)",
    binary: "npx",
    baseArgs: [],
    resumeArgs: [],
    extraArgs: [],
    promptMode: "none" as const,
    status: "claudeHooks" as const,
    builtin: true,
    execution: "acp" as const,
  };

  it("disables Resume for an ACP engine when the task has no stored acpSessionId", () => {
    (agentHooks.useAgents as ReturnType<typeof vi.fn>).mockReturnValue({
      agents: [acpAgent],
      loading: false,
      error: null,
    });
    useVigieStore.setState({
      tasks: [{ ...task, agent: "claude-acp", acpSessionId: null }],
      selectedTaskId: "task-1",
    });

    render(<TaskDetail />);
    expect(screen.getByRole("button", { name: /resume/i })).toBeDisabled();
  });

  it("enables Resume for an ACP engine once the task has a stored acpSessionId", async () => {
    (agentHooks.useAgents as ReturnType<typeof vi.fn>).mockReturnValue({
      agents: [acpAgent],
      loading: false,
      error: null,
    });
    useVigieStore.setState({
      tasks: [{ ...task, agent: "claude-acp", acpSessionId: "sess-abc" }],
      selectedTaskId: "task-1",
    });

    render(<TaskDetail />);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /resume/i })).not.toBeDisabled(),
    );
  });

  it("renders the run-state pill (not header Start/Resume/Stop) when the agent is running", () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a1" }],
      },
      activeTabByTask: { "task-1": AGENT_TAB },
    });

    render(<TaskDetail />);

    expect(screen.getByRole("button", { name: /stop agent/i })).toBeInTheDocument();
    expect(screen.queryByText("Start agent")).not.toBeInTheDocument();
    expect(screen.queryByText("Resume")).not.toBeInTheDocument();
  });

  it("clicking Stop calls stop_session and removes the agent session", async () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a1" }],
      },
      activeTabByTask: { "task-1": AGENT_TAB },
    });
    invokeMock.mockResolvedValueOnce(undefined);

    render(<TaskDetail />);
    fireEvent.click(screen.getByRole("button", { name: /stop agent/i }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("stop_session", { sessionId: "a1" });
    });
    // removeAgentSession clears agent from sessions
    const sessions = useVigieStore.getState().sessionsByTask["task-1"];
    expect(sessions).toEqual([]);
  });

  it("a failed Stop surfaces the error and does NOT remove the agent session", async () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a1" }],
      },
      activeTabByTask: { "task-1": AGENT_TAB },
    });
    stopSession.mockRejectedValueOnce(new Error("no such process"));

    render(<TaskDetail />);
    fireEvent.click(screen.getByRole("button", { name: /stop agent/i }));

    await waitFor(() => {
      expect(useVigieStore.getState().errorByTask["task-1"]).toBe("no such process");
    });
    // the session is left in place so the user can see it and retry
    const sessions = useVigieStore.getState().sessionsByTask["task-1"];
    expect(sessions.some((s) => s.kind === "agent")).toBe(true);
  });

  it("renders TerminalHost so terminals persist regardless of selection", () => {
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    render(<TaskDetail />);

    expect(screen.getByTestId("terminal-host")).toBeInTheDocument();
  });

  it("TerminalHost DOM node is NOT remounted when selectedTaskId changes (keep-alive)", () => {
    const task2: Task = {
      id: "task-2",
      repoId: "repo-1",
      title: "Other task",
      worktreePath: "/tmp/wt/other",
      branch: "other",
      baseBranch: "main",
      status: "idle",
      createdAt: 1,
      updatedAt: 1,
      inPlace: false,
    };

    useVigieStore.setState({ tasks: [task, task2], selectedTaskId: "task-1" });

    const { rerender } = render(<TaskDetail />);
    const hostBefore = screen.getByTestId("terminal-host");

    // Deselect (null)
    useVigieStore.setState({ selectedTaskId: null });
    rerender(<TaskDetail />);
    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);

    // Select task-2
    useVigieStore.setState({ selectedTaskId: "task-2" });
    rerender(<TaskDetail />);
    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);
  });

  it("TerminalHost DOM node is NOT remounted when the store's tasks array is replaced (keep-alive, TASK-120 refreshSnapshot)", () => {
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    render(<TaskDetail />);
    const hostBefore = screen.getByTestId("terminal-host");

    // Simulate refreshSnapshot()/list_state swapping the tasks array with a
    // brand-new reference (same task content, new array + object identity).
    act(() => {
      useVigieStore.getState().setTasks([{ ...task }]);
    });

    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);
  });

  it("shows ReviewPanel by default when a task is selected", () => {
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    render(<TaskDetail />);

    expect(screen.getByTestId("review-panel")).toBeInTheDocument();
  });

  // ── Error banner (surface agent error detail) ──────────────────────────────

  it("shows the error banner with the reason when the task has a stored error", () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      errorByTask: { "task-1": "Rate limited by the model API." },
    });

    render(<TaskDetail />);

    const banner = screen.getByTestId("task-error-banner");
    expect(banner).toHaveTextContent("Rate limited by the model API.");
  });

  it("does not show the error banner when the task has no stored error", () => {
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1", errorByTask: {} });

    render(<TaskDetail />);

    expect(screen.queryByTestId("task-error-banner")).not.toBeInTheDocument();
  });

  it("dismisses the error banner on click", async () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      errorByTask: { "task-1": "Billing error." },
    });

    render(<TaskDetail />);
    expect(screen.getByTestId("task-error-banner")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /dismiss error/i }));

    expect(screen.queryByTestId("task-error-banner")).not.toBeInTheDocument();
  });

  it("TerminalHost DOM node is NOT remounted when the error banner appears (keep-alive)", () => {
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1", errorByTask: {} });

    render(<TaskDetail />);
    const hostBefore = screen.getByTestId("terminal-host");
    expect(screen.queryByTestId("task-error-banner")).not.toBeInTheDocument();

    // A StopFailure arrives and the banner mounts — the always-mounted
    // <TerminalHost/> (which holds the live PTY) must keep its DOM identity.
    act(() => {
      useVigieStore.getState().setTaskError("task-1", "The model API is overloaded.");
    });

    expect(screen.getByTestId("task-error-banner")).toBeInTheDocument();
    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);
  });

  it("toggling Hide diff (via … menu) removes ReviewPanel but keeps TerminalHost mounted", async () => {
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    render(<TaskDetail />);

    // ReviewPanel and TerminalHost both visible initially
    expect(screen.getByTestId("review-panel")).toBeInTheDocument();
    expect(screen.getByTestId("terminal-host")).toBeInTheDocument();

    // Open menu and click Hide diff
    await userEvent.click(screen.getByRole("button", { name: /diff options/i }));
    await userEvent.click(screen.getByRole("menuitem", { name: /hide diff/i }));

    // ReviewPanel gone, TerminalHost still there
    expect(screen.queryByTestId("review-panel")).not.toBeInTheDocument();
    expect(screen.getByTestId("terminal-host")).toBeInTheDocument();
  });

  it("clicking the Changes rail after hiding brings ReviewPanel back", async () => {
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    render(<TaskDetail />);

    await userEvent.click(screen.getByRole("button", { name: /diff options/i }));
    await userEvent.click(screen.getByRole("menuitem", { name: /hide diff/i }));
    expect(screen.queryByTestId("review-panel")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /show changes/i }));
    expect(screen.getByTestId("review-panel")).toBeInTheDocument();
  });

  it("position toggle (via … menu) switches to bottom layout: adds body--bottom class", async () => {
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    render(<TaskDetail />);

    // Default position is right — open menu and pick Bottom split
    await userEvent.click(screen.getByRole("button", { name: /diff options/i }));
    await userEvent.click(screen.getByRole("menuitem", { name: /bottom split/i }));

    // The body element should have the modifier class
    const body = document.querySelector(".task-detail__body");
    expect(body?.className).toContain("task-detail__body--bottom");
  });

  it("position toggle again (via … menu) reverts to row layout (no modifier class)", async () => {
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    render(<TaskDetail />);

    // Switch to bottom
    await userEvent.click(screen.getByRole("button", { name: /diff options/i }));
    await userEvent.click(screen.getByRole("menuitem", { name: /bottom split/i }));
    const body = document.querySelector(".task-detail__body");
    expect(body?.className).toContain("task-detail__body--bottom");

    // Switch back to right
    await userEvent.click(screen.getByRole("button", { name: /diff options/i }));
    await userEvent.click(screen.getByRole("menuitem", { name: /right split/i }));
    expect(body?.className).not.toContain("task-detail__body--bottom");
  });

  it("diff position is persisted to localStorage via the … menu", async () => {
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    render(<TaskDetail />);

    expect(localStorage.getItem("vigie.diffPosition")).not.toBe("bottom");

    await userEvent.click(screen.getByRole("button", { name: /diff options/i }));
    await userEvent.click(screen.getByRole("menuitem", { name: /bottom split/i }));

    expect(localStorage.getItem("vigie.diffPosition")).toBe("bottom");
  });

  it("diff position is restored from localStorage on mount (body--bottom class applied)", () => {
    localStorage.setItem("vigie.diffPosition", "bottom");
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    render(<TaskDetail />);

    const body = document.querySelector(".task-detail__body");
    expect(body?.className).toContain("task-detail__body--bottom");
    // resize handle should be --y (vertical bottom split)
    expect(document.querySelector(".resize-handle--y")).toBeInTheDocument();
  });

  it("TerminalHost stays mounted across position toggle (keep-alive)", async () => {
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    const { rerender } = render(<TaskDetail />);
    const hostBefore = screen.getByTestId("terminal-host");

    // Toggle to bottom via menu
    await userEvent.click(screen.getByRole("button", { name: /diff options/i }));
    await userEvent.click(screen.getByRole("menuitem", { name: /bottom split/i }));
    rerender(<TaskDetail />);

    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);

    // Toggle back to right via menu
    await userEvent.click(screen.getByRole("button", { name: /diff options/i }));
    await userEvent.click(screen.getByRole("menuitem", { name: /right split/i }));
    rerender(<TaskDetail />);

    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);
  });

  it("TerminalHost DOM node is NOT remounted when the diff area is hidden and shown (keep-alive)", async () => {
    // TerminalHost is in .task-detail__terminal-area and ReviewPanel is in
    // .task-detail__diff-area — they are siblings. Hiding/showing the diff area
    // must NOT remount TerminalHost (the terminal process lives in it).
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    const { rerender } = render(<TaskDetail />);
    const hostBefore = screen.getByTestId("terminal-host");

    // Hide the diff area via menu (ReviewPanel unmounts from the DOM)
    await userEvent.click(screen.getByRole("button", { name: /diff options/i }));
    await userEvent.click(screen.getByRole("menuitem", { name: /hide diff/i }));
    rerender(<TaskDetail />);
    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);

    // Show the diff area again via rail (ReviewPanel remounts, TerminalHost must stay)
    await userEvent.click(screen.getByRole("button", { name: /show changes/i }));
    rerender(<TaskDetail />);
    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);
  });

  it("maximizing Spec/Docs adds body--spec-max and removes the resize handle; restoring reverts", async () => {
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    render(<TaskDetail />);
    const body = document.querySelector(".task-detail__body");

    // Split layout initially: no maximize class, resize handle present.
    expect(body?.className).not.toContain("task-detail__body--spec-max");
    expect(document.querySelector(".resize-handle")).toBeInTheDocument();
    expect(screen.getByTestId("review-panel").getAttribute("data-spec-max")).toBe("false");

    // Maximize.
    await userEvent.click(screen.getByRole("button", { name: /maximize spec and docs/i }));
    expect(body?.className).toContain("task-detail__body--spec-max");
    expect(document.querySelector(".resize-handle")).not.toBeInTheDocument();
    expect(screen.getByTestId("review-panel").getAttribute("data-spec-max")).toBe("true");

    // Restore.
    await userEvent.click(screen.getByRole("button", { name: /restore spec and docs/i }));
    expect(body?.className).not.toContain("task-detail__body--spec-max");
    expect(document.querySelector(".resize-handle")).toBeInTheDocument();
    expect(screen.getByTestId("review-panel").getAttribute("data-spec-max")).toBe("false");
  });

  it("TerminalHost DOM node is NOT remounted when Spec/Docs is maximized and restored (keep-alive)", async () => {
    // Maximizing collapses the terminal-area to zero size but must keep
    // <TerminalHost/> mounted (the PTY lives in it) — same rule as Hide diff.
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    const { rerender } = render(<TaskDetail />);
    const hostBefore = screen.getByTestId("terminal-host");

    // Maximize (terminal collapses to zero, TerminalHost must stay).
    await userEvent.click(screen.getByRole("button", { name: /maximize spec and docs/i }));
    rerender(<TaskDetail />);
    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);

    // Restore the split (terminal regains its size, TerminalHost must stay).
    await userEvent.click(screen.getByRole("button", { name: /restore spec and docs/i }));
    rerender(<TaskDetail />);
    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);
  });

  it("TerminalHost is NOT remounted when a live agent mounts the pill (keep-alive)", () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: { "task-1": [] },
      activeTabByTask: { "task-1": AGENT_TAB },
    });

    const { rerender } = render(<TaskDetail />);
    const hostBefore = screen.getByTestId("terminal-host");

    // Agent starts → placeholder is replaced by the pill, TerminalHost must stay.
    useVigieStore.setState({
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a1" }],
      },
    });
    rerender(<TaskDetail />);

    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);
  });
});

describe("TaskDetail — ticket key display (TASK-16)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    localStorage.clear();
    useVigieStore.setState({
      repos: [],
      tasks: [],
      selectedTaskId: null,
      sessionsByTask: {},
      activeTabByTask: {},
      errorByTask: {},
    });
  });

  it("renders the ticket key chip and the title in the header for a keyed task", () => {
    useVigieStore.setState({
      tasks: [{ ...task, ticketKey: "TST-1", title: "Fix login" }],
      selectedTaskId: "task-1",
    });
    render(<TaskDetail />);
    expect(screen.getByText("TST-1")).toBeInTheDocument();
    expect(screen.getByText("Fix login")).toBeInTheDocument();
  });

  it("uses the key as the heading for a key-only task", () => {
    useVigieStore.setState({
      tasks: [{ ...task, ticketKey: "TST-2", title: "" }],
      selectedTaskId: "task-1",
    });
    render(<TaskDetail />);
    expect(screen.getByRole("heading", { name: "TST-2" })).toBeInTheDocument();
  });
});

describe("TaskDetail — resizable terminal/diff split (TASK-17)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    localStorage.clear();
    useVigieStore.setState({
      repos: [],
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {},
      activeTabByTask: {},
    });
  });

  it("renders a resize-handle separator when diff is shown (right position)", () => {
    render(<TaskDetail />);
    // diff shown by default with task selected
    const sep = document.querySelector(".resize-handle");
    expect(sep).toBeInTheDocument();
    expect(sep).toHaveClass("resize-handle--x");
    expect(sep?.getAttribute("role")).toBe("separator");
  });

  it("does NOT render a resize-handle when diff is hidden", async () => {
    render(<TaskDetail />);
    await userEvent.click(screen.getByRole("button", { name: /diff options/i }));
    await userEvent.click(screen.getByRole("menuitem", { name: /hide diff/i }));
    expect(document.querySelector(".resize-handle")).not.toBeInTheDocument();
  });

  it("renders resize-handle--y when diffPosition is bottom", () => {
    localStorage.setItem("vigie.diffPosition", "bottom");
    render(<TaskDetail />);
    const sep = document.querySelector(".resize-handle");
    expect(sep).toBeInTheDocument();
    expect(sep).toHaveClass("resize-handle--y");
    expect(sep).not.toHaveClass("resize-handle--x");
  });

  it("drag right divider → updates vigie.diffWidth and persists", () => {
    render(<TaskDetail />);

    // Stub body getBoundingClientRect so math is deterministic.
    // bodyRight=1200, drag to clientX=720 → 1200-720=480 (no change)
    // drag to clientX=820 → 1200-820=380 → within 240..bodyWidth-200
    const body = document.querySelector(".task-detail__body") as HTMLElement;
    body.getBoundingClientRect = vi.fn(() => ({
      left: 0,
      right: 1200,
      top: 0,
      bottom: 768,
      width: 1200,
      height: 768,
      x: 0,
      y: 0,
      toJSON: () => {},
    }));

    const sep = document.querySelector(".resize-handle") as HTMLElement;
    fireEvent.mouseDown(sep);
    fireEvent.mouseMove(window, { clientX: 820 });
    fireEvent.mouseUp(window);

    // 1200 - 820 = 380; within clamp(240, 1200-200=1000) → 380
    expect(localStorage.getItem("vigie.diffWidth")).toBe("380");
    // The diff-area should have width:380px inline style
    const diffArea = document.querySelector(".task-detail__diff-area") as HTMLElement;
    expect(diffArea.style.width).toBe("380px");
  });

  it("drag right divider clamps to min 240", () => {
    render(<TaskDetail />);

    const body = document.querySelector(".task-detail__body") as HTMLElement;
    body.getBoundingClientRect = vi.fn(() => ({
      left: 0, right: 1200, top: 0, bottom: 768,
      width: 1200, height: 768, x: 0, y: 0, toJSON: () => {},
    }));

    const sep = document.querySelector(".resize-handle") as HTMLElement;
    fireEvent.mouseDown(sep);
    // clientX=1100 → 1200-1100=100, clamped to 240
    fireEvent.mouseMove(window, { clientX: 1100 });
    fireEvent.mouseUp(window);

    expect(localStorage.getItem("vigie.diffWidth")).toBe("240");
  });

  it("drag bottom divider → updates vigie.diffHeight and persists", () => {
    localStorage.setItem("vigie.diffPosition", "bottom");
    render(<TaskDetail />);

    const body = document.querySelector(".task-detail__body") as HTMLElement;
    body.getBoundingClientRect = vi.fn(() => ({
      left: 0, right: 1200, top: 0, bottom: 768,
      width: 1200, height: 768, x: 0, y: 0, toJSON: () => {},
    }));

    const sep = document.querySelector(".resize-handle") as HTMLElement;
    fireEvent.mouseDown(sep);
    // clientY=600 → 768-600=168, within clamp(120, 768-120=648) → 168
    fireEvent.mouseMove(window, { clientY: 600 });
    fireEvent.mouseUp(window);

    expect(localStorage.getItem("vigie.diffHeight")).toBe("168");
    const diffArea = document.querySelector(".task-detail__diff-area") as HTMLElement;
    expect(diffArea.style.height).toBe("168px");
  });

  it("TerminalHost DOM node is NOT remounted during drag (keep-alive)", () => {
    render(<TaskDetail />);

    const body = document.querySelector(".task-detail__body") as HTMLElement;
    body.getBoundingClientRect = vi.fn(() => ({
      left: 0, right: 1200, top: 0, bottom: 768,
      width: 1200, height: 768, x: 0, y: 0, toJSON: () => {},
    }));

    const hostBefore = screen.getByTestId("terminal-host");
    const sep = document.querySelector(".resize-handle") as HTMLElement;

    fireEvent.mouseDown(sep);
    fireEvent.mouseMove(window, { clientX: 800 });
    fireEvent.mouseMove(window, { clientX: 850 });
    fireEvent.mouseUp(window);

    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);
  });
});

describe("TaskDetail — Finish flow (TASK-39: opens shared modal)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    // Modal mounts fetch get_pr_status + get_changed_files; default them to safe values.
    invokeMock.mockResolvedValue(undefined);
    localStorage.clear();
    useVigieStore.setState({
      repos: [],
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {},
      activeTabByTask: {},
    });
  });

  it("clicking 'Finish task' opens the FinishTaskModal with context (no inline strip)", () => {
    render(<TaskDetail />);
    expect(screen.queryByRole("dialog", { name: /finish/i })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /finish task/i }));

    const dialog = screen.getByRole("dialog", { name: /finish/i });
    expect(dialog).toBeInTheDocument();
    // Context is surfaced as text (branch also shows in the header, so scope to
    // the dialog), and the safe primary action is present.
    expect(within(dialog).getByText("fix-login-bug")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /keep branch/i })).toBeInTheDocument();
    // The old inline Discard-on-single-click affordance is gone (guarded in the modal).
    expect(screen.queryByRole("button", { name: /^discard branch$/i })).not.toBeInTheDocument();
  });

  it("in-place task: the modal opens but hides the Discard danger zone", () => {
    useVigieStore.setState({
      tasks: [{ ...task, inPlace: true }],
      selectedTaskId: "task-1",
      sessionsByTask: {},
      activeTabByTask: {},
    });
    render(<TaskDetail />);
    fireEvent.click(screen.getByRole("button", { name: /finish task/i }));

    expect(screen.getByRole("button", { name: /keep branch/i })).toBeInTheDocument();
    // "Discard branch" would be a no-op for in-place (branch is always preserved).
    expect(screen.queryByRole("button", { name: /discard branch instead/i })).not.toBeInTheDocument();
  });

  it("Cancel closes the modal without calling finish_task", () => {
    render(<TaskDetail />);
    fireEvent.click(screen.getByRole("button", { name: /finish task/i }));
    expect(screen.getByRole("dialog", { name: /finish/i })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^cancel$/i }));

    expect(screen.queryByRole("dialog", { name: /finish/i })).not.toBeInTheDocument();
    expect(invokeMock.mock.calls.filter((c) => c[0] === "finish_task")).toHaveLength(0);
  });

  it("opening the finish modal does NOT remount the TerminalHost (KEEP-ALIVE)", () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a1" }],
      },
      activeTabByTask: { "task-1": AGENT_TAB },
    });
    render(<TaskDetail />);
    const hostBefore = screen.getByTestId("terminal-host");

    fireEvent.click(screen.getByRole("button", { name: /finish task/i }));

    expect(screen.getByRole("dialog", { name: /finish/i })).toBeInTheDocument();
    // The modal is a fixed-overlay sibling — the host node must be identical.
    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);
  });
});

describe("TaskDetail — tab strip (TASK-24)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    stopSession.mockReset();
    localStorage.clear();
    useVigieStore.setState({
      repos: [],
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a1" }],
      },
      activeTabByTask: { "task-1": AGENT_TAB },
    });
  });

  it("renders a Claude tab plus a + button, and clicking + adds a shell tab", async () => {
    render(<TaskDetail />);
    expect(screen.getByRole("tab", { name: /claude/i })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /new terminal/i }));
    expect(useVigieStore.getState().sessionsByTask["task-1"].some((s) => s.kind === "shell")).toBe(true);
  });

  it("labels the agent tab with the running agent's title, not a hardcoded Claude", () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Antigravity", lifecycle: true, backendId: "a1" }],
      },
      activeTabByTask: { "task-1": AGENT_TAB },
    });
    render(<TaskDetail />);
    expect(screen.getByRole("tab", { name: /antigravity/i })).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: /^claude$/i })).not.toBeInTheDocument();
  });

  it("closing a shell tab calls stopSession and removes it", async () => {
    // seed a shell with backendId "b9" active
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {
        "task-1": [
          { localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a1" },
          { localId: "shell-1", kind: "shell", status: "running", title: "shell", backendId: "b9" },
        ],
      },
      activeTabByTask: { "task-1": "shell-1" },
    });
    stopSession.mockResolvedValue(undefined);

    render(<TaskDetail />);
    await userEvent.click(screen.getByRole("button", { name: /close shell/i }));
    expect(stopSession).toHaveBeenCalledWith("b9");
    expect(useVigieStore.getState().sessionsByTask["task-1"].some((s) => s.kind === "shell")).toBe(false);
  });

  it("a failed shell close surfaces an error and keeps the shell tab", async () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {
        "task-1": [
          { localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a1" },
          { localId: "shell-1", kind: "shell", status: "running", title: "shell", backendId: "b9" },
        ],
      },
      activeTabByTask: { "task-1": "shell-1" },
    });
    stopSession.mockRejectedValueOnce(new Error("no such process"));

    render(<TaskDetail />);
    await userEvent.click(screen.getByRole("button", { name: /close shell/i }));

    await waitFor(() => {
      expect(useVigieStore.getState().errorByTask["task-1"]).toBe("no such process");
    });
    expect(useVigieStore.getState().sessionsByTask["task-1"].some((s) => s.kind === "shell")).toBe(true);
  });

  it("the Claude tab has no close button", () => {
    render(<TaskDetail />);
    const claudeTab = screen.getByRole("tab", { name: /claude/i });
    expect(within(claudeTab).queryByRole("button", { name: /close/i })).toBeNull();
  });

  it("shows the agent-not-running placeholder when no agent session and Claude tab active", () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: { "task-1": [] },
      activeTabByTask: { "task-1": AGENT_TAB },
    });
    render(<TaskDetail />);
    expect(screen.getByText(/agent not running/i)).toBeInTheDocument();
  });
});

describe("TaskDetail — agent picker (TASK-21)", () => {
  const agentFixtures = [
    { name: "claude", displayName: "Claude Code", binary: "claude", baseArgs: [], resumeArgs: ["--continue"], extraArgs: [], promptMode: "arg", status: "claudeHooks", builtin: true, modelsListArgs: null },
    { name: "aider", displayName: "Aider", binary: "aider", baseArgs: [], resumeArgs: [], extraArgs: [], promptMode: "arg", status: "lifecycle", builtin: true, modelsListArgs: null },
  ];

  let startAgentSessionMock: ReturnType<typeof vi.fn>;
  let realStartAgentSession: VigieState["startAgentSession"];

  beforeAll(() => {
    // Capture the real implementation before any test in this block replaces it.
    realStartAgentSession = useVigieStore.getState().startAgentSession;
  });

  beforeEach(() => {
    invokeMock.mockReset();
    listAgentsMock.mockClear();
    setTaskAgentMock.mockClear();
    setTaskModelMock.mockClear();
    localStorage.clear();
    startAgentSessionMock = vi.fn();
    useVigieStore.setState({
      repos: [],
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {},
      activeTabByTask: {},
      startAgentSession: startAgentSessionMock as unknown as VigieState["startAgentSession"],
    });
    // Provide agents to the AgentModelPicker via useAgents hook mock.
    (agentHooks.useAgents as ReturnType<typeof vi.fn>).mockReturnValue({ agents: agentFixtures, loading: false, error: null });
    (agentHooks.useAgentModels as ReturnType<typeof vi.fn>).mockReturnValue({ models: [], loading: false });
  });

  afterEach(() => {
    // Restore the real startAgentSession so other describe blocks aren't affected.
    useVigieStore.setState({ startAgentSession: realStartAgentSession });
  });

  it("lists agents in the start picker, persists agent and model on change", async () => {
    render(<TaskDetail />);

    // Open the AgentModelPicker and select "Aider".
    await userEvent.click(screen.getByTestId("amp-trigger"));
    await userEvent.click(screen.getByText("Aider"));

    // Agent and model should be persisted immediately.
    expect(setTaskAgentMock).toHaveBeenCalledWith("task-1", "aider");
    expect(setTaskModelMock).toHaveBeenCalledWith("task-1", null);

    // Resume is disabled for aider (empty resumeArgs).
    await waitFor(() => {
      expect(screen.getByRole("button", { name: /resume/i })).toBeDisabled();
    });

    // Start passes lifecycle:true for a lifecycle agent.
    await userEvent.click(screen.getByRole("button", { name: /start agent/i }));
    await waitFor(() => {
      expect(startAgentSessionMock).toHaveBeenCalledWith("task-1", false, { label: "Aider", lifecycle: true });
    });
  });

  it("shows persisted model on picker trigger when task has model and no user override (TASK-93 regression)", async () => {
    // Task with a persisted model
    const taskWithModel: Task = {
      ...task,
      id: "task-model-1",
      model: "zhipuai-coding-plan/glm-5.2",
    };
    useVigieStore.setState({
      tasks: [taskWithModel],
      selectedTaskId: "task-model-1",
      startAgentSession: startAgentSessionMock as unknown as VigieState["startAgentSession"],
    });

    render(<TaskDetail />);

    // The picker trigger should display the persisted model.
    const trigger = screen.getByTestId("amp-trigger");
    expect(trigger.textContent).toContain("zhipuai-coding-plan/glm-5.2");
  });
});

describe("TaskDetail — diff controls relocated (TASK-24 Task 5)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    localStorage.clear();
    useVigieStore.setState({
      repos: [],
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {},
      activeTabByTask: {},
    });
  });

  it("the terminal bar no longer shows the diff toggle buttons", () => {
    render(<TaskDetail />);
    expect(screen.queryByRole("button", { name: /hide diff/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /diff: (right|bottom)/i })).toBeNull();
  });

  it("hiding the diff shows a Changes rail that restores it", async () => {
    render(<TaskDetail />); // showDiff defaults true
    await userEvent.click(screen.getByRole("button", { name: /diff options/i }));
    await userEvent.click(screen.getByRole("menuitem", { name: /hide diff/i }));
    const rail = screen.getByRole("button", { name: /show changes/i });
    await userEvent.click(rail);
    expect(screen.getByRole("button", { name: /diff options/i })).toBeInTheDocument();
  });
});

describe("TaskDetail — queued placeholder for pending tasks (TASK-90)", () => {
  const pendingTask: Task = {
    ...task,
    status: "pending",
    worktreePath: "",
    branch: "",
  };

  beforeEach(() => {
    invokeMock.mockReset();
    localStorage.clear();
    useVigieStore.setState({
      repos: [],
      tasks: [pendingTask],
      selectedTaskId: "task-1",
      sessionsByTask: {},
      activeTabByTask: {},
    });
  });

  it("shows a queued placeholder for a pending task and no start controls", () => {
    render(<TaskDetail />);

    // Scope to the placeholder itself — StatusBanner also has role="status",
    // so assert on the dedicated queued-placeholder node, not by role alone.
    const placeholder = document.querySelector(".task-detail__queued");
    expect(placeholder).toBeInTheDocument();
    expect(placeholder).toHaveTextContent(/queued/i);

    // No agent Start/Resume controls, and no Finish/Open PR header actions,
    // for a queued task (it has no worktree/agent/PR yet).
    expect(screen.queryByRole("button", { name: /start/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /resume/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /finish task/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /open pr/i })).toBeNull();
  });

  it("keeps the TerminalHost mounted for a pending task (KEEP-ALIVE)", () => {
    render(<TaskDetail />);

    // TerminalHost host node must still be present (identity invariant) —
    // a queued task must never cause TerminalHost to unmount.
    expect(screen.getByTestId("terminal-host")).toBeInTheDocument();
    expect(document.querySelector(".terminal-pane__body")).toBeInTheDocument();
  });

  it("lists the task's actual blockers in the queued placeholder (TASK-177)", () => {
    const withBlockers: Task = {
      ...pendingTask,
      blockedBy: [
        { taskId: "b1", title: "Build the API", status: "working" },
        { taskId: "b2", title: null, status: null },
      ],
    };
    useVigieStore.setState({ tasks: [withBlockers], selectedTaskId: withBlockers.id });
    render(<TaskDetail />);
    const placeholder = document.querySelector(".task-detail__queued");
    expect(placeholder).toBeTruthy();
    expect(placeholder).toHaveTextContent("Build the API");
    // A dangling blocker with no title falls back to its id.
    expect(placeholder).toHaveTextContent("b2");
  });
});

describe("TaskDetail — ACP surface (TASK-244)", () => {
  const acpTask: Task = {
    id: "task-1",
    repoId: "repo-1",
    title: "ACP task",
    worktreePath: "/tmp/wt/acp",
    branch: "acp",
    baseBranch: "main",
    status: "idle",
    createdAt: 1,
    updatedAt: 1,
    inPlace: false,
    agent: "claude-acp",
  };

  beforeEach(() => {
    invokeMock.mockReset();
    localStorage.clear();
    useVigieStore.setState({
      repos: [],
      tasks: [acpTask],
      selectedTaskId: "task-1",
      sessionsByTask: {},
      activeTabByTask: { "task-1": AGENT_TAB },
      errorByTask: {},
    });
  });

  const acpSession = () => ({
    localId: AGENT_TAB,
    kind: "agent" as const,
    status: "running" as const,
    title: "Claude Code (ACP)",
    backendId: "acp-1",
    engine: "acp" as const,
  });

  it("renders the AcpSurface for a live ACP agent session (agent tab active)", () => {
    useVigieStore.setState({ sessionsByTask: { "task-1": [acpSession()] } });
    render(<TaskDetail />);
    expect(screen.getByTestId("acp-surface")).toBeInTheDocument();
    expect(screen.getByTestId("acp-surface").dataset.taskId).toBe("task-1");
  });

  it("renders NO AcpSurface for a PTY agent session", () => {
    useVigieStore.setState({
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a1", engine: "pty" }],
      },
    });
    render(<TaskDetail />);
    expect(screen.queryByTestId("acp-surface")).toBeNull();
  });

  it("TerminalHost DOM node is NOT remounted when the ACP surface appears and disappears (keep-alive)", () => {
    render(<TaskDetail />);
    const hostBefore = screen.getByTestId("terminal-host");
    expect(screen.queryByTestId("acp-surface")).toBeNull();

    // ACP agent session starts → surface mounts as a SIBLING of the host.
    act(() => {
      useVigieStore.setState({ sessionsByTask: { "task-1": [acpSession()] } });
    });
    expect(screen.getByTestId("acp-surface")).toBeInTheDocument();
    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);

    // Session removed (exit) → surface unmounts; host identity unchanged.
    act(() => {
      useVigieStore.getState().removeAgentSession("task-1");
    });
    expect(screen.queryByTestId("acp-surface")).toBeNull();
    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);
  });

  it("switching to a shell tab swaps the ACP surface out without remounting TerminalHost (keep-alive)", () => {
    useVigieStore.setState({
      sessionsByTask: {
        "task-1": [acpSession(), { localId: "sh1", kind: "shell", status: "running", title: "shell" }],
      },
    });
    render(<TaskDetail />);
    const hostBefore = screen.getByTestId("terminal-host");
    expect(screen.getByTestId("acp-surface")).toBeInTheDocument();

    act(() => {
      useVigieStore.getState().setActiveTab("task-1", "sh1");
    });
    expect(screen.queryByTestId("acp-surface")).toBeNull();
    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);

    act(() => {
      useVigieStore.getState().setActiveTab("task-1", AGENT_TAB);
    });
    expect(screen.getByTestId("acp-surface")).toBeInTheDocument();
    expect(screen.getByTestId("terminal-host")).toBe(hostBefore);
  });
});
