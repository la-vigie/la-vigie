import { useState } from "react";
import { composePrompt } from "./comments";
import type { Comment } from "./comments";
import "./comments.css";

export interface ReviewFooterProps {
  comments: Comment[];
  onDiscard: () => void;
  onSubmit: (prompt: string) => Promise<void> | void;
}

export function ReviewFooter({ comments, onDiscard, onSubmit }: ReviewFooterProps) {
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (comments.length === 0) return null;

  // One click sends — no confirm/preview step. Edit individual comments before
  // submitting if you want to tweak the wording.
  const submit = async () => {
    setSending(true);
    setError(null);
    try {
      await onSubmit(composePrompt(comments));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSending(false);
    }
  };

  return (
    <div className="review-footer">
      {error && (
        <p className="review-footer__error" role="alert">
          {error}
        </p>
      )}
      <div className="review-footer__bar">
        <span className="review-footer__count">
          <span className="review-footer__badge">{comments.length}</span>
          comments pending
        </span>
        <span className="review-footer__spacer" />
        <button
          type="button"
          className="btn btn--ghost"
          onClick={onDiscard}
          disabled={sending}
        >
          Discard
        </button>
        <button
          type="button"
          className="btn btn--primary"
          onClick={submit}
          disabled={sending}
        >
          {sending ? "Submitting…" : "Submit to Claude →"}
        </button>
      </div>
    </div>
  );
}
