import { StrictMode } from "react";
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useDiffComments } from "./useDiffComments";
import type { ComposerAnchor } from "./comments";

const anchor: ComposerAnchor = {
  filePath: "src/foo.ts", changeKey: "I42", side: "new", line: 42, lineText: "x",
};

describe("useDiffComments", () => {
  it("opens a composer, adds a comment from it, and closes the composer", () => {
    const { result } = renderHook(({ id }) => useDiffComments(id), {
      initialProps: { id: "task-1" },
    });

    act(() => result.current.openComposer(anchor));
    expect(result.current.composer).toEqual(anchor);

    act(() => result.current.addComment("rename this"));
    expect(result.current.composer).toBeNull();
    expect(result.current.comments).toHaveLength(1);
    expect(result.current.comments[0]).toMatchObject({
      filePath: "src/foo.ts", changeKey: "I42", line: 42, body: "rename this",
    });
    expect(result.current.comments[0].id).toBeTruthy();
  });

  it("edits and removes comments", () => {
    const { result } = renderHook(() => useDiffComments("task-1"));
    act(() => result.current.openComposer(anchor));
    act(() => result.current.addComment("first"));
    const id = result.current.comments[0].id;

    act(() => result.current.updateComment(id, "second"));
    expect(result.current.comments[0].body).toBe("second");

    act(() => result.current.removeComment(id));
    expect(result.current.comments).toHaveLength(0);
  });

  it("adds exactly one comment under StrictMode (no double-fire)", () => {
    const { result } = renderHook(() => useDiffComments("task-1"), { wrapper: StrictMode });
    act(() => result.current.openComposer(anchor));
    act(() => result.current.addComment("once"));
    expect(result.current.comments).toHaveLength(1);
  });

  it("clears all ephemeral state when the taskId changes", () => {
    const { result, rerender } = renderHook(({ id }) => useDiffComments(id), {
      initialProps: { id: "task-1" },
    });
    act(() => result.current.openComposer(anchor));
    act(() => result.current.addComment("note"));
    expect(result.current.comments).toHaveLength(1);

    rerender({ id: "task-2" });
    expect(result.current.comments).toHaveLength(0);
    expect(result.current.composer).toBeNull();
  });
});
