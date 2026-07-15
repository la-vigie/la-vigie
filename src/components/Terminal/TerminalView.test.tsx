import { render, waitFor, act } from "@testing-library/react";
import type { ReactElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalView, isShiftEnterKeydown, translatePtyInput, shouldActivateLink } from "./TerminalView";
import {
  TerminalPaneMetricsContext,
  setCachedCell,
  resetCellCacheForTests,
} from "./TerminalPaneMetrics";
import type { PaneMetrics, PaneSize, PaneSubscriber } from "./TerminalPaneMetrics";
import { useVigieStore, AGENT_TAB } from "../../store";

const {
  invokeMock,
  startAgentMock,
  startShellMock,
  openUrlMock,
  ChannelInstances,
  terminalInstances,
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
      resize: ReturnType<typeof vi.fn>;
      attachCustomKeyEventHandler: ReturnType<typeof vi.fn>;
      cols: number;
      rows: number;
      dataCallback: ((data: string) => void) | null;
      keyEventHandler: ((e: KeyboardEvent) => boolean) | null;
      options: { linkHandler?: { activate: (event: MouseEvent, uri: string) => void } };
    }>,
    webLinksAddonInstances: [] as Array<{
      handler: ((event: MouseEvent, uri: string) => void) | undefined;
    }>,
  }));

// A hand-built PaneMetrics (the invariant `.terminal-pane__body` in production).
// Tests drive terminal sizing through this stub instead of a real
// ResizeObserver, exercising the deterministic pane-derived path (TASK-227).
function makeStubPaneMetrics(initial: PaneSize) {
  let size = initial;
  const subs = new Set<PaneSubscriber>();
  const metrics: PaneMetrics = {
    getSize: () => size,
    subscribe: (cb) => {
      subs.add(cb);
      return () => subs.delete(cb);
    },
  };
  // Simulate a genuine pane resize (window/split-drag/sidebar).
  const emit = (next: PaneSize) => {
    size = next;
    subs.forEach((cb) => cb(size));
  };
  return { metrics, emit };
}

function withPane(node: ReactElement, metrics: PaneMetrics): ReactElement {
  return (
    <TerminalPaneMetricsContext.Provider value={metrics}>
      {node}
    </TerminalPaneMetricsContext.Provider>
  );
}

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
    resize = vi.fn((cols: number, rows: number) => {
      this.cols = cols;
      this.rows = rows;
    });
    keyEventHandler: ((e: KeyboardEvent) => boolean) | null = null;
    attachCustomKeyEventHandler = vi.fn((cb: (e: KeyboardEvent) => boolean) => {
      this.keyEventHandler = cb;
    });
    cols = 80;
    rows = 24;
    options: { linkHandler?: { activate: (event: MouseEvent, uri: string) => void } };
    constructor(options?: { linkHandler?: { activate: (event: MouseEvent, uri: string) => void } }) {
      this.options = options ?? {};
      terminalInstances.push(this);
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
    webLinksAddonInstances.length = 0;
    resetCellCacheForTests();
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

    it("opens a Cmd/Ctrl+clicked OSC 8 hyperlink via the linkHandler; a plain click does not", async () => {
      useVigieStore.getState().startAgentSession("task-1", false);
      startAgentMock.mockResolvedValueOnce("agent-1");

      render(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />);

      await waitFor(() => expect(terminalInstances.length).toBe(1));
      const activate = terminalInstances[0].options.linkHandler!.activate;

      // Plain click: selects text, does not open (e.g. the "PR #123" OSC 8 link).
      activate({ metaKey: false, ctrlKey: false } as MouseEvent, "https://github.com/o/r/pull/123");
      expect(openUrlMock).not.toHaveBeenCalled();

      // Cmd/Ctrl+click: opens the embedded URL via the opener plugin.
      activate({ metaKey: true, ctrlKey: false } as MouseEvent, "https://github.com/o/r/pull/123");
      expect(openUrlMock).toHaveBeenCalledWith("https://github.com/o/r/pull/123");
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

  it("resizes the PTY to the pane-derived size once the session starts", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValue("agent-1");
    setCachedCell({ width: 8, height: 16 });
    const { metrics } = makeStubPaneMetrics({ width: 800, height: 480 });

    render(withPane(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />, metrics));

    // After start_agent resolves, the PTY (spawned at a default size) must be
    // resized to the pane-derived grid (floor((800−14)/8)=98, floor(480/16)=30).
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("resize_session", {
        sessionId: "agent-1",
        cols: 98,
        rows: 30,
      });
    });
  });

  it("still syncs the PTY post-spawn even if a pane resize already sized xterm to the same grid (dedup ordering)", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    setCachedCell({ width: 8, height: 16 });
    invokeMock.mockResolvedValue(undefined);
    const { metrics, emit } = makeStubPaneMetrics({ width: 800, height: 480 });

    // Hold start_agent so a pane resize can fire while the session id is unknown.
    let resolveStart: (id: string) => void = () => {};
    startAgentMock.mockImplementation(() => new Promise<string>((res) => (resolveStart = res)));

    render(withPane(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />, metrics));
    const term = terminalInstances[0];

    // A pane resize lands BEFORE the session is live: it sizes xterm (98×30) but
    // cannot sync the PTY yet (no id). This is the ordering that previously let a
    // combined dedup swallow the first PTY sync.
    act(() => emit({ width: 800, height: 480 }));
    expect(term.resize).toHaveBeenCalledWith(98, 30);
    expect(invokeMock.mock.calls.filter(([cmd]) => cmd === "resize_session")).toHaveLength(0);

    // Session becomes live: the post-spawn sync must reach the PTY even though
    // xterm's grid is already 98×30 (unchanged).
    await act(async () => {
      resolveStart("agent-1");
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect(invokeMock).toHaveBeenCalledWith("resize_session", {
      sessionId: "agent-1",
      cols: 98,
      rows: 30,
    });
  });

  it("sizes the terminal from the pane dimensions (not a per-child measurement) once the session is live (TASK-227)", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValue("agent-1");
    invokeMock.mockResolvedValue(undefined);
    // Cell size is a pure function of the font — seed the shared cache (as the
    // first laid-out terminal would) so sizing needs no real xterm renderer.
    setCachedCell({ width: 8, height: 16 });
    const { metrics } = makeStubPaneMetrics({ width: 800, height: 480 });

    render(withPane(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />, metrics));
    await waitFor(() => {
      const s = useVigieStore.getState().sessionsByTask["task-1"]?.find((x) => x.localId === AGENT_TAB);
      expect(s?.backendId).toBe("agent-1");
    });
    const term = terminalInstances[0];

    // Grid is computed directly from the pane box: cols = floor((800 − 14
    // scrollbar) / 8) = 98, rows = floor(480 / 16) = 30 — derived from the pane
    // dimensions, never from measuring the (possibly collapsed) child.
    expect(term.resize).toHaveBeenCalledWith(98, 30);
    expect(invokeMock).toHaveBeenCalledWith("resize_session", {
      sessionId: "agent-1",
      cols: 98,
      rows: 30,
    });
  });

  it("applies the cached pane size in a single frame on show — no settle loop, no per-child re-reads (TASK-227)", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValue("agent-1");
    invokeMock.mockResolvedValue(undefined);
    // Model a surface mounted while HIDDEN before any terminal had measured the
    // cell: its post-spawn sizing can't compute yet (no cell), so nothing is
    // applied. Seed the shared cell cache afterwards (as a sibling visible
    // terminal would), then reveal this surface.
    const { metrics } = makeStubPaneMetrics({ width: 800, height: 480 });

    const { rerender } = render(
      withPane(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={true} />, metrics),
    );
    await waitFor(() => {
      const s = useVigieStore.getState().sessionsByTask["task-1"]?.find((x) => x.localId === AGENT_TAB);
      expect(s?.backendId).toBe("agent-1");
    });
    const term = terminalInstances[0];
    // No cell was available while hidden → nothing sized yet.
    expect(term.resize).not.toHaveBeenCalled();
    setCachedCell({ width: 8, height: 16 });
    invokeMock.mockClear();

    rerender(withPane(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />, metrics));
    // Drive many frames: a settle loop would keep re-firing across a budget.
    await act(async () => {
      for (let f = 0; f < 14; f++) {
        await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
      }
    });

    // Deterministic: the cached pane size is applied exactly once (single rAF)
    // and de-duped thereafter — not re-fit every frame across a budget.
    expect(term.resize).toHaveBeenCalledTimes(1);
    expect(term.resize).toHaveBeenCalledWith(98, 30);
    const resizeCalls = invokeMock.mock.calls.filter(([cmd]) => cmd === "resize_session");
    expect(resizeCalls).toHaveLength(1);
  });

  it("re-fits the visible surface when the pane genuinely resizes (window/split-drag/sidebar) (TASK-227)", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValue("agent-1");
    invokeMock.mockResolvedValue(undefined);
    setCachedCell({ width: 8, height: 16 });
    const { metrics, emit } = makeStubPaneMetrics({ width: 800, height: 480 });

    render(withPane(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />, metrics));
    await waitFor(() => {
      const s = useVigieStore.getState().sessionsByTask["task-1"]?.find((x) => x.localId === AGENT_TAB);
      expect(s?.backendId).toBe("agent-1");
    });
    const term = terminalInstances[0];
    invokeMock.mockClear();

    // A genuine pane resize fires the pane subscription for every mounted
    // surface. New grid: cols = floor((400 − 14)/8) = 48, rows = floor(240/16) = 15.
    act(() => emit({ width: 400, height: 240 }));

    expect(term.resize).toHaveBeenCalledWith(48, 15);
    expect(invokeMock).toHaveBeenCalledWith("resize_session", {
      sessionId: "agent-1",
      cols: 48,
      rows: 15,
    });
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

  it("never syncs a size when the pane is unmeasured (0×0) — the deterministic safety net (TASK-227)", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValue("agent-1");
    invokeMock.mockResolvedValue(undefined);
    setCachedCell({ width: 8, height: 16 });
    // A pane that hasn't been laid out reports 0×0. computeGrid returns null for
    // it, so no degenerate size ever reaches xterm or the PTY. This replaces the
    // TASK-220 MIN_PTY_COLS floor: instead of rejecting "suspiciously narrow"
    // fits by a magic column count, we never compute a grid from an unmeasured
    // pane in the first place (and never measure the collapsing child at all).
    const { metrics } = makeStubPaneMetrics({ width: 0, height: 0 });

    const { rerender } = render(
      withPane(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={true} />, metrics),
    );
    await waitFor(() => {
      const s = useVigieStore.getState().sessionsByTask["task-1"]?.find((x) => x.localId === AGENT_TAB);
      expect(s?.backendId).toBe("agent-1");
    });
    const term = terminalInstances[0];
    invokeMock.mockClear();

    rerender(withPane(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={false} />, metrics));
    await act(async () => {
      for (let f = 0; f < 14; f++) {
        await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
      }
    });

    expect(term.resize).not.toHaveBeenCalled();
    const resizeCalls = invokeMock.mock.calls.filter(([cmd]) => cmd === "resize_session");
    expect(resizeCalls).toHaveLength(0);
  });

  it("keeps a HIDDEN surface's PTY synced on a genuine pane resize (zero switch transient) (TASK-227)", async () => {
    useVigieStore.getState().startAgentSession("task-1", false);
    startAgentMock.mockResolvedValue("agent-1");
    invokeMock.mockResolvedValue(undefined);
    setCachedCell({ width: 8, height: 16 });
    const { metrics, emit } = makeStubPaneMetrics({ width: 800, height: 480 });

    // The surface is HIDDEN the whole time — it never becomes visible.
    render(withPane(<TerminalView taskId="task-1" localId={AGENT_TAB} kind="agent" hidden={true} />, metrics));
    await waitFor(() => {
      const s = useVigieStore.getState().sessionsByTask["task-1"]?.find((x) => x.localId === AGENT_TAB);
      expect(s?.backendId).toBe("agent-1");
    });
    const term = terminalInstances[0];
    invokeMock.mockClear();

    // A genuine pane resize fires the subscription for hidden surfaces too, so
    // their PTY tracks the pane — which is why switching to them later has
    // nothing to correct. New grid: floor((400−14)/8)=48, floor(240/16)=15.
    act(() => emit({ width: 400, height: 240 }));

    expect(term.resize).toHaveBeenCalledWith(48, 15);
    expect(invokeMock).toHaveBeenCalledWith("resize_session", {
      sessionId: "agent-1",
      cols: 48,
      rows: 15,
    });
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
