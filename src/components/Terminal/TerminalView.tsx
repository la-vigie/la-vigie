import { useContext, useEffect, useRef } from "react";
import { Channel } from "@tauri-apps/api/core";
import { Terminal } from "@xterm/xterm";
import { WebLinksAddon } from "@xterm/addon-web-links";
import "@xterm/xterm/css/xterm.css";
import { startAgent, startShell, openOrchestratorTerminal, writeSession, resizeSession, stopSession, openUrl } from "../../api";
import type { PtyEvent } from "../../api";
import { useVigieStore, repoIdFromSurface } from "../../store";
import type { SessionKind } from "../../store";
import {
  TerminalPaneMetricsContext,
  computeGrid,
  getCachedCell,
  setCachedCell,
  readCellSize,
} from "./TerminalPaneMetrics";
import type { PaneMetrics } from "./TerminalPaneMetrics";

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
  // The invariant pane's size + subscription (TASK-227). Null only when a
  // TerminalView is rendered outside a provider (bare, e.g. some unit tests);
  // then sizing is simply skipped.
  const paneMetrics = useContext(TerminalPaneMetricsContext);
  const paneMetricsRef = useRef<PaneMetrics | null>(paneMetrics);
  paneMetricsRef.current = paneMetrics;
  const setSessionInfo = useVigieStore((state) => state.setSessionInfo);
  const removeAgentSession = useVigieStore((state) => state.removeAgentSession);
  const removeOrchestratorSession = useVigieStore((state) => state.removeOrchestratorSession);
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
  // Last grid we synced to the PTY, deduped independently of xterm's own grid:
  // a resize can be applied to xterm before the session id exists (so the PTY
  // sync is deferred), and the post-spawn sync must still fire even though the
  // grid is unchanged. Tracking the PTY separately keeps that first sync from
  // being deduped away.
  const lastPtyGridRef = useRef<{ cols: number; rows: number } | null>(null);
  // The mount effect's pane-derived sizing fn, exposed so the becomes-visible
  // effect can re-derive the size on show without duplicating the logic.
  const applyGridRef = useRef<() => void>(() => {});

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const term = new Terminal({
      allowProposedApi: true,
      fontSize: 13,
      // Handle OSC 8 hyperlinks — text that carries an embedded URL distinct from
      // what's shown (e.g. Claude Code's "PR #123" link at the bottom of its TUI).
      // The WebLinksAddon below only regex-matches plaintext URLs, so OSC 8 links
      // need this handler to be Cmd/Ctrl+clickable. Opens via the opener plugin,
      // mirroring the WebLinksAddon handler. allowNonHttpProtocols defaults to
      // false, so non-http(s) link targets are ignored (no XSS surface).
      linkHandler: {
        activate: (event, uri) => {
          if (shouldActivateLink(event)) {
            openUrl(uri).catch(() => {});
          }
        },
      },
    });
    termRef.current = term;
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

    let disposed = false;
    agentIdRef.current = undefined;
    exitedBeforeReadyRef.current = false;

    const channel = new Channel<PtyEvent>();
    channel.onmessage = (event) => {
      // A disposed/unmounted view must not process PTY events — mirrors the
      // `disposed` guard on the spawn promise below. Without this, an exit event
      // delivered to an orphaned channel (e.g. React StrictMode's double-mount,
      // where the orchestrator's stop-and-respawn kills the first mount's process)
      // would call removeAgentSession/removeOrchestratorSession and tear down the
      // live store session, reverting the surface to its placeholder.
      if (disposed) return;
      if (event.type === "data") {
        term.write(base64ToUint8Array(event.data));
      } else {
        term.write(`\r\n[process exited: ${event.code}]\r\n`);
        if (kind === "agent") {
          // Remove the agent session so the Claude terminal reverts to a
          // placeholder and the next startAgentSession mounts a fresh one.
          removeAgentSession(taskId);
        } else if (kind === "orchestrator") {
          // taskId is the `orchestrator:{repoId}` surface key. Drop the session
          // so the pane shows the "Open orchestrator" affordance; reopening
          // starts a fresh session (the desktop path spawns without `--continue`;
          // see open_orchestrator_terminal).
          removeOrchestratorSession(repoIdFromSurface(taskId));
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

    // Deterministically size the terminal from the invariant pane (TASK-227),
    // never from measuring this (possibly just-unhidden, still-collapsed) child.
    // Read the pane's cached pixel box + the shared char-cell size and compute
    // the grid directly; apply to xterm and the PTY. Because the pane is always
    // laid out, this is correct on every surface switch with no settle loop.
    const applyGrid = () => {
      const metrics = paneMetricsRef.current;
      if (!metrics) return;
      // Harvest the char-cell size from this terminal if it's the first one
      // laid out (renderer measured); otherwise reuse the shared cache.
      const measured = readCellSize(term);
      if (measured) setCachedCell(measured);
      const cell = getCachedCell() ?? measured;
      if (!cell) return;
      const { width, height } = metrics.getSize();
      const grid = computeGrid(width, height, cell);
      if (!grid) return;
      // Resize xterm only when its grid actually changes.
      if (term.cols !== grid.cols || term.rows !== grid.rows) {
        term.resize(grid.cols, grid.rows);
      }
      // Sync the PTY only when the session is live AND the size it last saw
      // changed. Deduped separately from xterm so the first post-spawn sync
      // always lands, even if an earlier (pre-spawn) resize already sized xterm
      // to the same grid.
      if (agentIdRef.current) {
        const lastPty = lastPtyGridRef.current;
        if (!lastPty || lastPty.cols !== grid.cols || lastPty.rows !== grid.rows) {
          lastPtyGridRef.current = grid;
          resizeSession(agentIdRef.current, grid.cols, grid.rows);
        }
      }
    };
    applyGridRef.current = applyGrid;

    // Re-fit on every genuine pane resize (window, split-drag, sidebar). All
    // mounted surfaces subscribe and share one pane size + one cell cache, so
    // each keeps its own xterm+PTY synced even while hidden — which is exactly
    // why a later surface switch has nothing to correct (zero transient).
    const unsubscribe = paneMetricsRef.current?.subscribe(applyGrid);

    const spawn =
      kind === "shell"
        ? startShell(taskId, channel)
        : kind === "orchestrator"
          ? openOrchestratorTerminal(repoIdFromSurface(taskId), channel)
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
      // so no earlier sizing could resize it. Now that the session is running,
      // sync the PTY to the pane-derived grid.
      applyGrid();
    });

    return () => {
      disposed = true;
      unsubscribe?.();
      term.dispose();
      termRef.current = null;
    };
    // Mount-only effect: this terminal/session is created once and kept alive
    // for the lifetime of this component (see KEEP-ALIVE rule).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // When this terminal becomes visible again (its task/surface is re-selected),
  // re-derive its size and move keyboard focus into it so typing reaches the
  // agent without a manual click (auto-focus on task/agent switch — KEEP-ALIVE:
  // the terminal is the already-mounted instance, never remounted).
  useEffect(() => {
    if (hidden) return;
    const raf = requestAnimationFrame(() => {
      // Apply the pane-derived grid (TASK-227). Because sizing comes from the
      // always-laid-out pane — not from measuring this just-unhidden child —
      // the size is already correct: there is no collapse transient to settle,
      // so this is a single deterministic apply, not a frame-budget loop. It
      // also picks up the shared char-cell size if THIS is the first terminal
      // to actually lay out.
      applyGridRef.current();
      // xterm freezes its renderer while the container is display:none, so on a
      // plain session switch (same size) the buffer is unchanged and the
      // viewport stays stale/blank until an interaction forces a refresh.
      // Repaint the visible rows explicitly so the output shows immediately
      // (TASK-84); focus once (no repeated focus-stealing — there's no loop).
      const term = termRef.current;
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
