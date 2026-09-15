import { beforeEach, describe, expect, it, vi } from "vitest";
import { AGENT_TAB, useVigieStore } from "./index";
import type { AgentSpec, Repo, Task } from "./index";

// Engine routing (PTY vs ACP) in startAgentSession + the store-owned ACP
// session lifecycle (spawn, event reduction, prompt/permission/mode, exit).

const { invokeMock, MockChannel } = vi.hoisted(() => {
  class MockChannel {
    onmessage: ((event: unknown) => void) | null = null;
  }
  return { invokeMock: vi.fn(), MockChannel };
});
type MockChannel = InstanceType<typeof MockChannel>;

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  Channel: MockChannel,
}));

const acpSpec: AgentSpec = {
  name: "claude-acp",
  displayName: "Claude Code (ACP)",
  binary: "npx",
  baseArgs: [],
  resumeArgs: [],
  extraArgs: [],
  promptMode: "none",
  status: "lifecycle",
  builtin: true,
  execution: "acp",
};

const ptySpec: AgentSpec = {
  name: "claude",
  displayName: "Claude",
  binary: "claude",
  baseArgs: [],
  resumeArgs: ["--continue"],
  extraArgs: [],
  promptMode: "none",
  status: "claudeHooks",
  builtin: true,
  execution: "pty",
};

function makeTask(agent: string | null): Task {
  return {
    id: "t1",
    repoId: "r1",
    title: "Task",
    worktreePath: "/wt",
    branch: "b",
    baseBranch: "main",
    status: "idle",
    createdAt: 0,
    updatedAt: 0,
    agent,
    inPlace: false,
  };
}

const repo: Repo = {
  id: "r1",
  name: "repo",
  path: "/r",
  defaultBranch: "main",
  inPlaceDefault: false,
};

const flush = () => new Promise((r) => setTimeout(r, 0));

/** The Channel passed to the last start_acp_agent invoke. */
function spawnedChannel(): MockChannel {
  const call = [...invokeMock.mock.calls].reverse().find((c) => c[0] === "start_acp_agent");
  expect(call).toBeTruthy();
  return (call![1] as { onEvent: MockChannel }).onEvent;
}

beforeEach(() => {
  invokeMock.mockReset().mockResolvedValue("acp-backend-1");
  useVigieStore.setState({
    repos: [repo],
    tasks: [makeTask("claude-acp")],
    agents: [acpSpec, ptySpec],
    agentsLoaded: true,
    sessionsByTask: {},
    activeTabByTask: {},
    attentionByTask: {},
    consoleByAgentId: {},
    errorByTask: {},
    acpByTask: {},
    selectedTaskId: "t1",
  });
});

describe("engine routing in startAgentSession", () => {
  it("routes an acp-execution spec to start_acp_agent and marks the session engine", async () => {
    useVigieStore.getState().startAgentSession("t1", false, undefined, "kickoff");
    const session = useVigieStore.getState().sessionsByTask["t1"][0];
    expect(session).toMatchObject({
      kind: "agent",
      engine: "acp",
      status: "starting",
      title: "Claude Code (ACP)",
      lifecycle: true,
    });
    // The timeline is initialized eagerly so the surface can render.
    expect(useVigieStore.getState().acpByTask["t1"]).toMatchObject({ items: [], sessionId: null });

    expect(invokeMock).toHaveBeenCalledWith(
      "start_acp_agent",
      expect.objectContaining({ taskId: "t1", resume: false, initialPrompt: "kickoff" }),
    );
    await flush();
    expect(useVigieStore.getState().sessionsByTask["t1"][0]).toMatchObject({
      backendId: "acp-backend-1",
      status: "running",
    });
  });

  it("loads the catalog before routing when it isn't loaded yet, so an ACP task isn't misrouted to start_agent (TASK-252)", async () => {
    // Simulate a launch firing before the boot-time catalog load resolved:
    // empty catalog, agentsLoaded false. Without the guard the ACP task would
    // fall through to the PTY `start_agent` command (which rejects it).
    useVigieStore.setState({ tasks: [makeTask("claude-acp")], agents: [], agentsLoaded: false });
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "list_agents" ? Promise.resolve([acpSpec, ptySpec]) : Promise.resolve("acp-backend-1"),
    );

    await useVigieStore.getState().startAgentSession("t1", false);

    expect(useVigieStore.getState().sessionsByTask["t1"][0].engine).toBe("acp");
    expect(invokeMock).toHaveBeenCalledWith("start_acp_agent", expect.anything());
    expect(invokeMock).not.toHaveBeenCalledWith("start_agent", expect.anything());
  });

  it("leaves agentsLoaded false on a failed catalog load so the next launch retries (TASK-252)", async () => {
    useVigieStore.setState({ agents: [], agentsLoaded: false, agentsError: null });
    invokeMock.mockReset().mockRejectedValueOnce(new Error("list_agents boom"));

    await useVigieStore.getState().loadAgents();

    // Not flipped to loaded, so `startAgentSession`'s `!agentsLoaded` guard
    // will retry instead of routing blind on the empty catalog.
    expect(useVigieStore.getState().agentsLoaded).toBe(false);
    expect(useVigieStore.getState().agentsError).toContain("boom");
  });

  it("routes a pty spec to the PTY path (no start_acp_agent, no engine acp)", () => {
    useVigieStore.setState({ tasks: [makeTask("claude")] });
    useVigieStore.getState().startAgentSession("t1", false);
    const session = useVigieStore.getState().sessionsByTask["t1"][0];
    expect(session.engine).toBe("pty");
    expect(invokeMock).not.toHaveBeenCalledWith("start_acp_agent", expect.anything());
    expect(useVigieStore.getState().acpByTask["t1"]).toBeUndefined();
  });

  it("defaults to PTY when the spec is unknown or the catalog is empty", () => {
    useVigieStore.setState({ tasks: [makeTask("mystery-agent")], agents: [] });
    useVigieStore.getState().startAgentSession("t1", false);
    expect(useVigieStore.getState().sessionsByTask["t1"][0].engine).toBe("pty");
    expect(invokeMock).not.toHaveBeenCalledWith("start_acp_agent", expect.anything());
  });

  it("falls back to the repo default agent for routing when the task has none", () => {
    useVigieStore.setState({
      tasks: [makeTask(null)],
      repos: [{ ...repo, defaultAgent: "claude-acp" }],
    });
    useVigieStore.getState().startAgentSession("t1", false);
    expect(useVigieStore.getState().sessionsByTask["t1"][0].engine).toBe("acp");
  });

  it("resets a stale timeline when a fresh ACP session starts", () => {
    useVigieStore.getState().applyAcpEvent("t1", {
      type: "messageChunk",
      role: "assistant",
      text: "old run",
      messageId: null,
    });
    expect(useVigieStore.getState().acpByTask["t1"].items).toHaveLength(1);
    useVigieStore.getState().startAgentSession("t1", false);
    expect(useVigieStore.getState().acpByTask["t1"].items).toHaveLength(0);
  });
});

describe("ACP session lifecycle", () => {
  it("reduces channel events into the task timeline", async () => {
    useVigieStore.getState().startAgentSession("t1", false);
    await flush();
    const channel = spawnedChannel();
    channel.onmessage?.({ type: "sessionStarted", sessionId: "s-1", modes: null, models: null });
    channel.onmessage?.({ type: "messageChunk", role: "assistant", text: "hi", messageId: "m1" });
    const t = useVigieStore.getState().acpByTask["t1"];
    expect(t.sessionId).toBe("s-1");
    expect(t.items).toMatchObject([{ kind: "message", role: "assistant", text: "hi" }]);
  });

  it("exit event tears the session down like a PTY exit (session gone, timeline gone, stop_session fired)", async () => {
    useVigieStore.getState().startAgentSession("t1", false);
    await flush();
    const channel = spawnedChannel();
    channel.onmessage?.({ type: "exit", code: 0 });
    const s = useVigieStore.getState();
    expect(s.sessionsByTask["t1"].some((x) => x.kind === "agent")).toBe(false);
    expect(s.acpByTask["t1"]).toBeUndefined();
    expect(invokeMock).toHaveBeenCalledWith("stop_session", { sessionId: "acp-backend-1" });
  });

  it("a failed spawn removes the session and surfaces the error", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "start_acp_agent" ? Promise.reject(new Error("no such agent binary")) : Promise.resolve(),
    );
    useVigieStore.getState().startAgentSession("t1", false);
    await flush();
    const s = useVigieStore.getState();
    expect(s.sessionsByTask["t1"].some((x) => x.kind === "agent")).toBe(false);
    expect(s.errorByTask["t1"]).toBe("no such agent binary");
  });

  it("stops a session whose task was torn down while the spawn was in flight", async () => {
    let resolveSpawn: (id: string) => void = () => {};
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "start_acp_agent") return new Promise((r) => (resolveSpawn = r));
      return Promise.resolve();
    });
    useVigieStore.getState().startAgentSession("t1", false);
    useVigieStore.getState().removeAgentSession("t1");
    resolveSpawn("late-id");
    await flush();
    expect(invokeMock).toHaveBeenCalledWith("stop_session", { sessionId: "late-id" });
    expect(useVigieStore.getState().sessionsByTask["t1"].some((x) => x.kind === "agent")).toBe(false);
  });
});

describe("ACP actions", () => {
  const seedRunning = async () => {
    useVigieStore.getState().startAgentSession("t1", false);
    await flush();
  };

  it("sendAcpPrompt appends an optimistic user bubble and invokes acp_prompt", async () => {
    await seedRunning();
    await useVigieStore.getState().sendAcpPrompt("t1", "do the thing");
    expect(invokeMock).toHaveBeenCalledWith("acp_prompt", {
      sessionId: "acp-backend-1",
      text: "do the thing",
    });
    const items = useVigieStore.getState().acpByTask["t1"].items;
    expect(items).toMatchObject([{ kind: "message", role: "user", text: "do the thing", local: true }]);
  });

  it("sendAcpPrompt rejects when the session has no backend id yet", async () => {
    useVigieStore.setState({
      sessionsByTask: {
        t1: [{ localId: AGENT_TAB, kind: "agent", status: "starting", title: "x", engine: "acp" }],
      },
    });
    await expect(useVigieStore.getState().sendAcpPrompt("t1", "hi")).rejects.toThrow(
      "ACP session not ready",
    );
    expect(invokeMock).not.toHaveBeenCalledWith("acp_prompt", expect.anything());
  });

  it("cancelAcpTurn invokes acp_cancel on the live session", async () => {
    await seedRunning();
    await useVigieStore.getState().cancelAcpTurn("t1");
    expect(invokeMock).toHaveBeenCalledWith("acp_cancel", { sessionId: "acp-backend-1" });
  });

  it("respondAcpPermission answers and clears the pending request", async () => {
    await seedRunning();
    useVigieStore.getState().applyAcpEvent("t1", {
      type: "permissionRequest",
      requestId: "req-1",
      options: [{ optionId: "allow", name: "Allow", kind: "allow_once" }],
      toolCall: { title: "Edit foo.rs" },
    });
    expect(useVigieStore.getState().acpByTask["t1"].permission).not.toBeNull();
    await useVigieStore.getState().respondAcpPermission("t1", "req-1", "allow");
    expect(invokeMock).toHaveBeenCalledWith("acp_respond_permission", {
      sessionId: "acp-backend-1",
      requestId: "req-1",
      optionId: "allow",
    });
    expect(useVigieStore.getState().acpByTask["t1"].permission).toBeNull();
  });

  it("setAcpMode invokes acp_set_mode and optimistically updates the mode", async () => {
    await seedRunning();
    spawnedChannel().onmessage?.({
      type: "sessionStarted",
      sessionId: "s-1",
      modes: { currentModeId: "default", availableModes: [{ id: "default", name: "Default" }, { id: "yolo", name: "Yolo" }] },
      models: null,
    });
    await useVigieStore.getState().setAcpMode("t1", "yolo");
    expect(invokeMock).toHaveBeenCalledWith("acp_set_mode", {
      sessionId: "acp-backend-1",
      modeId: "yolo",
    });
    expect(useVigieStore.getState().acpByTask["t1"].modes?.currentModeId).toBe("yolo");
  });
});
