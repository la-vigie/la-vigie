import { render, waitFor, act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalView, isShiftEnterKeydown, translatePtyInput, shouldActivateLink } from "./TerminalView";
import { useVigieStore, AGENT_TAB } from "../../store";

const {
  invokeMock,
  startAgentMock,
  startShellMock,
  openUrlMock,
  ChannelInstances,
  terminalInstances,
  fitAddonInstances,
  webLinksAddonInstances,
} = vi.hoisted(() => ({
    invokeMock: vi.fn(),
    startAgentMock: vi.fn(),
    startShellMock: vi.fn(),
    openUrlMock: vi.fn(),
    ChannelInstances: [] as Array<{ onmessage: ((event: unknown) => void) | null }>,
    terminalInstances: [] as Array<{
      open: ReturnType<typeof vi.fn>;
      write: ReturnType<typeof vi.fn>;
      onData: ReturnType<typeof vi.fn>;
      loadAddon: ReturnType<typeof vi.fn>;
      dispose: ReturnType<typeof vi.fn>;
      focus: ReturnType<typeof vi.fn>;
      refresh: ReturnType<typeof vi.fn>;
      attachCustomKeyEventHandler: ReturnType<typeof vi.fn>;
      cols: number;
      rows: number;
      dataCallback: ((data: string) => void) | null;
      keyEventHandler: ((e: KeyboardEvent) => boolean) | null;
    }>,
    fitAddonInstances: [] as Array<{ fit: ReturnType<typeof vi.fn> }>,
    webLinksAddonInstances: [] as Array<{
      handler: ((event: MouseEvent, uri: string) => void) | undefined;
    }>,
  }));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
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
    startShell: startShellMock,
    openUrl: openUrlMock,
  };
});

vi.mock("@xterm/addon-web-links", () => ({
  WebLinksAddon: class {
    handler: ((event: MouseEvent, uri: string) => void) | undefined;
    constructor(handler?: (event: MouseEvent, uri: string) => void) {
      this.handler = handler;
      webLinksAddonInstances.push(this);
    }
  },
}));

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    open = vi.fn();
    write = vi.fn();
    dataCallback: ((data: string) => void) | null = null;
    onData = vi.fn((cb: (data: string) => void) => {
      this.dataCallback = cb;
    });
    loadAddon = vi.fn();
    dispose = vi.fn();
    focus = vi.fn();
    refresh = vi.fn();
    keyEventHandler: ((e: KeyboardEvent) => boolean) | null = null;
    attachCustomKeyEventHandler = vi.fn((cb: (e: KeyboardEvent) => boolean) => {
      this.keyEventHandler = cb;
    });
    cols = 80;
    rows = 24;
    constructor() {
      terminalInstances.push(this);
    }
  },
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit = vi.fn();
    constructor() {
      fitAddonInstances.push(this);
    }
  },
}));

// jsdom doesn't implement ResizeObserver.
class ResizeObserverStub {
  observe = vi.fn();
  disconnect = vi.fn();
  unobserve = vi.fn();
}

describe("TerminalView", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    startAgentMock.mockReset();
    startShellMock.mockReset();
    openUrlMock.mockReset();
    openUrlMock.mockResolvedValue(undefined);
    ChannelInstances.length = 0;
    terminalInstances.length = 0;
    fitAddonInstances.length = 0;
    webLinksAddonInstances.length = 0;
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

  it("starts the agent on mount with the taskId/resume from the store and a channel", async () => {
    useVigieStore.getState().startAgentSession("task-1", true);
    startAgentMock.mockResolvedValueOnce("agent-1");

    render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);

    await waitFor(() => {
      expect(startAgentMock).toHaveBeenCalledWith("task-1", true, expect.anything(), undefined);
    });

    await waitFor(() => {
      const session = useVigieStore.getState().sessionsByTask["task-1"]?.find((s) => s.localId === AGENT_TAB);
      expect(session).toMatchObject({
        status: "running",
        resume: true,
        backendId: "agent-1",
      });
    });
  });

  it("writes decoded data events to the terminal", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValueOnce("agent-1");

    render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);

    await waitFor(() => expect(ChannelInstances[0]?.onmessage).not.toBeNull());

    const encoded = btoa("hello");
    ChannelInstances[0].onmessage?.({ type: "data", data: encoded });

    const term = terminalInstances[0];
    expect(term.write).toHaveBeenCalledTimes(1);
    const written = term.write.mock.calls[0][0] as Uint8Array;
    expect(new TextDecoder().decode(written)).toBe("hello");
  });

  it("forwards keystrokes from onData to write_session", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValueOnce("agent-1");

    render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);

    await waitFor(() => {
      const session = useVigieStore.getState().sessionsByTask["task-1"]?.find((s) => s.localId === AGENT_TAB);
      expect(session?.backendId).toBe("agent-1");
    });

    const term = terminalInstances[0];
    term.dataCallback?.("ls\n");

    expect(invokeMock).toHaveBeenCalledWith("write_session", {
      sessionId: "agent-1",
      data: "ls\n",
    });
  });

  describe("Shift+Enter newline helpers", () => {
    it("isShiftEnterKeydown is true only for a Shift+Enter keydown", () => {
      expect(isShiftEnterKeydown({ type: "keydown", key: "Enter", shiftKey: true })).toBe(true);
      expect(isShiftEnterKeydown({ type: "keydown", key: "Enter", shiftKey: false })).toBe(false);
      expect(isShiftEnterKeydown({ type: "keyup", key: "Enter", shiftKey: true })).toBe(false);
      expect(isShiftEnterKeydown({ type: "keydown", key: "a", shiftKey: true })).toBe(false);
    });

    it("translatePtyInput turns a CR into a LF only when a Shift+Enter is pending", () => {
      expect(translatePtyInput("\r", true)).toBe("\n");
      expect(translatePtyInput("\r", false)).toBe("\r");
      expect(translatePtyInput("ls", true)).toBe("ls");
      expect(translatePtyInput("\n", true)).toBe("\n");
    });
  });

  describe("clickable terminal links", () => {
    it("shouldActivateLink is true only with the Cmd or Ctrl modifier", () => {
      expect(shouldActivateLink({ metaKey: true, ctrlKey: false })).toBe(true);
      expect(shouldActivateLink({ metaKey: false, ctrlKey: true })).toBe(true);
      expect(shouldActivateLink({ metaKey: false, ctrlKey: false })).toBe(false);
    });

    it("loads the web-links addon on mount", async () => {
      useVigieStore.getState().startAgentSession("task-1", false);
      startAgentMock.mockResolvedValueOnce("agent-1");

      render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);

      await waitFor(() => expect(webLinksAddonInstances.length).toBe(1));
      const term = terminalInstances[0];
      expect(term.loadAddon).toHaveBeenCalledWith(webLinksAddonInstances[0]);
    });

    it("opens a Cmd/Ctrl+clicked link via the opener; a plain click does not", async () => {
      useVigieStore.getState().startAgentSession("task-1", false);
      startAgentMock.mockResolvedValueOnce("agent-1");

      render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);

      await waitFor(() => expect(webLinksAddonInstances.length).toBe(1));
      const handler = webLinksAddonInstances[0].handler!;

      // Plain click: selects text, does not open.
      handler({ metaKey: false, ctrlKey: false } as MouseEvent, "https://example.com");
      expect(openUrlMock).not.toHaveBeenCalled();

      // Cmd/Ctrl+click: opens the URL via the opener plugin.
      handler({ metaKey: true, ctrlKey: false } as MouseEvent, "https://example.com");
      expect(openUrlMock).toHaveBeenCalledWith("https://example.com");
    });
  });

  it("Shift+Enter turns the CR xterm emits into a newline (insert, not submit)", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValueOnce("agent-1");

    render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);

    await waitFor(() => {
      const session = useVigieStore.getState().sessionsByTask["task-1"]?.find((s) => s.localId === AGENT_TAB);
      expect(session?.backendId).toBe("agent-1");
    });

    const term = terminalInstances[0];
    // Shift+Enter keydown (we let xterm process it), then xterm's resulting CR.
    term.keyEventHandler!({ type: "keydown", key: "Enter", shiftKey: true } as KeyboardEvent);
    term.dataCallback?.("\r");

    expect(invokeMock).toHaveBeenCalledWith("write_session", {
      sessionId: "agent-1",
      data: "\n",
    });
  });

  it("plain Enter still submits (CR passes through unchanged)", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValueOnce("agent-1");

    render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);

    await waitFor(() => {
      const session = useVigieStore.getState().sessionsByTask["task-1"]?.find((s) => s.localId === AGENT_TAB);
      expect(session?.backendId).toBe("agent-1");
    });

    const term = terminalInstances[0];
    term.keyEventHandler!({ type: "keydown", key: "Enter", shiftKey: false } as KeyboardEvent);
    term.dataCallback?.("\r");

    expect(invokeMock).toHaveBeenCalledWith("write_session", {
      sessionId: "agent-1",
      data: "\r",
    });
  });

  it("on exit event for agent kind, removes the agent session (enables clean remount)", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    // Default any other invoke (e.g. resize_session after start) to a resolved
    // promise; start_agent specifically returns the agent id.
    invokeMock.mockResolvedValue(undefined);
    startAgentMock.mockResolvedValueOnce("agent-1");

    render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);

    await waitFor(() => expect(ChannelInstances[0]?.onmessage).not.toBeNull());

    ChannelInstances[0].onmessage?.({ type: "exit", code: 0 });

    await waitFor(() => {
      // removeAgentSession clears the agent session from sessionsByTask
      const sessions = useVigieStore.getState().sessionsByTask["task-1"];
      expect(sessions).toEqual([]);
    });
    expect(invokeMock).toHaveBeenCalledWith("stop_session", {
      sessionId: "agent-1",
    });
  });

  it("on exit event for shell kind, marks the session exited (output stays readable)", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    useVigieStore.getState().addShellSession("task-1");
    const shellSession = useVigieStore.getState().sessionsByTask["task-1"][1];
    invokeMock.mockResolvedValue(undefined);
    startShellMock.mockResolvedValueOnce("shell-1");

    render(<TerminalView taskId="task-1" localId={shellSession.localId} kind="shell" hidden={false} />);

    await waitFor(() => expect(ChannelInstances[0]?.onmessage).not.toBeNull());

    ChannelInstances[0].onmessage?.({ type: "exit", code: 0 });

    await waitFor(() => {
      const session = useVigieStore.getState().sessionsByTask["task-1"]?.find((s) => s.localId === shellSession.localId);
      expect(session?.status).toBe("exited");
    });
  });

  it("toggles visibility via display style, not unmounting, when hidden changes", () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValueOnce("agent-1");

    const { container, rerender } = render(
      <TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />,
    );
    const wrapper = container.firstElementChild as HTMLElement;
    expect(wrapper.style.display).toBe("block");

    rerender(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={true} />);
    expect(wrapper.style.display).toBe("none");
    // Still the same DOM node => not unmounted.
    expect(container.firstElementChild).toBe(wrapper);
  });

  it("disposes the terminal on unmount", () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValueOnce("agent-1");

    const { unmount } = render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);
    const term = terminalInstances[0];

    unmount();

    expect(term.dispose).toHaveBeenCalledTimes(1);
  });

  it("stops the agent when exit arrives before start_session resolves", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);

    // Hold start_agent's resolution so we can fire the exit event first.
    let resolveStart: (id: string) => void = () => {};
    const startPromise = new Promise<string>((res) => {
      resolveStart = res;
    });
    startAgentMock.mockImplementation(() => startPromise);
    invokeMock.mockResolvedValue(undefined); // stop_session, etc.

    render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);

    await waitFor(() => expect(ChannelInstances[0]?.onmessage).not.toBeNull());

    // Exit fires while agentId is still unknown: must not call stop_session yet.
    ChannelInstances[0].onmessage?.({ type: "exit", code: 1 });
    expect(invokeMock).not.toHaveBeenCalledWith("stop_session", expect.anything());

    // Once start_agent resolves, the deferred stop must fire with the real id.
    resolveStart("agent-1");

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("stop_session", {
        sessionId: "agent-1",
      });
    });
  });

  it("resizes the PTY to the fitted size once the session starts", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValue("agent-1");

    render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);

    // After start_agent resolves, the PTY (spawned at a default size) must be
    // resized to the fitted terminal dimensions (mock Terminal is 80x24).
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("resize_session", {
        sessionId: "agent-1",
        cols: 80,
        rows: 24,
      });
    });
  });

  it("re-fits the terminal when it becomes visible again (prevents collapse on task switch)", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValue("agent-1");

    const { rerender } = render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={true} />);
    const fit = fitAddonInstances[0].fit;
    const callsWhileHidden = fit.mock.calls.length;

    rerender(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);
    await act(async () => {
      await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
    });

    expect(fit.mock.calls.length).toBeGreaterThan(callsWhileHidden);
  });

  it("forces a repaint when it becomes visible again (fixes blank/stale viewport on session switch)", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValue("agent-1");

    const { rerender } = render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={true} />);
    const term = terminalInstances[0];
    // While hidden, the becomes-visible effect bails out — no repaint forced.
    expect(term.refresh).not.toHaveBeenCalled();

    rerender(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);
    await act(async () => {
      await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
    });

    // fit() is a no-op on a same-size switch, so a repaint of all visible rows
    // must be forced explicitly (mock Terminal is 80x24 => rows-1 = 23).
    expect(term.refresh).toHaveBeenCalledWith(0, term.rows - 1);
  });

  it("focuses the terminal when it becomes visible again (auto-focus on task switch)", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValue("agent-1");

    const { rerender } = render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={true} />);
    const term = terminalInstances[0];
    expect(term.focus).not.toHaveBeenCalled();

    rerender(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);
    await act(async () => {
      await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
    });

    expect(term.focus).toHaveBeenCalled();
  });

  it("spawns a shell via startShell when kind is shell", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    useVigieStore.getState().addShellSession("task-1");
    const shellSession = useVigieStore.getState().sessionsByTask["task-1"][1];
    startShellMock.mockResolvedValueOnce("shell-backend-1");

    render(<TerminalView taskId="task-1" localId={shellSession.localId} kind="shell" hidden={false} />);
    await waitFor(() => expect(startShellMock).toHaveBeenCalledWith("task-1", expect.anything()));
  });

  it("spawns the agent via startAgent when kind is agent", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValueOnce("agent-1");

    render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);
    await waitFor(() => expect(startAgentMock).toHaveBeenCalled());
  });

  it("passes the session's initialPrompt to startAgent", async () => {
    useVigieStore.getState().startAgentSession("task-1", false, undefined, "seed me");
    startAgentMock.mockResolvedValueOnce("agent-1");

    render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);

    await waitFor(() => expect(startAgentMock).toHaveBeenCalled());
    expect(startAgentMock).toHaveBeenCalledWith("task-1", false, expect.anything(), "seed me");
  });
});
