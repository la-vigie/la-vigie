import { useEffect } from "react";
import { onSetupOutput, onSetupStatus } from "../api";
import { useVigieStore } from "../store";

export function useSetupStatus() {
  const appendSetupOutput = useVigieStore((s) => s.appendSetupOutput);
  const setSetupStatus = useVigieStore((s) => s.setSetupStatus);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];

    const setup = async () => {
      const offOut = await onSetupOutput(({ taskId, data }) => appendSetupOutput(taskId, data));
      const offStatus = await onSetupStatus(({ taskId, status, exitCode }) =>
        setSetupStatus(taskId, status, exitCode),
      );
      if (cancelled) {
        offOut();
        offStatus();
        return;
      }
      unlisteners.push(offOut, offStatus);
    };

    setup();

    return () => {
      cancelled = true;
      for (const off of unlisteners) off();
    };
  }, [appendSetupOutput, setSetupStatus]);
}
