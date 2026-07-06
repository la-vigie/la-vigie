import type { Task, Repo } from "../store";
import { SOUND_EVENTS, type SoundEvent } from "../sound/types";
import { taskName } from "../lib/taskName";

/** SoundEvent → human label, reusing the Settings labels so popup and UI match. */
const STATE_LABELS = Object.fromEntries(
  SOUND_EVENTS.map(({ key, label }) => [key, label]),
) as Record<SoundEvent, string>;

export interface NotificationContent {
  title: string;
  body: string;
}

/** Build the title/body for an agent lifecycle notification. */
export function formatNotification(
  task: Task,
  repo: Repo | undefined,
  event: SoundEvent,
): NotificationContent {
  const name = taskName(task);
  const title = task.ticketKey && task.ticketKey !== name ? `${task.ticketKey} · ${name}` : name;
  const label = STATE_LABELS[event];
  const body = repo ? `${label} — ${repo.name}/${task.branch}` : label;
  return { title, body };
}
