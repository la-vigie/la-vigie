import { describe, expect, it } from "vitest";
import {
  complete,
  createTour,
  currentStep,
  goTo,
  isFirst,
  isLast,
  next,
  prev,
  skip,
  startTour,
  type TourStep,
} from "./tourMachine";

const STEPS: TourStep[] = [
  { id: "welcome", title: "Welcome", body: "", anchor: null },
  { id: "add-repo", section: "core", title: "Add repo", body: "", anchor: "add-repo" },
  { id: "new-task", section: "core", title: "New task", body: "", anchor: "new-task" },
  { id: "finish", title: "Finish", body: "", anchor: null },
];

describe("tourMachine", () => {
  it("createTour starts idle at index 0", () => {
    const t = createTour(STEPS);
    expect(t.status).toBe("idle");
    expect(t.index).toBe(0);
    expect(currentStep(t)?.id).toBe("welcome");
  });

  it("startTour activates at the first step by default", () => {
    const t = startTour(createTour(STEPS));
    expect(t.status).toBe("active");
    expect(t.index).toBe(0);
  });

  it("startTour resumes at a known id", () => {
    const t = startTour(createTour(STEPS), "new-task");
    expect(t.status).toBe("active");
    expect(currentStep(t)?.id).toBe("new-task");
  });

  it("startTour falls back to first step for an unknown id", () => {
    const t = startTour(createTour(STEPS), "does-not-exist");
    expect(t.index).toBe(0);
    expect(t.status).toBe("active");
  });

  it("next advances one step", () => {
    const t = next(startTour(createTour(STEPS)));
    expect(currentStep(t)?.id).toBe("add-repo");
    expect(t.status).toBe("active");
  });

  it("next past the last step completes the tour and pins the index", () => {
    let t = startTour(createTour(STEPS), "finish");
    expect(isLast(t)).toBe(true);
    t = next(t);
    expect(t.status).toBe("completed");
    expect(t.index).toBe(STEPS.length - 1);
    expect(currentStep(t)?.id).toBe("finish");
  });

  it("next is a no-op when not active", () => {
    const idle = createTour(STEPS);
    expect(next(idle)).toBe(idle);
  });

  it("prev goes back and clamps at the first step", () => {
    let t = startTour(createTour(STEPS), "new-task");
    t = prev(t);
    expect(currentStep(t)?.id).toBe("add-repo");
    t = prev(t);
    t = prev(t);
    expect(t.index).toBe(0);
    expect(isFirst(t)).toBe(true);
  });

  it("goTo jumps by id, ignores unknown ids", () => {
    const active = startTour(createTour(STEPS));
    expect(currentStep(goTo(active, "finish"))?.id).toBe("finish");
    expect(goTo(active, "nope")).toBe(active);
  });

  it("goTo is a no-op when not active", () => {
    const idle = createTour(STEPS);
    expect(goTo(idle, "finish")).toBe(idle);
  });

  it("skip dismisses; complete completes", () => {
    const active = startTour(createTour(STEPS));
    expect(skip(active).status).toBe("dismissed");
    expect(complete(active).status).toBe("completed");
  });

  it("reducers do not mutate the input state", () => {
    const active = startTour(createTour(STEPS));
    const before = JSON.stringify(active);
    next(active);
    prev(active);
    skip(active);
    goTo(active, "finish");
    expect(JSON.stringify(active)).toBe(before);
  });

  it("handles an empty step list without throwing", () => {
    const t = startTour(createTour([]));
    expect(currentStep(t)).toBeNull();
    expect(next(t).status).toBe("completed");
  });
});
