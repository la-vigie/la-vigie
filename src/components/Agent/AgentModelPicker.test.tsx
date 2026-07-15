import { render, screen, fireEvent, within } from "@testing-library/react";
import { it, expect, vi, beforeEach } from "vitest";
import { AgentModelPicker } from "./AgentModelPicker";
import * as hooks from "../../hooks/useAgents";

vi.mock("../../hooks/useAgents");

// claude: takes `--model` but can't enumerate (free-text). opencode: enumerates
// (list). aider: takes no model at all (no control). Mirrors the real specs.
const AGENTS = [
  { name: "claude", displayName: "Claude Code", modelArg: "--model", modelsListArgs: null },
  { name: "opencode", displayName: "OpenCode", modelArg: "--model", modelsListArgs: ["models"] },
  { name: "aider", displayName: "Aider", modelArg: null, modelsListArgs: null },
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
  fireEvent.click(await screen.findByText("Aider"));
  expect(onChange).toHaveBeenCalledWith("aider", null);
  expect(screen.queryByRole("menu")).toBeNull();
});

it("selecting a model from an enumerating agent emits agent + model", async () => {
  const onChange = vi.fn();
  render(<AgentModelPicker agent="opencode" model={null} onChange={onChange} />);
  fireEvent.click(screen.getByTestId("amp-trigger"));
  // highlight opencode in the agent list, then pick its model
  fireEvent.mouseEnter(within(screen.getByRole("menu")).getByText("OpenCode"));
  fireEvent.click(await screen.findByText("zhipuai-coding-plan/glm-5.2"));
  expect(onChange).toHaveBeenCalledWith("opencode", "zhipuai-coding-plan/glm-5.2");
});

it("MODEL pane is absent for an agent that takes no model", () => {
  render(<AgentModelPicker agent="aider" model={null} onChange={() => {}} />);
  // hovered defaults to the current agent ("aider"), which has no modelArg
  fireEvent.click(screen.getByTestId("amp-trigger"));
  expect(screen.queryByText("MODEL")).toBeNull();
});

it("empty model list renders a selectable Default model row that emits (agent, null)", async () => {
  const onChange = vi.fn();
  // opencode advertises models but returns none
  (hooks.useAgentModels as any).mockImplementation(() => ({ models: [], loading: false }));
  render(<AgentModelPicker agent="aider" model={null} onChange={onChange} />);
  fireEvent.click(screen.getByTestId("amp-trigger"));
  // Hover opencode so its (empty) model pane opens
  fireEvent.mouseEnter(within(screen.getByRole("menu")).getByText("OpenCode"));
  const defaultRow = await screen.findByText("Default model");
  fireEvent.click(defaultRow);
  expect(onChange).toHaveBeenCalledWith("opencode", null);
  expect(screen.queryByRole("menu")).toBeNull();
});

// ── Free-text branch (TASK-209): engines with modelArg but no modelsListArgs ──

it("free-text: typing a model id and pressing Enter commits it", () => {
  const onChange = vi.fn();
  render(<AgentModelPicker agent="claude" model={null} onChange={onChange} />);
  fireEvent.click(screen.getByTestId("amp-trigger"));
  // Opening on claude (current) reveals its free-text pane.
  const input = screen.getByTestId("amp-model-input");
  fireEvent.change(input, { target: { value: "opus-4.8" } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(onChange).toHaveBeenCalledWith("claude", "opus-4.8");
  expect(screen.queryByRole("menu")).toBeNull();
});

it("free-text: a quick-pick alias chip commits that alias", () => {
  const onChange = vi.fn();
  render(<AgentModelPicker agent="claude" model={null} onChange={onChange} />);
  fireEvent.click(screen.getByTestId("amp-trigger"));
  fireEvent.click(screen.getByRole("button", { name: "sonnet" }));
  expect(onChange).toHaveBeenCalledWith("claude", "sonnet");
});

it("free-text: input seeds from the current model", () => {
  render(<AgentModelPicker agent="claude" model="opus" onChange={() => {}} />);
  fireEvent.click(screen.getByTestId("amp-trigger"));
  expect(screen.getByTestId("amp-model-input")).toHaveValue("opus");
});

it("free-text: Default model row commits (agent, null) — unset", () => {
  const onChange = vi.fn();
  render(<AgentModelPicker agent="claude" model="opus" onChange={onChange} />);
  fireEvent.click(screen.getByTestId("amp-trigger"));
  fireEvent.click(screen.getByText("Default model"));
  expect(onChange).toHaveBeenCalledWith("claude", null);
});

it("free-text: an empty input via Set model commits null", () => {
  const onChange = vi.fn();
  render(<AgentModelPicker agent="claude" model="opus" onChange={onChange} />);
  fireEvent.click(screen.getByTestId("amp-trigger"));
  fireEvent.change(screen.getByTestId("amp-model-input"), { target: { value: "  " } });
  fireEvent.click(screen.getByRole("button", { name: "Set model" }));
  expect(onChange).toHaveBeenCalledWith("claude", null);
});

it("free-text: picking a free-text agent from the list reveals its pane, not a commit", async () => {
  const onChange = vi.fn();
  render(<AgentModelPicker agent="opencode" model={null} onChange={onChange} />);
  fireEvent.click(screen.getByTestId("amp-trigger"));
  fireEvent.click(await screen.findByText("Claude Code"));
  // No commit yet — the free-text pane opens and awaits a model.
  expect(onChange).not.toHaveBeenCalled();
  expect(screen.getByTestId("amp-model-input")).toBeInTheDocument();
  expect(screen.getByRole("menu")).toBeInTheDocument();
});

it("outside click closes the popover", () => {
  render(<AgentModelPicker agent="aider" model={null} onChange={() => {}} />);
  fireEvent.click(screen.getByTestId("amp-trigger"));
  expect(screen.getByRole("menu")).toBeInTheDocument();
  fireEvent.mouseDown(document.body);
  expect(screen.queryByRole("menu")).toBeNull();
});

it("reopening resets hovered to the current agent", () => {
  render(<AgentModelPicker agent="aider" model={null} onChange={() => {}} />);
  // Open and hover opencode so its model pane appears
  fireEvent.click(screen.getByTestId("amp-trigger"));
  fireEvent.mouseEnter(within(screen.getByRole("menu")).getByText("OpenCode"));
  expect(screen.queryByText("MODEL")).not.toBeNull();
  // Close then reopen — hovered should reset to "aider" (no model pane)
  fireEvent.click(screen.getByTestId("amp-trigger"));
  fireEvent.click(screen.getByTestId("amp-trigger"));
  expect(screen.queryByText("MODEL")).toBeNull();
});

it("portals the popover to document.body so a clipping ancestor can't truncate it", () => {
  const { container } = render(
    <div style={{ overflow: "hidden" }}>
      <AgentModelPicker agent="aider" model={null} onChange={() => {}} />
    </div>,
  );
  fireEvent.click(screen.getByTestId("amp-trigger"));
  const menu = screen.getByRole("menu");
  // The popover lives under <body>, not inside the (overflow-hidden) wrapper.
  expect(menu.parentElement).toBe(document.body);
  expect(container.contains(menu)).toBe(false);
});
