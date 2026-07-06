import { useRef } from "react";
import { Sidebar } from "./components/Sidebar/Sidebar";
import { TaskDetail } from "./components/TaskDetail/TaskDetail";
import { TitleBar } from "./components/TitleBar/TitleBar";
import { useAgentStatus } from "./hooks/useAgentStatus";
import { useAgentConsole } from "./hooks/useAgentConsole";
import { useSetupStatus } from "./hooks/useSetupStatus";
import { useTaskLaunch } from "./hooks/useTaskLaunch";
import { useTaskRename } from "./hooks/useTaskRename";
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
    </div>
  );
}

export default App;
