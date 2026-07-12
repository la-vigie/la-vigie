import { useEffect } from "react";
import { onTaskCreated } from "../api";
import { useVigieStore } from "../store";

/// Listen for `task_created` (TASK-90: the backend created a task the frontend
/// didn't initiate — e.g. a pending/queued task from MCP start_task) and
/// refresh so it appears in the sidebar live.
export function useTaskCreated() {
  const refresh = useVigieStore((s) => s.refresh);
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    onTaskCreated(() => {
      void refresh();
    }).then((off) => {
      if (cancelled) off();
      else unlisten = off;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refresh]);
}
