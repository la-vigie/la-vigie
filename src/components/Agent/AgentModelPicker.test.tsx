import { render, screen, fireEvent, within } from "@testing-library/react";
import { it, expect, vi, beforeEach } from "vitest";
import { AgentModelPicker } from "./AgentModelPicker";
import * as hooks from "../../hooks/useAgents";

vi.mock("../../hooks/useAgents");

const AGENTS = [
  { name: "claude", displayName: "Claude Code", modelsListArgs: null },
  { name: "opencode", displayName: "OpenCode", modelsListArgs: ["models"] },
];

beforeEach(() => {
  vi.clearAllMocks();
  (hooks.useAgents as any).mockReturnValue({ agents: AGENTS, loading: false, error: null });
  (hooks.useAgentModels as any).mockImplementation((n: string) =>
    n === "opencode" ? { models: ["zhipuai-coding-plan/glm-5.2"], loading: false } : { models: [], loading: false });
});

it("trigger shows agent and selected model", () => {
  render(<AgentModelPicker agent="opencode" model="zhipuai-coding-plan/glm-5.2" onChange={() => {}} />);
  expect(screen.getByTestId("amp-trigger")).toHaveTextContent("OpenCode");
  expect(screen.getByTestId("amp-trigger")).toHaveTextContent("zhipuai-coding-plan/glm-5.2");
});

it("selecting a no-model agent emits null model and closes", async () => {
  const onChange = vi.fn();
  render(<AgentModelPicker agent="opencode" model={null} onChange={onChange} />);
  fireEvent.click(screen.getByTestId("amp-trigger"));
  fireEvent.click(await screen.findByText("Claude Code"));
  expect(onChange).toHaveBeenCalledWith("claude", null);
});

it("selecting a model emits agent + model", async () => {
  const onChange = vi.fn();
  render(<AgentModelPicker agent="opencode" model={null} onChange={onChange} />);
  fireEvent.click(screen.getByTestId("amp-trigger"));
  // highlight opencode in the agent list, then pick its model
  fireEvent.mouseEnter(within(screen.getByRole("menu")).getByText("OpenCode"));
  fireEvent.click(await screen.findByText("zhipuai-coding-plan/glm-5.2"));
  expect(onChange).toHaveBeenCalledWith("opencode", "zhipuai-coding-plan/glm-5.2");
});

it("MODEL pane is absent when hovered agent has no models", () => {
  render(<AgentModelPicker agent="claude" model={null} onChange={() => {}} />);
  // Open the popover — hovered defaults to the current agent ("claude"), which has no modelsListArgs
  fireEvent.click(screen.getByTestId("amp-trigger"));
  expect(screen.queryByText("MODEL")).toBeNull();
});

it("empty model list renders a selectable Default model row that emits (agent, null)", async () => {
  const onChange = vi.fn();
  // opencode advertises models but returns none
  (hooks.useAgentModels as any).mockImplementation(() => ({ models: [], loading: false }));
  render(<AgentModelPicker agent="claude" model={null} onChange={onChange} />);
  fireEvent.click(screen.getByTestId("amp-trigger"));
  // Hover opencode so its model pane opens (empty)
  fireEvent.mouseEnter(within(screen.getByRole("menu")).getByText("OpenCode"));
  const defaultRow = await screen.findByText("Default model");
  fireEvent.click(defaultRow);
  expect(onChange).toHaveBeenCalledWith("opencode", null);
  expect(screen.queryByRole("menu")).toBeNull();
});

it("outside click closes the popover", () => {
  render(<AgentModelPicker agent="claude" model={null} onChange={() => {}} />);
  fireEvent.click(screen.getByTestId("amp-trigger"));
  expect(screen.getByRole("menu")).toBeInTheDocument();
  fireEvent.mouseDown(document.body);
  expect(screen.queryByRole("menu")).toBeNull();
});

it("reopening resets hovered to the current agent", () => {
  render(<AgentModelPicker agent="claude" model={null} onChange={() => {}} />);
  // Open and hover opencode so its model pane appears
  fireEvent.click(screen.getByTestId("amp-trigger"));
  fireEvent.mouseEnter(within(screen.getByRole("menu")).getByText("OpenCode"));
  expect(screen.queryByText("MODEL")).not.toBeNull();
  // Close then reopen — hovered should reset to "claude" (no model pane)
  fireEvent.click(screen.getByTestId("amp-trigger"));
  fireEvent.click(screen.getByTestId("amp-trigger"));
  expect(screen.queryByText("MODEL")).toBeNull();
});

it("portals the popover to document.body so a clipping ancestor can't truncate it", () => {
  const { container } = render(
    <div style={{ overflow: "hidden" }}>
      <AgentModelPicker agent="claude" model={null} onChange={() => {}} />
    </div>,
  );
  fireEvent.click(screen.getByTestId("amp-trigger"));
  const menu = screen.getByRole("menu");
  // The popover lives under <body>, not inside the (overflow-hidden) wrapper.
  expect(menu.parentElement).toBe(document.body);
  expect(container.contains(menu)).toBe(false);
});
