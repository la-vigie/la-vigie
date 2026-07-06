import { useEffect } from "react";
import { onAgentConsole } from "../api";
import { useVigieStore } from "../store";

export function useAgentConsole() {
  const setAgentConsole = useVigieStore((state) => state.setAgentConsole);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    const setup = async () => {
      const fn = await onAgentConsole(({ agentId, ...rest }) => {
        setAgentConsole(agentId, rest);
      });
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
    };
  }, [setAgentConsole]);
}
