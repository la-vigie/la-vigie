import { useState } from "react";
import "./comments.css";

export interface CommentComposerProps {
  filePath: string;
  line: number;
  initialBody?: string;
  onSave: (body: string) => void;
  onCancel: () => void;
}

export function CommentComposer({ filePath, line, initialBody = "", onSave, onCancel }: CommentComposerProps) {
  const [body, setBody] = useState(initialBody);
  const fileName = filePath.split("/").pop() ?? filePath;
  const save = () => {
    const trimmed = body.trim();
    if (trimmed) onSave(trimmed);
  };
  return (
    <div className="comment-composer">
      <span className="comment-avatar" aria-hidden>M</span>
      <div className="comment-composer__main">
        <textarea
          className="comment-composer__input"
          value={body}
          autoFocus
          placeholder="Leave a comment for the agent…"
          aria-label={`Comment on ${fileName}:${line}`}
          onChange={(e) => setBody(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              save();
            }
          }}
        />
        <div className="comment-composer__footer">
          <span className="comment-composer__hint">{fileName}:{line} · ⌘↵ to save</span>
          <div className="comment-composer__actions">
            <button type="button" className="btn btn--ghost" onClick={onCancel}>Cancel</button>
            <button type="button" className="btn btn--primary" onClick={save} disabled={!body.trim()}>
              Add comment
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
