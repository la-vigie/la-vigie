import { useEffect } from "react";
import { onTaskLaunched } from "../api";
import { combineInitialPrompts } from "../lib/combineInitialPrompts";
import { useVigieStore } from "../store";

/// Listen for `task_launched` (emitted when an agent self-dispatches a task via
/// MCP, TASK-89) and start the new task's agent on the existing path: refresh so
/// the task is in the store, select it, then `startAgentSession`. The agent tab
/// label defaults to "Claude" (cosmetic); the backend resolves the real agent.
export function useTaskLaunch(): void {
  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];

    const setup = async () => {
      const off = await onTaskLaunched(async ({ taskId, initialPrompt }) => {
        const { refresh, setSelectedTask, startAgentSession } = useVigieStore.getState();
        await refresh();
        const { tasks, repos } = useVigieStore.getState();
        const task = tasks.find((t) => t.id === taskId);
        const repo = task ? repos.find((r) => r.id === task.repoId) : undefined;
        setSelectedTask(taskId);
        startAgentSession(
          taskId,
          false,
          undefined,
          combineInitialPrompts(repo?.initialPrompt, initialPrompt),
        );
      });
      if (cancelled) {
        off();
        return;
      }
      unlisteners.push(off);
    };

    setup();

    return () => {
      cancelled = true;
      for (const off of unlisteners) off();
    };
  }, []);
}
