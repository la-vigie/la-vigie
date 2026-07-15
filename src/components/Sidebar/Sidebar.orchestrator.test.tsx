import { afterEach, describe, expect, it, vi } from "vitest";
import { render, cleanup, fireEvent, screen } from "@testing-library/react";
import { RepoSection } from "./Sidebar";
import { useVigieStore, orchestratorSurfaceId } from "../../store";
import type { Repo } from "../../store";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

afterEach(cleanup);

const repo: Repo = { id: "r1", name: "acme", path: "/x", defaultBranch: "main", inPlaceDefault: false };

describe("Sidebar orchestrator row", () => {
  it("clicking Orchestrator selects the repo's orchestrator and opens its session", () => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_agents") return Promise.resolve([]);
      return Promise.resolve({ repos: [], tasks: [] });
    });
    useVigieStore.setState({
      selectedTaskId: null,
      selectedOrchestratorRepoId: null,
      sessionsByTask: {},
      activeTabByTask: {},
      tasks: [],
    } as any);

    render(<RepoSection repo={repo} search="" />);

    fireEvent.click(screen.getByRole("button", { name: /orchestrator/i }));

    expect(useVigieStore.getState().selectedOrchestratorRepoId).toBe("r1");
    expect(
      useVigieStore.getState().sessionsByTask[orchestratorSurfaceId("r1")],
    ).toHaveLength(1);
  });

  it("highlights the row when this repo's orchestrator is selected", () => {
    useVigieStore.setState({
      selectedTaskId: null,
      selectedOrchestratorRepoId: "r1",
      sessionsByTask: {},
      activeTabByTask: {},
      tasks: [],
    } as any);

    render(<RepoSection repo={repo} search="" />);

    expect(screen.getByRole("button", { name: /orchestrator/i })).toHaveClass(
      "sidebar__orchestrator-row--selected",
    );
  });
});
