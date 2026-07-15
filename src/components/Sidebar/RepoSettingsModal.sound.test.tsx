import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RepoSettingsModal } from "./RepoSettingsModal";
import type { Repo } from "../../store";
import { useVigieStore } from "../../store";

const { invokeMock, openMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  openMock: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: openMock }));

const updateRepoMock = vi.fn().mockResolvedValue({});
vi.mock("../../api", async (orig) => ({
  ...(await orig<typeof import("../../api")>()),
  updateRepo: (...args: unknown[]) => updateRepoMock(...args),
}));

const repo: Repo = {
  id: "repo-a",
  name: "sound-test-repo",
  path: "/tmp/repo-a",
  defaultBranch: "main",
  soundSettings: null,
  inPlaceDefault: false,
};

/** Returns the soundSettings arg (8th positional, 0-based index 7) from the last call. */
function soundSettingsArg(mock: ReturnType<typeof vi.fn>): unknown {
  const calls = mock.mock.calls;
  const lastCall = calls[calls.length - 1];
  return lastCall[7];
}

describe("RepoSettingsModal – sound override", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_repo_branches") return Promise.resolve(["main"]);
      // The merged modal renders <AgentPicker>, which loads via list_agents and
      // calls .map on the result — must resolve to an array.
      if (cmd === "list_agents") return Promise.resolve([]);
      return Promise.resolve({ repos: [], tasks: [] });
    });
    openMock.mockReset();
    updateRepoMock.mockReset();
    updateRepoMock.mockResolvedValue({});
    useVigieStore.setState({
      worktreesRoot: "/global/wt",
      repos: [repo],
      tasks: [],
      selectedTaskId: null,
      sessionsByTask: {},
    });
  });

  it("saves a repo force-mute override", async () => {
    render(<RepoSettingsModal repo={repo} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("Repo sound mute"), {
      target: { value: "off" },
    });
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() => expect(updateRepoMock).toHaveBeenCalled());
    expect(JSON.parse(soundSettingsArg(updateRepoMock) as string)).toEqual({ muted: true });
  });

  it("passing no overrides saves null soundSettings", async () => {
    render(<RepoSettingsModal repo={repo} onClose={() => {}} />);
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() => expect(updateRepoMock).toHaveBeenCalled());
    expect(soundSettingsArg(updateRepoMock)).toBeNull();
  });

  it("setting mute to On (unmuted) saves muted:false override", async () => {
    render(<RepoSettingsModal repo={repo} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("Repo sound mute"), {
      target: { value: "on" },
    });
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() => expect(updateRepoMock).toHaveBeenCalled());
    expect(JSON.parse(soundSettingsArg(updateRepoMock) as string)).toEqual({ muted: false });
  });

  it("setting mute then back to inherit saves null soundSettings", async () => {
    render(<RepoSettingsModal repo={repo} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("Repo sound mute"), {
      target: { value: "off" },
    });
    fireEvent.change(screen.getByLabelText("Repo sound mute"), {
      target: { value: "inherit" },
    });
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() => expect(updateRepoMock).toHaveBeenCalled());
    expect(soundSettingsArg(updateRepoMock)).toBeNull();
  });

  it("setting an event sound saves a minimal override", async () => {
    render(<RepoSettingsModal repo={repo} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("Completed sound"), {
      target: { value: "error" },
    });
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() => expect(updateRepoMock).toHaveBeenCalled());
    expect(JSON.parse(soundSettingsArg(updateRepoMock) as string)).toEqual({
      events: { completed: { sound: "error" } },
    });
  });

  it("lists custom sounds in the repo per-event picker", async () => {
    useVigieStore.setState({
      customSounds: [{ id: "custom:abc", label: "My Ding", ext: "mp3" }],
    });
    render(<RepoSettingsModal repo={repo} onClose={() => {}} />);
    const options = await screen.findAllByRole("option", { name: "My Ding" });
    expect(options.length).toBeGreaterThan(0);
  });
});
