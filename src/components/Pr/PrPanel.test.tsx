import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PrPanel } from "./PrPanel";
import { useVigieStore } from "../../store";
import type { Task, Repo } from "../../store";

// Mock invoke + Channel
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  Channel: class {
    onmessage: ((event: unknown) => void) | null = null;
  },
}));

// Mock plugin-opener
const { openUrlMock } = vi.hoisted(() => ({ openUrlMock: vi.fn() }));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: openUrlMock,
}));

// Shared test fixtures
const repo: Repo = {
  id: "repo-1",
  name: "my-repo",
  path: "/repos/my-repo",
  defaultBranch: "main",
  remoteUrl: "https://github.com/foo/bar",
};

const repoNoRemote: Repo = {
  id: "repo-1",
  name: "my-repo",
  path: "/repos/my-repo",
  defaultBranch: "main",
  remoteUrl: undefined,
};

const task: Task = {
  id: "task-1",
  repoId: "repo-1",
  title: "Fix login bug",
  worktreePath: "/tmp/wt/fix-login",
  branch: "fix-login",
  baseBranch: "main",
  status: "idle",
  createdAt: 1,
  updatedAt: 1,
};

const prStatus = {
  number: 7,
  url: "https://github.com/foo/bar/pull/7",
  title: "Fix login bug",
  state: "open",
  isDraft: false,
  mergeable: "MERGEABLE",
  reviewDecision: "APPROVED",
  checks: [
    { name: "CI / tests", status: "success" },
    { name: "CI / lint", status: "failure" },
  ],
};

const prComments = [
  {
    author: "alice",
    body: "Looks good to me.",
    createdAt: "2024-06-01T10:00:00Z",
    path: null,
    line: null,
    kind: "issue_comment",
    state: null,
  },
  {
    author: "bob",
    body: "Please fix this.",
    createdAt: "2024-06-01T11:00:00Z",
    path: "src/login.ts",
    line: 42,
    kind: "inline",
    state: null,
  },
  {
    author: "carol",
    body: "LGTM",
    createdAt: "2024-06-01T12:00:00Z",
    path: null,
    line: null,
    kind: "review",
    state: "APPROVED",
  },
];

describe("PrPanel", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    openUrlMock.mockReset();
    useVigieStore.setState({
      repos: [repo],
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {},
      activeTabByTask: {},
    });
  });

  // State 1: gh not available
  it("shows 'GitHub CLI not found' when gh is not available", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "gh_status") return Promise.resolve({ available: false, authenticated: false });
      return Promise.resolve(null);
    });

    render(<PrPanel taskId="task-1" />);

    await waitFor(() => {
      expect(screen.getByText(/github cli not found/i)).toBeInTheDocument();
    });

    // Should NOT call get_pr_status
    expect(invokeMock).not.toHaveBeenCalledWith("get_pr_status", expect.anything());
  });

  // State 1b: gh available but not authenticated
  it("shows 'Run gh auth login' when gh is not authenticated", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "gh_status") return Promise.resolve({ available: true, authenticated: false });
      return Promise.resolve(null);
    });

    render(<PrPanel taskId="task-1" />);

    await waitFor(() => {
      expect(screen.getByText(/gh auth login/i)).toBeInTheDocument();
    });

    expect(invokeMock).not.toHaveBeenCalledWith("get_pr_status", expect.anything());
  });

  // State 2: repo with no remoteUrl
  it("shows 'Add a remote to create a PR' when repo has no remoteUrl", async () => {
    useVigieStore.setState({
      repos: [repoNoRemote],
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {},
      activeTabByTask: {},
    });
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "gh_status") return Promise.resolve({ available: true, authenticated: true });
      return Promise.resolve(null);
    });

    render(<PrPanel taskId="task-1" />);

    await waitFor(() => {
      expect(screen.getByText(/add a remote to create a pr/i)).toBeInTheDocument();
    });
  });

  // State 3: no PR — create form (late-loading task title)
  it("prefills create form title when task is set in store after mount", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "gh_status") return Promise.resolve({ available: true, authenticated: true });
      if (cmd === "get_pr_status") return Promise.resolve(null);
      return Promise.resolve(null);
    });

    // Start with no tasks so task?.title is undefined at mount
    useVigieStore.setState({
      repos: [repo],
      tasks: [],
      selectedTaskId: "task-1",
      sessionsByTask: {},
      activeTabByTask: {},
    });

    render(<PrPanel taskId="task-1" />);

    // Now set the task — simulates late-loading
    useVigieStore.setState({ tasks: [task] });

    await waitFor(() => {
      const titleInput = screen.getByRole("textbox", { name: /title/i });
      expect(titleInput).toHaveValue("Fix login bug");
    });
  });

  // State 3: no PR — create form
  it("renders create form with task title prefilled when no PR exists", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "gh_status") return Promise.resolve({ available: true, authenticated: true });
      if (cmd === "get_pr_status") return Promise.resolve(null);
      return Promise.resolve(null);
    });

    render(<PrPanel taskId="task-1" />);

    await waitFor(() => {
      expect(screen.getByRole("textbox", { name: /title/i })).toBeInTheDocument();
    });

    const titleInput = screen.getByRole("textbox", { name: /title/i });
    expect(titleInput).toHaveValue("Fix login bug");
    expect(screen.getByRole("textbox", { name: /body/i })).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: /draft/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /create/i })).toBeInTheDocument();
  });

  it("clicking Create calls create_pr with taskId, title, body, draft", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "gh_status") return Promise.resolve({ available: true, authenticated: true });
      if (cmd === "get_pr_status") return Promise.resolve(null);
      if (cmd === "create_pr") return Promise.resolve({ number: 9, url: "https://github.com/foo/bar/pull/9" });
      return Promise.resolve(null);
    });

    render(<PrPanel taskId="task-1" />);

    await waitFor(() => {
      expect(screen.getByRole("textbox", { name: /title/i })).toBeInTheDocument();
    });

    const titleInput = screen.getByRole("textbox", { name: /title/i });
    fireEvent.change(titleInput, { target: { value: "My custom title" } });

    const bodyInput = screen.getByRole("textbox", { name: /body/i });
    fireEvent.change(bodyInput, { target: { value: "My body text" } });

    const draftCheckbox = screen.getByRole("checkbox", { name: /draft/i });
    fireEvent.click(draftCheckbox);

    fireEvent.click(screen.getByRole("button", { name: /create/i }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("create_pr", {
        taskId: "task-1",
        title: "My custom title",
        body: "My body text",
        draft: true,
      });
    });
  });

  // State 4: PR exists
  it("renders PR status, checks, comments, and Open in browser button", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "gh_status") return Promise.resolve({ available: true, authenticated: true });
      if (cmd === "get_pr_status") return Promise.resolve(prStatus);
      if (cmd === "get_pr_comments") return Promise.resolve(prComments);
      return Promise.resolve(null);
    });

    render(<PrPanel taskId="task-1" />);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /open in browser/i })).toBeInTheDocument();
    });

    // State badge (the PR state is "open")
    expect(screen.getByText("open")).toBeInTheDocument();

    // Checks
    expect(screen.getByText("CI / tests")).toBeInTheDocument();
    expect(screen.getByText("CI / lint")).toBeInTheDocument();

    // Comments: issue_comment
    expect(screen.getByText("alice")).toBeInTheDocument();
    expect(screen.getByText("Looks good to me.")).toBeInTheDocument();

    // Inline comment shows path:line
    expect(screen.getByText(/src\/login\.ts/)).toBeInTheDocument();

    // Review comment shows its state (at least one element with "approved" text)
    expect(screen.getByText("carol")).toBeInTheDocument();
    expect(screen.getAllByText(/approved/i).length).toBeGreaterThanOrEqual(1);
  });

  it("clicking Open in browser calls openUrl with the PR url", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "gh_status") return Promise.resolve({ available: true, authenticated: true });
      if (cmd === "get_pr_status") return Promise.resolve(prStatus);
      if (cmd === "get_pr_comments") return Promise.resolve(prComments);
      return Promise.resolve(null);
    });
    openUrlMock.mockResolvedValueOnce(undefined);

    render(<PrPanel taskId="task-1" />);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /open in browser/i })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: /open in browser/i }));

    expect(openUrlMock).toHaveBeenCalledWith("https://github.com/foo/bar/pull/7");
  });

  it("Refresh button re-fetches PR status and comments", async () => {
    let callCount = 0;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "gh_status") return Promise.resolve({ available: true, authenticated: true });
      if (cmd === "get_pr_status") {
        callCount++;
        return Promise.resolve(prStatus);
      }
      if (cmd === "get_pr_comments") return Promise.resolve(prComments);
      return Promise.resolve(null);
    });

    render(<PrPanel taskId="task-1" />);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /refresh/i })).toBeInTheDocument();
    });

    const beforeCount = callCount;
    fireEvent.click(screen.getByRole("button", { name: /refresh/i }));

    await waitFor(() => {
      expect(callCount).toBeGreaterThan(beforeCount);
    });
  });
});
