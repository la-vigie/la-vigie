import { useCallback, useEffect, useState } from "react";

// Per-file collapse state for the diff, persisted in localStorage so it survives
// diff refreshes, agent-status re-renders, scope switches, and app restarts.
// Keyed per task; the value is the set of collapsed file paths.

const keyFor = (taskId: string) => `vigie.diffCollapsed.${taskId}`;

function load(taskId: string): Set<string> {
  try {
    const raw = localStorage.getItem(keyFor(taskId));
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? new Set(parsed as string[]) : new Set();
  } catch {
    return new Set();
  }
}

export interface CollapsedFiles {
  isCollapsed: (path: string) => boolean;
  toggle: (path: string) => void;
  collapseAll: (paths: string[]) => void;
  expandAll: () => void;
}

export function useCollapsedFiles(taskId: string): CollapsedFiles {
  const [collapsed, setCollapsed] = useState<Set<string>>(() => load(taskId));

  // Reload when the task changes so each task gets its own state.
  useEffect(() => {
    setCollapsed(load(taskId));
  }, [taskId]);

  useEffect(() => {
    try {
      localStorage.setItem(keyFor(taskId), JSON.stringify([...collapsed]));
    } catch {
      // best-effort persistence
    }
  }, [taskId, collapsed]);

  const isCollapsed = useCallback((path: string) => collapsed.has(path), [collapsed]);

  const toggle = useCallback((path: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  const collapseAll = useCallback((paths: string[]) => {
    setCollapsed(new Set(paths));
  }, []);

  const expandAll = useCallback(() => {
    setCollapsed(new Set());
  }, []);

  return { isCollapsed, toggle, collapseAll, expandAll };
}
