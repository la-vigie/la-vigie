import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Tour } from "./Tour";
import { useVigieStore } from "../../store";
import { createTour, startTour } from "../../tour/tourMachine";
import { TOUR_STEPS } from "../../tour/steps";
import { loadOnboarding } from "../../tour/persistence";

// jsdom lacks ResizeObserver, which the overlay uses to track the anchor.
class RO {
  observe() {}
  unobserve() {}
  disconnect() {}
}

function injectAnchor(key: string): HTMLElement {
  const el = document.createElement("button");
  el.setAttribute("data-tour", key);
  document.body.appendChild(el);
  return el;
}

describe("Tour overlay", () => {
  beforeEach(() => {
    vi.stubGlobal("ResizeObserver", RO);
    localStorage.clear();
    useVigieStore.setState({
      onboarding: createTour(TOUR_STEPS),
      onboardingStatus: "pending",
      tasks: [],
      selectedTaskId: null,
    });
  });

  afterEach(() => {
    document.querySelectorAll("[data-tour]").forEach((n) => n.remove());
    vi.unstubAllGlobals();
  });

  it("auto-starts a pending tour and shows the centered welcome step", async () => {
    render(<Tour />);
    await waitFor(() => expect(screen.getByRole("dialog", { name: "Product tour" })).toBeTruthy());
    expect(screen.getByText(TOUR_STEPS[0].title)).toBeTruthy();
    // welcome has no anchor → full backdrop, no spotlight
    expect(document.querySelector(".tour__backdrop")).toBeTruthy();
    expect(document.querySelector(".tour__spotlight")).toBeNull();
    expect(useVigieStore.getState().onboardingStatus).toBe("active");
  });

  it("does not render when the tour is completed/dismissed", () => {
    useVigieStore.setState({ onboarding: createTour(TOUR_STEPS), onboardingStatus: "completed" });
    render(<Tour />);
    expect(document.querySelector(".tour")).toBeNull();
  });

  it("Next advances and persists the step id; anchored step shows a spotlight", async () => {
    injectAnchor("add-repo");
    render(<Tour />);
    await waitFor(() => expect(screen.getByText(TOUR_STEPS[0].title)).toBeTruthy());

    act(() => screen.getByRole("button", { name: "Next" }).click());

    await waitFor(() => expect(screen.getByText(TOUR_STEPS[1].title)).toBeTruthy());
    await waitFor(() => expect(document.querySelector(".tour__spotlight")).toBeTruthy());
    expect(loadOnboarding().stepId).toBe(TOUR_STEPS[1].id);
  });

  it("Back returns to the previous step", async () => {
    render(<Tour />);
    await waitFor(() => screen.getByText(TOUR_STEPS[0].title));
    act(() => screen.getByRole("button", { name: "Next" }).click());
    await waitFor(() => screen.getByText(TOUR_STEPS[1].title));
    act(() => screen.getByRole("button", { name: "Back" }).click());
    await waitFor(() => expect(screen.getByText(TOUR_STEPS[0].title)).toBeTruthy());
  });

  it("Skip dismisses the tour, unmounts the overlay, and persists 'dismissed'", async () => {
    render(<Tour />);
    await waitFor(() => screen.getByRole("dialog", { name: "Product tour" }));
    act(() => screen.getByRole("button", { name: "Skip tour" }).click());
    await waitFor(() => expect(document.querySelector(".tour")).toBeNull());
    expect(loadOnboarding().status).toBe("dismissed");
  });

  it("Escape within the bubble skips the tour (scoped, not global)", async () => {
    render(<Tour />);
    await waitFor(() => screen.getByRole("dialog", { name: "Product tour" }));
    // Scoped to the bubble: a keydown on a control inside it bubbles to the
    // bubble's onKeyDown. (A global window handler would hijack real inputs.)
    fireEvent.keyDown(screen.getByRole("button", { name: "Next" }), { key: "Escape" });
    await waitFor(() => expect(document.querySelector(".tour")).toBeNull());
  });

  it("does not attach a global keydown handler that hijacks Enter/Arrows", async () => {
    render(<Tour />);
    await waitFor(() => screen.getByText(TOUR_STEPS[0].title));
    // A keydown dispatched on window (as if typing in a real input elsewhere)
    // must NOT advance or dismiss the tour.
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight" }));
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(useVigieStore.getState().onboarding.status).toBe("active");
    expect(useVigieStore.getState().onboarding.index).toBe(0);
  });

  it("degrades to a centered card when an anchored step's node is absent", async () => {
    // Start directly on an anchored step whose DOM node was never injected.
    useVigieStore.setState({ onboardingStatus: "completed" });
    render(<Tour />);
    act(() => {
      useVigieStore.getState().startOnboarding();
      useVigieStore.getState().onboardingNext(); // -> add-repo (anchor absent)
    });
    await waitFor(() => expect(screen.getByText(TOUR_STEPS[1].title)).toBeTruthy());
    await waitFor(() => expect(document.querySelector(".tour__backdrop")).toBeTruthy());
    expect(document.querySelector(".tour__spotlight")).toBeNull();
  });

  it("select-first-task reveal does NOT yank the user off an orchestrator surface", async () => {
    // On an orchestrator surface (no task selected) the core-loop anchors are
    // absent, so the reveal fires — but it must not select a task, which would
    // clear selectedOrchestratorRepoId.
    useVigieStore.setState({
      tasks: [{ id: "t1" } as never],
      selectedTaskId: null,
      selectedOrchestratorRepoId: "r1",
      onboarding: startTour(createTour(TOUR_STEPS), "start-agent"), // reveal step, anchor absent
      onboardingStatus: "active",
    });
    render(<Tour />);
    await waitFor(() => expect(screen.getByText(/Start an agent/)).toBeTruthy());
    // Let the reveal + rAF retries run.
    await new Promise((r) => setTimeout(r, 60));
    expect(useVigieStore.getState().selectedOrchestratorRepoId).toBe("r1");
    expect(useVigieStore.getState().selectedTaskId).toBeNull();
  });

  it("repositions on window resize without crashing", async () => {
    injectAnchor("add-repo");
    render(<Tour />);
    await waitFor(() => screen.getByText(TOUR_STEPS[0].title));
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(screen.getByRole("dialog", { name: "Product tour" })).toBeTruthy();
  });

  it("shows Done on the last step and completing it persists 'completed'", async () => {
    render(<Tour />);
    await waitFor(() => screen.getByText(TOUR_STEPS[0].title));
    // Walk to the last step.
    for (let i = 0; i < TOUR_STEPS.length - 1; i += 1) {
      act(() => useVigieStore.getState().onboardingNext());
    }
    await waitFor(() => expect(screen.getByRole("button", { name: "Done" })).toBeTruthy());
    act(() => screen.getByRole("button", { name: "Done" }).click());
    await waitFor(() => expect(document.querySelector(".tour")).toBeNull());
    expect(loadOnboarding().status).toBe("completed");
  });
});
