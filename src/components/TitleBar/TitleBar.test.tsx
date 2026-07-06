import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TitleBar } from "./TitleBar";
import { useVigieStore } from "../../store";
import { DEFAULT_SOUND_SETTINGS, type SoundSettings } from "../../sound/types";

// SettingsModal calls listAgents() on mount; mock the api module so tests
// don't hit the Tauri invoke layer.
const { listAgentsMock } = vi.hoisted(() => ({ listAgentsMock: vi.fn() }));
vi.mock("../../api", () => ({
  listAgents: listAgentsMock,
  upsertCustomAgent: vi.fn(),
  deleteCustomAgent: vi.fn(),
  remoteStatus: vi.fn().mockResolvedValue({ active: false, sleepInhibited: false }),
  enableRemote: vi.fn().mockResolvedValue({ active: false, sleepInhibited: false }),
  disableRemote: vi.fn().mockResolvedValue({ active: false, sleepInhibited: false }),
}));

describe("TitleBar", () => {
  beforeEach(() => {
    listAgentsMock.mockReset();
    listAgentsMock.mockResolvedValue([]);
    localStorage.clear();
    document.documentElement.setAttribute("data-theme", "dark");
    useVigieStore.setState({
      repos: [],
      tasks: [],
      selectedTaskId: null,
      theme: "dark",
    } as any);
  });

  it("renders the La Vigie brand", () => {
    render(<TitleBar />);
    expect(screen.getByText("La Vigie")).toBeInTheDocument();
  });

  it("shows the logo image", () => {
    const { container } = render(<TitleBar />);
    const logo = container.querySelector("img.titlebar__logo");
    expect(logo).not.toBeNull();
    expect(logo).toHaveAttribute("src", "/logo.png");
  });

  it("toggling the theme flips the document data-theme attribute and the label", () => {
    render(<TitleBar />);

    const toggle = screen.getByRole("button", { name: /switch to light theme/i });
    fireEvent.click(toggle);

    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
    expect(useVigieStore.getState().theme).toBe("light");
    expect(
      screen.getByRole("button", { name: /switch to dark theme/i }),
    ).toBeInTheDocument();
  });

  it("marks the non-interactive title-bar elements as drag regions and leaves controls non-drag", () => {
    const { container } = render(<TitleBar />);

    // The bar fills its full width with these wrappers; each must be a drag
    // region or the window can't be dragged from them (AC2-74 regression).
    const header = container.querySelector("header.titlebar")!;
    const brand = container.querySelector(".titlebar__brand")!;
    const logo = container.querySelector("img.titlebar__logo")!;
    const wordmark = container.querySelector(".titlebar__wordmark")!;
    const right = container.querySelector(".titlebar__right")!;

    for (const el of [header, brand, logo, wordmark, right]) {
      expect(el).toHaveAttribute("data-tauri-drag-region");
    }

    // Interactive controls must NOT carry the attribute (so clicks don't drag).
    for (const button of Array.from(container.querySelectorAll("button"))) {
      expect(button).not.toHaveAttribute("data-tauri-drag-region");
    }
  });

  it("shows the repo/branch breadcrumb for the selected task", () => {
    useVigieStore.setState({
      repos: [{ id: "r1", name: "home-mgmt", path: "/x", defaultBranch: "main" }],
      tasks: [
        {
          id: "t1",
          repoId: "r1",
          title: "Repo overview",
          worktreePath: "/x/wt",
          branch: "repo-overview",
          baseBranch: "main",
          status: "working",
          createdAt: 1,
          updatedAt: 1,
        },
      ],
      selectedTaskId: "t1",
    } as any);

    render(<TitleBar />);
    expect(screen.getByText("home-mgmt")).toBeInTheDocument();
    expect(screen.getByText("repo-overview")).toBeInTheDocument();
  });

  describe("gear / settings button (AC2-21)", () => {
    it("renders a gear button with aria-label='Settings'", () => {
      render(<TitleBar />);
      expect(screen.getByRole("button", { name: "Settings" })).toBeInTheDocument();
    });

    it("gear button does not carry data-tauri-drag-region", () => {
      render(<TitleBar />);
      const gear = screen.getByRole("button", { name: "Settings" });
      expect(gear).not.toHaveAttribute("data-tauri-drag-region");
    });

    it("clicking the gear button opens the Settings modal (role=dialog appears)", async () => {
      render(<TitleBar />);
      expect(screen.queryByRole("dialog", { name: "Settings" })).toBeNull();
      fireEvent.click(screen.getByRole("button", { name: "Settings" }));
      await waitFor(() =>
        expect(screen.getByRole("dialog", { name: "Settings" })).toBeInTheDocument(),
      );
    });

    it("closing the modal removes the dialog from the DOM", async () => {
      render(<TitleBar />);
      fireEvent.click(screen.getByRole("button", { name: "Settings" }));
      await waitFor(() =>
        expect(screen.getByRole("dialog", { name: "Settings" })).toBeInTheDocument(),
      );
      // Close via the Close button inside the modal
      fireEvent.click(screen.getByRole("button", { name: "Close" }));
      expect(screen.queryByRole("dialog", { name: "Settings" })).toBeNull();
    });
  });
});

describe("TitleBar mute toggle", () => {
  beforeEach(() => {
    useVigieStore.setState({
      repos: [], tasks: [], selectedTaskId: null,
      soundSettings: { ...DEFAULT_SOUND_SETTINGS, muted: false },
      setSoundSettings: async (next: SoundSettings) => useVigieStore.setState({ soundSettings: next }),
    } as never);
  });

  it("toggles muted when the mute button is clicked", () => {
    render(<TitleBar />);
    fireEvent.click(screen.getByLabelText("Mute notification sounds"));
    expect(useVigieStore.getState().soundSettings.muted).toBe(true);
    // now shows the unmute affordance
    expect(screen.getByLabelText("Unmute notification sounds")).toBeInTheDocument();
  });
});
