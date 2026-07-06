import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NewTaskForm } from "./Sidebar";
import { useVigieStore } from "../../store";
import type { AgentSpec, Repo } from "../../store";
import * as agentHooks from "../../hooks/useAgents";

vi.mock("../../hooks/useAgents");

const { createTaskMock } = vi.hoisted(() => ({
  createTaskMock: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../api", () => ({
  createTask: createTaskMock,
  addRepo: vi.fn(),
}));

// Default safe hook mocks.
beforeEach(() => {
  (agentHooks.useAgents as ReturnType<typeof vi.fn>).mockReturnValue({ agents: [], loading: false, error: null });
  (agentHooks.useAgentModels as ReturnType<typeof vi.fn>).mockReturnValue({ models: [], loading: false });
});

const repo = (over: Partial<Repo> = {}): Repo => ({
  id: "r1",
  name: "r",
  path: "/r",
  defaultBranch: "main",
  autoStartAgent: false,
  initialPrompt: null,
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

describe("NewTaskForm agent picker (AC2-21)", () => {
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
