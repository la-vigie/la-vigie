import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useVigieStore } from "../../store";
import { currentStep, isFirst, isLast } from "../../tour/tourMachine";
import {
  computeCoachMarkPosition,
  computeSpotlightRect,
  type Placed,
  type Rect,
} from "../../tour/position";
import "./Tour.css";

// Non-destructive reveal actions: surface an anchor that isn't currently in the
// DOM WITHOUT creating/starting/deleting anything. Read live state via getState
// so we never act on a stale closure.
const REVEAL_ACTIONS: Record<string, () => void> = {
  "select-first-task": () => {
    const { tasks, selectedTaskId, selectedOrchestratorRepoId, setSelectedTask } =
      useVigieStore.getState();
    // Only auto-select when the user isn't already looking at a task OR an
    // orchestrator surface — selecting a task clears selectedOrchestratorRepoId,
    // so without the orchestrator guard this would silently yank a user off a
    // running orchestrator session they're watching.
    if (!selectedTaskId && !selectedOrchestratorRepoId && tasks.length > 0) {
      setSelectedTask(tasks[0].id);
    }
  },
};

// How many animation frames to keep looking for a late-mounting anchor after a
// reveal, before giving up and degrading to a centered card.
const RESOLVE_FRAMES = 8;
const SPOTLIGHT_PADDING = 6;

function anchorSelector(key: string): string {
  return `[data-tour="${key}"]`;
}

/**
 * First-run product tour. Rendered once from <App/>; portals to document.body
 * so it is a sibling of everything and an ancestor of NOTHING — in particular it
 * never wraps or remounts <TerminalHost/> (KEEP-ALIVE). It highlights real UI by
 * reading each anchor's getBoundingClientRect and drawing a spotlight + bubble
 * over it; it never inserts DOM into the highlighted subtree.
 */
export function Tour() {
  const onboarding = useVigieStore((s) => s.onboarding);
  const onboardingStatus = useVigieStore((s) => s.onboardingStatus);
  const startOnboarding = useVigieStore((s) => s.startOnboarding);
  const onboardingNext = useVigieStore((s) => s.onboardingNext);
  const onboardingPrev = useVigieStore((s) => s.onboardingPrev);
  const skipOnboarding = useVigieStore((s) => s.skipOnboarding);

  const active = onboarding.status === "active";
  const step = active ? currentStep(onboarding) : null;

  const bubbleRef = useRef<HTMLDivElement>(null);
  const [anchorEl, setAnchorEl] = useState<HTMLElement | null>(null);
  const [anchorRect, setAnchorRect] = useState<Rect | null>(null);
  const [bubbleSize, setBubbleSize] = useState({ width: 320, height: 180 });

  // Auto-start a first-run ("pending") tour once, after mount.
  useEffect(() => {
    if (onboardingStatus === "pending") startOnboarding();
  }, [onboardingStatus, startOnboarding]);

  // Resolve the current step's anchor element — with an optional non-destructive
  // reveal and a bounded rAF retry for late-mounting nodes. Degrade to centered
  // (anchorEl = null) if it never appears. Re-runs whenever the step changes.
  const stepId = step?.id;
  const stepAnchor = step?.anchor ?? null;
  const stepReveal = step?.reveal;
  useEffect(() => {
    if (!active) {
      setAnchorEl(null);
      return;
    }
    if (!stepAnchor) {
      setAnchorEl(null);
      return;
    }
    // Clear the previous step's anchor up front so the spotlight/bubble never
    // keep highlighting the old element during the reveal + rAF-retry window.
    // If the new anchor is already in the DOM, tryResolve() sets it back in the
    // same synchronous pass (React batches → no flicker).
    setAnchorEl(null);
    let cancelled = false;
    let frame = 0;
    let raf = 0;
    let revealed = false;
    const tryResolve = () => {
      if (cancelled) return;
      const el = document.querySelector<HTMLElement>(anchorSelector(stepAnchor));
      if (el) {
        setAnchorEl(el);
        return;
      }
      if (stepReveal && !revealed) {
        revealed = true;
        REVEAL_ACTIONS[stepReveal]?.();
      }
      if (frame < RESOLVE_FRAMES) {
        frame += 1;
        raf = requestAnimationFrame(tryResolve);
      } else {
        setAnchorEl(null); // give up → centered fallback
      }
    };
    tryResolve();
    return () => {
      cancelled = true;
      if (raf) cancelAnimationFrame(raf);
    };
  }, [active, stepId, stepAnchor, stepReveal]);

  // Track the resolved anchor's rect, kept fresh across layout/resize/scroll.
  useLayoutEffect(() => {
    if (!anchorEl) {
      setAnchorRect(null);
      return;
    }
    const measure = () => {
      const r = anchorEl.getBoundingClientRect();
      setAnchorRect({ top: r.top, left: r.left, width: r.width, height: r.height });
    };
    // rAF-coalesce: the capturing scroll listener fires for scrolls of ANY
    // element in the app, so measure at most once per frame to avoid jank.
    let raf = 0;
    const scheduleMeasure = () => {
      if (raf) return;
      raf = requestAnimationFrame(() => {
        raf = 0;
        measure();
      });
    };
    measure();
    const ro = new ResizeObserver(scheduleMeasure);
    ro.observe(anchorEl);
    window.addEventListener("resize", scheduleMeasure);
    window.addEventListener("scroll", scheduleMeasure, true);
    return () => {
      if (raf) cancelAnimationFrame(raf);
      ro.disconnect();
      window.removeEventListener("resize", scheduleMeasure);
      window.removeEventListener("scroll", scheduleMeasure, true);
    };
  }, [anchorEl]);

  // Measure the bubble so positioning can flip/clamp against its real size.
  useLayoutEffect(() => {
    const el = bubbleRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setBubbleSize((prev) =>
      prev.width === r.width && prev.height === r.height
        ? prev
        : { width: r.width, height: r.height },
    );
  }, [stepId, anchorRect]);

  // Force a re-render on window resize so anchorLESS (centered) steps re-center
  // against the new viewport — anchored steps re-measure via the rect effect's
  // own resize handler, but that effect is inactive when there's no anchor.
  const [, setViewportTick] = useState(0);
  useEffect(() => {
    if (!active) return;
    const onResize = () => setViewportTick((t) => t + 1);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [active]);

  // Keyboard nav is scoped to the bubble (NOT window): the tour is deliberately
  // non-modal / click-through, so a global handler would hijack Enter/Arrows/
  // Escape from the real inputs and terminal the user is meant to keep using.
  // Enter is intentionally left to the focused button's native activation (no
  // double-advance); we only handle Arrows (nav) and Escape (skip) here.
  const handleBubbleKey = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        skipOnboarding();
      } else if (e.key === "ArrowRight") {
        e.preventDefault();
        onboardingNext();
      } else if (e.key === "ArrowLeft") {
        e.preventDefault();
        onboardingPrev();
      }
    },
    [skipOnboarding, onboardingNext, onboardingPrev],
  );

  if (!active || !step) return null;

  const viewport = { width: window.innerWidth, height: window.innerHeight };
  const placed: Placed = computeCoachMarkPosition(
    anchorRect,
    bubbleSize,
    viewport,
    step.placement ?? "auto",
  );
  const spotlight = anchorRect ? computeSpotlightRect(anchorRect, SPOTLIGHT_PADDING) : null;
  const stepNumber = onboarding.index + 1;
  const total = onboarding.steps.length;
  const last = isLast(onboarding);
  const first = isFirst(onboarding);

  const overlay = (
    <div className="tour" role="dialog" aria-modal="false" aria-label="Product tour">
      {spotlight ? (
        <div
          className="tour__spotlight"
          style={{
            top: spotlight.top,
            left: spotlight.left,
            width: spotlight.width,
            height: spotlight.height,
          }}
        />
      ) : (
        <div className="tour__backdrop" />
      )}
      <div
        ref={bubbleRef}
        className={"tour__bubble tour__bubble--" + placed.placement}
        style={{ top: placed.top, left: placed.left }}
        onKeyDown={handleBubbleKey}
      >
        <div className="tour__header">
          <h3 className="tour__title">{step.title}</h3>
          <button
            type="button"
            className="tour__skip"
            aria-label="Skip tour"
            onClick={() => skipOnboarding()}
          >
            Skip
          </button>
        </div>
        <p className="tour__body">{step.body}</p>
        <div className="tour__footer">
          <span className="tour__progress" aria-live="polite">
            {stepNumber} / {total}
          </span>
          <div className="tour__nav">
            {!first && (
              <button type="button" className="btn btn--ghost" onClick={() => onboardingPrev()}>
                Back
              </button>
            )}
            <button type="button" className="btn btn--primary" onClick={() => onboardingNext()}>
              {last ? "Done" : "Next"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );

  return createPortal(overlay, document.body);
}
