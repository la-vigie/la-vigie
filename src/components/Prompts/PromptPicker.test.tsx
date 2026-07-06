import { describe, it, expect, vi, beforeEach } from "vitest";
import { useRef, useState } from "react";
import { render, screen, fireEvent } from "@testing-library/react";
import { PromptPicker } from "./PromptPicker";
import { useVigieStore } from "../../store";

beforeEach(() => {
  useVigieStore.setState({
    prompts: [
      { id: "a", label: "No brainstorm, go", body: "Get started, no brainstorm.", position: 0 },
      { id: "b", label: "Write tests first", body: "Use TDD.", position: 1 },
    ],
  });
});

describe("PromptPicker", () => {
  it("opens and inserts the selected prompt's body", () => {
    const onSelect = vi.fn();
    render(<PromptPicker onSelect={onSelect} onManage={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /library/i }));
    fireEvent.click(screen.getByText("No brainstorm, go"));
    expect(onSelect).toHaveBeenCalledWith("Get started, no brainstorm.");
  });

  it("invokes onManage from the manage item", () => {
    const onManage = vi.fn();
    render(<PromptPicker onSelect={() => {}} onManage={onManage} />);
    fireEvent.click(screen.getByRole("button", { name: /library/i }));
    fireEvent.click(screen.getByText(/manage prompts/i));
    expect(onManage).toHaveBeenCalled();
  });

  it("shows only the manage item when there are no prompts", () => {
    useVigieStore.setState({ prompts: [] });
    render(<PromptPicker onSelect={() => {}} onManage={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /library/i }));
    expect(screen.queryByText("No brainstorm, go")).toBeNull();
    expect(screen.getByText(/manage prompts/i)).toBeTruthy();
  });

  it("closes the menu on Escape", () => {
    render(<PromptPicker onSelect={() => {}} onManage={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /library/i }));
    expect(screen.getByRole("menu")).toBeTruthy();
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape", code: "Escape", bubbles: true });
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("closes the menu after selecting a prompt", () => {
    render(<PromptPicker onSelect={() => {}} onManage={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /library/i }));
    fireEvent.click(screen.getByText("No brainstorm, go"));
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("closes the menu on an outside click", () => {
    render(<PromptPicker onSelect={() => {}} onManage={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /library/i }));
    expect(screen.getByRole("menu")).toBeTruthy();
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("closes when onSelect updates parent state (new-task scenario)", () => {
    function Wrapper() {
      const [text, setText] = useState("");
      return (
        <>
          <textarea data-testid="ta" value={text} readOnly />
          <PromptPicker onSelect={(b) => setText((t) => t + b)} onManage={() => {}} />
        </>
      );
    }
    render(<Wrapper />);
    fireEvent.click(screen.getByRole("button", { name: /library/i }));
    fireEvent.click(screen.getByText("No brainstorm, go"));
    expect((screen.getByTestId("ta") as HTMLTextAreaElement).value).toBe("Get started, no brainstorm.");
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("closes with an insertPrompt-style onSelect (rAF focus on a ref textarea)", () => {
    function Wrapper() {
      const [text, setText] = useState("");
      const ref = useRef<HTMLTextAreaElement>(null);
      const insert = (body: string) => {
        const el = ref.current;
        const start = el?.selectionStart ?? text.length;
        const end = el?.selectionEnd ?? text.length;
        setText(text.slice(0, start) + body + text.slice(end));
        requestAnimationFrame(() => ref.current?.focus());
      };
      return (
        <>
          <textarea ref={ref} data-testid="ta" value={text} onChange={() => {}} />
          <PromptPicker onSelect={insert} onManage={() => {}} />
        </>
      );
    }
    render(<Wrapper />);
    fireEvent.click(screen.getByRole("button", { name: /library/i }));
    fireEvent.click(screen.getByText("No brainstorm, go"));
    expect(screen.queryByRole("menu")).toBeNull();
  });
});
