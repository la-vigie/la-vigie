import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FinishTaskModal } from "./FinishTaskModal";
import { useVigieStore } from "../../store";
import type { Task } from "../../store";

const { invokeMock, stopSession } = vi.hoisted(() => {
  const invokeMock = vi.fn();
  // Route stopSession through invokeMock so we can assert stop→finish ordering.
  const stopSession = vi.fn((sessionId: string) => invokeMock("stop_session", { sessionId }));
  return { invokeMock, stopSession };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  Channel: class {
    onmessage: ((event: unknown) => void) | null = null;
  },
}));

vi.mock("../../api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api")>();
  return { ...actual, stopSession };
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

const openPr = {
  number: 7,
  url: "https://github.com/foo/bar/pull/7",
  title: "Fix login bug",
  state: "OPEN",
  isDraft: false,
  mergeable: "MERGEABLE",
  reviewDecision: null,
  checks: [],
};

// Resolve the modal's mount-time context fetches (PR + changed files) plus any
// finish/stop/refresh calls. Per-test overrides layer on top via mockImplementation.
function defaultInvoke(prResult: unknown = null, changed: unknown[] = []) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "get_pr_status") return Promise.resolve(prResult);
    if (cmd === "get_changed_files") return Promise.resolve(changed);
    return Promise.resolve(undefined);
  });
}

beforeEach(() => {
  invokeMock.mockReset();
  stopSession.mockClear();
  stopSession.mockImplementation((sessionId: string) => invokeMock("stop_session", { sessionId }));
  defaultInvoke();
  useVigieStore.setState({
    tasks: [task],
    selectedTaskId: "task-1",
    sessionsByTask: {},
    activeTabByTask: {},
  });
});

describe("FinishTaskModal — context surfacing", () => {
  it("shows branch, base, and (once loaded) a clean working tree", async () => {
    render(<FinishTaskModal task={task} onClose={vi.fn()} />);
    expect(screen.getByRole("dialog", { name: /finish/i })).toBeInTheDocument();
    expect(screen.getByText("fix-login-bug")).toBeInTheDocument();
    expect(screen.getByText("main")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText(/working tree clean/i)).toBeInTheDocument());
  });

  it("surfaces uncommitted-change count as text", async () => {
    defaultInvoke(null, [{ path: "a" }, { path: "b" }]);
    render(<FinishTaskModal task={task} onClose={vi.fn()} />);
    await waitFor(() =>
      expect(screen.getByText(/2 uncommitted changes/i)).toBeInTheDocument(),
    );
  });

  it("shows PR '#7 open' and a primary Merge action for an OPEN PR", async () => {
    defaultInvoke(openPr, []);
    render(<FinishTaskModal task={task} onClose={vi.fn()} />);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /merge pr & finish/i })).toBeInTheDocument(),
    );
    expect(screen.getByText("#7 open")).toBeInTheDocument();
  });

  it("shows PR 'none' and no Merge action when there is no PR", async () => {
    render(<FinishTaskModal task={task} onClose={vi.fn()} />);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_pr_status", { taskId: "task-1" }));
    expect(screen.getByText("none")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /merge pr & finish/i })).not.toBeInTheDocument();
  });
});

describe("FinishTaskModal — guarded Discard", () => {
  it("Discard is not actionable until armed; arming reveals a red confirm", async () => {
    render(<FinishTaskModal task={task} onClose={vi.fn()} />);
    // No confirm button yet — only the de-emphasized arm affordance.
    expect(screen.queryByRole("button", { name: /^discard fix-login-bug$/i })).not.toBeInTheDocument();
    const arm = screen.getByRole("button", { name: /discard branch instead/i });

    fireEvent.click(arm);

    expect(screen.getByRole("button", { name: /^discard fix-login-bug$/i })).toBeInTheDocument();
    // finish_task must NOT have fired just from arming.
    expect(invokeMock.mock.calls.filter((c) => c[0] === "finish_task")).toHaveLength(0);
  });

  it("Back un-arms the confirm without finishing", () => {
    render(<FinishTaskModal task={task} onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /discard branch instead/i }));
    fireEvent.click(screen.getByRole("button", { name: /^back$/i }));
    expect(screen.queryByRole("button", { name: /^discard fix-login-bug$/i })).not.toBeInTheDocument();
    expect(invokeMock.mock.calls.filter((c) => c[0] === "finish_task")).toHaveLength(0);
  });

  it("confirming Discard with a running agent stops it, then finish_task discard, clears selection", async () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {
        "task-1": [{ localId: "agent", kind: "agent", status: "running", title: "Claude", backendId: "agent-99" }],
      },
      activeTabByTask: { "task-1": "agent" },
    });
    const onClose = vi.fn();
    render(<FinishTaskModal task={task} onClose={onClose} />);

    fireEvent.click(screen.getByRole("button", { name: /discard branch instead/i }));
    fireEvent.click(screen.getByRole("button", { name: /^discard fix-login-bug$/i }));

    await waitFor(() => {
      expect(useVigieStore.getState().selectedTaskId).toBeNull();
      expect(invokeMock).toHaveBeenCalledWith("stop_session", { sessionId: "agent-99" });
      expect(invokeMock).toHaveBeenCalledWith("finish_task", { taskId: "task-1", mode: "discard" });
    });
    const calls = invokeMock.mock.calls.map((c) => c[0]);
    expect(calls.indexOf("stop_session")).toBeLessThan(calls.indexOf("finish_task"));
    expect(onClose).toHaveBeenCalled();
  });

  it("in-place task hides the entire Discard danger zone", () => {
    render(<FinishTaskModal task={{ ...task, inPlace: true }} onClose={vi.fn()} />);
    expect(screen.queryByRole("button", { name: /discard branch instead/i })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /keep branch/i })).toBeInTheDocument();
  });
});

describe("FinishTaskModal — Keep / Merge / dismiss", () => {
  it("Keep branch (no agent) calls finish_task keep and no stop_session", async () => {
    render(<FinishTaskModal task={task} onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /keep branch/i }));
    await waitFor(() => expect(useVigieStore.getState().selectedTaskId).toBeNull());
    expect(invokeMock).toHaveBeenCalledWith("finish_task", { taskId: "task-1", mode: "keep" });
    expect(invokeMock.mock.calls.filter((c) => c[0] === "stop_session")).toHaveLength(0);
  });

  it("Keep stops BOTH the agent and shell backend sessions before finishing", async () => {
    useVigieStore.setState({
      tasks: [task],
      selectedTaskId: "task-1",
      sessionsByTask: {
        "task-1": [
          { localId: "agent", kind: "agent", status: "running", title: "Claude", backendId: "agent-b" },
          { localId: "shell-1", kind: "shell", status: "running", title: "shell", backendId: "shell-b" },
        ],
      },
      activeTabByTask: { "task-1": "agent" },
    });
    render(<FinishTaskModal task={task} onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /keep branch/i }));

    await waitFor(() => expect(useVigieStore.getState().selectedTaskId).toBeNull());
    expect(invokeMock).toHaveBeenCalledWith("stop_session", { sessionId: "agent-b" });
    expect(invokeMock).toHaveBeenCalledWith("stop_session", { sessionId: "shell-b" });
    // Both stops precede finish_task.
    const calls = invokeMock.mock.calls.map((c) => c[0]);
    const lastStop = Math.max(...calls.reduce((acc: number[], c, i) => (c === "stop_session" ? [...acc, i] : acc), []));
    expect(lastStop).toBeLessThan(calls.indexOf("finish_task"));
  });

  it("Merge PR & finish calls finish_task merge", async () => {
    defaultInvoke(openPr, []);
    render(<FinishTaskModal task={task} onClose={vi.fn()} />);
    const merge = await screen.findByRole("button", { name: /merge pr & finish/i });
    fireEvent.click(merge);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("finish_task", { taskId: "task-1", mode: "merge" }),
    );
  });

  it("a finish_task rejection shows an error and keeps the selection", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_pr_status") return Promise.resolve(null);
      if (cmd === "get_changed_files") return Promise.resolve([]);
      if (cmd === "finish_task") return Promise.reject(new Error("worktree busy"));
      return Promise.resolve(undefined);
    });
    render(<FinishTaskModal task={task} onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /keep branch/i }));
    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent("worktree busy");
    });
    expect(useVigieStore.getState().selectedTaskId).toBe("task-1");
  });

  it("Cancel closes without finishing", () => {
    const onClose = vi.fn();
    render(<FinishTaskModal task={task} onClose={onClose} />);
    fireEvent.click(screen.getByRole("button", { name: /^cancel$/i }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(invokeMock.mock.calls.filter((c) => c[0] === "finish_task")).toHaveLength(0);
  });

  it("Escape closes without finishing", () => {
    const onClose = vi.fn();
    render(<FinishTaskModal task={task} onClose={onClose} />);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
