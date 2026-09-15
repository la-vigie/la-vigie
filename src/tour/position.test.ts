import { describe, expect, it } from "vitest";
import { computeCoachMarkPosition, computeSpotlightRect, type Rect } from "./position";

const VIEWPORT = { width: 1000, height: 800 };
const TOOLTIP = { width: 300, height: 160 };

describe("computeCoachMarkPosition", () => {
  it("centers when there is no anchor", () => {
    const p = computeCoachMarkPosition(null, TOOLTIP, VIEWPORT);
    expect(p.placement).toBe("center");
    expect(p.left).toBeCloseTo((VIEWPORT.width - TOOLTIP.width) / 2);
    expect(p.top).toBeCloseTo((VIEWPORT.height - TOOLTIP.height) / 2);
  });

  it("places below an anchor with room underneath (auto)", () => {
    const anchor: Rect = { top: 100, left: 400, width: 120, height: 40 };
    const p = computeCoachMarkPosition(anchor, TOOLTIP, VIEWPORT);
    expect(p.placement).toBe("bottom");
    expect(p.top).toBeGreaterThan(anchor.top + anchor.height);
  });

  it("flips to top when there is no room below", () => {
    const anchor: Rect = { top: 700, left: 400, width: 120, height: 40 };
    const p = computeCoachMarkPosition(anchor, TOOLTIP, VIEWPORT);
    expect(p.placement).toBe("top");
    expect(p.top).toBeLessThan(anchor.top);
  });

  it("honors an explicit preferred side when it fits", () => {
    const anchor: Rect = { top: 380, left: 500, width: 120, height: 40 };
    const p = computeCoachMarkPosition(anchor, TOOLTIP, VIEWPORT, "right");
    expect(p.placement).toBe("right");
    expect(p.left).toBeGreaterThan(anchor.left + anchor.width);
  });

  it("flips away from a preferred side that overflows", () => {
    // Anchor hard against the right edge — prefer right, which can't fit;
    // it must flip to some other fitting side and stay fully on-screen.
    const anchor: Rect = { top: 380, left: 940, width: 50, height: 40 };
    const p = computeCoachMarkPosition(anchor, TOOLTIP, VIEWPORT, "right");
    expect(p.placement).not.toBe("right");
    expect(p.left + TOOLTIP.width).toBeLessThanOrEqual(VIEWPORT.width);
  });

  it("flips to the opposite side when only that side fits", () => {
    // A tall tooltip near the right edge with no vertical room forces left.
    const tall = { width: 300, height: 780 };
    const anchor: Rect = { top: 380, left: 940, width: 50, height: 40 };
    const p = computeCoachMarkPosition(anchor, tall, VIEWPORT, "right");
    expect(p.placement).toBe("left");
  });

  it("clamps the bubble fully on-screen", () => {
    const anchor: Rect = { top: 10, left: 5, width: 20, height: 20 };
    const p = computeCoachMarkPosition(anchor, TOOLTIP, VIEWPORT);
    expect(p.left).toBeGreaterThanOrEqual(0);
    expect(p.top).toBeGreaterThanOrEqual(0);
    expect(p.left + TOOLTIP.width).toBeLessThanOrEqual(VIEWPORT.width);
    expect(p.top + TOOLTIP.height).toBeLessThanOrEqual(VIEWPORT.height);
  });
});

describe("computeSpotlightRect", () => {
  it("grows the anchor by padding on every side", () => {
    const anchor: Rect = { top: 100, left: 200, width: 120, height: 40 };
    const r = computeSpotlightRect(anchor, 8);
    expect(r.top).toBe(92);
    expect(r.left).toBe(192);
    expect(r.width).toBe(136);
    expect(r.height).toBe(56);
  });

  it("clips origin to 0 and shrinks the box near the top-left edge", () => {
    const anchor: Rect = { top: 2, left: 3, width: 100, height: 50 };
    const r = computeSpotlightRect(anchor, 8);
    expect(r.top).toBe(0);
    expect(r.left).toBe(0);
    // padded 116 wide, but 5px clipped off the left (rawLeft = -5)
    expect(r.width).toBe(116 - 5);
    // padded 66 tall, but 6px clipped off the top (rawTop = -6)
    expect(r.height).toBe(66 - 6);
  });

  it("never yields a negative width/height for an anchor far off-screen", () => {
    const anchor: Rect = { top: -2000, left: -1000, width: 50, height: 40 };
    const r = computeSpotlightRect(anchor, 6);
    expect(r.width).toBeGreaterThanOrEqual(0);
    expect(r.height).toBeGreaterThanOrEqual(0);
  });
});
