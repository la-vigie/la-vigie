import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SpecDock, handleDocLinkClick } from "./SpecDock";
import { listTaskDocs, openUrl, readTaskDoc } from "../../api";

vi.mock("../../api", () => ({
  listTaskDocs: vi.fn(),
  readTaskDoc: vi.fn(),
  openUrl: vi.fn(),
}));

// Render markdown as plain text so we assert content flow, not library internals.
vi.mock("react-markdown", () => ({
  default: ({ children }: { children: string }) => (
    <div data-testid="markdown">{children}</div>
  ),
}));
vi.mock("remark-gfm", () => ({ default: () => undefined }));

const listMock = vi.mocked(listTaskDocs);
const readMock = vi.mocked(readTaskDoc);
const openUrlMock = vi.mocked(openUrl);

describe("SpecDock", () => {
  beforeEach(() => {
    localStorage.clear();
    // Start expanded so the body is visible without a click.
    localStorage.setItem("vigie.specDockCollapsed", "false");
    listMock.mockReset();
    readMock.mockReset();
    openUrlMock.mockReset();
    openUrlMock.mockResolvedValue(undefined);
  });

  describe("handleDocLinkClick", () => {
    it("opens absolute http(s) links in the browser and blocks navigation", () => {
      const e = { preventDefault: vi.fn() };
      handleDocLinkClick(e, "https://example.com/x");
      expect(e.preventDefault).toHaveBeenCalled();
      expect(openUrlMock).toHaveBeenCalledWith("https://example.com/x");
    });

    it("blocks navigation for relative links without opening anything", () => {
      const e = { preventDefault: vi.fn() };
      handleDocLinkClick(e, "docs/superpowers/plans/p.md");
      expect(e.preventDefault).toHaveBeenCalled();
      expect(openUrlMock).not.toHaveBeenCalled();
    });
  });

  it("shows an empty state when the task has no docs", async () => {
    listMock.mockResolvedValue([]);
    render(<SpecDock taskId="t1" />);
    expect(await screen.findByText(/no spec or docs found/i)).toBeInTheDocument();
  });

  it("renders the markdown of the single resolved doc", async () => {
    listMock.mockResolvedValue([{ id: "memory/spec_TASK-54.md", label: "Spec (TASK-54)" }]);
    readMock.mockResolvedValue("# Spec body");
    render(<SpecDock taskId="t1" />);
    expect(await screen.findByTestId("markdown")).toHaveTextContent("# Spec body");
    expect(readMock).toHaveBeenCalledWith("t1", "memory/spec_TASK-54.md");
  });

  it("offers a picker and switches docs when multiple are present", async () => {
    listMock.mockResolvedValue([
      { id: "memory/spec_TASK-54.md", label: "Spec (TASK-54)" },
      { id: "docs/superpowers/plans/p.md", label: "Plan: p.md" },
    ]);
    readMock.mockImplementation((_t, id) =>
      Promise.resolve(id.includes("plans") ? "# Plan body" : "# Spec body"),
    );
    render(<SpecDock taskId="t1" />);
    // Defaults to the first doc.
    expect(await screen.findByTestId("markdown")).toHaveTextContent("# Spec body");
    // Switch to the plan.
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: /select document/i }),
      "docs/superpowers/plans/p.md",
    );
    await waitFor(() =>
      expect(screen.getByTestId("markdown")).toHaveTextContent("# Plan body"),
    );
  });

  describe("maximize affordance", () => {
    it("shows a Maximize button when onToggleMaximize is provided and fires it", async () => {
      listMock.mockResolvedValue([]);
      const onToggleMaximize = vi.fn();
      render(<SpecDock taskId="t1" onToggleMaximize={onToggleMaximize} />);

      await userEvent.click(
        await screen.findByRole("button", { name: /maximize spec and docs/i }),
      );
      expect(onToggleMaximize).toHaveBeenCalledTimes(1);
    });

    it("when maximized: renders expanded even if collapsed, shows Restore, hides grip + collapse", async () => {
      // Collapsed in storage, but maximized must force the expanded view.
      localStorage.setItem("vigie.specDockCollapsed", "true");
      listMock.mockResolvedValue([{ id: "memory/spec.md", label: "Spec" }]);
      readMock.mockResolvedValue("# Body");
      render(<SpecDock taskId="t1" maximized onToggleMaximize={vi.fn()} />);

      // Expanded body is visible (not the one-line collapsed bar).
      expect(await screen.findByTestId("markdown")).toHaveTextContent("# Body");
      // Restore (not Maximize), no resize grip, no collapse button.
      expect(screen.getByRole("button", { name: /restore spec and docs/i })).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: /maximize spec and docs/i })).not.toBeInTheDocument();
      expect(screen.queryByRole("separator", { name: /resize spec and docs/i })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: /collapse spec and docs/i })).not.toBeInTheDocument();
      // The section fills the pane (modifier class) rather than a fixed height.
      const section = document.querySelector(".spec-dock");
      expect(section?.className).toContain("spec-dock--max");
      expect((section as HTMLElement | null)?.style.height).toBe("");
    });

    it("does not render the Maximize button when no handler is provided", async () => {
      listMock.mockResolvedValue([]);
      render(<SpecDock taskId="t1" />);
      await screen.findByText(/no spec or docs found/i);
      expect(screen.queryByRole("button", { name: /maximize spec and docs/i })).not.toBeInTheDocument();
    });
  });
});
