import { beforeEach, describe, expect, it } from "vitest";
import { useVigieStore } from "./index";
import { createTour } from "../tour/tourMachine";
import { TOUR_STEPS } from "../tour/steps";
import { loadOnboarding } from "../tour/persistence";

// The store's onboarding slice delegates to the pure machine and persists to
// localStorage. These tests drive the actions and assert both the in-memory
// state and the persisted record.
describe("store onboarding slice", () => {
  beforeEach(() => {
    localStorage.clear();
    useVigieStore.setState({
      onboarding: createTour(TOUR_STEPS),
      onboardingStatus: "pending",
    });
  });

  it("startOnboarding activates and persists 'active'", () => {
    useVigieStore.getState().startOnboarding();
    expect(useVigieStore.getState().onboarding.status).toBe("active");
    expect(useVigieStore.getState().onboardingStatus).toBe("active");
    expect(loadOnboarding().status).toBe("active");
  });

  it("onboardingNext advances and persists the step id", () => {
    useVigieStore.getState().startOnboarding();
    useVigieStore.getState().onboardingNext();
    const s = useVigieStore.getState().onboarding;
    expect(s.index).toBe(1);
    expect(loadOnboarding().stepId).toBe(TOUR_STEPS[1].id);
  });

  it("onboardingPrev clamps at the first step", () => {
    useVigieStore.getState().startOnboarding();
    useVigieStore.getState().onboardingPrev();
    expect(useVigieStore.getState().onboarding.index).toBe(0);
  });

  it("advancing off the last step completes and persists 'completed'", () => {
    useVigieStore.getState().startOnboarding();
    // Walk to the end.
    for (let i = 0; i < TOUR_STEPS.length; i += 1) {
      useVigieStore.getState().onboardingNext();
    }
    expect(useVigieStore.getState().onboarding.status).toBe("completed");
    expect(useVigieStore.getState().onboardingStatus).toBe("completed");
    expect(loadOnboarding().status).toBe("completed");
  });

  it("skipOnboarding dismisses and persists 'dismissed'", () => {
    useVigieStore.getState().startOnboarding();
    useVigieStore.getState().skipOnboarding();
    expect(useVigieStore.getState().onboarding.status).toBe("dismissed");
    expect(useVigieStore.getState().onboardingStatus).toBe("dismissed");
    expect(loadOnboarding().status).toBe("dismissed");
  });
});
