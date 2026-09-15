import { useEffect, useState } from "react";
import { getChangedFiles, getPrStatus, type PrStatus } from "../../api";
import { useVigieStore } from "../../store";
import type { Task } from "../../store";
import { finishModalView } from "./finishModalView";

interface FinishTaskModalProps {
  task: Task;
  onClose: () => void;
}

/**
 * Shared Finish-task confirmation modal. Opened from both the
 * TaskDetail header "Finish task" button and the Sidebar right-click "Finish…"
 * item. Surfaces branch / base / PR-state / uncommitted-changes as text, with a
 * clear primary (Merge or Keep) / secondary / guarded-danger (Discard)
 * hierarchy.
 *
 * KEEP-ALIVE: this renders a fixed-position backdrop as a plain sibling in the
 * caller's tree — it never wraps or remounts <TerminalHost/>. Teardown of the
 * selected task's PTY is delegated to the store `finishTask` action (same path
 * as `deleteTask`), which stops sessions before clearing selection.
 */
export function FinishTaskModal({ task, onClose }: FinishTaskModalProps) {
  const finishTask = useVigieStore((s) => s.finishTask);
  const [pr, setPr] = useState<PrStatus | null>(null);
  const [changedCount, setChangedCount] = useState<number | null>(null);
  // Two-step guard for the destructive Discard: de-emphasized until "armed",
  // then an explicit red confirm. Pure gate — trivially unit-testable.
  const [discardArmed, setDiscardArmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Fetch PR status + uncommitted-change count for the context block. Both are
  // best-effort: a failure leaves the field in its "none"/clean-ish state
  // rather than blocking the modal.
  useEffect(() => {
    let cancelled = false;
    getPrStatus(task.id)
      .then((result) => { if (!cancelled) setPr(result); })
      .catch(() => { if (!cancelled) setPr(null); });
    getChangedFiles(task.id, "uncommitted")
      .then((files) => { if (!cancelled) setChangedCount(files.length); })
      .catch(() => { if (!cancelled) setChangedCount(0); });
    return () => { cancelled = true; };
  }, [task.id]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, busy]);

  const view = finishModalView(task, pr, changedCount);

  const run = async (mode: "keep" | "discard" | "merge") => {
    setBusy(true);
    setError(null);
    try {
      await finishTask(task.id, mode);
      // Selection/teardown handled by the store action; close the modal.
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setBusy(false);
    }
  };

  return (
    <div className="new-task-modal__backdrop" role="presentation" onClick={() => { if (!busy) onClose(); }}>
      <div
        className="new-task-modal finish-modal"
        role="dialog"
        aria-modal="true"
        aria-label={`Finish task ${task.title}`}
        onClick={(e) => e.stopPropagation()}
      >
        <header className="new-task-modal__header">
          <h2 className="new-task-modal__title">Finish "{task.title}"?</h2>
          <button type="button" className="new-task-modal__close" onClick={onClose} aria-label="Close" disabled={busy}>
            ✕
          </button>
        </header>

        <div className="new-task-modal__body">
          <dl className="finish-modal__context">
            <div className="finish-modal__row">
              <dt>Branch</dt>
              <dd><code>{view.branch}</code></dd>
            </div>
            <div className="finish-modal__row">
              <dt>Base</dt>
              <dd><code>{view.baseBranch}</code></dd>
            </div>
            <div className="finish-modal__row">
              <dt>Pull request</dt>
              <dd className={`finish-modal__pr finish-modal__pr--${view.prState}`}>{view.prText}</dd>
            </div>
            <div className="finish-modal__row">
              <dt>Uncommitted</dt>
              <dd>{view.dirtyText}</dd>
            </div>
          </dl>

          {/* Safe actions — primary/secondary hierarchy from the view model. */}
          <div className="finish-modal__actions">
            {view.showMerge && (
              <button
                type="button"
                className="btn btn--primary"
                onClick={() => run("merge")}
                disabled={busy}
              >
                Merge PR &amp; finish
              </button>
            )}
            <button
              type="button"
              className={"btn" + (view.primaryMode === "keep" ? " btn--primary" : "")}
              onClick={() => run("keep")}
              disabled={busy}
            >
              Keep branch
            </button>
          </div>
          <p className="finish-modal__hint">
            Keeping preserves the worktree's branch{view.showMerge ? "" : " so you can open a PR later"}.
          </p>

          {/* Danger zone — separated + de-emphasized + explicit two-step confirm.
              Hidden for in-place tasks (teardown never deletes their branch). */}
          {view.showDiscard && (
            <div className="finish-modal__danger">
              {!discardArmed ? (
                <button
                  type="button"
                  className="finish-modal__discard-arm"
                  onClick={() => setDiscardArmed(true)}
                  disabled={busy}
                >
                  Discard branch instead…
                </button>
              ) : (
                <div className="finish-modal__discard-confirm" role="group" aria-label="Confirm discard">
                  <p className="finish-modal__discard-warning">
                    This permanently deletes the worktree <strong>and</strong> the branch{" "}
                    <code>{view.branch}</code>. This can't be undone.
                  </p>
                  <div className="finish-modal__discard-buttons">
                    <button
                      type="button"
                      className="btn btn--ghost"
                      onClick={() => setDiscardArmed(false)}
                      disabled={busy}
                    >
                      Back
                    </button>
                    <button
                      type="button"
                      className="btn btn--danger"
                      onClick={() => run("discard")}
                      disabled={busy}
                    >
                      Discard {view.branch}
                    </button>
                  </div>
                </div>
              )}
              <p className="finish-modal__danger-aside">
                Just removing it from La Vigie? Right-click the task → <strong>Delete</strong> keeps the branch.
              </p>
            </div>
          )}

          {error && (
            <p className="finish-modal__error" role="alert">
              {error}
            </p>
          )}
        </div>

        <footer className="new-task-modal__footer">
          <button type="button" className="btn btn--ghost" onClick={onClose} disabled={busy}>
            Cancel
          </button>
        </footer>
      </div>
    </div>
  );
}
