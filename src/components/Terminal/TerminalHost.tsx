import { useVigieStore } from "../../store";
import { TerminalView } from "./TerminalView";

/**
 * Renders one persistent TerminalView per session across all tasks, keyed by
 * `${taskId}:${session.localId}`. Keying by this composite (rather than
 * conditionally rendering only the selected task) is what keeps each terminal
 * mounted — and its PTY stream alive — across task-selection changes.
 * Sessions are hidden via display:none, never unmounted (KEEP-ALIVE invariant).
 */
export function TerminalHost() {
  const sessionsByTask = useVigieStore((state) => state.sessionsByTask);
  const activeTabByTask = useVigieStore((state) => state.activeTabByTask);
  const selectedTaskId = useVigieStore((state) => state.selectedTaskId);

  return (
    <>
      {Object.entries(sessionsByTask).flatMap(([taskId, sessions]) =>
        sessions.map((session) => (
          <TerminalView
            key={`${taskId}:${session.localId}`}
            taskId={taskId}
            localId={session.localId}
            kind={session.kind}
            hidden={!(taskId === selectedTaskId && activeTabByTask[taskId] === session.localId)}
          />
        )),
      )}
    </>
  );
}
