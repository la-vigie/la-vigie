import { useEffect, useState } from "react";
import type { Task } from "../../store";

interface DeleteTaskModalProps {
  task: Task;
  onCancel: () => void;
  onConfirm: (deleteBranch: boolean) => void | Promise<void>;
}

export function DeleteTaskModal({ task, onCancel, onConfirm }: DeleteTaskModalProps) {
  const [deleteBranch, setDeleteBranch] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel, busy]);

  const handleConfirm = async () => {
    setBusy(true);
    setError(null);
    try {
      await onConfirm(deleteBranch);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="new-task-modal__backdrop" role="presentation" onClick={onCancel}>
      <div
        className="new-task-modal"
        role="dialog"
        aria-modal="true"
        aria-label={`Delete task ${task.title}`}
        onClick={(e) => e.stopPropagation()}
      >
        <header className="new-task-modal__header">
          <h2 className="new-task-modal__title">Delete task "{task.title}"?</h2>
        </header>
        <div className="new-task-modal__body">
          <p>This removes the task's worktree and can't be undone.</p>
          <label className="delete-task-modal__checkbox">
            <input
              type="checkbox"
              checked={deleteBranch}
              onChange={(e) => setDeleteBranch(e.target.checked)}
            />{" "}
            Also delete the branch <code>{task.branch}</code>
          </label>
          {error && (
            <p className="delete-task-modal__error" role="alert">
              {error}
            </p>
          )}
        </div>
        <footer className="new-task-modal__footer">
          <button type="button" className="btn btn--ghost" onClick={onCancel} disabled={busy}>
            Cancel
          </button>
          <button type="button" className="btn btn--danger" onClick={handleConfirm} disabled={busy}>
            Delete
          </button>
        </footer>
      </div>
    </div>
  );
}
