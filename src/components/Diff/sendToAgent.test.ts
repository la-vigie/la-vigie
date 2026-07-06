import { beforeEach, describe, expect, it, vi } from "vitest";
import { sendToAgent } from "./sendToAgent";
import { useVigieStore, AGENT_TAB } from "../../store";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

describe("sendToAgent", () => {
  beforeEach(() => {
    invokeMock.mockReset().mockResolvedValue(undefined);
    useVigieStore.setState({ sessionsByTask: {}, activeTabByTask: {} } as any);
  });

  it("writes bracketed-paste prompt (no trailing CR) to a running agent", async () => {
    useVigieStore.setState({
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude", backendId: "a1" }],
      },
      activeTabByTask: { "task-1": AGENT_TAB },
    } as any);

    await sendToAgent("task-1", "do the thing");

    expect(invokeMock).toHaveBeenCalledWith("write_session", {
      sessionId: "a1",
      data: "\x1b[200~do the thing\x1b[201~",
    });
  });

  it("auto-starts the agent, then writes once it is running (start before write)", async () => {
    const p = sendToAgent("task-1", "review notes");

    // startAgentSession set status "starting" with no backendId yet.
    const sessions = useVigieStore.getState().sessionsByTask["task-1"];
    expect(sessions).toMatchObject([{ status: "starting" }]);
    expect(invokeMock).not.toHaveBeenCalledWith("write_session", expect.anything());

    // Simulate the terminal coming up.
    useVigieStore.getState().setSessionInfo("task-1", AGENT_TAB, { backendId: "a9", status: "running" });

    await p;
    expect(invokeMock).toHaveBeenCalledWith("write_session", {
      sessionId: "a9",
      data: "\x1b[200~review notes\x1b[201~",
    });
  });

  it("writes raw text for Mistral Vibe (no bracketed paste)", async () => {
    useVigieStore.setState({
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Mistral Vibe", backendId: "a1" }],
      },
      activeTabByTask: { "task-1": AGENT_TAB },
    } as any);

    await sendToAgent("task-1", "do the thing");

    expect(invokeMock).toHaveBeenCalledWith("write_session", {
      sessionId: "a1",
      data: "do the thing",
    });
  });
});
