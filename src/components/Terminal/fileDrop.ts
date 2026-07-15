import { wrapBracketedPaste } from "../Diff/comments";
import { orchestratorSurfaceId, type VigieState } from "../../store";

// Backslash-escape characters a POSIX shell treats specially, mirroring what
// Terminal.app inserts when you drag a file in. Claude Code shell-unescapes the
// pasted path before reading it. Letters (incl. unicode), digits and path-safe
// punctuation are left alone.
export function shellEscapePath(path: string): string {
  return path.replace(/([ \t!"#$&'()*;<>?\[\\\]^`{|}~])/g, "\\$1");
}

// Join shell-escaped paths with spaces and wrap in bracketed paste so Claude
// Code's image detection fires (vs. plain text, which it ignores).
export function buildDropPayload(paths: string[]): string {
  return wrapBracketedPaste(paths.map(shellEscapePath).join(" "));
}

export interface DropPoint {
  x: number;
  y: number;
}

// Tauri reports the drop position in PHYSICAL pixels; getBoundingClientRect is in
// CSS pixels. Convert before comparing or hit-testing breaks on scaled displays.
export function isWithinRect(physical: DropPoint, rect: DOMRect, dpr: number): boolean {
  const x = physical.x / dpr;
  const y = physical.y / dpr;
  return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
}

// The PTY backendId of the session shown in the selected surface's active tab, or
// undefined if there is no selection / no spawned session. The selected surface is
// the orchestrator chat (`orchestrator:{repoId}`) if one is selected, else the task
// — mirroring TerminalHost's `selectedSurfaceId` precedence so a drop over the
// orchestrator terminal resolves its PTY too (TASK-221).
export function resolveActiveBackendId(state: VigieState): string | undefined {
  const surfaceId = state.selectedOrchestratorRepoId
    ? orchestratorSurfaceId(state.selectedOrchestratorRepoId)
    : state.selectedTaskId;
  if (!surfaceId) return undefined;
  const activeLocalId = state.activeTabByTask[surfaceId];
  const session = state.sessionsByTask[surfaceId]?.find((s) => s.localId === activeLocalId);
  return session?.backendId;
}
