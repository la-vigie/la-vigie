import { useCallback, useEffect, useState } from "react";
import { ghStatus, getPrStatus, openUrl } from "../../api";
import type { PrStatus } from "../../api";
import { useVigieStore } from "../../store";
import { PrPanel } from "./PrPanel";
import "./PrDock.css";

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}

interface ChecksSummary {
  total: number;
  failing: number;
  pending: number;
}

function summarizeChecks(pr: PrStatus | null): ChecksSummary | null {
  if (!pr || pr.checks.length === 0) return null;
  return {
    total: pr.checks.length,
    failing: pr.checks.filter((c) => c.status === "failure").length,
    pending: pr.checks.filter((c) => c.status === "pending").length,
  };
}

function ChecksGlance({ summary }: { summary: ChecksSummary }) {
  if (summary.failing > 0) {
    return (
      <span className="pr-dock__checks pr-dock__checks--fail">
        ✗ {summary.failing} failing
      </span>
    );
  }
  if (summary.pending > 0) {
    return (
      <span className="pr-dock__checks pr-dock__checks--pending">
        … {summary.pending} pending
      </span>
    );
  }
  return (
    <span className="pr-dock__checks pr-dock__checks--ok">
      ✓ {summary.total} checks
    </span>
  );
}

// PR-state pill colour, matching the GitHub semantics used elsewhere.
function stateClass(state: string): string {
  switch (state.toUpperCase()) {
    case "OPEN":
      return "pr-dock__state--open";
    case "MERGED":
      return "pr-dock__state--merged";
    case "CLOSED":
      return "pr-dock__state--closed";
    default:
      return "";
  }
}

export interface PrDockProps {
  taskId: string;
  refreshToken?: number;
}

/**
 * "Option D" — the Pull request as a collapsible bottom dock under the Changes
 * diff. Collapsed it is a one-line status bar (state / mergeable / checks);
 * expanded it shows the full PrPanel and can be drag-resized via the top grip.
 *
 * The dock owns a lightweight PR-status fetch so the collapsed bar stays
 * glanceable without expanding; the expanded body reuses PrPanel for the full
 * create/comments/checks functionality.
 */
export function PrDock({ taskId, refreshToken }: PrDockProps) {
  const tasks = useVigieStore((s) => s.tasks);
  const repos = useVigieStore((s) => s.repos);
  const task = tasks.find((t) => t.id === taskId);
  const repo = repos.find((r) => r.id === task?.repoId);

  const [collapsed, setCollapsed] = useState<boolean>(
    () => localStorage.getItem("vigie.prDockCollapsed") !== "false",
  );
  const [height, setHeight] = useState<number>(
    () => Number(localStorage.getItem("vigie.prDockHeight")) || 240,
  );
  const [pr, setPr] = useState<PrStatus | null>(null);

  useEffect(() => {
    localStorage.setItem("vigie.prDockCollapsed", String(collapsed));
  }, [collapsed]);

  useEffect(() => {
    localStorage.setItem("vigie.prDockHeight", String(height));
  }, [height]);

  // Lightweight status fetch for the bar/header. Only when gh is usable and the
  // repo has a remote; failures are swallowed (the dock just shows "no PR").
  const loadSummary = useCallback(async () => {
    try {
      const gh = await ghStatus();
      if (!gh.available || !gh.authenticated || !repo?.remoteUrl) {
        setPr(null);
        return;
      }
      setPr(await getPrStatus(taskId));
    } catch {
      setPr(null);
    }
  }, [taskId, repo?.remoteUrl]);

  useEffect(() => {
    loadSummary();
  }, [loadSummary, refreshToken]);

  const handleGripMouseDown = (e: React.MouseEvent) => {
    const startY = e.clientY;
    const startH = height;
    const onMove = (ev: MouseEvent) => {
      setHeight(clamp(startH + (startY - ev.clientY), 120, 560));
    };
    const onUp = () => {
      document.body.style.userSelect = "";
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    document.body.style.userSelect = "none";
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  const checks = summarizeChecks(pr);

  // ── Collapsed: one-line status bar ────────────────────────────────────────
  if (collapsed) {
    return (
      <button
        type="button"
        className="pr-dock__bar"
        onClick={() => setCollapsed(false)}
        aria-expanded={false}
        aria-label="Expand pull request dock"
      >
        <span className="pr-dock__bar-title">Pull request</span>
        {pr ? (
          <>
            <span className="pr-dock__num">#{pr.number}</span>
            <span className={"pr-dock__state " + stateClass(pr.state)}>
              {pr.state}
            </span>
            <span className="pr-dock__sep">·</span>
            <span className="pr-dock__mergeable">
              {pr.mergeable === "MERGEABLE" ? "mergeable" : pr.mergeable.toLowerCase()}
            </span>
            {checks && (
              <>
                <span className="pr-dock__sep">·</span>
                <ChecksGlance summary={checks} />
              </>
            )}
          </>
        ) : (
          <span className="pr-dock__muted">no PR yet</span>
        )}
        <span className="pr-dock__spacer" />
        <span className="pr-dock__chevron" aria-hidden>⌃</span>
      </button>
    );
  }

  // ── Expanded: grip + header + full PrPanel ────────────────────────────────
  return (
    <section className="pr-dock" style={{ height }}>
      <div
        className="pr-dock__grip"
        role="separator"
        aria-orientation="horizontal"
        aria-label="Resize pull request dock"
        onMouseDown={handleGripMouseDown}
      >
        <span className="pr-dock__grip-handle" aria-hidden />
      </div>
      <div className="pr-dock__header">
        <span className="pr-dock__bar-title">Pull request</span>
        {pr ? (
          <>
            <span className="pr-dock__num">#{pr.number}</span>
            <span className={"pr-dock__state " + stateClass(pr.state)}>
              {pr.state}
            </span>
          </>
        ) : (
          <span className="pr-dock__muted">no PR yet</span>
        )}
        {task && (
          <span className="pr-dock__branches">
            {task.branch} → {task.baseBranch}
          </span>
        )}
        <span className="pr-dock__spacer" />
        {pr?.url && (
          <button
            type="button"
            className="icon-btn"
            aria-label="Open pull request in browser"
            title="Open in browser"
            onClick={() => openUrl(pr.url).catch(() => {})}
          >
            ↗
          </button>
        )}
        <button
          type="button"
          className="icon-btn"
          aria-label="Collapse pull request dock"
          onClick={() => setCollapsed(true)}
        >
          ⌄
        </button>
      </div>
      <div className="pr-dock__body">
        <PrPanel taskId={taskId} refreshToken={refreshToken} />
      </div>
    </section>
  );
}
