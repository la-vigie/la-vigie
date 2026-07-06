import { writeSession } from "../../api";
import { useVigieStore } from "../../store";
import type { VigieState } from "../../store";
import { AGENT_TAB } from "../../store";
import { wrapBracketedPaste } from "./comments";

const READY_TIMEOUT_MS = 15000;

function readyAgentId(state: VigieState, taskId: string): string | undefined {
  const agentSession = state.sessionsByTask[taskId]?.find(
    (s) => s.kind === "agent" && s.localId === AGENT_TAB,
  );
  return agentSession && agentSession.status === "running" && agentSession.backendId
    ? agentSession.backendId
    : undefined;
}

// Resolve with the backendId once the task's agent session is running; reject on timeout.
export function waitForAgentReady(taskId: string, timeoutMs = READY_TIMEOUT_MS): Promise<string> {
  const immediate = readyAgentId(useVigieStore.getState(), taskId);
  if (immediate) return Promise.resolve(immediate);

  return new Promise<string>((resolve, reject) => {
    const timer = setTimeout(() => {
      unsub();
      reject(new Error("Agent did not become ready in time"));
    }, timeoutMs);
    const unsub = useVigieStore.subscribe((state) => {
      const id = readyAgentId(state, taskId);
      if (id) {
        clearTimeout(timer);
        unsub();
        resolve(id);
      }
    });
  });
}

// Deliver a composed prompt to the task's agent PTY. Starts a session first when
// none is live, waits for it to be ready, then pastes (no submit).
// Mistral Vibe does not support bracketed paste sequences, so we send raw text for it.
function shouldUseBracketedPaste(agentSession?: { title?: string }): boolean {
  // Mistral Vibe CLI does not handle \x1b[200~ / \x1b[201~ bracketed paste sequences
  if (agentSession?.title?.toLowerCase().includes("mistral")) {
    return false;
  }
  return true;
}

export async function sendToAgent(taskId: string, prompt: string): Promise<void> {
  const state = useVigieStore.getState();
  const agentSession = state.sessionsByTask[taskId]?.find((s) => s.kind === "agent");
  const live = agentSession && (agentSession.status === "running" || agentSession.status === "starting");
  if (!live) {
    state.startAgentSession(taskId, false);
  }
  const agentId = await waitForAgentReady(taskId);
  // Re-fetch session after wait (it may have changed)
  const updatedSession = useVigieStore.getState().sessionsByTask[taskId]?.find((s) => s.kind === "agent");
  const data = shouldUseBracketedPaste(updatedSession) ? wrapBracketedPaste(prompt) : prompt;
  await writeSession(agentId, data);
}
