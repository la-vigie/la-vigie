import { useVigieStore, orchestratorSurfaceId } from "../../store";
import { TerminalView } from "./TerminalView";

/**
 * Renders one persistent TerminalView per session across all surfaces (tasks and
 * per-repo orchestrator chats), keyed by `${surfaceId}:${session.localId}` where
 * a surfaceId is a task id or `orchestrator:{repoId}`. Keying by this composite
 * (rather than conditionally rendering only the selected surface) is what keeps
 * each terminal mounted — and its PTY stream alive — across selection changes.
 * Sessions are hidden via display:none, never unmounted (KEEP-ALIVE invariant).
 */
export function TerminalHost() {
  const sessionsByTask = useVigieStore((state) => state.sessionsByTask);
  const activeTabByTask = useVigieStore((state) => state.activeTabByTask);
  const selectedTaskId = useVigieStore((state) => state.selectedTaskId);
  const selectedOrchestratorRepoId = useVigieStore((state) => state.selectedOrchestratorRepoId);

  // The single selected surface: an orchestrator repo (if any) wins, else the task.
  const selectedSurfaceId = selectedOrchestratorRepoId
    ? orchestratorSurfaceId(selectedOrchestratorRepoId)
    : selectedTaskId;

  return (
    <>
      {Object.entries(sessionsByTask).flatMap(([surfaceId, sessions]) =>
        sessions.map((session) => (
          <TerminalView
            key={`${surfaceId}:${session.localId}`}
            taskId={surfaceId}
            localId={session.localId}
            kind={session.kind}
            hidden={!(surfaceId === selectedSurfaceId && activeTabByTask[surfaceId] === session.localId)}
          />
        )),
      )}
    </>
  );
}
