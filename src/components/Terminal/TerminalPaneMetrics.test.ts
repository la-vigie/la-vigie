import { afterEach, describe, expect, it } from "vitest";
import {
  computeGrid,
  getCachedCell,
  setCachedCell,
  resetCellCacheForTests,
  XTERM_SCROLLBAR_WIDTH,
  MINIMUM_COLS,
  MINIMUM_ROWS,
} from "./TerminalPaneMetrics";

describe("computeGrid", () => {
  it("derives the grid from the pane dimensions and cell size", () => {
    // cols = floor((800 − 14 scrollbar) / 8) = floor(98.25) = 98
    // rows = floor(480 / 16) = 30
    expect(computeGrid(800, 480, { width: 8, height: 16 })).toEqual({ cols: 98, rows: 30 });
  });

  it("subtracts the scrollbar width from the available width only (not height)", () => {
    const withScrollbar = computeGrid(1000, 500, { width: 10, height: 20 });
    // cols = floor((1000 − 14) / 10) = floor(98.6) = 98; rows = floor(500/20) = 25
    expect(withScrollbar).toEqual({ cols: 98, rows: 25 });
    // Passing scrollbar=0 recovers the un-deducted column count (100).
    expect(computeGrid(1000, 500, { width: 10, height: 20 }, 0)).toEqual({ cols: 100, rows: 25 });
    // The default equals the documented constant.
    expect(XTERM_SCROLLBAR_WIDTH).toBe(14);
  });

  it("returns null for an unmeasured pane (0×0) — the deterministic safety net", () => {
    expect(computeGrid(0, 0, { width: 8, height: 16 })).toBeNull();
    expect(computeGrid(0, 480, { width: 8, height: 16 })).toBeNull();
    expect(computeGrid(800, 0, { width: 8, height: 16 })).toBeNull();
    expect(computeGrid(-10, 480, { width: 8, height: 16 })).toBeNull();
  });

  it("returns null for a degenerate cell size (not-yet-measured renderer)", () => {
    expect(computeGrid(800, 480, { width: 0, height: 16 })).toBeNull();
    expect(computeGrid(800, 480, { width: 8, height: 0 })).toBeNull();
  });

  it("clamps to xterm's minimum cols/rows for a very small pane", () => {
    // availableWidth = 5 − 14 = −9 → floor(negative) then clamped up to the floor.
    const grid = computeGrid(5, 5, { width: 8, height: 16 });
    expect(grid).toEqual({ cols: MINIMUM_COLS, rows: MINIMUM_ROWS });
  });
});

describe("cell cache", () => {
  afterEach(() => resetCellCacheForTests());

  it("caches the first valid measurement and ignores later ones (font is constant)", () => {
    expect(getCachedCell()).toBeNull();
    setCachedCell({ width: 8, height: 16 });
    expect(getCachedCell()).toEqual({ width: 8, height: 16 });
    // A subsequent (different) measurement does not overwrite the first.
    setCachedCell({ width: 9, height: 18 });
    expect(getCachedCell()).toEqual({ width: 8, height: 16 });
  });

  it("ignores a degenerate (zero) measurement", () => {
    setCachedCell({ width: 0, height: 0 });
    expect(getCachedCell()).toBeNull();
  });
});
