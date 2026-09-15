import { useEffect, useState } from "react";
import { listAgentModels } from "../api";
import { useVigieStore } from "../store";

// The catalog also drives engine routing (startAgentSession picks PTY vs ACP
// by spec.execution), so it lives in zustand. Each fresh consumer mount
// re-fetches (keeping the catalog current after Settings edits); loadAgents
// dedups concurrent in-flight calls.
export function useAgents() {
  const agents = useVigieStore((s) => s.agents);
  const loaded = useVigieStore((s) => s.agentsLoaded);
  const error = useVigieStore((s) => s.agentsError);
  useEffect(() => {
    void useVigieStore.getState().loadAgents();
  }, []);
  // An errored load is not "still loading" — loadAgents leaves `agentsLoaded`
  // false on failure (so it retries) but surfaces `agentsError`, so treat a set
  // error as done to avoid a perpetual spinner.
  return { agents, loading: !loaded && !error, error };
}

export function useAgentModels(agentName: string | undefined) {
  const [models, setModels] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  useEffect(() => {
    if (!agentName) { setModels([]); return; }
    let live = true;
    setLoading(true);
    listAgentModels(agentName)
      .then((m) => { if (live) { setModels(m); setLoading(false); } })
      .catch((e) => {
        if (live) { setModels([]); setLoading(false); }
        console.error("listAgentModels failed:", e);
      });
    return () => { live = false; };
  }, [agentName]);
  return { models, loading };
}
