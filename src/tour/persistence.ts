// localStorage persistence for the onboarding tour — the thin I/O shell around
// the pure tourMachine. Mirrors the `vigie.theme` pattern: read at store init,
// write on each transition. Persists only {status, stepId} (never the whole
// step list, which is code-defined), so a step-list change can't corrupt state.

import type { TourState } from "./tourMachine";

const KEY = "vigie.onboarding";

export type PersistedStatus = "pending" | "active" | "completed" | "dismissed";

export interface PersistedOnboarding {
  status: PersistedStatus;
  /** Last step id shown — used to resume an interrupted (active) tour. */
  stepId: string | null;
}

/** Read the persisted record. Absent/corrupt → first-run "pending". */
export function loadOnboarding(): PersistedOnboarding {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return { status: "pending", stepId: null };
    const parsed = JSON.parse(raw) as Partial<PersistedOnboarding>;
    const status = parsed.status;
    if (
      status === "pending" ||
      status === "active" ||
      status === "completed" ||
      status === "dismissed"
    ) {
      return { status, stepId: typeof parsed.stepId === "string" ? parsed.stepId : null };
    }
    return { status: "pending", stepId: null };
  } catch {
    return { status: "pending", stepId: null };
  }
}

/**
 * Persist a tour state. An `idle` machine maps back to `pending` (not started
 * yet); `active` records the current step id so the tour resumes there.
 */
export function saveOnboarding(state: TourState): void {
  const status: PersistedStatus = state.status === "idle" ? "pending" : state.status;
  const stepId = state.status === "active" ? (state.steps[state.index]?.id ?? null) : null;
  try {
    localStorage.setItem(KEY, JSON.stringify({ status, stepId } satisfies PersistedOnboarding));
  } catch {
    // Storage may be unavailable (private mode / quota) — the tour still works
    // for the session; it just won't remember across launches.
  }
}
