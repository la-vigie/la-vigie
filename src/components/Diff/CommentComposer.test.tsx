import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { CommentComposer } from "./CommentComposer";
import { InlineComment } from "./InlineComment";
import type { Comment } from "./comments";

describe("CommentComposer", () => {
  it("saves the trimmed body via button and via Cmd+Enter, and cancels", () => {
    const onSave = vi.fn();
    const onCancel = vi.fn();
    const { rerender } = render(
      <CommentComposer filePath="src/people/crud.ts" line={43} onSave={onSave} onCancel={onCancel} />,
    );

    const box = screen.getByRole("textbox", { name: /crud\.ts:43/i });
    fireEvent.change(box, { target: { value: "  pull the column list into a const  " } });
    fireEvent.click(screen.getByRole("button", { name: /add comment/i }));
    expect(onSave).toHaveBeenCalledWith("pull the column list into a const");

    rerender(<CommentComposer filePath="a.ts" line={1} onSave={onSave} onCancel={onCancel} />);
    const box2 = screen.getByRole("textbox");
    fireEvent.change(box2, { target: { value: "via shortcut" } });
    fireEvent.keyDown(box2, { key: "Enter", metaKey: true });
    expect(onSave).toHaveBeenCalledWith("via shortcut");

    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(onCancel).toHaveBeenCalled();
  });

  it("disables Add comment when empty", () => {
    render(<CommentComposer filePath="a.ts" line={1} onSave={vi.fn()} onCancel={vi.fn()} />);
    expect(screen.getByRole("button", { name: /add comment/i })).toBeDisabled();
  });
});

describe("InlineComment", () => {
  const comment: Comment = {
    id: "c1", filePath: "src/foo.ts", changeKey: "I42", side: "new",
    line: 42, lineText: "x", body: "Good guard. Add a test.",
  };
  it("shows the body, line, and Edit/Delete actions", () => {
    const onEdit = vi.fn();
    const onDelete = vi.fn();
    render(<InlineComment comment={comment} onEdit={onEdit} onDelete={onDelete} />);
    expect(screen.getByText("Good guard. Add a test.")).toBeInTheDocument();
    expect(screen.getByText(/on line 42/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /edit/i }));
    fireEvent.click(screen.getByRole("button", { name: /delete/i }));
    expect(onEdit).toHaveBeenCalled();
    expect(onDelete).toHaveBeenCalled();
  });
});
