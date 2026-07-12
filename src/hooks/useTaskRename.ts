import { useEffect } from "react";
import { onTaskRenamed } from "../api";
import { useVigieStore } from "../store";

/// Listen for `task_renamed` events (TASK-40: an agent renamed its own task via
/// the HookBridge) and patch the task title in the store so the sidebar/header
/// update live.
export function useTaskRename() {
  const setTaskTitle = useVigieStore((s) => s.setTaskTitle);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    onTaskRenamed(({ taskId, title }) => setTaskTitle(taskId, title)).then((off) => {
      if (cancelled) off();
      else unlisten = off;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [setTaskTitle]);
}
