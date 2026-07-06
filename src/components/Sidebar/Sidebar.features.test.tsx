import { render, screen, fireEvent } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Sidebar } from "./Sidebar";
import { useVigieStore } from "../../store";
import type { Repo, Task } from "../../store";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

const repo: Repo = {
  id: "repo-a",
  name: "home-mgmt",
  path: "/tmp/repo-a",
  defaultBranch: "main",
};

const overview: Task = {
  id: "t1",
  repoId: "repo-a",
  title: "Repo overview",
  worktreePath: "/tmp/wt/1",
  branch: "repo-overview",
  baseBranch: "main",
  status: "working",
  createdAt: 1,
  updatedAt: 1,
  prNumber: 128,
};

const flaky: Task = {
  id: "t2",
  repoId: "repo-a",
  title: "flaky-test-fix",
  worktreePath: "/tmp/wt/2",
  branch: "flaky-test-fix",
  baseBranch: "main",
  status: "needs_attention",
  createdAt: 1,
  updatedAt: 1,
};

describe("Sidebar — search and PR badge", () => {
  beforeEach(() => {
    localStorage.clear();
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ repos: [], tasks: [] });
    useVigieStore.setState({
      repos: [repo],
      tasks: [overview, flaky],
      selectedTaskId: null,
      sidebarCollapsed: false,
      sidebarWidth: 260,
      sessionsByTask: {},
      activeTabByTask: {},
    } as any);
  });

  it("shows a PR badge for tasks that have a prNumber, and not for those without", () => {
    render(<Sidebar />);
    const overviewRow = screen.getByText("Repo overview").closest("button");
    const flakyRow = screen.getByText("flaky-test-fix").closest("button");
    expect(overviewRow?.textContent).toContain("PR");
    expect(flakyRow?.textContent).not.toContain("PR");
  });

  it("filters the task list by title as the user types in the search box", () => {
    render(<Sidebar />);
    expect(screen.getByText("Repo overview")).toBeInTheDocument();
    expect(screen.getByText("flaky-test-fix")).toBeInTheDocument();

    fireEvent.change(screen.getByRole("textbox", { name: /search tasks/i }), {
      target: { value: "flaky" },
    });

    expect(screen.queryByText("Repo overview")).not.toBeInTheDocument();
    expect(screen.getByText("flaky-test-fix")).toBeInTheDocument();
  });

  it("shows an attention cue on a flagged non-selected task, and not when it is selected", () => {
    // Another task ("t1") is selected; the flagged task ("t2") is not.
    useVigieStore.setState({
      attentionByTask: { t2: true },
      selectedTaskId: "t1",
    } as any);
    const { rerender } = render(<Sidebar />);
    expect(
      screen.getByText("flaky-test-fix").closest("button")?.className,
    ).toContain("sidebar__task--attention");
    expect(screen.getByLabelText("needs attention")).toBeInTheDocument();

    // The selected task never flags itself.
    useVigieStore.setState({ selectedTaskId: "t2" } as any);
    rerender(<Sidebar />);
    expect(
      screen.getByText("flaky-test-fix").closest("button")?.className,
    ).not.toContain("sidebar__task--attention");
    expect(screen.queryByLabelText("needs attention")).toBeNull();
  });

  it("opens the repo settings modal from the gear button", () => {
    render(<Sidebar />);
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByLabelText("Settings for home-mgmt"));
    expect(screen.getByRole("dialog")).toBeTruthy();
  });
});
