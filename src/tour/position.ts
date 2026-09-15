// Pure coach-mark positioning helpers.
//
// Given the anchor's rect (or null), the tooltip size, and the viewport, decide
// where to place the coach-mark bubble and the spotlight cutout. Pure functions
// of the current rects — the overlay re-runs them on resize/scroll/mutation, so
// positioning is inherently resilient to layout changes (no cached geometry).

export interface Rect {
  top: number;
  left: number;
  width: number;
  height: number;
}

export interface Size {
  width: number;
  height: number;
}

export interface Viewport {
  width: number;
  height: number;
}

export type ResolvedPlacement = "top" | "bottom" | "left" | "right" | "center";
export type PreferredPlacement = ResolvedPlacement | "auto";

export interface Placed {
  top: number;
  left: number;
  placement: ResolvedPlacement;
}

/** Distance between the anchor edge and the coach-mark bubble. */
const DEFAULT_GAP = 12;
/** Minimum gap kept between the bubble and the viewport edge. */
const MARGIN = 8;

function clamp(value: number, min: number, max: number): number {
  if (max < min) return min;
  return Math.min(Math.max(value, min), max);
}

/** Does a bubble of `size` fit on `side` of `anchor` within `viewport`? */
function fits(
  side: "top" | "bottom" | "left" | "right",
  anchor: Rect,
  size: Size,
  viewport: Viewport,
  gap: number,
): boolean {
  switch (side) {
    case "top":
      return anchor.top - gap - size.height >= MARGIN;
    case "bottom":
      return anchor.top + anchor.height + gap + size.height <= viewport.height - MARGIN;
    case "left":
      return anchor.left - gap - size.width >= MARGIN;
    case "right":
      return anchor.left + anchor.width + gap + size.width <= viewport.width - MARGIN;
  }
}

/** Centered coordinates for an anchorless bubble. */
function centered(size: Size, viewport: Viewport): Placed {
  return {
    top: Math.max(MARGIN, (viewport.height - size.height) / 2),
    left: Math.max(MARGIN, (viewport.width - size.width) / 2),
    placement: "center",
  };
}

/**
 * Compute the coach-mark position.
 *
 * - `anchor: null` → centered card.
 * - Otherwise place on the preferred side; if it would overflow the viewport,
 *   flip to the first side that fits (bottom → top → right → left order, seeded
 *   by the preference). The final top/left is clamped so the bubble always stays
 *   fully on-screen.
 */
export function computeCoachMarkPosition(
  anchor: Rect | null,
  tooltip: Size,
  viewport: Viewport,
  prefer: PreferredPlacement = "auto",
  gap: number = DEFAULT_GAP,
): Placed {
  if (!anchor) return centered(tooltip, viewport);

  const order: Array<"top" | "bottom" | "left" | "right"> =
    prefer === "auto" || prefer === "center"
      ? ["bottom", "top", "right", "left"]
      : [prefer, ...(["bottom", "top", "right", "left"] as const).filter((s) => s !== prefer)];

  const side = order.find((s) => fits(s, anchor, tooltip, viewport, gap)) ?? order[0];

  const anchorCenterX = anchor.left + anchor.width / 2;
  const anchorCenterY = anchor.top + anchor.height / 2;

  let top: number;
  let left: number;
  switch (side) {
    case "top":
      top = anchor.top - gap - tooltip.height;
      left = anchorCenterX - tooltip.width / 2;
      break;
    case "bottom":
      top = anchor.top + anchor.height + gap;
      left = anchorCenterX - tooltip.width / 2;
      break;
    case "left":
      top = anchorCenterY - tooltip.height / 2;
      left = anchor.left - gap - tooltip.width;
      break;
    case "right":
      top = anchorCenterY - tooltip.height / 2;
      left = anchor.left + anchor.width + gap;
      break;
  }

  return {
    top: clamp(top, MARGIN, Math.max(MARGIN, viewport.height - tooltip.height - MARGIN)),
    left: clamp(left, MARGIN, Math.max(MARGIN, viewport.width - tooltip.width - MARGIN)),
    placement: side,
  };
}

/**
 * The spotlight cutout: the anchor rect grown by `padding` on every side and
 * clamped to non-negative origin. Used to draw the highlight ring / mask.
 */
export function computeSpotlightRect(anchor: Rect, padding: number): Rect {
  const rawTop = anchor.top - padding;
  const rawLeft = anchor.left - padding;
  return {
    top: Math.max(0, rawTop),
    left: Math.max(0, rawLeft),
    // When the padded rect runs off the top/left edge, clip the origin to 0 and
    // shrink the box by the clipped amount (Math.min(0, raw*) is negative).
    // Floor at 0 so an anchor scrolled fully off-screen can't yield a negative
    // (inverted) box.
    width: Math.max(0, anchor.width + padding * 2 + Math.min(0, rawLeft)),
    height: Math.max(0, anchor.height + padding * 2 + Math.min(0, rawTop)),
  };
}
