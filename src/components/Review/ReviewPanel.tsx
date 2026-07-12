import { useEffect, useRef, useState } from "react";
import { getChangedFiles } from "../../api";
import type { DiffScope } from "../../api";
import { useVigieStore } from "../../store";
import { DiffPanel } from "../Diff/DiffPanel";
import { PrDock } from "../Pr/PrDock";
import { SpecDock } from "../Spec/SpecDock";
import "./Review.css";

export interface ReviewPanelProps {
  taskId: string;
  showDiff: boolean;
  onToggleDiff: () => void;
  diffPosition: "right" | "bottom";
  onSetDiffPosition: (p: "right" | "bottom") => void;
  /** When true the Spec/Docs dock fills the whole review pane (Changes + PR
   *  hidden). Owned by TaskDetail because maximizing also collapses the
   *  terminal — see TaskDetail's specMaximized. */
  specMaximized: boolean;
  onToggleSpecMaximize: () => void;
}

/**
 * "Option D" layout: the Changes diff owns the review pane; the Pull request
 * lives in a collapsible bottom dock (PrDock).
 *
 * Changes has two scopes (TASK-18): "Uncommitted" (working tree vs HEAD,
 * commit-able) and "Compared to <base>" (the whole branch diff, read-only).
 * The shared Refresh re-fetches the diff, the scope counts, and the dock.
 */
export function ReviewPanel({ taskId, diffPosition, onSetDiffPosition, onToggleDiff, specMaximized, onToggleSpecMaximize }: ReviewPanelProps) {
  const baseBranch = useVigieStore(
    (s) => s.tasks.find((t) => t.id === taskId)?.baseBranch ?? "base",
  );
  const [scope, setScope] = useState<DiffScope>(
    () => (localStorage.getItem("vigie.diffScope") === "base" ? "base" : "uncommitted"),
  );
  // Bumping this asks the diff, the scope counts, and the PR dock to re-fetch.
  const [refreshToken, setRefreshToken] = useState(0);
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const [counts, setCounts] = useState<{ uncommitted: number | null; base: number | null }>({
    uncommitted: null,
    base: null,
  });

  useEffect(() => {
    localStorage.setItem("vigie.diffScope", scope);
  }, [scope]);

  useEffect(() => {
    if (!menuOpen) return;
    const handleMouseDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMenuOpen(false);
    };
    document.addEventListener("mousedown", handleMouseDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleMouseDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [menuOpen]);

  // File counts for both scopes, shown as badges on the toggle. Failures fall
  // back to null (badge hidden) rather than surfacing an error here.
  useEffect(() => {
    let cancelled = false;
    Promise.all([
      getChangedFiles(taskId, "uncommitted").then((f) => f.length).catch(() => null),
      getChangedFiles(taskId, "base").then((f) => f.length).catch(() => null),
    ]).then(([uncommitted, base]) => {
      if (!cancelled) setCounts({ uncommitted, base });
    });
    return () => {
      cancelled = true;
    };
  }, [taskId, refreshToken]);

  return (
    <div className={"review-panel" + (specMaximized ? " review-panel--spec-max" : "")}>
      {!specMaximized && (
      <div className="review-panel__changes">
        <div className="review-panel__changes-header">
          <span className="review-panel__changes-title">Changes</span>
          <div className="segmented review-panel__scope">
            <button
              type="button"
              className={
                "segmented__option" +
                (scope === "uncommitted" ? " segmented__option--active" : "")
              }
              aria-pressed={scope === "uncommitted"}
              onClick={() => setScope("uncommitted")}
            >
              Uncommitted
              {counts.uncommitted != null && (
                <span className="review-panel__count">{counts.uncommitted}</span>
              )}
            </button>
            <button
              type="button"
              className={
                "segmented__option" +
                (scope === "base" ? " segmented__option--active" : "")
              }
              aria-pressed={scope === "base"}
              onClick={() => setScope("base")}
            >
              Compared to {baseBranch}
              {counts.base != null && (
                <span className="review-panel__count">{counts.base}</span>
              )}
            </button>
          </div>
          <span className="review-panel__changes-spacer" />
          <button
            type="button"
            className="icon-btn"
            aria-label="Refresh changes"
            onClick={() => setRefreshToken((t) => t + 1)}
          >
            ↻
          </button>
          <div className="review-panel__menu" ref={menuRef}>
            <button type="button" className="icon-btn" aria-haspopup="menu" aria-expanded={menuOpen} aria-label="Diff options" onClick={() => setMenuOpen((o) => !o)}>…</button>
            {menuOpen && (
              <div role="menu" className="review-panel__menu-list">
                <button role="menuitem" type="button" onClick={() => { onSetDiffPosition("right"); setMenuOpen(false); }}>Right split {diffPosition === "right" ? "✓" : ""}</button>
                <button role="menuitem" type="button" onClick={() => { onSetDiffPosition("bottom"); setMenuOpen(false); }}>Bottom split {diffPosition === "bottom" ? "✓" : ""}</button>
                <button role="menuitem" type="button" onClick={() => { onToggleDiff(); setMenuOpen(false); }}>Hide diff</button>
              </div>
            )}
          </div>
        </div>
        <div className="review-panel__changes-body">
          <DiffPanel
            taskId={taskId}
            scope={scope}
            readOnly={scope === "base"}
            commentable={scope === "base"}
            refreshToken={refreshToken}
          />
        </div>
      </div>
      )}
      <SpecDock
        taskId={taskId}
        refreshToken={refreshToken}
        maximized={specMaximized}
        onToggleMaximize={onToggleSpecMaximize}
      />
      {!specMaximized && <PrDock taskId={taskId} refreshToken={refreshToken} />}
    </div>
  );
}
