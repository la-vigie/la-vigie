import { useEffect, useState } from "react";
import { getSetupState } from "../../api";
import { useVigieStore } from "../../store";

/**
 * Compact, dismissible setup strip shown in the task-detail header.
 * - While running / on failure it shows a one-line strip with a log toggle and ✕.
 * - On success it auto-hides (zero real estate) — the run already streamed live.
 * - ✕ dismisses the strip for the rest of the session.
 */
export function SetupPanel({ taskId }: { taskId: string }) {
  const entry = useVigieStore((s) => s.setupByTask[taskId]);
  const hydrateSetup = useVigieStore((s) => s.hydrateSetup);
  const dismissSetup = useVigieStore((s) => s.dismissSetup);
  const [expanded, setExpanded] = useState(false);

  // On task switch, hydrate from the backend so navigating back shows the
  // accumulated log/status even though events only fire while subscribed.
  useEffect(() => {
    let cancelled = false;
    setExpanded(false);
    (async () => {
      try {
        const state = await getSetupState(taskId);
        if (cancelled || !state || state.status == null) return;
        hydrateSetup(taskId, state.status, state.log, state.exitCode);
      } catch {
        // backend may not have setup data yet; ignore
      }
    })();
    return () => { cancelled = true; };
  }, [taskId, hydrateSetup]);

  // Hidden when: no state, dismissed, or finished successfully (auto-hide).
  if (!entry || entry.dismissed || entry.status === "succeeded") return null;

  const label =
    entry.status === "failed"
      ? typeof entry.exitCode === "number"
        ? `Setup failed (exit ${entry.exitCode})`
        : "Setup failed"
      : "Setting up…";

  return (
    <div
      className={`task-detail__setup task-detail__setup--${entry.status}`}
      role="status"
    >
      <div className="task-detail__setup-strip">
        <span
          className={`task-detail__setup-dot task-detail__setup-dot--${entry.status}`}
          aria-hidden
        />
        <span className="task-detail__setup-label">{label}</span>
        {entry.log && (
          <button
            type="button"
            className="task-detail__setup-toggle"
            aria-expanded={expanded}
            onClick={() => setExpanded((v) => !v)}
          >
            {expanded ? "hide log" : "log"}
          </button>
        )}
        <button
          type="button"
          className="task-detail__setup-close"
          aria-label="Dismiss setup status"
          onClick={() => dismissSetup(taskId)}
        >
          ✕
        </button>
      </div>
      {expanded && entry.log && (
        <pre className="task-detail__setup-log">{entry.log}</pre>
      )}
    </div>
  );
}
