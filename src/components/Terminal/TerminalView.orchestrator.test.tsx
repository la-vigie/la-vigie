import { render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalView } from "./TerminalView";
import { useVigieStore, orchestratorSurfaceId } from "../../store";

const { startAgentMock, openOrchestratorTerminalMock, ChannelInstances } = vi.hoisted(() => ({
  startAgentMock: vi.fn(),
  openOrchestratorTerminalMock: vi.fn(),
  ChannelInstances: [] as Array<{ onmessage: ((event: unknown) => void) | null }>,
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  Channel: class {
    onmessage: ((event: unknown) => void) | null = null;
    constructor() {
      ChannelInstances.push(this);
    }
  },
}));

vi.mock("../../api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api")>();
  return {
    ...actual,
    startAgent: startAgentMock,
    openOrchestratorTerminal: openOrchestratorTerminalMock,
  };
});

vi.mock("@xterm/addon-web-links", () => ({
  WebLinksAddon: class {
    constructor() {}
  },
}));

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    open = vi.fn();
    write = vi.fn();
    onData = vi.fn();
    loadAddon = vi.fn();
    dispose = vi.fn();
    focus = vi.fn();
    refresh = vi.fn();
    attachCustomKeyEventHandler = vi.fn();
    cols = 80;
    rows = 24;
    constructor() {}
  },
}));

// jsdom doesn't implement ResizeObserver.
class ResizeObserverStub {
  observe = vi.fn();
  disconnect = vi.fn();
  unobserve = vi.fn();
}

describe("TerminalView orchestrator spawn", () => {
  beforeEach(() => {
    startAgentMock.mockReset();
    openOrchestratorTerminalMock.mockReset();
    openOrchestratorTerminalMock.mockResolvedValue("agent-1");
    ChannelInstances.length = 0;
    useVigieStore.setState({
      repos: [],
      tasks: [],
      selectedTaskId: null,
      sessionsByTask: {},
      activeTabByTask: {},
    });
    vi.stubGlobal("ResizeObserver", ResizeObserverStub);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("spawns via openOrchestratorTerminal with the repo id derived from the surface id", async () => {
    render(
      <TerminalView
        taskId={orchestratorSurfaceId("r1")}
        localId="agent"
        kind="orchestrator"
        hidden={false}
      />,
    );

    await waitFor(() => expect(openOrchestratorTerminalMock).toHaveBeenCalledTimes(1));
    expect(openOrchestratorTerminalMock.mock.calls[0][0]).toBe("r1");
    expect(startAgentMock).not.toHaveBeenCalled();
  });

  it("does not tear down the store session when a PTY exit arrives after the view is disposed", async () => {
    // Repro of the StrictMode double-mount / stop-and-respawn bug: the first
    // mount's process is stopped when the second mount respawns, and its exit
    // event is delivered to the first (now-disposed) view's orphaned channel.
    // A disposed view must not mutate live store state.
    const key = orchestratorSurfaceId("r1");
    useVigieStore.setState({
      sessionsByTask: {
        [key]: [{ localId: "agent", kind: "orchestrator", status: "running", title: "Orchestrator" }],
      },
      activeTabByTask: { [key]: "agent" },
    });

    const { unmount } = render(
      <TerminalView taskId={key} localId="agent" kind="orchestrator" hidden={false} />,
    );
    await waitFor(() => expect(openOrchestratorTerminalMock).toHaveBeenCalledTimes(1));
    const channel = ChannelInstances[0];

    unmount();
    channel.onmessage?.({ type: "exit", code: 0 });

    // Without the disposed-guard this reverts the surface to its placeholder.
    expect(useVigieStore.getState().sessionsByTask[key]).toHaveLength(1);
  });
});
