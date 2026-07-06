import { describe, expect, it } from "vitest";
import { composePrompt, wrapBracketedPaste } from "./comments";
import type { Comment } from "./comments";

const mk = (over: Partial<Comment>): Comment => ({
  id: "c1", filePath: "src/foo.ts", changeKey: "I42", side: "new",
  line: 42, lineText: "if (!x) return;", body: "fix this", ...over,
});

describe("composePrompt", () => {
  it("numbers comments with file:line and body under a header", () => {
    const out = composePrompt([
      mk({ id: "c1", filePath: "src/people/crud.ts", line: 42, body: "rename to fetchActivePeople" }),
      mk({ id: "c2", filePath: "README.md", line: 78, body: "drop the stray test line" }),
    ]);
    expect(out).toBe(
      "Please address these review comments:\n\n" +
      "1. src/people/crud.ts:42 — rename to fetchActivePeople\n" +
      "2. README.md:78 — drop the stray test line",
    );
  });
});

describe("wrapBracketedPaste", () => {
  it("wraps text in bracketed-paste escapes with no trailing carriage return", () => {
    const wrapped = wrapBracketedPaste("hello\nworld");
    expect(wrapped).toBe("\x1b[200~hello\nworld\x1b[201~");
    expect(wrapped.endsWith("\r")).toBe(false);
  });
});
