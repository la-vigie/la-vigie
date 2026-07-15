import { createContext, useEffect, useRef } from "react";
import type { RefObject } from "react";
import type { Terminal } from "@xterm/xterm";

// Deterministic terminal sizing (TASK-227, approach B from the TASK-220 design
// analysis 396e3ee2).
//
// The recurring "narrow terminal on surface switch" bug (TASK-220 and
// predecessors) came from measuring the per-surface container div right after it
// flipped `display:none → block`: the WKWebView hasn't laid it out yet, so the
// read is a collapsed width, which drives a tiny xterm grid and — if propagated —
// a tiny PTY that hard-wraps the Claude TUI one word wide into permanent
// scrollback. PR #150/#156 patched this reactively (settle loop + a MIN_PTY_COLS
// PTY-sync floor).
//
// This module removes the transient at its source. `.terminal-pane__body`
// (TaskDetail's `terminalPaneRef`) is the ONE container that is never
// `display:none` — every surface renders into it via <TerminalHost/>, and only
// the per-TerminalView outer divs toggle visibility. So its pixel size is always
// known immediately, on every switch, without measuring the just-unhidden child.
// We observe that pane once, cache its size, cache the char-cell size (a pure
// function of the 13px font, identical for every surface), and compute the grid
// arithmetically — the same computation @xterm/addon-fit does, but sourced from
// the invariant pane instead of the collapsing child.

export interface PaneSize {
  width: number;
  height: number;
}

export interface CellSize {
  width: number;
  height: number;
}

// xterm reserves space for the scrollbar when computing columns. With our
// terminal options (default scrollback, no overviewRuler) @xterm/addon-fit uses
// `overviewRuler?.width || 14`, i.e. a constant 14px — mirror it so our column
// count matches FitAddon's to within its sub-pixel rounding.
export const XTERM_SCROLLBAR_WIDTH = 14;
// xterm's own floors (FitAddon uses max(2, …) cols / max(1, …) rows).
export const MINIMUM_COLS = 2;
export const MINIMUM_ROWS = 1;

/**
 * Compute the xterm grid directly from the pane's pixel box and the char-cell
 * size — the deterministic replacement for FitAddon measuring the child element.
 *
 * Returns `null` when either the cell or the pane has no real size yet (a pane
 * that hasn't been laid out reports 0×0). That null is the ONLY guard we keep
 * against pushing a degenerate size to the PTY — it is the never-firing
 * safety net that replaces TASK-220's `MIN_PTY_COLS`/`isSaneFit` floor: instead
 * of rejecting "suspiciously narrow" fits by a magic column count, we simply
 * never compute a grid from an unmeasured pane, because we never measure the
 * collapsing child in the first place.
 */
export function computeGrid(
  paneWidth: number,
  paneHeight: number,
  cell: CellSize,
  scrollbarWidth: number = XTERM_SCROLLBAR_WIDTH,
): { cols: number; rows: number } | null {
  if (!(cell.width > 0) || !(cell.height > 0)) return null;
  if (!(paneWidth > 0) || !(paneHeight > 0)) return null;
  const availableWidth = paneWidth - scrollbarWidth;
  const cols = Math.max(MINIMUM_COLS, Math.floor(availableWidth / cell.width));
  const rows = Math.max(MINIMUM_ROWS, Math.floor(paneHeight / cell.height));
  if (!Number.isFinite(cols) || !Number.isFinite(rows)) return null;
  return { cols, rows };
}

// The char-cell size is identical for every surface (same 13px font), so the
// first terminal that is actually laid out measures it and every other surface
// — including ones mounted while hidden — reuses it. Module-level singleton
// rather than per-view state precisely because it is shared across all surfaces.
let cachedCell: CellSize | null = null;

export function getCachedCell(): CellSize | null {
  return cachedCell;
}

export function setCachedCell(cell: CellSize): void {
  // First valid measurement wins; the font never changes, so there is nothing
  // to update afterwards.
  if (!cachedCell && cell.width > 0 && cell.height > 0) {
    cachedCell = cell;
  }
}

// Test seam only — production never clears the cache (the font is constant).
export function resetCellCacheForTests(): void {
  cachedCell = null;
}

/**
 * Read the char-cell size from a laid-out xterm's render service. This is the
 * same private field @xterm/addon-fit reads
 * (`_core._renderService.dimensions.css.cell`); wrapped defensively so a
 * not-yet-rendered terminal (hidden or pre-first-frame) yields `null` instead of
 * throwing. Returns `null` — never a zeroed cell — so a caller never caches a
 * degenerate size.
 */
export function readCellSize(term: Terminal): CellSize | null {
  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const cell = (term as any)?._core?._renderService?.dimensions?.css?.cell;
    if (cell && cell.width > 0 && cell.height > 0) {
      return { width: cell.width, height: cell.height };
    }
  } catch {
    // Private-API shape changed or unavailable — fall through to null.
  }
  return null;
}

export type PaneSubscriber = (size: PaneSize) => void;

export interface PaneMetrics {
  /** The pane's current pixel box (content-box; 0×0 before first layout). */
  getSize: () => PaneSize;
  /** Subscribe to genuine pane resizes (window, split-drag, sidebar). */
  subscribe: (cb: PaneSubscriber) => () => void;
}

export const TerminalPaneMetricsContext = createContext<PaneMetrics | null>(null);

/**
 * Install a single ResizeObserver on the invariant pane and expose its size +
 * a subscription to every terminal via context. Because the pane is never
 * `display:none`, this fires with correct dimensions on every genuine resize
 * (window, split-drag, sidebar) — no per-surface measurement, no collapse
 * transient. The returned object identity is stable across renders so consumers
 * don't churn.
 */
export function useProvidePaneMetrics(paneRef: RefObject<HTMLElement | null>): PaneMetrics {
  const sizeRef = useRef<PaneSize>({ width: 0, height: 0 });
  const subscribersRef = useRef<Set<PaneSubscriber>>(new Set());
  const metricsRef = useRef<PaneMetrics | null>(null);
  if (metricsRef.current === null) {
    metricsRef.current = {
      getSize: () => sizeRef.current,
      subscribe: (cb) => {
        subscribersRef.current.add(cb);
        return () => {
          subscribersRef.current.delete(cb);
        };
      },
    };
  }

  useEffect(() => {
    const el = paneRef.current;
    if (!el) return;

    const publish = (width: number, height: number) => {
      sizeRef.current = { width, height };
      subscribersRef.current.forEach((cb) => cb(sizeRef.current));
    };

    // Seed from the current layout so a terminal that subscribes after the
    // observer's first fire (or in a webview that batches the initial callback)
    // still gets a real size.
    const rect = el.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) publish(rect.width, rect.height);

    // Guard for jsdom (unit tests) where ResizeObserver is absent — those tests
    // drive sizing through a stub PaneMetrics, not this observer.
    let observer: ResizeObserver | undefined;
    if (typeof ResizeObserver !== "undefined") {
      observer = new ResizeObserver((entries) => {
        // content-box excludes any pane padding (there is none today) so it maps
        // 1:1 to the terminal area every surface fills.
        const box = entries[0]?.contentRect;
        if (box) publish(box.width, box.height);
      });
      observer.observe(el);
    }

    // Belt-and-suspenders: in some webviews a flex child's ResizeObserver
    // doesn't fire reliably on an OS-window resize, so re-measure on window
    // resize too (kept from the pre-TASK-227 code for the same reason).
    const onWindowResize = () => {
      const r = el.getBoundingClientRect();
      publish(r.width, r.height);
    };
    window.addEventListener("resize", onWindowResize);

    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", onWindowResize);
    };
  }, [paneRef]);

  return metricsRef.current;
}
