import { useEffect, useState } from "react";
import { listAgents, listAgentModels } from "../api";
import type { AgentSpec } from "../store";

export function useAgents() {
  const [agents, setAgents] = useState<AgentSpec[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    listAgents()
      .then((a) => { if (live) { setAgents(a); setLoading(false); } })
      .catch((e) => {
        if (live) { setError(String(e)); setLoading(false); }
        console.error("listAgents failed:", e);
      });
    return () => { live = false; };
  }, []);
  return { agents, loading, error };
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
