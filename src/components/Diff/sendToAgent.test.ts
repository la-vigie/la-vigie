import { beforeEach, describe, expect, it, vi } from "vitest";
import { sendToAgent } from "./sendToAgent";
import { useVigieStore, AGENT_TAB } from "../../store";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

describe("sendToAgent", () => {
  beforeEach(() => {
    invokeMock.mockReset().mockResolvedValue(undefined);
    // startAgentSession awaits the catalog before routing unless it's
    // already loaded — keep the auto-start flow synchronous here.
    useVigieStore.setState({ sessionsByTask: {}, activeTabByTask: {}, agentsLoaded: true } as any);
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

  it("routes an ACP agent session through acp_prompt, never write_session (TASK-244)", async () => {
    useVigieStore.setState({
      sessionsByTask: {
        "task-1": [{ localId: AGENT_TAB, kind: "agent", status: "running", title: "Claude Code (ACP)", backendId: "acp-1", engine: "acp" }],
      },
      activeTabByTask: { "task-1": AGENT_TAB },
    } as any);

    await sendToAgent("task-1", "do the thing");

    expect(invokeMock).toHaveBeenCalledWith("acp_prompt", {
      sessionId: "acp-1",
      text: "do the thing",
    });
    expect(invokeMock).not.toHaveBeenCalledWith("write_session", expect.anything());
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
