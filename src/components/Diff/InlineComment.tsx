import type { Comment } from "./comments";
import "./comments.css";

export interface InlineCommentProps {
  comment: Comment;
  onEdit: () => void;
  onDelete: () => void;
}

export function InlineComment({ comment, onEdit, onDelete }: InlineCommentProps) {
  return (
    <div className="inline-comment">
      <span className="comment-avatar" aria-hidden>M</span>
      <div className="inline-comment__main">
        <div className="inline-comment__head">
          <span className="inline-comment__author">You</span>
          <span className="inline-comment__loc">on line {comment.line}</span>
          <span className="inline-comment__actions">
            <button type="button" className="inline-comment__action" onClick={onEdit}>Edit</button>
            <button type="button" className="inline-comment__action" onClick={onDelete}>Delete</button>
          </span>
        </div>
        <p className="inline-comment__body">{comment.body}</p>
      </div>
    </div>
  );
}
