import { useState } from "react";
import { useVigieStore } from "../../store";

export interface TaskErrorBannerProps {
  taskId: string;
}

/**
 * Dismissible banner that explains a task's error state (a red dot alone is not
 * actionable). Reads the per-task error text captured out-of-band from the
 * agent's StopFailure hook. Dismiss is keyed by the message, so a *new/different*
 * error re-shows the banner; a backend clear (agent back to Working/Idle) removes
 * the error from the store and the banner disappears on its own.
 *
 * Rendered inside the TaskDetail header — NOT as a positional sibling of the
 * `.task-detail__body` (that would shift the body's child index and remount the
 * keep-alive <TerminalHost/>). See the KEEP-ALIVE invariant in TaskDetail.
 */
export function TaskErrorBanner({ taskId }: TaskErrorBannerProps) {
  const message = useVigieStore((s) => s.errorByTask[taskId]);
  const [dismissed, setDismissed] = useState<string | null>(null);

  if (!message || dismissed === message) return null;

  return (
    <div className="task-error-banner" role="alert" data-testid="task-error-banner">
      <span className="task-error-banner__icon" aria-hidden>
        ⚠
      </span>
      <span className="task-error-banner__text">{message}</span>
      <button
        type="button"
        className="task-error-banner__dismiss"
        aria-label="Dismiss error"
        onClick={() => setDismissed(message)}
      >
        ×
      </button>
    </div>
  );
}
