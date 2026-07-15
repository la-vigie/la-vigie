import { render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalHost } from "./TerminalHost";
import { useVigieStore, AGENT_TAB, orchestratorSurfaceId } from "../../store";

vi.mock("./TerminalView", () => ({
  TerminalView: ({
    taskId,
    localId,
    hidden,
  }: {
    taskId: string;
    localId: string;
    hidden: boolean;
  }) => (
    <div
      data-testid={`terminal-${taskId}-${localId}`}
      data-hidden={String(hidden)}
    />
  ),
}));

describe("TerminalHost", () => {
  beforeEach(() => {
    useVigieStore.setState({
      repos: [],
      tasks: [],
      selectedTaskId: null,
      selectedOrchestratorRepoId: null,
      sessionsByTask: {},
      activeTabByTask: {},
    });
  });

  it("renders nothing when there are no sessions", () => {
    const { container } = render(<TerminalHost />);
    expect(container.querySelectorAll("[data-testid]").length).toBe(0);
  });

  it("renders one TerminalView per session across tasks, hidden unless selected and active tab", () => {
    useVigieStore.setState({
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a1" }],
        "task-2": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a2" }],
      },
      activeTabByTask: {
        "task-1": AGENT_TAB,
        "task-2": AGENT_TAB,
      },
      selectedTaskId: "task-2",
    });

    const { getByTestId } = render(<TerminalHost />);

    expect(getByTestId(`terminal-task-1-${AGENT_TAB}`).dataset.hidden).toBe("true");
    expect(getByTestId(`terminal-task-2-${AGENT_TAB}`).dataset.hidden).toBe("false");
  });

  it("keeps previously-rendered terminals mounted when selection changes", () => {
    useVigieStore.setState({
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a1" }],
        "task-2": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a2" }],
      },
      activeTabByTask: {
        "task-1": AGENT_TAB,
        "task-2": AGENT_TAB,
      },
      selectedTaskId: "task-1",
    });

    const { getByTestId, rerender } = render(<TerminalHost />);
    const term1Before = getByTestId(`terminal-task-1-${AGENT_TAB}`);

    useVigieStore.getState().setSelectedTask("task-2");
    rerender(<TerminalHost />);

    const term1After = getByTestId(`terminal-task-1-${AGENT_TAB}`);
    expect(term1After).toBe(term1Before);
    expect(term1After.dataset.hidden).toBe("true");
  });

  it("switching the active tab within a task keeps the previously-active TerminalView DOM node mounted (keep-alive)", () => {
    const shellLocalId = "shell-abc";
    useVigieStore.setState({
      sessionsByTask: {
        "task-1": [
          { localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a1" },
          { localId: shellLocalId, kind: "shell", status: "running", title: "shell" },
        ],
      },
      activeTabByTask: { "task-1": AGENT_TAB },
      selectedTaskId: "task-1",
    });

    const { getByTestId, rerender } = render(<TerminalHost />);
    const agentTermBefore = getByTestId(`terminal-task-1-${AGENT_TAB}`);
    expect(agentTermBefore.dataset.hidden).toBe("false");

    // Switch active tab to the shell session
    useVigieStore.getState().setActiveTab("task-1", shellLocalId);
    rerender(<TerminalHost />);

    // Agent TerminalView node is still in the DOM (same reference), just hidden
    const agentTermAfter = getByTestId(`terminal-task-1-${AGENT_TAB}`);
    expect(agentTermAfter).toBe(agentTermBefore);
    expect(agentTermAfter.dataset.hidden).toBe("true");

    // Shell TerminalView is now visible
    expect(getByTestId(`terminal-task-1-${shellLocalId}`).dataset.hidden).toBe("false");
  });

  it("keeps both the task and orchestrator terminals mounted across a task<->orchestrator selection switch", () => {
    const orchSurfaceId = orchestratorSurfaceId("r1");
    useVigieStore.setState({
      selectedTaskId: "t1",
      selectedOrchestratorRepoId: null,
      sessionsByTask: {
        t1: [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude" }],
        [orchSurfaceId]: [{ localId: AGENT_TAB, kind: "orchestrator", status: "running", title: "Orchestrator" }],
      },
      activeTabByTask: { t1: AGENT_TAB, [orchSurfaceId]: AGENT_TAB },
    });

    const { getByTestId, rerender } = render(<TerminalHost />);
    const taskTermBefore = getByTestId(`terminal-t1-${AGENT_TAB}`);
    const orchTermBefore = getByTestId(`terminal-${orchSurfaceId}-${AGENT_TAB}`);
    expect(taskTermBefore.dataset.hidden).toBe("false");
    expect(orchTermBefore.dataset.hidden).toBe("true");

    // Switch selection to the orchestrator.
    useVigieStore.getState().setSelectedOrchestrator("r1");
    rerender(<TerminalHost />);

    const taskTermMid = getByTestId(`terminal-t1-${AGENT_TAB}`);
    const orchTermMid = getByTestId(`terminal-${orchSurfaceId}-${AGENT_TAB}`);
    expect(taskTermMid).toBe(taskTermBefore); // same DOM node — never remounted
    expect(orchTermMid).toBe(orchTermBefore);
    expect(taskTermMid.dataset.hidden).toBe("true");
    expect(orchTermMid.dataset.hidden).toBe("false");

    // Switch selection back to the task.
    useVigieStore.getState().setSelectedTask("t1");
    rerender(<TerminalHost />);

    const taskTermAfter = getByTestId(`terminal-t1-${AGENT_TAB}`);
    const orchTermAfter = getByTestId(`terminal-${orchSurfaceId}-${AGENT_TAB}`);
    expect(taskTermAfter).toBe(taskTermBefore); // still the same DOM node
    expect(orchTermAfter).toBe(orchTermBefore);
    expect(taskTermAfter.dataset.hidden).toBe("false");
    expect(orchTermAfter.dataset.hidden).toBe("true");
  });
});
