import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Sidebar } from "./Sidebar";
import { useVigieStore, AGENT_TAB } from "../../store";
import type { Repo, Task } from "../../store";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

const deleteTask = vi.fn().mockResolvedValue(undefined);

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

const repoA: Repo = {
  id: "repo-a",
  name: "repo-a-name",
  path: "/tmp/repo-a",
  defaultBranch: "main",
};

const repoB: Repo = {
  id: "repo-b",
  name: "repo-b-name",
  path: "/tmp/repo-b",
  defaultBranch: "main",
};

const taskA: Task = {
  id: "task-a",
  repoId: "repo-a",
  title: "Task in repo A",
  worktreePath: "/tmp/wt/a",
  branch: "task-in-repo-a",
  baseBranch: "main",
  status: "idle",
  createdAt: 1,
  updatedAt: 1,
};

const taskB: Task = {
  id: "task-b",
  repoId: "repo-b",
  title: "Task in repo B",
  worktreePath: "/tmp/wt/b",
  branch: "task-in-repo-b",
  baseBranch: "main",
  status: "working",
  createdAt: 1,
  updatedAt: 1,
};

describe("Sidebar", () => {
  beforeEach(() => {
    localStorage.clear();
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_agents") return Promise.resolve([]);
      return Promise.resolve({ repos: [], tasks: [] });
    });
    useVigieStore.setState({
      repos: [],
      tasks: [],
      selectedTaskId: null,
      sidebarCollapsed: false,
      sidebarWidth: 260,
    } as any);
  });

  it("renders the Repositories header and an empty-state message when there are no repos", () => {
    render(<Sidebar />);

    expect(
      screen.getByRole("heading", { name: "Repositories" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/no repositories/i)).toBeInTheDocument();
  });

  it("renders each repo with its tasks nested under the correct repo", () => {
    useVigieStore.setState({
      repos: [repoA, repoB],
      tasks: [taskA, taskB],
      selectedTaskId: null,
    });

    render(<Sidebar />);

    expect(screen.getByText("repo-a-name")).toBeInTheDocument();
    expect(screen.getByText("repo-b-name")).toBeInTheDocument();

    const repoAItem = screen.getByText("repo-a-name").closest("li");
    const repoBItem = screen.getByText("repo-b-name").closest("li");

    expect(repoAItem).not.toBeNull();
    expect(repoBItem).not.toBeNull();
    expect(repoAItem?.textContent).toContain("Task in repo A");
    expect(repoAItem?.textContent).not.toContain("Task in repo B");
    expect(repoBItem?.textContent).toContain("Task in repo B");
    expect(repoBItem?.textContent).not.toContain("Task in repo A");
  });

  it('has an "Add repository" button', () => {
    render(<Sidebar />);

    expect(
      screen.getByRole("button", { name: /add repository/i }),
    ).toBeInTheDocument();
  });

  it("shows live activity dot when an agent session has an activity for the task", () => {
    useVigieStore.setState({
      repos: [repoA],
      tasks: [taskA],
      selectedTaskId: null,
      sessionsByTask: {
        "task-a": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "agent-1", activity: "needs_attention" }],
      },
      activeTabByTask: { "task-a": AGENT_TAB },
    });

    render(<Sidebar />);

    // StatusDot renders: aria-label="status: needs_attention"
    const dot = screen.getByRole("status");
    expect(dot).toHaveAttribute("aria-label", "status: needs_attention");
  });

  it("shows task.status dot when the task has no agent session", () => {
    // taskB has status "working", no session entry
    useVigieStore.setState({
      repos: [repoB],
      tasks: [taskB],
      selectedTaskId: null,
      sessionsByTask: {},
      activeTabByTask: {},
    });

    render(<Sidebar />);

    const dot = screen.getByRole("status");
    expect(dot).toHaveAttribute("aria-label", "status: working");
  });

  it("surfaces an error and keeps the form open when create_task fails", async () => {
    useVigieStore.setState({
      repos: [repoA],
      tasks: [],
      selectedTaskId: null,
      sessionsByTask: {},
      activeTabByTask: {},
    });
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_agents") return Promise.resolve([]);
      if (cmd === "create_task") {
        return Promise.reject(
          new Error("failed to create worktree: invalid reference: TST-1"),
        );
      }
      return Promise.resolve({ repos: [repoA], tasks: [] });
    });

    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: /new task/i }));
    fireEvent.change(screen.getByLabelText("Task title"), {
      target: { value: "Repo overview" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create task" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/invalid reference: TST-1/i);
    // The running view stays open so the user can read the error and retry.
    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
  });

  it("can create a task from only a ticket ID (no title)", async () => {
    useVigieStore.setState({ repos: [repoA], tasks: [], selectedTaskId: null, sessionsByTask: {}, activeTabByTask: {} });
    let capturedArgs: any;
    invokeMock.mockImplementation((cmd: string, args: any) => {
      if (cmd === "list_agents") return Promise.resolve([]);
      if (cmd === "create_task") {
        capturedArgs = args;
        return new Promise(() => {}); // never resolves; we just want to inspect the call
      }
      return Promise.resolve({ repos: [repoA], tasks: [] });
    });

    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: /new task/i }));
    // Type only into the Ticket ID field; leave Task title empty
    fireEvent.change(screen.getByLabelText("Ticket ID (optional)"), { target: { value: "TST-1" } });
    fireEvent.click(screen.getByRole("button", { name: "Create task" }));

    await waitFor(() => expect(capturedArgs).toBeDefined());
    expect(capturedArgs.repoId).toBe("repo-a");
    expect(capturedArgs.title).toBe("");
    expect(capturedArgs.ticketKey).toBe("TST-1");
    expect(capturedArgs.baseBranch).toBeNull();
  });

  it("shows creating status and selects the task on success", async () => {
    useVigieStore.setState({ repos: [repoA], tasks: [], selectedTaskId: null, sessionsByTask: {}, activeTabByTask: {} });
    let capturedResolve: ((v: unknown) => void) | null = null;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_agents") return Promise.resolve([]);
      if (cmd === "create_task") {
        // Return a pending promise so we can observe the running phase
        return new Promise((resolve) => {
          capturedResolve = resolve;
        });
      }
      return Promise.resolve({ repos: [repoA], tasks: [] });
    });

    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: /new task/i }));
    fireEvent.change(screen.getByLabelText("Task title"), { target: { value: "Repo overview" } });
    fireEvent.click(screen.getByRole("button", { name: "Create task" }));

    // "Creating" should appear once the running phase is entered
    expect(await screen.findByText(/creating/i)).toBeInTheDocument();

    // Resolve the task and let the success path complete
    capturedResolve!({
      id: "new-task", repoId: "repo-a", title: "Repo overview",
      worktreePath: "/wt", branch: "repo-overview", baseBranch: "main",
      status: "idle", createdAt: 1, updatedAt: 1,
    });

    await waitFor(() => expect(useVigieStore.getState().selectedTaskId).toBe("new-task"));
  });

  it("shows an error and a Retry control when setup/create fails", async () => {
    useVigieStore.setState({ repos: [repoA], tasks: [], selectedTaskId: null, sessionsByTask: {}, activeTabByTask: {} });
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_agents") return Promise.resolve([]);
      if (cmd === "create_task") return Promise.reject(new Error("setup script failed (exit 1)"));
      return Promise.resolve({ repos: [repoA], tasks: [] });
    });

    render(<Sidebar />);
    fireEvent.click(screen.getByRole("button", { name: /new task/i }));
    fireEvent.change(screen.getByLabelText("Task title"), { target: { value: "Repo overview" } });
    fireEvent.click(screen.getByRole("button", { name: "Create task" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(/exit 1/i);
    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
  });

  it("shows the ticket key and title for a keyed task, and finds it by key in search", async () => {
    const keyedTask: Task = { ...taskA, id: "task-keyed", ticketKey: "TST-1", title: "Fix login" };
    useVigieStore.setState({
      repos: [repoA],
      tasks: [keyedTask],
      selectedTaskId: null,
      sessionsByTask: {},
      activeTabByTask: {},
    });
    render(<Sidebar />);

    expect(screen.getByText("TST-1")).toBeInTheDocument();
    expect(screen.getByText("Fix login")).toBeInTheDocument();

    fireEvent.change(screen.getByPlaceholderText(/search tasks/i), { target: { value: "TST-1" } });
    expect(screen.getByText("Fix login")).toBeInTheDocument();
  });

  it("shows just the key as the name for a key-only task (no duplicate)", () => {
    const keyOnlyTask: Task = { ...taskA, id: "task-key-only", ticketKey: "TST-2", title: "" };
    useVigieStore.setState({
      repos: [repoA],
      tasks: [keyOnlyTask],
      selectedTaskId: null,
      sessionsByTask: {},
      activeTabByTask: {},
    });
    render(<Sidebar />);

    expect(screen.getAllByText("TST-2")).toHaveLength(1);
  });

  describe("collapsible sidebar", () => {
    it("starts expanded and shows the collapse toggle button", () => {
      render(<Sidebar />);

      expect(
        screen.getByRole("heading", { name: "Repositories" }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /add repository/i }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /collapse sidebar/i }),
      ).toBeInTheDocument();
    });

    it("clicking collapse hides repo content and shows expand toggle", () => {
      render(<Sidebar />);

      fireEvent.click(
        screen.getByRole("button", { name: /collapse sidebar/i }),
      );

      expect(
        screen.queryByRole("heading", { name: "Repositories" }),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: /add repository/i }),
      ).not.toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /expand sidebar/i }),
      ).toBeInTheDocument();
    });

    it("clicking expand after collapse restores the repo content", () => {
      render(<Sidebar />);

      fireEvent.click(
        screen.getByRole("button", { name: /collapse sidebar/i }),
      );
      fireEvent.click(
        screen.getByRole("button", { name: /expand sidebar/i }),
      );

      expect(
        screen.getByRole("heading", { name: "Repositories" }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /add repository/i }),
      ).toBeInTheDocument();
    });

    it("persists collapsed state to localStorage", () => {
      render(<Sidebar />);

      expect(
        localStorage.getItem("vigie.sidebarCollapsed"),
      ).not.toBe("true");

      fireEvent.click(
        screen.getByRole("button", { name: /collapse sidebar/i }),
      );

      expect(localStorage.getItem("vigie.sidebarCollapsed")).toBe("true");
    });

    it("starts collapsed when store has sidebarCollapsed=true", () => {
      // The store now owns collapse state; set it directly (simulates what the
      // localStorage-initialised store would do on first load).
      useVigieStore.setState({
        sidebarCollapsed: true,
      } as any);

      render(<Sidebar />);

      expect(
        screen.queryByRole("heading", { name: "Repositories" }),
      ).not.toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /expand sidebar/i }),
      ).toBeInTheDocument();
    });

    it("updates localStorage to false when expanding", () => {
      useVigieStore.setState({
        sidebarCollapsed: true,
      } as any);
      localStorage.setItem("vigie.sidebarCollapsed", "true");

      render(<Sidebar />);

      fireEvent.click(
        screen.getByRole("button", { name: /expand sidebar/i }),
      );

      expect(localStorage.getItem("vigie.sidebarCollapsed")).toBe("false");
    });
  });

  describe("task-row context menu", () => {
    beforeEach(() => {
      vi.clearAllMocks();
      useVigieStore.setState({
        repos: [{ id: "r1", name: "demo", path: "/r", defaultBranch: "main" } as never],
        tasks: [{ id: "t1", repoId: "r1", title: "Fix login", branch: "b1", status: "idle" } as never],
        selectedTaskId: null,
        sessionsByTask: {},
        attentionByTask: {},
        setupByTask: {},
        deleteTask,
      } as never);
    });

    it("right-clicking a task row opens a context menu with Delete", () => {
      render(<Sidebar />);
      fireEvent.contextMenu(screen.getByText("Fix login"));
      expect(screen.getByRole("menuitem", { name: "Delete" })).toBeTruthy();
    });

    it("choosing Delete opens the confirm modal; confirming calls deleteTask", async () => {
      render(<Sidebar />);
      fireEvent.contextMenu(screen.getByText("Fix login"));
      fireEvent.click(screen.getByRole("menuitem", { name: "Delete" }));
      expect(screen.getByRole("dialog")).toBeTruthy();
      fireEvent.click(screen.getByRole("button", { name: "Delete" }));
      expect(deleteTask).toHaveBeenCalledWith("t1", false);
    });
  });

  describe("Hidden tasks", () => {
    const hideTask = vi.fn().mockResolvedValue(undefined);
    const reopenTask = vi.fn().mockResolvedValue(undefined);

    beforeEach(() => {
      vi.clearAllMocks();
      useVigieStore.setState({
        repos: [repoA],
        tasks: [],
        selectedTaskId: null,
        sessionsByTask: {},
        attentionByTask: {},
        setupByTask: {},
        hideTask,
        reopenTask,
      } as never);
    });

    it("filters out hidden tasks from active list", () => {
      const hiddenTask: Task = {
        ...taskA,
        id: "hidden-1",
        title: "Hidden Task",
        hidden: true,
      };
      const visibleTask: Task = {
        ...taskA,
        id: "visible-1",
        title: "Visible Task",
        hidden: false,
      };

      useVigieStore.setState({
        repos: [repoA],
        tasks: [hiddenTask, visibleTask],
        selectedTaskId: null,
        sessionsByTask: {},
        attentionByTask: {},
        setupByTask: {},
        hideTask,
        reopenTask,
      } as never);

      render(<Sidebar />);

      expect(screen.getByText("Visible Task")).toBeInTheDocument();
      expect(screen.queryByText("Hidden Task")).not.toBeInTheDocument();
    });

    it("renders Hidden section with hidden tasks", () => {
      const hiddenTask: Task = {
        ...taskA,
        id: "hidden-1",
        title: "Hidden Task",
        hidden: true,
      };

      useVigieStore.setState({
        repos: [repoA],
        tasks: [hiddenTask],
        selectedTaskId: null,
        sessionsByTask: {},
        attentionByTask: {},
        setupByTask: {},
        hideTask,
        reopenTask,
      } as never);

      render(<Sidebar />);

      expect(screen.getByText(/↑ 1 hidden/)).toBeInTheDocument();
      fireEvent.click(screen.getByText(/↑ 1 hidden/));
      expect(screen.getByText("Hidden Task")).toBeInTheDocument();
    });

    it("shows Hide action in context menu for active tasks", () => {
      useVigieStore.setState({
        repos: [repoA],
        tasks: [taskA],
        selectedTaskId: null,
        sessionsByTask: {},
        attentionByTask: {},
        setupByTask: {},
        hideTask,
        reopenTask,
      } as never);

      render(<Sidebar />);
      fireEvent.contextMenu(screen.getByText("Task in repo A"));
      expect(screen.getByRole("menuitem", { name: "Hide" })).toBeInTheDocument();
    });

    it("shows Reopen action in context menu for hidden tasks", () => {
      const hiddenTask: Task = {
        ...taskA,
        id: "hidden-1",
        title: "Hidden Task",
        hidden: true,
      };

      useVigieStore.setState({
        repos: [repoA],
        tasks: [hiddenTask],
        selectedTaskId: null,
        sessionsByTask: {},
        attentionByTask: {},
        setupByTask: {},
        hideTask,
        reopenTask,
      } as never);

      render(<Sidebar />);
      fireEvent.click(screen.getByText(/↑ 1 hidden/));
      fireEvent.contextMenu(screen.getByText("Hidden Task"));
      expect(screen.getByRole("menuitem", { name: "Reopen" })).toBeInTheDocument();
    });

    it("clicking Hide action calls hideTask", () => {
      useVigieStore.setState({
        repos: [repoA],
        tasks: [taskA],
        selectedTaskId: null,
        sessionsByTask: {},
        attentionByTask: {},
        setupByTask: {},
        hideTask,
        reopenTask,
      } as never);

      render(<Sidebar />);
      fireEvent.contextMenu(screen.getByText("Task in repo A"));
      fireEvent.click(screen.getByRole("menuitem", { name: "Hide" }));
      expect(hideTask).toHaveBeenCalledWith("task-a");
    });

    it("clicking Reopen action calls reopenTask", () => {
      const hiddenTask: Task = {
        ...taskA,
        id: "hidden-1",
        title: "Hidden Task",
        hidden: true,
      };

      useVigieStore.setState({
        repos: [repoA],
        tasks: [hiddenTask],
        selectedTaskId: null,
        sessionsByTask: {},
        attentionByTask: {},
        setupByTask: {},
        hideTask,
        reopenTask,
      } as never);

      render(<Sidebar />);
      fireEvent.click(screen.getByText(/↑ 1 hidden/));
      fireEvent.contextMenu(screen.getByText("Hidden Task"));
      fireEvent.click(screen.getByRole("menuitem", { name: "Reopen" }));
      expect(reopenTask).toHaveBeenCalledWith("hidden-1");
    });

    it("clicking a hidden task selects it but does not reopen it", () => {
      const hiddenTask: Task = {
        ...taskA,
        id: "hidden-1",
        title: "Hidden Task",
        hidden: true,
      };
      const setSelectedTask = vi.fn();

      useVigieStore.setState({
        repos: [repoA],
        tasks: [hiddenTask],
        selectedTaskId: null,
        sessionsByTask: {},
        attentionByTask: {},
        setupByTask: {},
        hideTask,
        reopenTask,
        setSelectedTask,
      } as never);

      render(<Sidebar />);
      fireEvent.click(screen.getByText(/↑ 1 hidden/));
      fireEvent.click(screen.getByText("Hidden Task"));
      expect(setSelectedTask).toHaveBeenCalledWith("hidden-1");
      expect(reopenTask).not.toHaveBeenCalled();
    });
  });
});
