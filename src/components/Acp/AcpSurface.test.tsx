import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

import { AcpSurface } from "./AcpSurface";
import { useVigieStore } from "../../store";
import { emptyTimeline, type AcpTimelineState } from "../../acp/timeline";
import type { TerminalSession } from "../../store";
import { AGENT_TAB } from "../../store";

// The resume "restoring" affordance. A resumed session shows
// "running" the instant `start_acp_agent` returns, but for the session/load
// path no history has arrived until the (slow) reconnect handshake finishes —
// signalled by SessionStarted landing (`sessionId` set). Until then the surface
// must read "restoring", not a misleading "ready", and the composer stays gated.

const { MockChannel } = vi.hoisted(() => {
  class MockChannel {
    onmessage: ((event: unknown) => void) | null = null;
  }
  return { MockChannel };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  Channel: MockChannel,
}));

function agentSession(over: Partial<TerminalSession> = {}): TerminalSession {
  return {
    localId: AGENT_TAB,
    kind: "agent",
    status: "running",
    title: "Claude (ACP)",
    backendId: "agent-1",
    engine: "acp",
    ...over,
  };
}

function setState(session: TerminalSession, timeline: AcpTimelineState) {
  useVigieStore.setState({
    sessionsByTask: { "task-1": [session] },
    acpByTask: { "task-1": timeline },
  });
}

describe("AcpSurface — resume restoring state", () => {
  beforeEach(() => {
    useVigieStore.setState({ sessionsByTask: {}, acpByTask: {} });
  });

  it("shows 'Restoring session history…' and disables the composer while a resume has no SessionStarted yet", () => {
    // resume=true, running, but sessionId still null (reconnect in flight).
    setState(agentSession({ resume: true }), { ...emptyTimeline(), sessionId: null });
    render(<AcpSurface taskId="task-1" />);

    expect(screen.getByText("Restoring session history…")).toBeInTheDocument();
    expect(screen.getByLabelText("Message the agent")).toBeDisabled();
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
  });

  it("flips to 'ready' once SessionStarted lands (sessionId set)", () => {
    setState(agentSession({ resume: true }), { ...emptyTimeline(), sessionId: "sess-abc" });
    render(<AcpSurface taskId="task-1" />);

    expect(screen.getByText("Session ready — send a prompt below.")).toBeInTheDocument();
    expect(screen.getByLabelText("Message the agent")).not.toBeDisabled();
  });

  it("a fresh (non-resume) session is never 'restoring' — it shows the ready/empty state", () => {
    setState(agentSession({ resume: false }), { ...emptyTimeline(), sessionId: "sess-fresh" });
    render(<AcpSurface taskId="task-1" />);

    expect(screen.queryByText("Restoring session history…")).not.toBeInTheDocument();
    expect(screen.getByText("Session ready — send a prompt below.")).toBeInTheDocument();
  });
});
