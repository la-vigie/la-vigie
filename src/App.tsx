import { useEffect, useRef } from "react";
import { Sidebar } from "./components/Sidebar/Sidebar";
import { TaskDetail } from "./components/TaskDetail/TaskDetail";
import { TitleBar } from "./components/TitleBar/TitleBar";
import { Tour } from "./components/Tour/Tour";
import { useAgentStatus } from "./hooks/useAgentStatus";
import { useAgentConsole } from "./hooks/useAgentConsole";
import { useFocusRefresh } from "./hooks/useFocusRefresh";
import { useSetupStatus } from "./hooks/useSetupStatus";
import { useTaskCreated } from "./hooks/useTaskCreated";
import { useTaskLaunch } from "./hooks/useTaskLaunch";
import { useTaskRename } from "./hooks/useTaskRename";
import { useTaskRemoved } from "./hooks/useTaskRemoved";
import { useTraySelect } from "./hooks/useTraySelect";
import { useVigieStore } from "./store";
import "./App.css";

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}

function App() {
  useAgentStatus();
  useAgentConsole();
  useSetupStatus();
  useTaskLaunch();
  useTaskRename();
  useTaskRemoved();
  useTaskCreated();
  useTraySelect();
  useFocusRefresh();

  // Load the agent-spec catalog up front: engine routing (PTY vs ACP) in
  // startAgentSession resolves against it, including for agents started via
  // task_launched events before any picker has mounted.
  useEffect(() => {
    void useVigieStore.getState().loadAgents();
  }, []);

  const sidebarCollapsed = useVigieStore((state) => state.sidebarCollapsed);
  const setSidebarWidth = useVigieStore((state) => state.setSidebarWidth);
  const layoutRef = useRef<HTMLDivElement>(null);

  const handleResizeMouseDown = () => {
    const onMouseMove = (e: MouseEvent) => {
      const layoutRect = layoutRef.current?.getBoundingClientRect();
      const layoutLeft = layoutRect?.left ?? 0;
      setSidebarWidth(clamp(e.clientX - layoutLeft, 180, 520));
    };
    const onMouseUp = () => {
      document.body.style.userSelect = "";
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    };
    document.body.style.userSelect = "none";
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
  };

  return (
    <div className="app-shell">
      <TitleBar />
      <div className="app-layout" ref={layoutRef}>
        <Sidebar />
        {!sidebarCollapsed && (
          <div
            className="resize-handle resize-handle--x"
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize sidebar"
            onMouseDown={handleResizeMouseDown}
          />
        )}
        <TaskDetail />
      </div>
      {/* Portaled to document.body — a sibling of the app, ancestor of nothing.
          Never wraps <TerminalHost/> (KEEP-ALIVE). */}
      <Tour />
    </div>
  );
}

export default App;
