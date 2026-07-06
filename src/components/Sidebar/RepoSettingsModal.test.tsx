import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RepoSettingsModal } from "./RepoSettingsModal";
import type { Repo, Task } from "../../store";
import { useVigieStore } from "../../store";

const { invokeMock, openMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  openMock: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: openMock }));

const repo: Repo = {
  id: "repo-a",
  name: "home-mgmt",
  path: "/tmp/repo-a",
  defaultBranch: "main",
  remoteUrl: "git@github.com:me/home-mgmt.git",
};

const agentFixtures = [
  { name: "claude", displayName: "Claude Code" },
  { name: "antigravity", displayName: "Antigravity" },
];

describe("RepoSettingsModal", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    // refresh() -> list_state resolves to an empty snapshot; the base-branch
    // dropdown loads via list_repo_branches; AgentPicker loads via list_agents.
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_repo_branches") return Promise.resolve(["main"]);
      if (cmd === "list_agents") return Promise.resolve(agentFixtures);
      if (cmd === "list_agent_models") return Promise.resolve([]);
      if (cmd === "set_repo_default_model") return Promise.resolve();
      return Promise.resolve({ repos: [], tasks: [] });
    });
    openMock.mockReset();
    useVigieStore.setState({ worktreesRoot: "/global/wt", repos: [repo], tasks: [], selectedTaskId: null, sessionsByTask: {} });
  });

  const taskForRepo = (id: string, repoId: string): Task => ({
    id,
    repoId,
    title: id,
    worktreePath: `/tmp/wt/${id}`,
    branch: id,
    baseBranch: "main",
    status: "idle",
    createdAt: 0,
    updatedAt: 0,
  });

  describe("danger zone (AC2-69)", () => {
    it("does not show the inline confirm until Remove repository is clicked", () => {
      render(<RepoSettingsModal repo={repo} onClose={() => {}} />);
      expect(screen.getByText("Remove repository")).toBeTruthy();
      expect(screen.queryByText("Remove", { selector: "button" })).toBeNull();
    });

    it("shows a confirm naming the repo and its task count, and Remove repository does not call remove_repo yet", () => {
      useVigieStore.setState({
        tasks: [taskForRepo("t1", "repo-a"), taskForRepo("t2", "repo-a"), taskForRepo("t3", "other")],
      });
      render(<RepoSettingsModal repo={repo} onClose={() => {}} />);
      fireEvent.click(screen.getByText("Remove repository"));

      // Confirm copy mentions the repo name and the count of its 2 tasks.
      const confirmText = screen.getByText(/and its 2 tasks/);
      expect(confirmText.textContent).toContain("home-mgmt");
      expect(invokeMock).not.toHaveBeenCalledWith("remove_repo", expect.anything());
    });

    it("Cancel in the confirm aborts without calling remove_repo", () => {
      render(<RepoSettingsModal repo={repo} onClose={() => {}} />);
      fireEvent.click(screen.getByText("Remove repository"));
      fireEvent.click(screen.getByText("Cancel"));
      expect(invokeMock).not.toHaveBeenCalledWith("remove_repo", expect.anything());
    });

    it("confirming invokes remove_repo, refreshes, then closes", async () => {
      const onClose = vi.fn();
      render(<RepoSettingsModal repo={repo} onClose={onClose} />);
      fireEvent.click(screen.getByText("Remove repository"));
      fireEvent.click(screen.getByRole("button", { name: "Remove" }));

      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith("remove_repo", { repoId: "repo-a" }),
      );
      await waitFor(() => {
        expect(invokeMock).toHaveBeenCalledWith("list_state");
        expect(onClose).toHaveBeenCalled();
      });
    });
  });

  it("renders editable and read-only fields", () => {
    render(<RepoSettingsModal repo={repo} onClose={() => {}} />);
    expect(screen.getByDisplayValue("home-mgmt")).toBeTruthy();
    expect(screen.getByDisplayValue("main")).toBeTruthy();
    expect(screen.getByText("/tmp/repo-a")).toBeTruthy();
    expect(screen.getByText("git@github.com:me/home-mgmt.git")).toBeTruthy();
  });

  it("saves trimmed values, refreshes, then closes", async () => {
    const onClose = vi.fn();
    render(<RepoSettingsModal repo={repo} onClose={onClose} />);
    fireEvent.change(screen.getByDisplayValue("home-mgmt"), {
      target: { value: "  renamed  " },
    });
    fireEvent.click(screen.getByText("Save"));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("update_repo", {
        repoId: "repo-a",
        name: "renamed",
        defaultBranch: "main",
        worktreeRoot: null,
        setupCommand: null,
        autoStartAgent: false,
        initialPrompt: null,
        soundSettings: null,
        fetchRemoteBase: null,
        defaultAgent: "claude",
      }),
    );
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("list_state");
      expect(onClose).toHaveBeenCalled();
    });
  });

  it("closes on Escape", () => {
    const onClose = vi.fn();
    render(<RepoSettingsModal repo={repo} onClose={onClose} />);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });

  it("blocks save when the name is empty", () => {
    render(<RepoSettingsModal repo={repo} onClose={() => {}} />);
    fireEvent.change(screen.getByDisplayValue("home-mgmt"), {
      target: { value: "   " },
    });
    fireEvent.click(screen.getByText("Save"));
    expect(invokeMock).not.toHaveBeenCalledWith(
      "update_repo",
      expect.anything(),
    );
  });

  it("shows the global default path and a Default badge when worktree root is unset", () => {
    render(<RepoSettingsModal repo={repo} onClose={() => {}} />);
    expect(screen.getByText("/global/wt/repo-a")).toBeTruthy();
    expect(screen.getByText("Default")).toBeTruthy();
  });

  it("loads the base branch dropdown from list_repo_branches", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_repo_branches") return Promise.resolve(["develop", "main"]);
      if (cmd === "list_agents") return Promise.resolve(agentFixtures);
      return Promise.resolve({ repos: [], tasks: [] });
    });
    render(<RepoSettingsModal repo={repo} onClose={() => {}} />);
    await waitFor(() =>
      expect(screen.getByRole("option", { name: "develop" })).toBeTruthy(),
    );
    expect(invokeMock).toHaveBeenCalledWith("list_repo_branches", { repoId: "repo-a" });
  });

  it("Choose location… sets the worktree root from the folder picker", async () => {
    openMock.mockResolvedValue("/Users/me/picked");
    render(<RepoSettingsModal repo={repo} onClose={() => {}} />);
    fireEvent.click(screen.getByText("Choose location…"));
    await waitFor(() => expect(screen.getByText("/Users/me/picked")).toBeTruthy());
    expect(openMock).toHaveBeenCalledWith({ directory: true });
  });

  it("saves the chosen worktree root", async () => {
    openMock.mockResolvedValue("/Users/me/picked");
    const onClose = vi.fn();
    render(<RepoSettingsModal repo={repo} onClose={onClose} />);

    fireEvent.click(screen.getByText("Choose location…"));
    await waitFor(() => expect(screen.getByText("/Users/me/picked")).toBeTruthy());
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("update_repo", {
        repoId: "repo-a",
        name: "home-mgmt",
        defaultBranch: "main",
        worktreeRoot: "/Users/me/picked",
        setupCommand: null,
        autoStartAgent: false,
        initialPrompt: null,
        soundSettings: null,
        fetchRemoteBase: null,
        defaultAgent: "claude",
      }),
    );
  });

  it("shows the repo's setup command in the input", () => {
    const withCmd: Repo = { ...repo, setupCommand: "cwt" };
    render(<RepoSettingsModal repo={withCmd} onClose={() => {}} />);
    expect(screen.getByDisplayValue("cwt")).toBeTruthy();
  });

  it("saves the repo fetch-remote-base override", async () => {
    const repoWithOverride: Repo = { ...repo, fetchRemoteBase: null };
    render(<RepoSettingsModal repo={repoWithOverride} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("Fetch remote base"), {
      target: { value: "off" },
    });
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("update_repo", expect.objectContaining({
        fetchRemoteBase: false,
      })),
    );
  });

  it("saves an edited setup command", async () => {
    const onClose = vi.fn();
    render(<RepoSettingsModal repo={repo} onClose={onClose} />);
    fireEvent.change(screen.getByLabelText("Setup command"), {
      target: { value: "  make setup  " },
    });
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("update_repo", {
        repoId: "repo-a",
        name: "home-mgmt",
        defaultBranch: "main",
        worktreeRoot: null,
        setupCommand: "make setup",
        autoStartAgent: false,
        initialPrompt: null,
        soundSettings: null,
        fetchRemoteBase: null,
        defaultAgent: "claude",
      }),
    );
  });

  describe("default agent + model (AC2-21 / AC2-93)", () => {
    it("falls back to 'claude' when repo has no defaultAgent", async () => {
      render(<RepoSettingsModal repo={repo} onClose={() => {}} />);
      expect(await screen.findByTestId("amp-trigger")).toHaveTextContent("Claude Code");
    });

    it("seeds the picker from repo.defaultAgent", async () => {
      const withAgent: Repo = { ...repo, defaultAgent: "antigravity" };
      render(<RepoSettingsModal repo={withAgent} onClose={() => {}} />);
      expect(await screen.findByTestId("amp-trigger")).toHaveTextContent("Antigravity");
    });

    it("saves the chosen agent via update_repo + setRepoDefaultModel on save", async () => {
      const onClose = vi.fn();
      render(<RepoSettingsModal repo={repo} onClose={onClose} />);
      // Open the combined picker and choose a (no-model) agent from the menu.
      fireEvent.click(await screen.findByTestId("amp-trigger"));
      fireEvent.click(await screen.findByText("Antigravity"));
      fireEvent.click(screen.getByText("Save"));
      // The agent override now rides along in the single update_repo round-trip.
      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith(
          "update_repo",
          expect.objectContaining({ repoId: "repo-a", defaultAgent: "antigravity" }),
        ),
      );
      // antigravity advertises no models, so the default model is cleared (null).
      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith("set_repo_default_model", {
          repoId: "repo-a",
          model: null,
        }),
      );
      await waitFor(() => expect(onClose).toHaveBeenCalled());
    });
  });

  it("Clear resets the worktree root and saves null", async () => {
    const preset: Repo = { ...repo, worktreeRoot: "/preset/wt" };
    const onClose = vi.fn();
    render(<RepoSettingsModal repo={preset} onClose={onClose} />);

    // The preset path is shown, and Clear is available.
    expect(screen.getByText("/preset/wt")).toBeTruthy();
    fireEvent.click(screen.getByText("Clear"));

    // Display reverts to the global default; Clear disappears.
    expect(screen.getByText("/global/wt/repo-a")).toBeTruthy();
    expect(screen.getByText("Default")).toBeTruthy();
    expect(screen.queryByText("Clear")).toBeNull();

    fireEvent.click(screen.getByText("Save"));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("update_repo", {
        repoId: "repo-a",
        name: "home-mgmt",
        defaultBranch: "main",
        worktreeRoot: null,
        setupCommand: null,
        autoStartAgent: false,
        initialPrompt: null,
        soundSettings: null,
        fetchRemoteBase: null,
        defaultAgent: "claude",
      }),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });
});
