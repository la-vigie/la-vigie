import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { TaskDetail } from "./TaskDetail";
import { useVigieStore } from "../../store";
import type { Task, VigieState } from "../../store";
import { AGENT_TAB } from "../../store";
import * as agentHooks from "../../hooks/useAgents";

vi.mock("../../hooks/useAgents");

const { invokeMock, stopSession, listAgentsMock, setTaskAgentMock, setTaskModelMock } = vi.hoisted(() => {
  const invokeMock = vi.fn();
  // stopSession delegates to invokeMock so existing assertions on invokeMock("stop_session")
  // continue to work after the api module is mocked at the component boundary.
  const stopSession = vi.fn((sessionId: string) => invokeMock("stop_session", { sessionId }));
  // listAgents, setTaskAgent, setTaskModel default to safe values so existing tests don't throw.
  const listAgentsMock = vi.fn().mockResolvedValue([]);
  const setTaskAgentMock = vi.fn().mockResolvedValue(undefined);
  const setTaskModelMock = vi.fn().mockResolvedValue(undefined);
  return { invokeMock, stopSession, listAgentsMock, setTaskAgentMock, setTaskModelMock };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  Channel: class {
    onmessage: ((event: unknown) => void) | null = null;
  },
}));

// Replace stopSession/listAgents/setTaskAgent/setTaskModel with spies so TaskDetail.tsx calls them instead of the real api.
vi.mock("../../api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api")>();
  return { ...actual, stopSession, listAgents: listAgentsMock, setTaskAgent: setTaskAgentMock, setTaskModel: setTaskModelMock };
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

    // Start agent + Resume live in the in-pane placeholder now.
    expect(screen.getByText("Start agent")).toBeInTheDocument();
    expect(screen.getByText("Resume")).toBeInTheDocument();
    expect(screen.queryByText("Stop")).not.toBeInTheDocument();
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
    // Start agent now lives only in the in-pane placeholder.
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
    // Resume is enabled now that the claude spec (with resumeArgs) is available.
    await vi.waitFor(() => expect(screen.getByRole("button", { name: /resume/i })).not.toBeDisabled());
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

    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("stop_session", { sessionId: "a1" });
    });
    // removeAgentSession clears agent from sessions
    const sessions = useVigieStore.getState().sessionsByTask["task-1"];
    expect(sessions).toEqual([]);
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

  it("shows ReviewPanel by default when a task is selected", () => {
    useVigieStore.setState({ tasks: [task], selectedTaskId: "task-1" });

    render(<TaskDetail />);

    expect(screen.getByTestId("review-panel")).toBeInTheDocument();
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

describe("TaskDetail — ticket key display (AC2-16)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    localStorage.clear();
    useVigieStore.setState({
      repos: [],
      tasks: [],
      selectedTaskId: null,
      sessionsByTask: {},
      activeTabByTask: {},
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

describe("TaskDetail — resizable terminal/diff split (AC2-17)", () => {
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

describe("TaskDetail — Finish flow", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    // Default: no PR for this task (T5 adds a get_pr_status call when confirm opens)
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

  it("clicking 'Finish task' reveals Keep branch, Discard branch, and Cancel", () => {
    render(<TaskDetail />);

    expect(screen.queryByText("Keep branch")).not.toBeInTheDocument();
    expect(screen.queryByText("Discard branch")).not.toBeInTheDocument();
    expect(screen.queryByText("Cancel")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /finish task/i }));

    expect(screen.getByRole("button", { name: /keep branch/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /discard branch/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /cancel/i })).toBeInTheDocument();
  });

  it("clicking Cancel hides the confirmation without calling finish_task", () => {
    render(<TaskDetail />);

    fireEvent.click(screen.getByRole("button", { name: /finish task/i }));
    expect(screen.getByRole("button", { name: /cancel/i })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));

    expect(screen.queryByText("Keep branch")).not.toBeInTheDocument();
    expect(screen.queryByText("Discard branch")).not.toBeInTheDocument();
    expect(screen.queryByText("Cancel")).not.toBeInTheDocument();
    // Opening finish confirm now fetches PR status (T5) — only assert finish_task was not called
    const finishCalls = invokeMock.mock.calls.filter((c) => c[0] === "finish_task");
    expect(finishCalls).toHaveLength(0);
  });

  it("with a running agent, clicking 'Discard branch' stops the agent then calls finish_task discard, clears selection", async () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "agent-99" }],
      },
      activeTabByTask: { "task-1": AGENT_TAB },
    });
    invokeMock.mockResolvedValue(undefined);

    render(<TaskDetail />);
    fireEvent.click(screen.getByRole("button", { name: /finish task/i }));
    fireEvent.click(screen.getByRole("button", { name: /discard branch/i }));

    await vi.waitFor(() => {
      expect(useVigieStore.getState().selectedTaskId).toBeNull();
      expect(invokeMock).toHaveBeenCalledWith("stop_session", { sessionId: "agent-99" });
      expect(invokeMock).toHaveBeenCalledWith("finish_task", { taskId: "task-1", mode: "discard" });
      // stop_session must be called before finish_task
      const calls = invokeMock.mock.calls.map((c) => c[0]);
      expect(calls.indexOf("stop_session")).toBeLessThan(calls.indexOf("finish_task"));
    });
  });

  it("with no agent, clicking 'Keep branch' calls finish_task keep and does NOT call stop_session", async () => {
    invokeMock.mockResolvedValue(undefined);

    render(<TaskDetail />);
    fireEvent.click(screen.getByRole("button", { name: /finish task/i }));
    fireEvent.click(screen.getByRole("button", { name: /keep branch/i }));

    await vi.waitFor(() => {
      expect(useVigieStore.getState().selectedTaskId).toBeNull();
    });

    expect(invokeMock).toHaveBeenCalledWith("finish_task", { taskId: "task-1", mode: "keep" });
    const stopCalls = invokeMock.mock.calls.filter((c) => c[0] === "stop_session");
    expect(stopCalls).toHaveLength(0);
  });
});

describe("TaskDetail — tab strip (AC2-24)", () => {
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

describe("TaskDetail — Merge PR & finish (T5)", () => {
  const openPrStatus = {
    number: 7,
    url: "https://github.com/foo/bar/pull/7",
    title: "Fix login bug",
    state: "OPEN",
    isDraft: false,
    mergeable: "MERGEABLE",
    reviewDecision: null,
    checks: [],
  };

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

  it("with an OPEN PR, opening finish confirm shows 'Merge PR & finish' button", async () => {
    // get_pr_status called when confirm opens
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_pr_status") return Promise.resolve(openPrStatus);
      return Promise.resolve(undefined);
    });

    render(<TaskDetail />);
    fireEvent.click(screen.getByRole("button", { name: /finish task/i }));

    await vi.waitFor(() => {
      expect(screen.getByRole("button", { name: /merge pr & finish/i })).toBeInTheDocument();
    });
  });

  it("with no PR (null), finish confirm does NOT show 'Merge PR & finish'", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_pr_status") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });

    render(<TaskDetail />);
    fireEvent.click(screen.getByRole("button", { name: /finish task/i }));

    // Wait for PR fetch to settle (get_pr_status returns null)
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_pr_status", { taskId: "task-1" });
    });

    expect(screen.queryByRole("button", { name: /merge pr & finish/i })).not.toBeInTheDocument();
    // Keep/Discard/Cancel are still present
    expect(screen.getByRole("button", { name: /keep branch/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /discard branch/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /cancel/i })).toBeInTheDocument();
  });

  it("with OPEN PR and no running agent, clicking 'Merge PR & finish' calls finish_task with mode merge and clears selection", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_pr_status") return Promise.resolve(openPrStatus);
      return Promise.resolve(undefined);
    });

    render(<TaskDetail />);
    fireEvent.click(screen.getByRole("button", { name: /finish task/i }));

    await vi.waitFor(() => {
      expect(screen.getByRole("button", { name: /merge pr & finish/i })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: /merge pr & finish/i }));

    await vi.waitFor(() => {
      expect(useVigieStore.getState().selectedTaskId).toBeNull();
      expect(invokeMock).toHaveBeenCalledWith("finish_task", { taskId: "task-1", mode: "merge" });
    });

    const stopCalls = invokeMock.mock.calls.filter((c) => c[0] === "stop_session");
    expect(stopCalls).toHaveLength(0);
  });

  it("with OPEN PR and a running agent, clicking 'Merge PR & finish' stops the agent first then calls finish_task merge", async () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "agent-42" }],
      },
      activeTabByTask: { "task-1": AGENT_TAB },
    });
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_pr_status") return Promise.resolve(openPrStatus);
      return Promise.resolve(undefined);
    });

    render(<TaskDetail />);
    fireEvent.click(screen.getByRole("button", { name: /finish task/i }));

    await vi.waitFor(() => {
      expect(screen.getByRole("button", { name: /merge pr & finish/i })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: /merge pr & finish/i }));

    await vi.waitFor(() => {
      expect(useVigieStore.getState().selectedTaskId).toBeNull();
      expect(invokeMock).toHaveBeenCalledWith("stop_session", { sessionId: "agent-42" });
      expect(invokeMock).toHaveBeenCalledWith("finish_task", { taskId: "task-1", mode: "merge" });
      const calls = invokeMock.mock.calls.map((c) => c[0]).filter((c) => c !== "get_pr_status");
      expect(calls.indexOf("stop_session")).toBeLessThan(calls.indexOf("finish_task"));
    });
  });

  it("if finish_task rejects on merge, shows error and does NOT clear selection", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_pr_status") return Promise.resolve(openPrStatus);
      if (cmd === "finish_task") return Promise.reject(new Error("merge conflict"));
      return Promise.resolve(undefined);
    });

    render(<TaskDetail />);
    fireEvent.click(screen.getByRole("button", { name: /finish task/i }));

    await vi.waitFor(() => {
      expect(screen.getByRole("button", { name: /merge pr & finish/i })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: /merge pr & finish/i }));

    await vi.waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
      expect(screen.getByRole("alert")).toHaveTextContent("merge conflict");
    });

    // Task not dropped
    expect(useVigieStore.getState().selectedTaskId).toBe("task-1");
  });
});

describe("TaskDetail — Finish stops ALL sessions (Fix: shell PTY leak)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    stopSession.mockReset();
    localStorage.clear();
    useVigieStore.setState({
      repos: [],
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {},
      activeTabByTask: {},
    });
  });

  it("on 'Keep branch', stops both agent and shell backend sessions before finishing", async () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {
        "task-1": [
          { localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "agent-b" },
          { localId: "shell-1", kind: "shell", status: "running", title: "shell", backendId: "shell-b" },
        ],
      },
      activeTabByTask: { "task-1": AGENT_TAB },
    });
    stopSession.mockResolvedValue(undefined);
    invokeMock.mockResolvedValue(undefined);

    render(<TaskDetail />);
    fireEvent.click(screen.getByRole("button", { name: /finish task/i }));
    fireEvent.click(screen.getByRole("button", { name: /keep branch/i }));

    await vi.waitFor(() => {
      expect(useVigieStore.getState().selectedTaskId).toBeNull();
    });

    // Both backend sessions must have been stopped
    expect(stopSession).toHaveBeenCalledWith("agent-b");
    expect(stopSession).toHaveBeenCalledWith("shell-b");
    // And finish_task must have been called after
    expect(invokeMock).toHaveBeenCalledWith("finish_task", { taskId: "task-1", mode: "keep" });
    // stop calls must precede finish_task in the invokeMock call list
    const calls = invokeMock.mock.calls.map((c) => c[0]).filter((c) => c !== "get_pr_status");
    const lastStopIdx = Math.max(
      calls.lastIndexOf("stop_session"),
      // stopSession routes through invokeMock so stop_session appears there
    );
    expect(lastStopIdx).toBeLessThan(calls.indexOf("finish_task"));
  });
});

describe("TaskDetail — agent picker (AC2-21)", () => {
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
    await vi.waitFor(() => {
      expect(screen.getByRole("button", { name: /resume/i })).toBeDisabled();
    });

    // Start passes lifecycle:true for a lifecycle agent.
    await userEvent.click(screen.getByRole("button", { name: /start agent/i }));
    await vi.waitFor(() => {
      expect(startAgentSessionMock).toHaveBeenCalledWith("task-1", false, { label: "Aider", lifecycle: true });
    });
  });

  it("shows persisted model on picker trigger when task has model and no user override (AC2-93 regression)", async () => {
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

describe("TaskDetail — diff controls relocated (AC2-24 Task 5)", () => {
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
