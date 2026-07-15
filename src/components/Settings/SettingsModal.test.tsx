import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SettingsModal } from "./SettingsModal";
import { useVigieStore } from "../../store";
import { DEFAULT_SOUND_SETTINGS, type SoundSettings } from "../../sound/types";

const { listAgentsMock, upsertCustomAgentMock, deleteCustomAgentMock, deleteCustomSoundMock } =
  vi.hoisted(() => ({
    listAgentsMock: vi.fn(),
    upsertCustomAgentMock: vi.fn(),
    deleteCustomAgentMock: vi.fn(),
    deleteCustomSoundMock: vi.fn(),
  }));

vi.mock("../../api", () => ({
  listAgents: listAgentsMock,
  upsertCustomAgent: upsertCustomAgentMock,
  deleteCustomAgent: deleteCustomAgentMock,
  deleteCustomSound: deleteCustomSoundMock,
  remoteStatus: vi.fn().mockResolvedValue({ active: false, sleepInhibited: false }),
  enableRemote: vi.fn().mockResolvedValue({ active: false, sleepInhibited: false }),
  disableRemote: vi.fn().mockResolvedValue({ active: false, sleepInhibited: false }),
}));

vi.mock("../../sound/import", () => ({
  pickAndImportSound: vi.fn().mockResolvedValue({ id: "custom:x", label: "X", ext: "mp3" }),
}));

const builtinClaude = {
  name: "claude",
  displayName: "Claude Code",
  binary: "claude",
  baseArgs: [] as string[],
  resumeArgs: ["--resume"] as string[],
  extraArgs: [] as string[],
  promptMode: "stdin" as const,
  status: "claudeHooks" as const,
  builtin: true,
};

const customAgent = {
  name: "my-agent",
  displayName: "My Agent",
  binary: "/usr/local/bin/my-agent",
  baseArgs: ["--flag"] as string[],
  resumeArgs: [] as string[],
  extraArgs: [] as string[],
  promptMode: "arg" as const,
  status: "lifecycle" as const,
  builtin: false,
};

describe("SettingsModal", () => {
  beforeEach(() => {
    listAgentsMock.mockReset();
    upsertCustomAgentMock.mockReset();
    deleteCustomAgentMock.mockReset();
    deleteCustomSoundMock.mockReset();
    upsertCustomAgentMock.mockResolvedValue(undefined);
    deleteCustomAgentMock.mockResolvedValue(undefined);
    deleteCustomSoundMock.mockResolvedValue(undefined);
  });

  it("renders with role=dialog and aria-label=Settings", async () => {
    listAgentsMock.mockResolvedValue([]);
    render(<SettingsModal onClose={() => {}} />);
    expect(screen.getByRole("dialog", { name: "Settings" })).toBeInTheDocument();
  });

  it("lists built-in agents read-only with no Remove control", async () => {
    listAgentsMock.mockResolvedValue([builtinClaude]);
    render(<SettingsModal onClose={() => {}} />);
    expect(await screen.findByText("Claude Code")).toBeInTheDocument();
    // Built-in rows must carry no Remove button at all
    expect(screen.queryByRole("button", { name: /remove/i })).toBeNull();
  });

  it("lists custom agents with a Remove control", async () => {
    listAgentsMock.mockResolvedValue([customAgent]);
    render(<SettingsModal onClose={() => {}} />);
    expect(await screen.findByText("My Agent")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /remove/i })).toBeInTheDocument();
  });

  it("built-in agents have no Edit or Remove controls; custom agents do", async () => {
    listAgentsMock.mockResolvedValue([builtinClaude, customAgent]);
    render(<SettingsModal onClose={() => {}} />);
    await screen.findByText("Claude Code");
    await screen.findByText("My Agent");
    // Only one Remove button (for the custom agent)
    const removeButtons = screen.getAllByRole("button", { name: /remove/i });
    expect(removeButtons).toHaveLength(1);
    // Only one Edit button
    const editButtons = screen.getAllByRole("button", { name: /edit/i });
    expect(editButtons).toHaveLength(1);
  });

  it("calls upsertCustomAgent with parsed spec on add", async () => {
    listAgentsMock.mockResolvedValue([]);
    render(<SettingsModal onClose={() => {}} />);
    await waitFor(() => expect(listAgentsMock).toHaveBeenCalledOnce());

    // Open the add form
    fireEvent.click(screen.getByRole("button", { name: /add agent/i }));

    // Fill in the form
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "my-bot" } });
    fireEvent.change(screen.getByLabelText("Display name"), { target: { value: "My Bot" } });
    fireEvent.change(screen.getByLabelText("Binary"), { target: { value: "/usr/bin/bot" } });
    fireEvent.change(screen.getByLabelText("Base args"), {
      target: { value: "--flag1\n--flag2\n" },
    });
    fireEvent.change(screen.getByLabelText("Resume args"), {
      target: { value: "--resume" },
    });
    // Extra args left empty

    fireEvent.click(screen.getByRole("button", { name: /^add$/i }));

    await waitFor(() =>
      expect(upsertCustomAgentMock).toHaveBeenCalledWith({
        name: "my-bot",
        displayName: "My Bot",
        binary: "/usr/bin/bot",
        baseArgs: ["--flag1", "--flag2"],
        resumeArgs: ["--resume"],
        extraArgs: [],
        promptMode: "stdin",
        status: "lifecycle",
        builtin: false,
      }),
    );
  });

  it("defaults displayName to name when left blank", async () => {
    listAgentsMock.mockResolvedValue([]);
    render(<SettingsModal onClose={() => {}} />);
    await waitFor(() => expect(listAgentsMock).toHaveBeenCalledOnce());

    fireEvent.click(screen.getByRole("button", { name: /add agent/i }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "bot" } });
    // Leave displayName blank
    fireEvent.change(screen.getByLabelText("Binary"), { target: { value: "/usr/bin/bot" } });
    fireEvent.click(screen.getByRole("button", { name: /^add$/i }));

    await waitFor(() =>
      expect(upsertCustomAgentMock).toHaveBeenCalledWith(
        expect.objectContaining({ name: "bot", displayName: "bot" }),
      ),
    );
  });

  it("calls deleteCustomAgent with the agent name on Remove", async () => {
    listAgentsMock.mockResolvedValue([customAgent]);
    render(<SettingsModal onClose={() => {}} />);
    await screen.findByText("My Agent");

    fireEvent.click(screen.getByRole("button", { name: /remove/i }));

    await waitFor(() =>
      expect(deleteCustomAgentMock).toHaveBeenCalledWith("my-agent"),
    );
  });

  it("reloads the agent list after a successful add", async () => {
    listAgentsMock
      .mockResolvedValueOnce([]) // initial load
      .mockResolvedValueOnce([customAgent]); // reload after add
    render(<SettingsModal onClose={() => {}} />);
    await waitFor(() => expect(listAgentsMock).toHaveBeenCalledOnce());

    fireEvent.click(screen.getByRole("button", { name: /add agent/i }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "my-agent" } });
    fireEvent.change(screen.getByLabelText("Binary"), { target: { value: "/usr/bin/x" } });
    fireEvent.click(screen.getByRole("button", { name: /^add$/i }));

    await waitFor(() => expect(listAgentsMock).toHaveBeenCalledTimes(2));
    await screen.findByText("My Agent");
  });

  it("reloads the agent list after a successful remove", async () => {
    listAgentsMock
      .mockResolvedValueOnce([customAgent]) // initial
      .mockResolvedValueOnce([]); // after remove
    render(<SettingsModal onClose={() => {}} />);
    await screen.findByText("My Agent");

    fireEvent.click(screen.getByRole("button", { name: /remove/i }));

    await waitFor(() => expect(listAgentsMock).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByText("My Agent")).toBeNull());
  });

  it("surfaces upsertCustomAgent errors in the UI", async () => {
    listAgentsMock.mockResolvedValue([]);
    upsertCustomAgentMock.mockRejectedValue(new Error("name collides with built-in"));
    render(<SettingsModal onClose={() => {}} />);
    await waitFor(() => expect(listAgentsMock).toHaveBeenCalledOnce());

    fireEvent.click(screen.getByRole("button", { name: /add agent/i }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "claude" } });
    fireEvent.change(screen.getByLabelText("Binary"), { target: { value: "claude" } });
    fireEvent.click(screen.getByRole("button", { name: /^add$/i }));

    await screen.findByText(/name collides with built-in/i);
  });

  it("closes on Escape", () => {
    listAgentsMock.mockResolvedValue([]);
    const onClose = vi.fn();
    render(<SettingsModal onClose={onClose} />);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });

  it("closes when clicking the backdrop", () => {
    listAgentsMock.mockResolvedValue([]);
    const onClose = vi.fn();
    const { container } = render(<SettingsModal onClose={onClose} />);
    const backdrop = container.querySelector(".settings__backdrop")!;
    fireEvent.click(backdrop);
    expect(onClose).toHaveBeenCalled();
  });

  it("does not close when clicking inside the dialog", () => {
    listAgentsMock.mockResolvedValue([]);
    const onClose = vi.fn();
    render(<SettingsModal onClose={onClose} />);
    fireEvent.click(screen.getByRole("dialog"));
    expect(onClose).not.toHaveBeenCalled();
  });

  it("Name input is readOnly while editing an existing agent to prevent orphan entries", async () => {
    listAgentsMock
      .mockResolvedValueOnce([customAgent]) // initial load
      .mockResolvedValueOnce([customAgent]); // reload after save
    render(<SettingsModal onClose={() => {}} />);
    await screen.findByText("My Agent");

    fireEvent.click(screen.getByRole("button", { name: /edit/i }));

    const nameInput = screen.getByLabelText("Name") as HTMLInputElement;
    // Name must be non-editable to prevent the orphan-entry bug.
    expect(nameInput.readOnly).toBe(true);
    // The original name is still in the field so upsert uses it.
    expect(nameInput.value).toBe("my-agent");

    // Submitting calls upsertCustomAgent with the original name, not a renamed one.
    fireEvent.click(screen.getByRole("button", { name: /save/i }));
    await waitFor(() =>
      expect(upsertCustomAgentMock).toHaveBeenCalledWith(
        expect.objectContaining({ name: "my-agent" }),
      ),
    );
  });

  it("Edit prefills the form with the agent's values", async () => {
    listAgentsMock.mockResolvedValue([customAgent]);
    render(<SettingsModal onClose={() => {}} />);
    await screen.findByText("My Agent");

    fireEvent.click(screen.getByRole("button", { name: /edit/i }));

    expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("my-agent");
    expect((screen.getByLabelText("Binary") as HTMLInputElement).value).toBe(
      "/usr/local/bin/my-agent",
    );
    // baseArgs joined: "--flag"
    expect((screen.getByLabelText("Base args") as HTMLTextAreaElement).value).toBe("--flag");
  });
});

describe("SettingsModal — tab switching", () => {
  beforeEach(() => {
    listAgentsMock.mockReset();
    listAgentsMock.mockResolvedValue([]);
  });

  it("shows the Agents tab by default and switches panels on tab click", async () => {
    render(<SettingsModal onClose={() => {}} />);

    // Agents is the default tab: its "Add agent" control is visible…
    expect(await screen.findByRole("button", { name: /add agent/i })).toBeInTheDocument();
    // …and other tabs' content is not mounted yet.
    expect(screen.queryByLabelText("Enable alerts")).toBeNull();
    expect(screen.queryByRole("button", { name: /enable remote/i })).toBeNull();

    // Switch to Notifications: its content mounts, Agents content unmounts.
    fireEvent.click(screen.getByRole("tab", { name: "Notifications" }));
    expect(screen.getByLabelText("Enable alerts")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /add agent/i })).toBeNull();

    // Switch to General: worktree toggle mounts, notification content unmounts.
    fireEvent.click(screen.getByRole("tab", { name: "General" }));
    expect(
      screen.getByLabelText("Base new worktrees on the latest remote base branch"),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("Enable alerts")).toBeNull();

    // Switch to Prompts: the prompt manager section mounts.
    fireEvent.click(screen.getByRole("tab", { name: "Prompts" }));
    expect(screen.getByRole("heading", { name: "Prompts" })).toBeInTheDocument();
  });

  it("marks the active tab with aria-selected", async () => {
    render(<SettingsModal onClose={() => {}} />);
    expect(screen.getByRole("tab", { name: "Agents" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    fireEvent.click(screen.getByRole("tab", { name: "Remote" }));
    expect(screen.getByRole("tab", { name: "Remote" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tab", { name: "Agents" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
  });
});

describe("SettingsModal — notification sounds section", () => {
  beforeEach(() => {
    listAgentsMock.mockReset();
    listAgentsMock.mockResolvedValue([]);
    useVigieStore.setState({
      soundSettings: { ...DEFAULT_SOUND_SETTINGS },
      setSoundSettings: async (next: SoundSettings) =>
        useVigieStore.setState({ soundSettings: next }),
    } as never);
  });

  it("toggles the fetch-remote-base app setting", () => {
    useVigieStore.setState({
      fetchRemoteBase: true,
      setFetchRemoteBase: async (enabled: boolean) =>
        useVigieStore.setState({ fetchRemoteBase: enabled }),
    } as never);
    render(<SettingsModal onClose={() => {}} />);
    // "New worktrees" lives on the General tab now.
    fireEvent.click(screen.getByRole("tab", { name: "General" }));
    const toggle = screen.getByLabelText("Base new worktrees on the latest remote base branch");
    expect(toggle).toBeChecked();
    fireEvent.click(toggle);
    expect(useVigieStore.getState().fetchRemoteBase).toBe(false);
  });

  it("toggles a per-event enable", () => {
    render(<SettingsModal onClose={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Notifications" }));
    fireEvent.click(screen.getByLabelText("Enable Completed alerts"));
    expect(useVigieStore.getState().soundSettings.events.completed.enabled).toBe(false);
  });

  it("changes a per-event sound", () => {
    render(<SettingsModal onClose={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Notifications" }));
    fireEvent.change(screen.getByLabelText("Completed sound"), {
      target: { value: "ready-work" },
    });
    expect(useVigieStore.getState().soundSettings.events.completed.sound).toBe("ready-work");
  });

  it("Add sound… imports and refreshes the custom library", async () => {
    const refreshCustomSounds = vi.fn();
    useVigieStore.setState({ refreshCustomSounds, customSounds: [] } as never);
    const { pickAndImportSound } = await import("../../sound/import");
    render(<SettingsModal onClose={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Notifications" }));
    fireEvent.click(screen.getByRole("button", { name: "Add sound…" }));
    await waitFor(() => expect(pickAndImportSound).toHaveBeenCalled());
  });
});
