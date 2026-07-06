import { useEffect, useRef } from "react";
import { Channel } from "@tauri-apps/api/core";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import "@xterm/xterm/css/xterm.css";
import { startAgent, startShell, writeSession, resizeSession, stopSession, openUrl } from "../../api";
import type { PtyEvent } from "../../api";
import { useVigieStore } from "../../store";
import type { SessionKind } from "../../store";

export interface TerminalViewProps {
  taskId: string;
  localId: string;
  kind: SessionKind;
  hidden: boolean;
}

export function base64ToUint8Array(base64: string): Uint8Array {
  const bin = atob(base64);
  return Uint8Array.from(bin, (c) => c.charCodeAt(0));
}

// xterm.js emits "\r" (CR = submit) for both Enter and Shift+Enter — it doesn't
// distinguish the modifier, and suppressing the key event doesn't stop the CR
// (Shift+Enter inserts a newline into xterm's hidden textarea, whose input path
// emits the CR anyway). So instead of fighting suppression we remember when the
// last keydown was Shift+Enter and translate the resulting CR into a LF (\x0a),
// which Claude Code inserts as a newline instead of submitting.
export function isShiftEnterKeydown(e: {
  type: string;
  key: string;
  shiftKey: boolean;
}): boolean {
  return e.type === "keydown" && e.key === "Enter" && e.shiftKey;
}

// Translate the byte xterm sends to the PTY: a CR produced by a Shift+Enter
// becomes a LF (insert newline, no submit); everything else passes through.
export function translatePtyInput(data: string, shiftEnterPending: boolean): string {
  return shiftEnterPending && data === "\r" ? "\n" : data;
}

// A terminal link should only open when Cmd (mac) / Ctrl (others) is held, the
// usual terminal-emulator convention — so a plain click/drag still selects text.
export function shouldActivateLink(e: { metaKey: boolean; ctrlKey: boolean }): boolean {
  return e.metaKey || e.ctrlKey;
}

export function TerminalView({ taskId, localId, kind, hidden }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const setSessionInfo = useVigieStore((state) => state.setSessionInfo);
  const removeAgentSession = useVigieStore((state) => state.removeAgentSession);
  // Read the resume flag once, at mount time, from whatever the store holds
  // for this task/session right now.
  const resumeRef = useRef(
    kind === "agent"
      ? (useVigieStore.getState().sessionsByTask[taskId]?.find((s) => s.localId === localId)?.resume ?? false)
      : false,
  );
  const initialPromptRef = useRef<string | undefined>(
    kind === "agent"
      ? useVigieStore.getState().sessionsByTask[taskId]?.find((s) => s.localId === localId)?.initialPrompt
      : undefined,
  );
  const agentIdRef = useRef<string | undefined>(undefined);
  const exitedBeforeReadyRef = useRef<boolean>(false);
  // True when the most recent keydown was Shift+Enter, so the CR it produces in
  // onData can be translated to a newline (see translatePtyInput).
  const shiftEnterPendingRef = useRef<boolean>(false);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const term = new Terminal({ allowProposedApi: true, fontSize: 13 });
    const fitAddon = new FitAddon();
    termRef.current = term;
    fitRef.current = fitAddon;
    term.loadAddon(fitAddon);
    // Make http(s) URLs Cmd/Ctrl+clickable, opening in the OS default browser via
    // the opener plugin (not window.open, which a Tauri webview can't honor).
    term.loadAddon(
      new WebLinksAddon((event, uri) => {
        if (shouldActivateLink(event)) {
          openUrl(uri).catch(() => {});
        }
      }),
    );
    term.open(container);
    fitAddon.fit();

    let disposed = false;
    agentIdRef.current = undefined;
    exitedBeforeReadyRef.current = false;

    const channel = new Channel<PtyEvent>();
    channel.onmessage = (event) => {
      if (event.type === "data") {
        term.write(base64ToUint8Array(event.data));
      } else {
        term.write(`\r\n[process exited: ${event.code}]\r\n`);
        if (kind === "agent") {
          // Remove the agent session so the Claude terminal reverts to a
          // placeholder and the next startAgentSession mounts a fresh one.
          removeAgentSession(taskId);
        } else {
          // Keep the shell's exited output readable.
          setSessionInfo(taskId, localId, { status: "exited" });
        }
        if (agentIdRef.current) {
          stopSession(agentIdRef.current).catch(() => {});
        } else {
          exitedBeforeReadyRef.current = true;
        }
      }
    };

    term.onData((data) => {
      // A CR from a Shift+Enter becomes a newline (insert, not submit).
      const out = translatePtyInput(data, shiftEnterPendingRef.current);
      shiftEnterPendingRef.current = false;
      if (agentIdRef.current) {
        writeSession(agentIdRef.current, out);
      }
    });

    // Remember whether the latest keydown was Shift+Enter so the CR it emits in
    // onData can be turned into a newline. Set on every keydown (true only for
    // Shift+Enter) so a later plain Enter can't be mistranslated.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type === "keydown") {
        shiftEnterPendingRef.current = isShiftEnterKeydown(e);
      }
      return true;
    });

    const refit = () => {
      fitAddon.fit();
      if (agentIdRef.current) {
        resizeSession(agentIdRef.current, term.cols, term.rows);
      }
    };

    const spawn =
      kind === "shell"
        ? startShell(taskId, channel)
        : startAgent(taskId, resumeRef.current, channel, initialPromptRef.current);

    spawn.then((id) => {
      if (disposed) return;
      agentIdRef.current = id;
      setSessionInfo(taskId, localId, { backendId: id, status: "running" });
      if (exitedBeforeReadyRef.current) {
        stopSession(id).catch(() => {});
        return;
      }
      // The PTY was spawned at a default size before the session id was known,
      // so the earlier fit() couldn't resize it. Now that the session is
      // running, sync the PTY to the fitted terminal size.
      refit();
    });

    const resizeObserver = new ResizeObserver(refit);
    resizeObserver.observe(container);
    // Also listen for window resizes directly: in some webviews a flex child's
    // ResizeObserver doesn't fire reliably when the OS window is resized.
    window.addEventListener("resize", refit);

    return () => {
      disposed = true;
      resizeObserver.disconnect();
      window.removeEventListener("resize", refit);
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
    // Mount-only effect: this terminal/session is created once and kept alive
    // for the lifetime of this component (see KEEP-ALIVE rule).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // When this terminal becomes visible again (its task is re-selected), the
  // container may have been size 0 while hidden, leaving xterm fit to a tiny
  // height. Re-fit on the next frame (after layout) so it fills the pane, and
  // move keyboard focus into it so typing reaches the agent without a manual
  // click (auto-focus on task/agent switch — KEEP-ALIVE: the terminal is the
  // already-mounted instance, never remounted).
  useEffect(() => {
    if (hidden) return;
    const raf = requestAnimationFrame(() => {
      fitRef.current?.fit();
      const term = termRef.current;
      if (term && agentIdRef.current) {
        resizeSession(agentIdRef.current, term.cols, term.rows);
      }
      // xterm freezes its renderer while the container is display:none, so on a
      // plain session switch (same size) fit() is a no-op and the viewport stays
      // stale/blank until an interaction forces a refresh. Repaint the visible
      // rows explicitly so the output shows immediately (AC2-84).
      term?.refresh(0, term.rows - 1);
      term?.focus();
    });
    return () => cancelAnimationFrame(raf);
  }, [hidden]);

  return (
    <div style={{ display: hidden ? "none" : "block", width: "100%", height: "100%" }}>
      <div ref={containerRef} style={{ width: "100%", height: "100%" }} />
    </div>
  );
}
