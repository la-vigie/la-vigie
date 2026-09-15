import { act, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TaskDetail } from "../TaskDetail/TaskDetail";
import { Tour } from "./Tour";
import { useVigieStore, AGENT_TAB } from "../../store";
import type { Task } from "../../store";
import { createTour, currentStep } from "../../tour/tourMachine";
import { TOUR_STEPS } from "../../tour/steps";

// KEEP-ALIVE: mounting and advancing the tour overlay must NEVER remount the
// terminal host. The overlay is a document.body portal — a sibling of the app,
// ancestor of nothing. This test renders the REAL TerminalHost (TerminalView
// mocked to an identifiable node, mirroring TerminalHost.test.tsx) inside the
// REAL TaskDetail, alongside <Tour/>, and asserts the terminal node keeps object
// identity while the tour mounts, advances through the `terminal` step, and
// unmounts — the same discipline as the existing keep-alive test.

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  Channel: class {
    onmessage: ((event: unknown) => void) | null = null;
  },
}));

vi.mock("../../hooks/useAgents", () => ({
  useAgents: () => ({ agents: [], loading: false, error: null }),
  useAgentModels: () => ({ models: [], loading: false }),
}));

vi.mock("../../hooks/useTerminalFileDrop", () => ({
  useTerminalFileDrop: () => false,
}));

// Real TerminalHost, but TerminalView is a stable identifiable stub.
vi.mock("../Terminal/TerminalView", () => ({
  TerminalView: ({ taskId, localId, hidden }: { taskId: string; localId: string; hidden: boolean }) => (
    <div data-testid={`terminal-${taskId}-${localId}`} data-hidden={String(hidden)} />
  ),
}));

// Heavy leaves that would pull in xterm / network — not under test here.
vi.mock("../Review/ReviewPanel", () => ({ ReviewPanel: () => <div data-testid="review-panel" /> }));
vi.mock("../Terminal/RunStatePill", () => ({ RunStatePill: () => <div data-testid="run-pill" /> }));
vi.mock("../TaskDetail/SetupPanel", () => ({ SetupPanel: () => null }));
vi.mock("../StatusBanner/StatusBanner", () => ({ StatusBanner: () => null }));
vi.mock("../Prompts/PromptPicker", () => ({ PromptPicker: () => null }));

class RO {
  observe() {}
  unobserve() {}
  disconnect() {}
}

const REPO = {
  id: "r1",
  name: "demo",
  path: "/repo",
  defaultBranch: "main",
  inPlaceDefault: false,
};

const TASK: Task = {
  id: "t1",
  repoId: "r1",
  title: "Demo task",
  worktreePath: "/wt/t1",
  branch: "feature/x",
  baseBranch: "main",
  status: "working",
  createdAt: 0,
  updatedAt: 0,
  inPlace: false,
} as Task;

describe("Tour KEEP-ALIVE (terminal host identity)", () => {
  beforeEach(() => {
    vi.stubGlobal("ResizeObserver", RO);
    localStorage.clear();
    invokeMock.mockResolvedValue(undefined);
    useVigieStore.setState({
      repos: [REPO as never],
      tasks: [TASK],
      selectedTaskId: "t1",
      selectedOrchestratorRepoId: null,
      sessionsByTask: {
        t1: [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a1" }],
      },
      activeTabByTask: { t1: AGENT_TAB },
      onboarding: createTour(TOUR_STEPS),
      onboardingStatus: "completed", // don't auto-start; we drive it explicitly
    });
  });

  afterEach(() => vi.unstubAllGlobals());

  it("keeps the terminal node mounted while the tour mounts, hits the terminal step, and unmounts", async () => {
    const { getByTestId } = render(
      <>
        <TaskDetail />
        <Tour />
      </>,
    );

    const termBefore = getByTestId(`terminal-t1-${AGENT_TAB}`);
    expect(document.querySelector(".tour")).toBeNull(); // tour not yet active

    // Start the tour and walk to the `terminal` step.
    act(() => useVigieStore.getState().startOnboarding());
    await waitFor(() => expect(document.querySelector(".tour")).toBeTruthy());

    let guard = 0;
    while (currentStep(useVigieStore.getState().onboarding)?.id !== "terminal" && guard < 20) {
      act(() => useVigieStore.getState().onboardingNext());
      guard += 1;
    }
    expect(currentStep(useVigieStore.getState().onboarding)?.id).toBe("terminal");

    // On the terminal step the SAME node is still mounted (never remounted).
    const termOnStep = getByTestId(`terminal-t1-${AGENT_TAB}`);
    expect(termOnStep).toBe(termBefore);
    expect(termOnStep.dataset.hidden).toBe("false");

    // Advance one more step — still the same node.
    act(() => useVigieStore.getState().onboardingNext());
    expect(getByTestId(`terminal-t1-${AGENT_TAB}`)).toBe(termBefore);

    // Dismiss the tour — overlay gone, terminal node still the same reference.
    act(() => useVigieStore.getState().skipOnboarding());
    await waitFor(() => expect(document.querySelector(".tour")).toBeNull());
    expect(getByTestId(`terminal-t1-${AGENT_TAB}`)).toBe(termBefore);
  });
});
