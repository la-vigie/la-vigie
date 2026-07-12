import { useEffect } from "react";
import { onTaskRemoved } from "../api";
import { useVigieStore } from "../store";

/// Listen for `task_removed` events (TASK-139: the backend tore down a task,
/// e.g. via self-teardown) and reproduce the GUI finish/delete behavior —
/// deselect-if-selected + refresh — so the sidebar drops it live.
export function useTaskRemoved() {
  const handleTaskRemoved = useVigieStore((s) => s.handleTaskRemoved);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    onTaskRemoved(({ taskId }) => handleTaskRemoved(taskId)).then((off) => {
      if (cancelled) off();
      else unlisten = off;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [handleTaskRemoved]);
}
