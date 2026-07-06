import { render, screen, fireEvent } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { useVigieStore } from "./store";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  Channel: class {
    onmessage: ((event: unknown) => void) | null = null;
  },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

vi.mock("./hooks/useAgentStatus", () => ({
  useAgentStatus: vi.fn(),
}));

vi.mock("./hooks/useAgentConsole", () => ({
  useAgentConsole: vi.fn(),
}));

vi.mock("./hooks/useSetupStatus", () => ({
  useSetupStatus: vi.fn(),
}));

vi.mock("./hooks/useTaskLaunch", () => ({
  useTaskLaunch: vi.fn(),
}));

vi.mock("./hooks/useTaskRename", () => ({
  useTaskRename: vi.fn(),
}));

vi.mock("./hooks/useTerminalFileDrop", () => ({
  useTerminalFileDrop: vi.fn().mockReturnValue(false),
}));

vi.mock("./components/Terminal/TerminalHost", () => ({
  TerminalHost: () => <div data-testid="terminal-host" />,
}));

vi.mock("./components/Diff/DiffPanel", () => ({
  DiffPanel: ({ taskId }: { taskId: string }) => (
    <div data-testid="diff-panel" data-task-id={taskId} />
  ),
}));

describe("App — sidebar resize handle (AC2-17)", () => {
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
      sessionsByTask: {},
      activeTabByTask: {},
      sidebarCollapsed: false,
      sidebarWidth: 260,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  });

  it("renders the sidebar resize handle when sidebar is expanded", () => {
    render(<App />);
    const handle = screen.getByRole("separator", { name: /resize sidebar/i });
    expect(handle).toBeInTheDocument();
    expect(handle).toHaveClass("resize-handle--x");
  });

  it("does NOT render the resize handle when sidebar is collapsed", () => {
    useVigieStore.setState({
      sidebarCollapsed: true,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);

    render(<App />);

    expect(screen.queryByRole("separator", { name: /resize sidebar/i })).not.toBeInTheDocument();
  });

  it("drag: mousedown → mousemove → mouseup updates sidebarWidth (clamped)", () => {
    const { container } = render(<App />);

    // Stub the app-layout getBoundingClientRect so math is deterministic.
    // layoutLeft = 50; drag to clientX=330 → 330-50 = 280 (within 180..520).
    const layout = container.querySelector(".app-layout") as HTMLElement;
    layout.getBoundingClientRect = vi.fn(() => ({
      left: 50,
      right: 1280,
      top: 0,
      bottom: 768,
      width: 1230,
      height: 768,
      x: 50,
      y: 0,
      toJSON: () => {},
    }));

    const handle = screen.getByRole("separator", { name: /resize sidebar/i });

    fireEvent.mouseDown(handle);
    // Simulate mousemove on window
    fireEvent.mouseMove(window, { clientX: 330 });
    fireEvent.mouseUp(window);

    expect(useVigieStore.getState().sidebarWidth).toBe(280);
    expect(localStorage.getItem("vigie.sidebarWidth")).toBe("280");
  });

  it("drag clamps to min 180", () => {
    const { container } = render(<App />);

    const layout = container.querySelector(".app-layout") as HTMLElement;
    layout.getBoundingClientRect = vi.fn(() => ({
      left: 50,
      right: 1280,
      top: 0,
      bottom: 768,
      width: 1230,
      height: 768,
      x: 50,
      y: 0,
      toJSON: () => {},
    }));

    const handle = screen.getByRole("separator", { name: /resize sidebar/i });
    fireEvent.mouseDown(handle);
    // clientX=100 → 100-50 = 50, clamped to 180
    fireEvent.mouseMove(window, { clientX: 100 });
    fireEvent.mouseUp(window);

    expect(useVigieStore.getState().sidebarWidth).toBe(180);
  });

  it("drag clamps to max 520", () => {
    const { container } = render(<App />);

    const layout = container.querySelector(".app-layout") as HTMLElement;
    layout.getBoundingClientRect = vi.fn(() => ({
      left: 50,
      right: 1280,
      top: 0,
      bottom: 768,
      width: 1230,
      height: 768,
      x: 50,
      y: 0,
      toJSON: () => {},
    }));

    const handle = screen.getByRole("separator", { name: /resize sidebar/i });
    fireEvent.mouseDown(handle);
    // clientX=800 → 800-50=750, clamped to 520
    fireEvent.mouseMove(window, { clientX: 800 });
    fireEvent.mouseUp(window);

    expect(useVigieStore.getState().sidebarWidth).toBe(520);
  });
});
