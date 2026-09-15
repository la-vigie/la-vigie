import { beforeEach, describe, expect, it } from "vitest";
import { loadOnboarding, saveOnboarding } from "./persistence";
import { createTour, startTour, next, skip, complete } from "./tourMachine";
import { TOUR_STEPS } from "./steps";

const KEY = "vigie.onboarding";

describe("onboarding persistence", () => {
  beforeEach(() => localStorage.clear());

  it("absent record → first-run pending", () => {
    expect(loadOnboarding()).toEqual({ status: "pending", stepId: null });
  });

  it("corrupt JSON → pending (never throws)", () => {
    localStorage.setItem(KEY, "{not json");
    expect(loadOnboarding()).toEqual({ status: "pending", stepId: null });
  });

  it("unknown status → pending", () => {
    localStorage.setItem(KEY, JSON.stringify({ status: "banana", stepId: "x" }));
    expect(loadOnboarding().status).toBe("pending");
  });

  it("saves active tours with the current step id (resume point)", () => {
    let t = startTour(createTour(TOUR_STEPS));
    t = next(t); // move off welcome
    saveOnboarding(t);
    const loaded = loadOnboarding();
    expect(loaded.status).toBe("active");
    expect(loaded.stepId).toBe(TOUR_STEPS[1].id);
  });

  it("idle maps to pending; dismissed/completed clear the step id", () => {
    saveOnboarding(createTour(TOUR_STEPS));
    expect(loadOnboarding()).toEqual({ status: "pending", stepId: null });

    saveOnboarding(skip(startTour(createTour(TOUR_STEPS))));
    expect(loadOnboarding()).toEqual({ status: "dismissed", stepId: null });

    saveOnboarding(complete(startTour(createTour(TOUR_STEPS))));
    expect(loadOnboarding()).toEqual({ status: "completed", stepId: null });
  });

  it("round-trips a resume: saved active step id restarts there", () => {
    let t = startTour(createTour(TOUR_STEPS));
    t = next(t);
    t = next(t);
    saveOnboarding(t);
    const { stepId } = loadOnboarding();
    const resumed = startTour(createTour(TOUR_STEPS), stepId);
    expect(resumed.status).toBe("active");
    expect(resumed.steps[resumed.index].id).toBe(TOUR_STEPS[2].id);
  });
});
