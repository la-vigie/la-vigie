import { useEffect, useRef, useState } from "react";
import type { Comment, ComposerAnchor } from "./comments";

let idCounter = 0;
function nextId(): string {
  idCounter += 1;
  return `cmt-${idCounter}`;
}

export interface DiffComments {
  comments: Comment[];
  composer: ComposerAnchor | null;
  editingId: string | null;
  openComposer: (anchor: ComposerAnchor) => void;
  cancelComposer: () => void;
  addComment: (body: string) => void;
  startEdit: (id: string) => void;
  updateComment: (id: string, body: string) => void;
  removeComment: (id: string) => void;
  clear: () => void;
}

export function useDiffComments(taskId: string): DiffComments {
  const [comments, setComments] = useState<Comment[]>([]);
  const [composer, setComposer] = useState<ComposerAnchor | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);

  // Ephemeral: wipe everything when the selected task changes.
  const prevTask = useRef(taskId);
  useEffect(() => {
    if (prevTask.current !== taskId) {
      prevTask.current = taskId;
      setComments([]);
      setComposer(null);
      setEditingId(null);
    }
  }, [taskId]);

  return {
    comments,
    composer,
    editingId,
    openComposer: (anchor) => {
      setEditingId(null);
      setComposer(anchor);
    },
    cancelComposer: () => setComposer(null),
    addComment: (body) => {
      if (!composer) return;
      setComments((prev) => [...prev, { id: nextId(), ...composer, body }]);
      setComposer(null);
    },
    startEdit: (id) => {
      setComposer(null);
      setEditingId(id);
    },
    updateComment: (id, body) => {
      setComments((prev) => prev.map((c) => (c.id === id ? { ...c, body } : c)));
      setEditingId(null);
    },
    removeComment: (id) => setComments((prev) => prev.filter((c) => c.id !== id)),
    clear: () => {
      setComments([]);
      setComposer(null);
      setEditingId(null);
    },
  };
}
