import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
// Render the QR as a probe so we can assert the exact value it encodes.
vi.mock("qrcode.react", () => ({
  QRCodeSVG: ({ value }: { value: string }) => <div data-testid="qr" data-value={value} />,
}));

import { useVigieStore } from "../../store";
import { SettingsModal } from "./SettingsModal";

describe("Settings — Remote access", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    useVigieStore.setState({ remote: { active: false, sleepInhibited: false } });
  });

  it("enabling remote calls enable_remote and shows the token", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "remote_status"
        ? Promise.resolve({ active: false, sleepInhibited: false })
        : cmd === "list_agents"
        ? Promise.resolve([])
        : Promise.resolve({ active: true, token: "tok-123", url: "https://mac.ts.net/", sleepInhibited: true }),
    );
    render(<SettingsModal onClose={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Remote" }));
    fireEvent.click(await screen.findByRole("button", { name: /enable remote/i }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("enable_remote"));
    expect(await screen.findByText(/tok-123/)).toBeInTheDocument();
    // TASK-104: held assertion is surfaced as a note.
    expect(screen.getByText(/system sleep is/i)).toBeInTheDocument();
    expect(screen.getByText(/AC power only/i)).toBeInTheDocument();
  });

  it("warns when the sleep assertion could not be acquired", async () => {
    useVigieStore.setState({
      remote: { active: true, token: "tok-xyz", url: "https://mac.ts.net/", sleepInhibited: false },
    });
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "list_agents" ? Promise.resolve([]) : Promise.resolve({ active: true, sleepInhibited: false }),
    );
    render(<SettingsModal onClose={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Remote" }));
    expect(await screen.findByText(/couldn’t prevent system sleep/i)).toBeInTheDocument();
  });

  it("shows a QR encoding the pairing URL with the token in the fragment", async () => {
    useVigieStore.setState({ remote: { active: true, token: "tok-123", url: "https://mac.ts.net/", sleepInhibited: true } });
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "remote_status"
        ? Promise.resolve({ active: true, token: "tok-123", url: "https://mac.ts.net/", sleepInhibited: true })
        : Promise.resolve([]),
    );
    render(<SettingsModal onClose={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Remote" }));
    const qr = await screen.findByTestId("qr");
    // Token rides in the URL fragment (#token=) — never a query param, never server-logged.
    expect(qr).toHaveAttribute("data-value", "https://mac.ts.net/#token=tok-123");
    // The screen-exposure tradeoff must be documented next to the QR.
    expect(screen.getByText(/anyone who can see this screen/i)).toBeInTheDocument();
  });

  it("renders no QR until both url and token are known", async () => {
    useVigieStore.setState({ remote: { active: true, token: "tok-123", sleepInhibited: true } });
    invokeMock.mockImplementation(() => Promise.resolve([]));
    render(<SettingsModal onClose={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Remote" }));
    await screen.findByText(/tok-123/);
    expect(screen.queryByTestId("qr")).not.toBeInTheDocument();
  });

  it("lists remote sessions when active and renders idle minutes", async () => {
    useVigieStore.setState({ remote: { active: true, token: "t", url: "https://m.ts.net/", sleepInhibited: true } });
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "list_remote_sessions"
        ? Promise.resolve([{ id: "concierge-agent-1", kind: "concierge", idleSecs: 90 }])
        : cmd === "remote_status"
        ? Promise.resolve({ active: true, token: "t", url: "https://m.ts.net/", sleepInhibited: true })
        : cmd === "stop_session"
        ? Promise.resolve(undefined)
        : Promise.resolve([]),
    );
    render(<SettingsModal onClose={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Remote" }));
    expect(await screen.findByText(/concierge · idle 1m/i)).toBeInTheDocument();
  });

  it("labels an orchestrator session by its repo and a concierge session globally", async () => {
    useVigieStore.setState({
      remote: { active: true, token: "t", url: "https://m.ts.net/", sleepInhibited: true },
      repos: [{ id: "r1", name: "My Repo", path: "/x", defaultBranch: "main", inPlaceDefault: false }],
    });
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "list_remote_sessions"
        ? Promise.resolve([
            { id: "orch-1", kind: "orchestrator", repoId: "r1", idleSecs: 30 },
            { id: "conc-1", kind: "concierge", idleSecs: 30 },
          ])
        : cmd === "remote_status"
        ? Promise.resolve({ active: true, token: "t", url: "https://m.ts.net/", sleepInhibited: true })
        : Promise.resolve([]),
    );
    render(<SettingsModal onClose={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Remote" }));
    // Orchestrator resolves its repo id to the repo name.
    expect(await screen.findByText(/orchestrator · My Repo/i)).toBeInTheDocument();
    // The legacy global session keeps its bare kind label (no repo).
    expect(screen.getByText(/^concierge · idle/i)).toBeInTheDocument();
  });

  it("falls back to the repo id when the repo is unknown", async () => {
    useVigieStore.setState({
      remote: { active: true, token: "t", url: "https://m.ts.net/", sleepInhibited: true },
      repos: [],
    });
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "list_remote_sessions"
        ? Promise.resolve([{ id: "orch-1", kind: "orchestrator", repoId: "r1", idleSecs: 0 }])
        : cmd === "remote_status"
        ? Promise.resolve({ active: true, token: "t", url: "https://m.ts.net/", sleepInhibited: true })
        : Promise.resolve([]),
    );
    render(<SettingsModal onClose={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Remote" }));
    expect(await screen.findByText(/orchestrator · r1/i)).toBeInTheDocument();
  });

  it("Open orchestrator invokes open_orchestrator for the repo", async () => {
    useVigieStore.setState({
      remote: { active: true, token: "t", url: "https://m.ts.net/", sleepInhibited: true },
      repos: [{ id: "r1", name: "My Repo", path: "/x", defaultBranch: "main", inPlaceDefault: false }],
    });
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "remote_status"
        ? Promise.resolve({ active: true, token: "t", url: "https://m.ts.net/", sleepInhibited: true })
        : cmd === "open_orchestrator"
        ? Promise.resolve(undefined)
        : Promise.resolve([]),
    );
    render(<SettingsModal onClose={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Remote" }));
    fireEvent.click(await screen.findByRole("button", { name: /open orchestrator/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("open_orchestrator", { repoId: "r1" }),
    );
  });

  it("Stop on a remote session calls stop_session and removes the row", async () => {
    useVigieStore.setState({ remote: { active: true, token: "t", url: "https://m.ts.net/", sleepInhibited: true } });
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "list_remote_sessions"
        ? Promise.resolve([{ id: "concierge-agent-1", kind: "concierge", idleSecs: 30 }])
        : cmd === "remote_status"
        ? Promise.resolve({ active: true, token: "t", url: "https://m.ts.net/", sleepInhibited: true })
        : cmd === "stop_session"
        ? Promise.resolve(undefined)
        : Promise.resolve([]),
    );
    render(<SettingsModal onClose={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Remote" }));
    fireEvent.click(await screen.findByRole("button", { name: /^stop$/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("stop_session", { sessionId: "concierge-agent-1" }),
    );
    await waitFor(() => expect(screen.queryByText(/concierge · idle/i)).not.toBeInTheDocument());
  });
});
