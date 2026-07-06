export type CommentSide = "old" | "new";

export interface Comment {
  id: string;
  filePath: string;
  changeKey: string;
  side: CommentSide;
  line: number;
  lineText: string;
  body: string;
}

// Where an open composer is anchored (carries the file so change keys, which are
// only unique per-file, don't collide across files).
export interface ComposerAnchor {
  filePath: string;
  changeKey: string;
  side: CommentSide;
  line: number;
  lineText: string;
}

export function composePrompt(comments: Comment[]): string {
  const items = comments.map((c, i) => `${i + 1}. ${c.filePath}:${c.line} — ${c.body}`);
  return `Please address these review comments:\n\n${items.join("\n")}`;
}

const PASTE_START = "\x1b[200~";
const PASTE_END = "\x1b[201~";

// Bracketed paste so a multi-line prompt is inserted as one paste and the TUI
// does not submit on the first newline. No trailing \r — the user presses Enter.
export function wrapBracketedPaste(text: string): string {
  return `${PASTE_START}${text}${PASTE_END}`;
}
