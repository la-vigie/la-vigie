import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ReviewPanel } from "./ReviewPanel";
import { useVigieStore } from "../../store";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  Channel: class {
    onmessage: ((event: unknown) => void) | null = null;
  },
}));

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

// Mock DiffPanel + PrPanel — we assert the Option D layout (diff owns the pane,
// PR is a collapsible dock), not their internals.
vi.mock("../Diff/DiffPanel", () => ({
  DiffPanel: ({ taskId, refreshToken }: { taskId: string; refreshToken?: number }) => (
    <div data-testid="diff-panel" data-task-id={taskId} data-refresh={String(refreshToken)} />
  ),
}));

vi.mock("../Pr/PrPanel", () => ({
  PrPanel: ({ taskId, refreshToken }: { taskId: string; refreshToken?: number }) => (
    <div data-testid="pr-panel" data-task-id={taskId} data-refresh={String(refreshToken)} />
  ),
}));

vi.mock("../Spec/SpecDock", () => ({
  SpecDock: ({ taskId, maximized, refreshToken }: { taskId: string; maximized?: boolean; refreshToken?: number }) => (
    <div data-testid="spec-dock" data-task-id={taskId} data-maximized={String(!!maximized)} data-refresh={String(refreshToken)} />
  ),
}));

describe("ReviewPanel — Option D (PR bottom dock)", () => {
  beforeEach(() => {
    localStorage.clear();
    invokeMock.mockReset();
    // ghStatus available + no PR — the dock shows "no PR yet".
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "gh_status") return Promise.resolve({ available: true, authenticated: true });
      if (cmd === "get_pr_status") return Promise.resolve(null);
      return Promise.resolve(null);
    });
    useVigieStore.setState({
      repos: [{ id: "repo-1", name: "repo", path: "/r", defaultBranch: "main", remoteUrl: "x" }],
      tasks: [
        {
          id: "task-1",
          repoId: "repo-1",
          title: "T",
          worktreePath: "/wt",
          branch: "feature",
          baseBranch: "main",
          status: "idle",
          createdAt: 1,
          updatedAt: 1,
        },
      ],
      selectedTaskId: "task-1",
    } as any);
  });

  const defaultProps = {
    taskId: "task-1",
    showDiff: true,
    onToggleDiff: vi.fn(),
    diffPosition: "right" as const,
    onSetDiffPosition: vi.fn(),
    specMaximized: false,
    onToggleSpecMaximize: vi.fn(),
  };

  it("renders the Changes diff and a collapsed PR dock bar by default", () => {
    render(<ReviewPanel {...defaultProps} />);

    expect(screen.getByTestId("diff-panel")).toHaveAttribute("data-task-id", "task-1");
    // Collapsed: a bar that expands the dock; the full PR panel is not mounted.
    expect(
      screen.getByRole("button", { name: /expand pull request dock/i }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("pr-panel")).not.toBeInTheDocument();
  });

  it("expanding the dock reveals the PR panel and a collapse control", () => {
    render(<ReviewPanel {...defaultProps} />);

    fireEvent.click(screen.getByRole("button", { name: /expand pull request dock/i }));

    expect(screen.getByTestId("pr-panel")).toHaveAttribute("data-task-id", "task-1");
    expect(
      screen.getByRole("button", { name: /collapse pull request dock/i }),
    ).toBeInTheDocument();
  });

  it("collapsing again hides the PR panel", () => {
    render(<ReviewPanel {...defaultProps} />);

    fireEvent.click(screen.getByRole("button", { name: /expand pull request dock/i }));
    expect(screen.getByTestId("pr-panel")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /collapse pull request dock/i }));
    expect(screen.queryByTestId("pr-panel")).not.toBeInTheDocument();
  });

  it("persists the collapsed state to localStorage", () => {
    render(<ReviewPanel {...defaultProps} />);

    fireEvent.click(screen.getByRole("button", { name: /expand pull request dock/i }));
    expect(localStorage.getItem("vigie.prDockCollapsed")).toBe("false");
  });

  it("renders the … Diff options menu with position and hide options", () => {
    render(<ReviewPanel {...defaultProps} />);

    const menuBtn = screen.getByRole("button", { name: /diff options/i });
    expect(menuBtn).toBeInTheDocument();

    fireEvent.click(menuBtn);

    expect(screen.getByRole("menuitem", { name: /right split/i })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: /bottom split/i })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: /hide diff/i })).toBeInTheDocument();
  });

  it("clicking 'Hide diff' in the … menu calls onToggleDiff", () => {
    const onToggleDiff = vi.fn();
    render(<ReviewPanel {...defaultProps} onToggleDiff={onToggleDiff} />);

    fireEvent.click(screen.getByRole("button", { name: /diff options/i }));
    fireEvent.click(screen.getByRole("menuitem", { name: /hide diff/i }));

    expect(onToggleDiff).toHaveBeenCalledOnce();
  });

  it("when specMaximized: only SpecDock renders (Changes diff + PR dock hidden) and it is told it's maximized", () => {
    render(<ReviewPanel {...defaultProps} specMaximized />);

    // SpecDock fills the pane and knows it's maximized.
    const dock = screen.getByTestId("spec-dock");
    expect(dock).toHaveAttribute("data-maximized", "true");
    expect(document.querySelector(".review-panel")?.className).toContain("review-panel--spec-max");

    // The Changes diff and the PR dock are not mounted in this mode.
    expect(screen.queryByTestId("diff-panel")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /diff options/i })).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /expand pull request dock/i }),
    ).not.toBeInTheDocument();
  });

  it("clicking 'Bottom split' calls onSetDiffPosition with 'bottom'", () => {
    const onSetDiffPosition = vi.fn();
    render(<ReviewPanel {...defaultProps} onSetDiffPosition={onSetDiffPosition} />);

    fireEvent.click(screen.getByRole("button", { name: /diff options/i }));
    fireEvent.click(screen.getByRole("menuitem", { name: /bottom split/i }));

    expect(onSetDiffPosition).toHaveBeenCalledWith("bottom");
  });

  it("pressing Escape while the … menu is open closes it", () => {
    render(<ReviewPanel {...defaultProps} />);

    fireEvent.click(screen.getByRole("button", { name: /diff options/i }));
    expect(screen.getByRole("menu")).toBeInTheDocument();

    fireEvent.keyDown(document, { key: "Escape" });

    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("clicking outside the … menu while it is open closes it", () => {
    render(<ReviewPanel {...defaultProps} />);

    fireEvent.click(screen.getByRole("button", { name: /diff options/i }));
    expect(screen.getByRole("menu")).toBeInTheDocument();

    fireEvent.mouseDown(document.body);

    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("renders the SpecDock alongside the diff and PR dock", () => {
    render(<ReviewPanel {...defaultProps} />);
    expect(screen.getByTestId("spec-dock")).toHaveAttribute("data-task-id", "task-1");
  });
});

describe("ReviewPanel — event-driven refresh (TASK-120)", () => {
  beforeEach(() => {
    localStorage.clear();
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "gh_status") return Promise.resolve({ available: true, authenticated: true });
      if (cmd === "get_pr_status") return Promise.resolve(null);
      if (cmd === "get_changed_files") return Promise.resolve([]);
      return Promise.resolve(null);
    });
    useVigieStore.setState({
      repos: [{ id: "repo-1", name: "repo", path: "/r", defaultBranch: "main", remoteUrl: "x" }],
      tasks: [{ id: "task-1", repoId: "repo-1", title: "T", worktreePath: "/wt", branch: "feature", baseBranch: "main", status: "idle", createdAt: 1, updatedAt: 1 }],
      selectedTaskId: "task-1",
      reviewNonceByTask: {},
      prNonceByTask: {},
    } as any);
  });

  const props = {
    taskId: "task-1",
    showDiff: true,
    onToggleDiff: vi.fn(),
    diffPosition: "right" as const,
    onSetDiffPosition: vi.fn(),
    specMaximized: false,
    onToggleSpecMaximize: vi.fn(),
  };

  it("bumpReview re-tokens Diff + Spec but not the PR dock", () => {
    render(<ReviewPanel {...props} />);
    // Expand the dock so PrPanel (and its refreshToken) is mounted.
    fireEvent.click(screen.getByRole("button", { name: /expand pull request dock/i }));

    const before = {
      diff: screen.getByTestId("diff-panel").getAttribute("data-refresh"),
      spec: screen.getByTestId("spec-dock").getAttribute("data-refresh"),
      pr: screen.getByTestId("pr-panel").getAttribute("data-refresh"),
    };

    act(() => useVigieStore.getState().bumpReview("task-1"));

    expect(screen.getByTestId("diff-panel").getAttribute("data-refresh")).not.toBe(before.diff);
    expect(screen.getByTestId("spec-dock").getAttribute("data-refresh")).not.toBe(before.spec);
    expect(screen.getByTestId("pr-panel").getAttribute("data-refresh")).toBe(before.pr);
  });

  it("bumpPr re-tokens the PR dock but not Diff/Spec", () => {
    render(<ReviewPanel {...props} />);
    fireEvent.click(screen.getByRole("button", { name: /expand pull request dock/i }));

    const before = {
      diff: screen.getByTestId("diff-panel").getAttribute("data-refresh"),
      pr: screen.getByTestId("pr-panel").getAttribute("data-refresh"),
    };

    act(() => useVigieStore.getState().bumpPr("task-1"));

    expect(screen.getByTestId("pr-panel").getAttribute("data-refresh")).not.toBe(before.pr);
    expect(screen.getByTestId("diff-panel").getAttribute("data-refresh")).toBe(before.diff);
  });

  it("manual ↻ re-tokens all three docks", () => {
    render(<ReviewPanel {...props} />);
    fireEvent.click(screen.getByRole("button", { name: /expand pull request dock/i }));

    const before = {
      diff: screen.getByTestId("diff-panel").getAttribute("data-refresh"),
      spec: screen.getByTestId("spec-dock").getAttribute("data-refresh"),
      pr: screen.getByTestId("pr-panel").getAttribute("data-refresh"),
    };

    fireEvent.click(screen.getByRole("button", { name: /refresh changes/i }));

    expect(screen.getByTestId("diff-panel").getAttribute("data-refresh")).not.toBe(before.diff);
    expect(screen.getByTestId("spec-dock").getAttribute("data-refresh")).not.toBe(before.spec);
    expect(screen.getByTestId("pr-panel").getAttribute("data-refresh")).not.toBe(before.pr);
  });
});
