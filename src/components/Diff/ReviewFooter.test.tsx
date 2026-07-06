import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ReviewFooter } from "./ReviewFooter";
import type { Comment } from "./comments";

const comments: Comment[] = [
  { id: "c1", filePath: "src/people/crud.ts", changeKey: "I42", side: "new", line: 42, lineText: "x", body: "rename" },
  { id: "c2", filePath: "README.md", changeKey: "I78", side: "new", line: 78, lineText: "y", body: "drop test line" },
];

describe("ReviewFooter", () => {
  it("renders nothing when there are no comments", () => {
    const { container } = render(<ReviewFooter comments={[]} onDiscard={vi.fn()} onSubmit={vi.fn()} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("shows the pending count and fires Discard", () => {
    const onDiscard = vi.fn();
    render(<ReviewFooter comments={comments} onDiscard={onDiscard} onSubmit={vi.fn()} />);
    expect(screen.getByText(/comments pending/i)).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /discard/i }));
    expect(onDiscard).toHaveBeenCalled();
  });

  it("submits the composed prompt in one click — no preview/confirm step", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(<ReviewFooter comments={comments} onDiscard={vi.fn()} onSubmit={onSubmit} />);

    // There is exactly one Submit-to-Claude control (no second one in a preview).
    const submitButtons = screen.getAllByRole("button", { name: /submit to claude/i });
    expect(submitButtons).toHaveLength(1);

    fireEvent.click(submitButtons[0]);

    // No preview textbox appears; the composed prompt is sent directly.
    expect(screen.queryByRole("textbox", { name: /prompt preview/i })).not.toBeInTheDocument();
    await waitFor(() =>
      expect(onSubmit).toHaveBeenCalledWith(
        "Please address these review comments:\n\n" +
          "1. src/people/crud.ts:42 — rename\n" +
          "2. README.md:78 — drop test line",
      ),
    );
  });

  it("surfaces an error and keeps the footer when onSubmit rejects", async () => {
    const onSubmit = vi.fn().mockRejectedValue(new Error("agent unavailable"));
    render(<ReviewFooter comments={comments} onDiscard={vi.fn()} onSubmit={onSubmit} />);

    fireEvent.click(screen.getByRole("button", { name: /submit to claude/i }));

    expect(await screen.findByRole("alert")).toHaveTextContent("agent unavailable");
    expect(screen.getByText(/comments pending/i)).toBeInTheDocument();
  });
});
