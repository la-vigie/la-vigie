import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useVigieStore } from "../store";
import { throttleLeading } from "../lib/debounce";

const FOCUS_THROTTLE_MS = 5000;

/// TASK-120: refresh data-bearing views when the window regains focus / becomes
/// visible — the cheap catch for out-of-band changes made while the app was in
/// the background (e.g. a PR merged on GitHub). Throttled so focus-flapping
/// doesn't spam git/gh.
export function useFocusRefresh(): void {
  useEffect(() => {
    let cancelled = false;
    let unlistenFocus: (() => void) | undefined;

    const onActive = throttleLeading(() => {
      const { refreshSnapshot, bumpReview, bumpPr, selectedTaskId } = useVigieStore.getState();
      void refreshSnapshot();
      if (selectedTaskId) {
        bumpReview(selectedTaskId);
        bumpPr(selectedTaskId);
      }
    }, FOCUS_THROTTLE_MS);

    const onVisibility = () => {
      if (document.visibilityState === "visible") onActive();
    };
    document.addEventListener("visibilitychange", onVisibility);

    getCurrentWindow()
      .onFocusChanged(({ payload }) => {
        if (payload) onActive();
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlistenFocus = fn;
      });

    return () => {
      cancelled = true;
      document.removeEventListener("visibilitychange", onVisibility);
      unlistenFocus?.();
    };
  }, []);
}
