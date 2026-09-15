import type { PrStatus } from "../../api";
import type { Task } from "../../store";

// Pure view-model for the Finish-task modal. Keeping the
// context-surfacing + primary/secondary/danger decisions here (rather than
// inline JSX) makes them unit-testable without rendering.

export type FinishPrState = "open" | "merged" | "other" | "none";

export interface FinishModalView {
  /** Worktree branch for this task. */
  branch: string;
  /** Base branch the task is based on. */
  baseBranch: string;
  /** One-of the PR lifecycle, normalized from the free-text `PrStatus.state`. */
  prState: FinishPrState;
  /** Human text for the PR row, e.g. "#7 open", "#7 merged", "none". */
  prText: string;
  /** Whether to offer "Merge PR & finish" (only for an OPEN PR). */
  showMerge: boolean;
  /** Which safe action is the primary (btn--primary): merge when a PR is open,
   *  otherwise keep. */
  primaryMode: "merge" | "keep";
  /** Text for the uncommitted-changes row. */
  dirtyText: string;
  /** True while the uncommitted-changes count is still loading. */
  dirtyLoading: boolean;
  /** Whether the guarded Discard action is available. In-place tasks never
   *  delete their branch (teardown only detaches), so Discard is hidden — it
   *  would be a no-op. */
  showDiscard: boolean;
}

/**
 * Derive the Finish-modal view from the task, its PR status, and the count of
 * uncommitted (working-tree-vs-HEAD) changes.
 *
 * @param changedCount number of uncommitted changes, or null while loading.
 */
export function finishModalView(
  task: Pick<Task, "branch" | "baseBranch" | "inPlace">,
  pr: PrStatus | null,
  changedCount: number | null,
): FinishModalView {
  const rawState = (pr?.state ?? "").toUpperCase();
  let prState: FinishPrState;
  if (!pr) prState = "none";
  else if (rawState === "OPEN") prState = "open";
  else if (rawState === "MERGED") prState = "merged";
  else prState = "other";

  const prText = pr ? `#${pr.number} ${(pr.state || "unknown").toLowerCase()}` : "none";
  const showMerge = prState === "open";

  const dirtyLoading = changedCount === null;
  let dirtyText: string;
  if (dirtyLoading) dirtyText = "checking…";
  else if (changedCount === 0) dirtyText = "working tree clean";
  else
    dirtyText =
      changedCount === 1
        ? "1 uncommitted change — kept in the branch"
        : `${changedCount} uncommitted changes — kept in the branch`;

  return {
    branch: task.branch,
    baseBranch: task.baseBranch,
    prState,
    prText,
    showMerge,
    primaryMode: showMerge ? "merge" : "keep",
    dirtyText,
    dirtyLoading,
    showDiscard: !task.inPlace,
  };
}
