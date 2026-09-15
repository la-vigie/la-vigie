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

/** Max chars of an error reason appended to a notification body. */
const REASON_MAX = 120;

/** Truncate a reason to a single tidy line for the notification body. */
function truncateReason(reason: string): string {
  const oneLine = reason.replace(/\s+/g, " ").trim();
  return oneLine.length > REASON_MAX ? `${oneLine.slice(0, REASON_MAX - 1)}…` : oneLine;
}

/** Build the title/body for an agent lifecycle notification. When a `reason` is
 *  given (e.g. a failed event's error text) it's appended, truncated, so the
 *  popup explains *why* — not just that something happened. */
export function formatNotification(
  task: Task,
  repo: Repo | undefined,
  event: SoundEvent,
  reason?: string | null,
): NotificationContent {
  const name = taskName(task);
  const title = task.ticketKey && task.ticketKey !== name ? `${task.ticketKey} · ${name}` : name;
  const label = STATE_LABELS[event];
  const base = repo ? `${label} — ${repo.name}/${task.branch}` : label;
  const trimmed = reason ? truncateReason(reason) : "";
  const body = trimmed ? `${base}: ${trimmed}` : base;
  return { title, body };
}
