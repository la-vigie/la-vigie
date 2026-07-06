import { beforeEach, describe, expect, it } from "vitest";
import { AGENT_TAB, useVigieStore } from "./index";

const reset = () =>
  useVigieStore.setState({
    sessionsByTask: {},
    activeTabByTask: {},
    attentionByTask: {},
    consoleByAgentId: {},
    selectedTaskId: null,
    tasks: [],
  });

describe("session store", () => {
  beforeEach(reset);

  it("startAgentSession creates the agent tab and makes it active", () => {
    useVigieStore.getState().startAgentSession("t1", false);
    const s = useVigieStore.getState();
    expect(s.sessionsByTask["t1"]).toHaveLength(1);
    expect(s.sessionsByTask["t1"][0]).toMatchObject({ localId: AGENT_TAB, kind: "agent", status: "starting", title: "Claude" });
    expect(s.activeTabByTask["t1"]).toBe(AGENT_TAB);
  });

  it("addShellSession appends a shell tab, numbers it, and activates it", () => {
    const { startAgentSession, addShellSession } = useVigieStore.getState();
    startAgentSession("t1", false);
    addShellSession("t1");
    addShellSession("t1");
    const sessions = useVigieStore.getState().sessionsByTask["t1"];
    expect(sessions.map((x) => x.title)).toEqual(["Claude", "shell", "shell 2"]);
    expect(useVigieStore.getState().activeTabByTask["t1"]).toBe(sessions[2].localId);
  });

  it("removeShellSession drops the tab and falls back to a neighbor when it was active", () => {
    const { addShellSession, removeShellSession } = useVigieStore.getState();
    useVigieStore.getState().startAgentSession("t1", false);
    addShellSession("t1");
    const shellId = useVigieStore.getState().sessionsByTask["t1"][1].localId;
    removeShellSession("t1", shellId);
    const s = useVigieStore.getState();
    expect(s.sessionsByTask["t1"].map((x) => x.localId)).toEqual([AGENT_TAB]);
    expect(s.activeTabByTask["t1"]).toBe(AGENT_TAB);
  });

  it("removeShellSession of the MIDDLE shell (when active) falls back to the NEXT shell, not AGENT_TAB", () => {
    const { startAgentSession, addShellSession, removeShellSession } = useVigieStore.getState();
    startAgentSession("t1", false);
    addShellSession("t1");
    addShellSession("t1");
    addShellSession("t1");
    const sessions = useVigieStore.getState().sessionsByTask["t1"];
    // sessions: [agent, shell, shell2, shell3] at indices 0,1,2,3
    const shell2Id = sessions[2].localId; // middle shell
    const shell3Id = sessions[3].localId; // next shell (index shifts after removal)
    // Make the middle shell active
    useVigieStore.getState().setActiveTab("t1", shell2Id);
    expect(useVigieStore.getState().activeTabByTask["t1"]).toBe(shell2Id);
    // Remove it
    removeShellSession("t1", shell2Id);
    const s = useVigieStore.getState();
    // After removal the former shell3 is now at index 2 (same index as removed shell2 was)
    // fallback: sessions[idx] where idx=2 is shell3
    expect(s.activeTabByTask["t1"]).toBe(shell3Id);
    expect(s.activeTabByTask["t1"]).not.toBe(AGENT_TAB);
  });

  it("removeAgentSession clears the agent session but leaves the active tab pointer", () => {
    useVigieStore.getState().startAgentSession("t1", false);
    useVigieStore.getState().removeAgentSession("t1");
    const s = useVigieStore.getState();
    expect(s.sessionsByTask["t1"]).toEqual([]);
    expect(s.activeTabByTask["t1"]).toBe(AGENT_TAB);
  });

  it("setSessionActivity flags attention only for a non-selected task's agent", () => {
    useVigieStore.setState({ selectedTaskId: "other" });
    useVigieStore.getState().startAgentSession("t1", false);
    useVigieStore.getState().setSessionInfo("t1", AGENT_TAB, { backendId: "b1", status: "running" });
    useVigieStore.getState().setSessionActivity("b1", "idle");
    expect(useVigieStore.getState().attentionByTask["t1"]).toBe(true);
  });

  it("setAgentConsole merges partial console updates by agent id", () => {
    useVigieStore.setState({ consoleByAgentId: {} });
    useVigieStore.getState().setAgentConsole("b1", { model: "Opus", contextRemainingPercent: 90 });
    useVigieStore.getState().setAgentConsole("b1", { mode: "auto" });
    expect(useVigieStore.getState().consoleByAgentId["b1"]).toEqual({ model: "Opus", contextRemainingPercent: 90, mode: "auto" });
  });

  it("clearTaskSessions removes all sessions and the attention flag", () => {
    useVigieStore.getState().startAgentSession("t1", false);
    useVigieStore.getState().addShellSession("t1");
    useVigieStore.setState({ attentionByTask: { t1: true } });
    useVigieStore.getState().clearTaskSessions("t1");
    const s = useVigieStore.getState();
    expect(s.sessionsByTask["t1"]).toBeUndefined();
    expect(s.attentionByTask["t1"]).toBeUndefined();
  });
});
