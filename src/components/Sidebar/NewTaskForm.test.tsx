import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NewTaskForm } from "./Sidebar";
import { useVigieStore } from "../../store";
import type { AgentSpec, Repo } from "../../store";
import type { Schedule } from "../../api";
import * as agentHooks from "../../hooks/useAgents";

vi.mock("../../hooks/useAgents");

const { createTaskMock, checkWorktreePathMock, createOneShotScheduleMock } = vi.hoisted(() => ({
  createTaskMock: vi.fn(),
  checkWorktreePathMock: vi.fn(),
  createOneShotScheduleMock: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../api", () => ({
  createTask: createTaskMock,
  checkWorktreePath: checkWorktreePathMock,
  addRepo: vi.fn(),
  createOneShotSchedule: createOneShotScheduleMock,
}));

// Default safe hook mocks.
beforeEach(() => {
  (agentHooks.useAgents as ReturnType<typeof vi.fn>).mockReturnValue({ agents: [], loading: false, error: null });
  (agentHooks.useAgentModels as ReturnType<typeof vi.fn>).mockReturnValue({ models: [], loading: false });
  // The worktree-path check runs on a debounce as inputs change; default it to
  // "vacant" (no warning) so existing tests are unaffected.
  checkWorktreePathMock.mockResolvedValue({ state: "vacant", path: "", message: null });
});

const repo = (over: Partial<Repo> = {}): Repo => ({
  id: "r1",
  name: "r",
  path: "/r",
  defaultBranch: "main",
  autoStartAgent: false,
  initialPrompt: null,
  inPlaceDefault: false,
  ...over,
});

afterEach(() => {
  vi.clearAllMocks();
  // Re-establish default hook mocks so later tests don't see stale values.
  (agentHooks.useAgents as ReturnType<typeof vi.fn>).mockReturnValue({ agents: [], loading: false, error: null });
  (agentHooks.useAgentModels as ReturnType<typeof vi.fn>).mockReturnValue({ models: [], loading: false });
  useVigieStore.setState({
    repos: [],
    tasks: [],
    selectedTaskId: null,
    sessionsByTask: {},
    activeTabByTask: {},
    attentionByTask: {},
  });
});

describe("NewTaskForm auto-launch", () => {
  const seedSpies = () => {
    const startSpy = vi.fn();
    useVigieStore.setState({
      startAgentSession: startSpy,
      refresh: vi.fn(),
      setSelectedTask: vi.fn(),
    });
    return startSpy;
  };

  it("auto-launches with the combined prompt when the start checkbox defaults on (repo default on)", async () => {
    createTaskMock.mockResolvedValue({ id: "task-1" });
    const startSpy = seedSpies();
    render(
      <NewTaskForm
        repo={repo({ autoStartAgent: true, initialPrompt: "repo ctx" })}
        onClose={vi.fn()}
      />,
    );
    await userEvent.type(screen.getByLabelText("Task title"), "build X");
    await userEvent.type(screen.getByLabelText("Initial prompt"), "task text");
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));
    await waitFor(() =>
      expect(startSpy).toHaveBeenCalledWith(
        "task-1",
        false,
        undefined,
        "repo ctx\n\ntask text",
      ),
    );
  });

  it("does not auto-launch when the start checkbox defaults off (repo default off)", async () => {
    createTaskMock.mockResolvedValue({ id: "task-2" });
    const startSpy = seedSpies();
    render(
      <NewTaskForm repo={repo({ autoStartAgent: false })} onClose={vi.fn()} />,
    );
    await userEvent.type(screen.getByLabelText("Task title"), "build Y");
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));
    await waitFor(() => expect(createTaskMock).toHaveBeenCalled());
    expect(startSpy).not.toHaveBeenCalled();
  });

  it("launches when the user ticks the checkbox even though the repo default is off", async () => {
    createTaskMock.mockResolvedValue({ id: "task-3" });
    const startSpy = seedSpies();
    render(
      <NewTaskForm repo={repo({ autoStartAgent: false })} onClose={vi.fn()} />,
    );
    await userEvent.type(screen.getByLabelText("Task title"), "build Z");
    await userEvent.type(screen.getByLabelText("Initial prompt"), "go");
    await userEvent.click(screen.getByLabelText("Start the agent immediately"));
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));
    await waitFor(() =>
      expect(startSpy).toHaveBeenCalledWith("task-3", false, undefined, "go"),
    );
  });

  it("does not launch when the user unticks the checkbox even though the repo default is on", async () => {
    createTaskMock.mockResolvedValue({ id: "task-4" });
    const startSpy = seedSpies();
    render(
      <NewTaskForm repo={repo({ autoStartAgent: true })} onClose={vi.fn()} />,
    );
    await userEvent.type(screen.getByLabelText("Task title"), "build Q");
    await userEvent.click(screen.getByLabelText("Start the agent immediately"));
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));
    await waitFor(() => expect(createTaskMock).toHaveBeenCalled());
    expect(startSpy).not.toHaveBeenCalled();
  });
});

describe("NewTaskForm skip repo prompt (TASK-160)", () => {
  const skipLabel = "Skip the repository prompt for this task";
  const seedSpies = () => {
    const startSpy = vi.fn();
    useVigieStore.setState({
      startAgentSession: startSpy,
      refresh: vi.fn(),
      setSelectedTask: vi.fn(),
    });
    return startSpy;
  };

  it("hides the skip toggle when the repo has no initialPrompt", () => {
    seedSpies();
    render(<NewTaskForm repo={repo({ initialPrompt: null })} onClose={vi.fn()} />);
    expect(screen.queryByLabelText(skipLabel)).toBeNull();
  });

  it("hides the skip toggle when the repo's initialPrompt is only whitespace", () => {
    seedSpies();
    render(<NewTaskForm repo={repo({ initialPrompt: "   " })} onClose={vi.fn()} />);
    expect(screen.queryByLabelText(skipLabel)).toBeNull();
  });

  it("shows the skip toggle when the repo has a non-empty initialPrompt", () => {
    seedSpies();
    render(<NewTaskForm repo={repo({ initialPrompt: "repo ctx" })} onClose={vi.fn()} />);
    expect(screen.getByLabelText(skipLabel)).toBeTruthy();
  });

  it("launches with only the task prompt when the skip toggle is ticked", async () => {
    createTaskMock.mockResolvedValue({ id: "task-skip" });
    const startSpy = seedSpies();
    render(
      <NewTaskForm
        repo={repo({ autoStartAgent: true, initialPrompt: "repo ctx" })}
        onClose={vi.fn()}
      />,
    );
    await userEvent.type(screen.getByLabelText("Task title"), "build X");
    await userEvent.type(screen.getByLabelText("Initial prompt"), "task text");
    await userEvent.click(screen.getByLabelText(skipLabel));
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));
    await waitFor(() =>
      expect(startSpy).toHaveBeenCalledWith("task-skip", false, undefined, "task text"),
    );
  });

  it("launches a bare agent when skip is ticked and the task prompt is empty", async () => {
    createTaskMock.mockResolvedValue({ id: "task-bare" });
    const startSpy = seedSpies();
    render(
      <NewTaskForm
        repo={repo({ autoStartAgent: true, initialPrompt: "repo ctx" })}
        onClose={vi.fn()}
      />,
    );
    await userEvent.type(screen.getByLabelText("Task title"), "build X");
    await userEvent.click(screen.getByLabelText(skipLabel));
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));
    await waitFor(() =>
      expect(startSpy).toHaveBeenCalledWith("task-bare", false, undefined, undefined),
    );
  });

  it("keeps prepending the repo prompt when the skip toggle is left off (default)", async () => {
    createTaskMock.mockResolvedValue({ id: "task-noskip" });
    const startSpy = seedSpies();
    render(
      <NewTaskForm
        repo={repo({ autoStartAgent: true, initialPrompt: "repo ctx" })}
        onClose={vi.fn()}
      />,
    );
    await userEvent.type(screen.getByLabelText("Task title"), "build X");
    await userEvent.type(screen.getByLabelText("Initial prompt"), "task text");
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));
    await waitFor(() =>
      expect(startSpy).toHaveBeenCalledWith(
        "task-noskip",
        false,
        undefined,
        "repo ctx\n\ntask text",
      ),
    );
  });
});

describe("NewTaskForm worktree-path warning (TASK-125)", () => {
  beforeEach(() => {
    useVigieStore.setState({ refresh: vi.fn(), setSelectedTask: vi.fn(), startAgentSession: vi.fn() });
  });

  it("shows an adopt notice when the derived worktree path already exists", async () => {
    checkWorktreePathMock.mockResolvedValue({
      state: "adopt",
      path: "/wt/r1/build-x",
      message: "A worktree already exists at /wt/r1/build-x — it will be reused.",
    });
    render(<NewTaskForm repo={repo()} onClose={vi.fn()} />);
    await userEvent.type(screen.getByLabelText("Task title"), "build X");
    await waitFor(() =>
      expect(screen.getByText(/it will be reused/i)).toBeTruthy(),
    );
    // The check is keyed on the current repo + trimmed inputs.
    expect(checkWorktreePathMock).toHaveBeenCalledWith("r1", "build X", undefined, undefined);
  });

  it("shows a conflict warning when the path is occupied by a mismatch", async () => {
    checkWorktreePathMock.mockResolvedValue({
      state: "conflict",
      path: "/wt/r1/build-y",
      message: "a directory already exists at /wt/r1/build-y but is not a git worktree",
    });
    render(<NewTaskForm repo={repo()} onClose={vi.fn()} />);
    await userEvent.type(screen.getByLabelText("Task title"), "build Y");
    await waitFor(() =>
      expect(screen.getByText(/is not a git worktree/i)).toBeTruthy(),
    );
  });

  it("shows a reclaim notice for a leftover/orphaned worktree", async () => {
    checkWorktreePathMock.mockResolvedValue({
      state: "reclaim",
      path: "/wt/r1/testing",
      message: "a leftover worktree already exists at /wt/r1/testing — it will be cleaned up and recreated",
    });
    render(<NewTaskForm repo={repo()} onClose={vi.fn()} />);
    await userEvent.type(screen.getByLabelText("Task title"), "Testing");
    await waitFor(() =>
      expect(screen.getByText(/cleaned up and recreated/i)).toBeTruthy(),
    );
    // Reclaim is benign (auto-resolves), so it is NOT styled as a conflict.
    expect(
      screen.getByRole("status").className.includes("new-task-form__worktree-warning--conflict"),
    ).toBe(false);
  });

  it("shows a reuse-branch notice when the branch already exists", async () => {
    checkWorktreePathMock.mockResolvedValue({
      state: "reuse-branch",
      path: "/wt/r1/testing",
      message: "Branch testing already exists — its commits will be reused.",
    });
    render(<NewTaskForm repo={repo()} onClose={vi.fn()} />);
    await userEvent.type(screen.getByLabelText("Task title"), "Testing");
    await waitFor(() =>
      expect(screen.getByText(/its commits will be reused/i)).toBeTruthy(),
    );
  });

  it("shows nothing when the path is vacant", async () => {
    checkWorktreePathMock.mockResolvedValue({ state: "vacant", path: "/wt/r1/z", message: null });
    render(<NewTaskForm repo={repo()} onClose={vi.fn()} />);
    await userEvent.type(screen.getByLabelText("Task title"), "build Z");
    await waitFor(() => expect(checkWorktreePathMock).toHaveBeenCalled());
    expect(screen.queryByRole("status")).toBeNull();
  });
});

describe("NewTaskForm in-place mode (TASK-163)", () => {
  beforeEach(() => {
    useVigieStore.setState({ refresh: vi.fn(), setSelectedTask: vi.fn(), startAgentSession: vi.fn() });
  });

  it("in-place checkbox passes inPlace + branchName and hides the worktree preview", async () => {
    createTaskMock.mockResolvedValue({ id: "t1" });

    render(<NewTaskForm repo={repo({ inPlaceDefault: false, initialPrompt: "" })} onClose={vi.fn()} />);

    await userEvent.type(screen.getByLabelText("Task title"), "Fix infra");
    await userEvent.click(screen.getByLabelText(/Work in place/i));
    await userEvent.type(screen.getByLabelText(/New branch name/i), "hotfix");
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));

    await waitFor(() => expect(createTaskMock).toHaveBeenCalled());
    const args = createTaskMock.mock.calls[0];
    // createTask(repoId, title, baseBranch, ticketKey, agent, model, autoApprove, inPlace, branchName)
    expect(args[7]).toBe(true); // inPlace
    expect(args[8]).toBe("hotfix"); // branchName
    // No worktree preview banner while in-place.
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("checking 'Start later' clears and disables in-place, hiding the branch-name field", async () => {
    render(<NewTaskForm repo={repo({ inPlaceDefault: false, initialPrompt: "" })} onClose={vi.fn()} />);

    await userEvent.type(screen.getByLabelText("Task title"), "Fix infra");
    await userEvent.click(screen.getByLabelText(/Work in place/i));
    expect(screen.getByLabelText(/New branch name/i)).toBeTruthy();

    await userEvent.click(screen.getByLabelText("Start the agent later"));

    // In-place is cleared and disabled, and its branch-name field is hidden.
    expect(screen.getByLabelText(/Work in place/i)).not.toBeChecked();
    expect(screen.getByLabelText(/Work in place/i)).toBeDisabled();
    expect(screen.queryByLabelText(/New branch name/i)).toBeNull();
  });
});

describe("NewTaskForm prompt library picker", () => {
  it("closes the Library dropdown after selecting a prompt (a wrapping <label> must not re-toggle it)", async () => {
    useVigieStore.setState({
      prompts: [{ id: "p1", label: "Testing", body: "no need to do anything", position: 0 }],
      openSettings: vi.fn(),
      refresh: vi.fn(),
      setSelectedTask: vi.fn(),
      startAgentSession: vi.fn(),
    });
    render(<NewTaskForm repo={repo()} onClose={vi.fn()} />);

    // Structural guard (the browser-only failure jsdom can't simulate): if the
    // Library button sits inside a <label>, clicks in that label get forwarded
    // to it (first labelable descendant), re-opening the menu after selection.
    expect(screen.getByRole("button", { name: /library/i }).closest("label")).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: /library/i }));
    expect(screen.getByRole("menu")).toBeTruthy();

    await userEvent.click(screen.getByText("Testing"));

    // The prompt body is inserted into the textarea...
    expect((screen.getByLabelText("Initial prompt") as HTMLTextAreaElement).value).toContain(
      "no need to do anything",
    );
    // ...and the dropdown is dismissed. Regression guard: the Prompt group must
    // not be a <label>, or the click gets forwarded to its first labelable
    // descendant (the Library button), re-opening the menu.
    expect(screen.queryByRole("menu")).toBeNull();
  });
});

describe("NewTaskForm agent picker (TASK-21)", () => {
  const agentFixtures: AgentSpec[] = [
    {
      name: "claude",
      displayName: "Claude Code",
      status: "claudeHooks",
      binary: "claude",
      baseArgs: [],
      resumeArgs: [],
      extraArgs: [],
      promptMode: "stdin",
      builtin: true,
    },
    {
      name: "antigravity",
      displayName: "Antigravity",
      status: "lifecycle",
      binary: "antigravity",
      baseArgs: [],
      resumeArgs: ["--resume"],
      extraArgs: [],
      promptMode: "stdin",
      builtin: true,
    },
  ];

  const seedSpies = () => {
    const startSpy = vi.fn();
    useVigieStore.setState({
      startAgentSession: startSpy,
      refresh: vi.fn(),
      setSelectedTask: vi.fn(),
    });
    return startSpy;
  };

  beforeEach(() => {
    // Provide agents to AgentModelPicker (and NewTaskForm's selectedAgent lookup) via hook mock.
    (agentHooks.useAgents as ReturnType<typeof vi.fn>).mockReturnValue({ agents: agentFixtures, loading: false, error: null });
    (agentHooks.useAgentModels as ReturnType<typeof vi.fn>).mockReturnValue({ models: [], loading: false });
  });

  it("seeds the picker from repo.defaultAgent ?? 'claude'", () => {
    seedSpies();
    render(
      <NewTaskForm repo={repo({ defaultAgent: "antigravity" })} onClose={vi.fn()} />,
    );
    // AgentModelPicker trigger shows the displayName of the current agent.
    expect(screen.getByTestId("amp-trigger")).toHaveTextContent("Antigravity");
  });

  it("falls back to 'claude' when repo.defaultAgent is not set", () => {
    seedSpies();
    render(<NewTaskForm repo={repo()} onClose={vi.fn()} />);
    expect(screen.getByTestId("amp-trigger")).toHaveTextContent("Claude Code");
  });

  it("passes the selected agent and model to createTask", async () => {
    createTaskMock.mockResolvedValue({ id: "task-agent" });
    seedSpies();
    render(<NewTaskForm repo={repo({ defaultAgent: null })} onClose={vi.fn()} />);
    await userEvent.type(screen.getByLabelText("Task title"), "build X");
    // Open picker and select Antigravity (no modelsListArgs → onChange fires immediately with null model).
    await userEvent.click(screen.getByTestId("amp-trigger"));
    await userEvent.click(screen.getByText("Antigravity"));
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));
    await waitFor(() =>
      expect(createTaskMock).toHaveBeenCalledWith(
        "r1",
        "build X",
        undefined,
        undefined,
        "antigravity",
        null,
        null,
        false,
        null,
      ),
    );
  });

  it("with auto-start ON and a lifecycle agent, startAgentSession gets {label, lifecycle: true}", async () => {
    createTaskMock.mockResolvedValue({ id: "task-lifecycle" });
    const startSpy = seedSpies();
    render(
      <NewTaskForm
        repo={repo({ autoStartAgent: true, defaultAgent: "antigravity" })}
        onClose={vi.fn()}
      />,
    );
    await userEvent.type(screen.getByLabelText("Task title"), "build X");
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));
    await waitFor(() =>
      expect(startSpy).toHaveBeenCalledWith(
        "task-lifecycle",
        false,
        { label: "Antigravity", lifecycle: true },
        undefined,
      ),
    );
  });

  it("with auto-start ON and a claudeHooks agent, startAgentSession gets {lifecycle: false}", async () => {
    createTaskMock.mockResolvedValue({ id: "task-hooks" });
    const startSpy = seedSpies();
    render(
      <NewTaskForm
        repo={repo({ autoStartAgent: true, defaultAgent: "claude" })}
        onClose={vi.fn()}
      />,
    );
    await userEvent.type(screen.getByLabelText("Task title"), "build X");
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));
    await waitFor(() =>
      expect(startSpy).toHaveBeenCalledWith(
        "task-hooks",
        false,
        { label: "Claude Code", lifecycle: false },
        undefined,
      ),
    );
  });
});

describe("NewTaskForm deferred launch (TASK-179)", () => {
  const seedSpies = () => {
    const startSpy = vi.fn();
    useVigieStore.setState({
      startAgentSession: startSpy,
      refresh: vi.fn(),
      setSelectedTask: vi.fn(),
    });
    return startSpy;
  };

  it("defers the launch as a one-shot when 'Start later' is checked", async () => {
    createOneShotScheduleMock.mockResolvedValue({} as Schedule);
    seedSpies();
    const onClose = vi.fn();
    render(<NewTaskForm repo={repo()} onClose={onClose} />);
    await userEvent.type(screen.getByLabelText("Task title"), "Quota resume");
    await userEvent.type(screen.getByLabelText("Initial prompt"), "/resume");
    await userEvent.click(screen.getByLabelText("Start the agent later"));
    await userEvent.clear(screen.getByLabelText("Start in hours"));
    await userEvent.type(screen.getByLabelText("Start in hours"), "3");
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));

    await waitFor(() =>
      expect(createOneShotScheduleMock).toHaveBeenCalledWith(
        expect.objectContaining({ name: "Quota resume", prompt: "/resume", inSeconds: 10800 }),
      ),
    );
    expect(createTaskMock).not.toHaveBeenCalled();
  });

  it("rejects an empty 'Start in hours' value and does not create anything", async () => {
    seedSpies();
    const onClose = vi.fn();
    render(<NewTaskForm repo={repo()} onClose={onClose} />);
    await userEvent.type(screen.getByLabelText("Task title"), "Quota resume");
    await userEvent.click(screen.getByLabelText("Start the agent later"));
    await userEvent.clear(screen.getByLabelText("Start in hours"));
    await userEvent.click(screen.getByRole("button", { name: "Create task" }));

    await waitFor(() =>
      expect(screen.getByText("Enter a positive number of hours.")).toBeInTheDocument(),
    );
    expect(createOneShotScheduleMock).not.toHaveBeenCalled();
    expect(createTaskMock).not.toHaveBeenCalled();
  });
});
