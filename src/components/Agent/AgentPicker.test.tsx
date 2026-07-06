import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AgentPicker } from "./AgentPicker";
import type { AgentSpec } from "../../store";

// --------------------------------------------------------------------------
// Mocks
// --------------------------------------------------------------------------

const { listAgentsMock } = vi.hoisted(() => {
  const listAgentsMock = vi.fn().mockResolvedValue([]);
  return { listAgentsMock };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  Channel: class {
    onmessage: ((event: unknown) => void) | null = null;
  },
}));

vi.mock("../../api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api")>();
  return { ...actual, listAgents: listAgentsMock };
});

// --------------------------------------------------------------------------
// Fixtures
// --------------------------------------------------------------------------

const AGENTS: AgentSpec[] = [
  {
    name: "claude",
    displayName: "Claude Code",
    binary: "claude",
    baseArgs: [],
    resumeArgs: ["--continue"],
    extraArgs: [],
    promptMode: "arg",
    status: "claudeHooks",
    builtin: true,
  },
  {
    name: "aider",
    displayName: "Aider",
    binary: "aider",
    baseArgs: [],
    resumeArgs: [],
    extraArgs: [],
    promptMode: "arg",
    status: "lifecycle",
    builtin: false,
  },
];

// --------------------------------------------------------------------------
// Tests — controlled (agents prop supplied)
// --------------------------------------------------------------------------

describe("AgentPicker — controlled (agents prop supplied)", () => {
  it("renders a label and select with the correct class names and aria-label", () => {
    render(
      <AgentPicker value="claude" onChange={vi.fn()} agents={AGENTS} />,
    );

    const label = document.querySelector(".agent-picker");
    expect(label).toBeInTheDocument();
    expect(document.querySelector(".agent-picker__label")).toBeInTheDocument();
    const select = document.querySelector(".agent-picker__select");
    expect(select).toBeInTheDocument();
    expect(select).toHaveAttribute("aria-label", "Agent");
  });

  it("renders each supplied agent as an option", () => {
    render(
      <AgentPicker value="claude" onChange={vi.fn()} agents={AGENTS} />,
    );

    expect(screen.getByRole("option", { name: "Claude Code" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Aider" })).toBeInTheDocument();
  });

  it("fires onChange with the selected agent name when the user changes the select", async () => {
    const onChange = vi.fn();
    render(
      <AgentPicker value="claude" onChange={onChange} agents={AGENTS} />,
    );

    await userEvent.selectOptions(screen.getByLabelText("Agent"), "aider");
    expect(onChange).toHaveBeenCalledWith("aider");
  });

  it("the select reflects the value prop", () => {
    render(
      <AgentPicker value="aider" onChange={vi.fn()} agents={AGENTS} />,
    );

    expect(screen.getByLabelText("Agent")).toHaveValue("aider");
  });

  it("renders nothing when agents is an empty array (guard)", () => {
    const { container } = render(
      <AgentPicker value="claude" onChange={vi.fn()} agents={[]} />,
    );

    expect(container).toBeEmptyDOMElement();
  });

  it("accepts a custom label prop", () => {
    render(
      <AgentPicker value="claude" onChange={vi.fn()} agents={AGENTS} label="Runtime" />,
    );

    expect(screen.getByText("Runtime")).toBeInTheDocument();
  });

  it("passes disabled prop through to the select", () => {
    render(
      <AgentPicker value="claude" onChange={vi.fn()} agents={AGENTS} disabled />,
    );

    expect(screen.getByLabelText("Agent")).toBeDisabled();
  });

  it("applies an extra className to the root element", () => {
    render(
      <AgentPicker value="claude" onChange={vi.fn()} agents={AGENTS} className="my-extra" />,
    );

    expect(document.querySelector(".agent-picker.my-extra")).toBeInTheDocument();
  });
});

// --------------------------------------------------------------------------
// Tests — uncontrolled (agents not supplied → internal listAgents load)
// --------------------------------------------------------------------------

describe("AgentPicker — uncontrolled (no agents prop, loads internally)", () => {
  it("renders nothing before the internal listAgents call resolves (guard)", () => {
    // Never resolves during this test.
    listAgentsMock.mockImplementation(() => new Promise(() => {}));

    const { container } = render(
      <AgentPicker value="claude" onChange={vi.fn()} />,
    );

    expect(container).toBeEmptyDOMElement();
  });

  it("renders options after listAgents resolves", async () => {
    listAgentsMock.mockResolvedValue(AGENTS);

    render(<AgentPicker value="claude" onChange={vi.fn()} />);

    // Wait for the internal load to settle and options to appear.
    expect(await screen.findByRole("option", { name: "Claude Code" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Aider" })).toBeInTheDocument();
  });

  it("fires onChange via the internally loaded agents list", async () => {
    listAgentsMock.mockResolvedValue(AGENTS);
    const onChange = vi.fn();

    render(<AgentPicker value="claude" onChange={onChange} />);
    await screen.findByRole("option", { name: "Aider" });

    await userEvent.selectOptions(screen.getByLabelText("Agent"), "aider");
    expect(onChange).toHaveBeenCalledWith("aider");
  });
});
