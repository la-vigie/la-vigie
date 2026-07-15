import { useEffect, useRef } from "react";
import { onAgentStatus, notifyAgentEvent, setNotificationFocusHandler, isMeetingActive } from "../api";
import { useVigieStore } from "../store";
import { debounce, keyedDebounce } from "../lib/debounce";
import { SoundPlayer } from "../sound/player";
import { resolveSound } from "../sound/resolve";
import { activityToEvent, knownSoundIds } from "../sound/types";
import { parseRepoOverride } from "../sound/safe-parse";
import { formatNotification } from "../notify/format";
import { getCurrentWindow } from "@tauri-apps/api/window";

export function useAgentStatus() {
  const setSessionActivity = useVigieStore((state) => state.setSessionActivity);

  // One player instance per hook lifetime so its cooldown state is stable
  // across events (avoids playing the same sound twice in rapid succession).
  const playerRef = useRef<SoundPlayer | null>(null);
  if (playerRef.current === null) playerRef.current = new SoundPlayer();

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    let windowFocused = false;
    let unlistenFocus: (() => void) | undefined;

    const win = getCurrentWindow();
    win.isFocused().then((f) => {
      windowFocused = f;
    });
    win.onFocusChanged(({ payload }) => {
      windowFocused = payload;
    }).then((fn) => {
      if (cancelled) fn();
      else unlistenFocus = fn;
    });

    const setup = async () => {
      // TASK-120: coalesce out-of-band refreshes. Snapshot is cheap (list_state);
      // review/pr hit git/gh so they debounce longer and per-task.
      const debouncedSnapshot = debounce(() => {
        void useVigieStore.getState().refreshSnapshot();
      }, 750);
      const debouncedReview = keyedDebounce((id) => {
        useVigieStore.getState().bumpReview(id);
      }, 1500);
      const debouncedPr = keyedDebounce((id) => {
        useVigieStore.getState().bumpPr(id);
      }, 8000);

      // Route notification taps to selecting the owning task.
      setNotificationFocusHandler((taskId) => {
        useVigieStore.getState().setSelectedTask(taskId);
      });

      const fn = await onAgentStatus(async ({ agentId, status }) => {
        setSessionActivity(agentId, status);

        // Resolve the owning task/repo once; reuse for both sound and notification.
        const state = useVigieStore.getState();
        const taskId = Object.entries(state.sessionsByTask).find(([, sessions]) =>
          sessions.some((s) => s.kind === "agent" && s.backendId === agentId),
        )?.[0];
        const task = taskId ? state.tasks.find((t) => t.id === taskId) : undefined;

        // TASK-120: keep sidebar/status live on every transition; refetch the
        // Review pane (git/fs + gh) only when the agent finished working.
        debouncedSnapshot();
        if (taskId && (status === "idle" || status === "error")) {
          debouncedReview(taskId);
          debouncedPr(taskId);
        }

        const event = activityToEvent(status);
        if (event) {
          const repo = task ? state.repos.find((r) => r.id === task.repoId) : undefined;
          const override = parseRepoOverride(repo?.soundSettings);

          // Automute (TASK-105): only pay the native meeting probe when automute
          // is actually enabled for this repo/app (off by default → no cost).
          const automuteOn = override.automute ?? state.soundSettings.automute;
          let inMeeting = false;
          if (automuteOn) {
            try {
              inMeeting = await isMeetingActive();
            } catch {
              inMeeting = false; // fail open: never silently swallow alerts
            }
          }

          const decision = resolveSound(
            state.soundSettings,
            override,
            event,
            knownSoundIds(state.customSounds),
            inMeeting,
          );
          if (decision.play) {
            if (decision.sound)
              void playerRef.current?.playSound(decision.sound, state.customSounds);
            if (task) {
              const selected = useVigieStore.getState().selectedTaskId;
              const suppressed = windowFocused && selected === task.id;
              if (!suppressed) {
                const { title, body } = formatNotification(task, repo, event);
                notifyAgentEvent({ title, body, taskId: task.id });
              }
            }
          }
        }
      });

      // If the component unmounted before the subscription resolved, tear it
      // down immediately so we don't leak the listener.
      if (cancelled) {
        fn();
        return;
      }
      unlisten = fn;
    };

    setup();

    return () => {
      cancelled = true;
      unlisten?.();
      unlistenFocus?.();
    };
  }, [setSessionActivity]);
}
