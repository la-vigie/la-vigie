import { beforeEach, describe, expect, it } from "vitest";
import {
  useVigieStore,
  orchestratorSurfaceId,
  repoIdFromSurface,
  AGENT_TAB,
} from "./index";

describe("orchestrator surface helpers", () => {
  it("round-trips repo id through the surface id", () => {
    expect(orchestratorSurfaceId("r1")).toBe("orchestrator:r1");
    expect(repoIdFromSurface("orchestrator:r1")).toBe("r1");
  });
  it("returns a plain task id unchanged", () => {
    expect(repoIdFromSurface("task-uuid-123")).toBe("task-uuid-123");
  });
});

describe("orchestrator selection + session state", () => {
  beforeEach(() => {
    useVigieStore.setState({
      selectedTaskId: null,
      selectedOrchestratorRepoId: null,
      sessionsByTask: {},
      activeTabByTask: {},
    });
  });

  it("selecting an orchestrator clears the selected task and vice versa", () => {
    useVigieStore.getState().setSelectedTask("t1");
    useVigieStore.getState().setSelectedOrchestrator("r1");
    expect(useVigieStore.getState().selectedTaskId).toBeNull();
    expect(useVigieStore.getState().selectedOrchestratorRepoId).toBe("r1");

    useVigieStore.getState().setSelectedTask("t2");
    expect(useVigieStore.getState().selectedOrchestratorRepoId).toBeNull();
    expect(useVigieStore.getState().selectedTaskId).toBe("t2");
  });

  it("startOrchestratorSession adds one orchestrator session under the surface key, idempotently", () => {
    const s = useVigieStore.getState();
    s.startOrchestratorSession("r1");
    s.startOrchestratorSession("r1"); // no duplicate
    const key = orchestratorSurfaceId("r1");
    const sessions = useVigieStore.getState().sessionsByTask[key];
    expect(sessions).toHaveLength(1);
    expect(sessions[0]).toMatchObject({ localId: AGENT_TAB, kind: "orchestrator" });
    expect(useVigieStore.getState().activeTabByTask[key]).toBe(AGENT_TAB);
  });

  it("removeOrchestratorSession clears the surface's sessions", () => {
    const s = useVigieStore.getState();
    s.startOrchestratorSession("r1");
    s.removeOrchestratorSession("r1");
    expect(useVigieStore.getState().sessionsByTask[orchestratorSurfaceId("r1")] ?? []).toHaveLength(0);
  });
});
