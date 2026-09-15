// Pure, side-effect-free state machine for the first-run product tour.
//
// This is the testable core (per repo convention): no localStorage, no
// Date.now, no React, no DOM. Reducers take a TourState and return a NEW
// TourState; the store slice (src/store/index.ts) wraps these with the
// localStorage persistence side-effect, and the <Tour/> overlay resolves
// each step's DOM anchor. Keeping the machine pure lets us unit-test every
// transition in isolation.

export type TourStatus = "idle" | "active" | "completed" | "dismissed";

export type TourSection = "core" | "schedules" | "remote";

export type TourPlacement = "top" | "bottom" | "left" | "right" | "auto";

export interface TourStep {
  /** Stable id — used for resume-by-id persistence (survives step reordering). */
  id: string;
  section?: TourSection;
  title: string;
  body: string;
  /**
   * data-tour key of the DOM node to anchor to, or null for an anchorless
   * centered card (welcome/finish, or a graceful fallback).
   */
  anchor: string | null;
  placement?: TourPlacement;
  /**
   * Optional reveal-action key. The overlay resolves this to a non-destructive
   * UI action (open a modal / select an existing task) that surfaces the anchor
   * when it isn't currently in the DOM. Never creates/starts/deletes anything.
   */
  reveal?: string;
}

export interface TourState {
  steps: TourStep[];
  index: number;
  status: TourStatus;
}

function clampIndex(index: number, length: number): number {
  if (length <= 0) return 0;
  return Math.min(Math.max(index, 0), length - 1);
}

/** Build an idle tour from a step list. */
export function createTour(steps: TourStep[]): TourState {
  return { steps, index: 0, status: "idle" };
}

/**
 * Activate the tour. When `atId` names a known step, resume there; otherwise
 * start at the first step. Unknown ids fall back to the first step (resilient
 * to a persisted id that no longer exists after a step-list change).
 */
export function startTour(state: TourState, atId?: string | null): TourState {
  let index = 0;
  if (atId != null) {
    const found = state.steps.findIndex((s) => s.id === atId);
    if (found >= 0) index = found;
  }
  return { ...state, index: clampIndex(index, state.steps.length), status: "active" };
}

/**
 * Advance. Past the last step this completes the tour (index pinned at the
 * last step so `currentStep` stays meaningful). No-op unless active.
 */
export function next(state: TourState): TourState {
  if (state.status !== "active") return state;
  if (state.steps.length === 0) return complete(state);
  if (state.index >= state.steps.length - 1) {
    return { ...state, status: "completed" };
  }
  return { ...state, index: state.index + 1 };
}

/** Go back one step (clamped at the first). No-op unless active. */
export function prev(state: TourState): TourState {
  if (state.status !== "active") return state;
  return { ...state, index: clampIndex(state.index - 1, state.steps.length) };
}

/** Jump to a step by id. No-op if the id is unknown or the tour isn't active. */
export function goTo(state: TourState, id: string): TourState {
  if (state.status !== "active") return state;
  const found = state.steps.findIndex((s) => s.id === id);
  if (found < 0) return state;
  return { ...state, index: found };
}

/** Skip/dismiss the tour (user opted out). */
export function skip(state: TourState): TourState {
  return { ...state, status: "dismissed" };
}

/** Mark the tour completed (finished the last step). */
export function complete(state: TourState): TourState {
  return { ...state, status: "completed" };
}

/** The step currently being shown, or null when there are no steps. */
export function currentStep(state: TourState): TourStep | null {
  if (state.steps.length === 0) return null;
  return state.steps[clampIndex(state.index, state.steps.length)] ?? null;
}

/** True when the current index is the final step. */
export function isLast(state: TourState): boolean {
  return state.steps.length > 0 && state.index >= state.steps.length - 1;
}

/** True when the current index is the first step. */
export function isFirst(state: TourState): boolean {
  return state.index <= 0;
}
