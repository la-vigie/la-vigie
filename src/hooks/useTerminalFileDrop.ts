// src/hooks/useTerminalFileDrop.ts
import { useEffect, useState, type RefObject } from "react";
import { onWebviewFileDrop, writeSession } from "../api";
import { useVigieStore } from "../store";
import { buildDropPayload, isWithinRect, resolveActiveBackendId } from "../components/Terminal/fileDrop";

// Webview-global file-drop listener scoped to the terminal pane via hit-testing.
// On a drop inside the pane, forwards the dropped paths to the active session's
// PTY as a bracketed paste. Returns whether a drag is currently over the pane.
export function useTerminalFileDrop(paneRef: RefObject<HTMLElement | null>): boolean {
  const [isDragOver, setIsDragOver] = useState(false);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    const inside = (pos: { x: number; y: number }): boolean => {
      const rect = paneRef.current?.getBoundingClientRect();
      return rect ? isWithinRect(pos, rect, window.devicePixelRatio) : false;
    };

    onWebviewFileDrop((payload) => {
      if (payload.type === "leave") {
        setIsDragOver(false);
        return;
      }
      if (payload.type === "drop") {
        const over = inside(payload.position);
        setIsDragOver(false);
        if (!over || payload.paths.length === 0) return;
        const backendId = resolveActiveBackendId(useVigieStore.getState());
        if (!backendId) return;
        writeSession(backendId, buildDropPayload(payload.paths)).catch(() => {});
        return;
      }
      // "enter" | "over"
      setIsDragOver(inside(payload.position));
    }).then((fn) => {
      if (cancelled) {
        fn();
        return;
      }
      unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
    // Mount-only: one webview-global listener for the component's lifetime. The
    // inside() helper reads paneRef.current at event time, so the ref need not
    // be a dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return isDragOver;
}
