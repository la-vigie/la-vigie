import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { PromptManager } from "./PromptManager";
import { useVigieStore } from "../../store";

beforeEach(() => {
  useVigieStore.setState({
    prompts: [
      { id: "a", label: "Alpha", body: "go a", position: 0 },
      { id: "b", label: "Beta", body: "go b", position: 1 },
    ],
    addPrompt: vi.fn().mockResolvedValue(undefined),
    editPrompt: vi.fn().mockResolvedValue(undefined),
    removePrompt: vi.fn().mockResolvedValue(undefined),
    movePrompt: vi.fn().mockResolvedValue(undefined),
  });
});

describe("PromptManager", () => {
  it("adds a new prompt", () => {
    render(<PromptManager />);
    fireEvent.change(screen.getByLabelText(/new prompt label/i), { target: { value: "Gamma" } });
    fireEvent.change(screen.getByLabelText(/new prompt body/i), { target: { value: "go g" } });
    fireEvent.click(screen.getByRole("button", { name: /add prompt/i }));
    expect(useVigieStore.getState().addPrompt).toHaveBeenCalledWith("Gamma", "go g");
  });

  it("deletes a prompt", () => {
    render(<PromptManager />);
    fireEvent.click(screen.getAllByRole("button", { name: /delete/i })[0]);
    expect(useVigieStore.getState().removePrompt).toHaveBeenCalledWith("a");
  });

  it("moves a prompt down", () => {
    render(<PromptManager />);
    fireEvent.click(screen.getByRole("button", { name: /move alpha down/i }));
    expect(useVigieStore.getState().movePrompt).toHaveBeenCalledWith("a", "down");
  });

  it("persists label edit on blur with current body", () => {
    render(<PromptManager />);
    const labelInput = screen.getByLabelText(/prompt 1 label/i);
    fireEvent.change(labelInput, { target: { value: "Alpha Edited" } });
    fireEvent.blur(labelInput);
    expect(useVigieStore.getState().editPrompt).toHaveBeenCalledWith("a", "Alpha Edited", "go a");
  });
});
