import type { Task } from "../store";

/**
 * The human-facing name for a task: its title, falling back to the ticket key
 * when the title is empty (a key-only task). Empty string only if neither is set.
 */
export function taskName(task: Task): string {
  return task.title.trim() || task.ticketKey || "";
}
