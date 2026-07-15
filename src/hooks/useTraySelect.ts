import { useEffect } from "react";
import { onTraySelectTask } from "../api";
import { useVigieStore } from "../store";

/// Listen for `tray_select_task` (TASK-204: the user picked a task from the
/// system-tray menu). The Rust side already brought the window to the front;
/// here we just select that task in the store — `setSelectedTask` also clears
/// the task's attention cue. Mirrors how `task_launched` is bridged (TASK-89).
export function useTraySelect(): void {
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    onTraySelectTask(({ taskId }) => {
      useVigieStore.getState().setSelectedTask(taskId);
    }).then((off) => {
      if (cancelled) off();
      else unlisten = off;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);
}
