import { useCallback, useEffect, useState } from "react";
import {
  createPr,
  ghStatus,
  getPrComments,
  getPrStatus,
  openUrl,
} from "../../api";
import type { GhStatus, PrComment, PrStatus } from "../../api";
import { useVigieStore } from "../../store";
import "./PrPanel.css";

export interface PrPanelProps {
  taskId: string;
  refreshToken?: number;
}

const CHECK_STATUS_COLORS: Record<string, string> = {
  success: "var(--green)",
  failure: "var(--red)",
  pending: "var(--status-attention)",
  neutral: "var(--text-faint)",
};

// GitHub's PR-state palette (case-insensitive): OPEN green, MERGED purple,
// CLOSED red. gh returns these uppercase.
export function prStateColor(state: string): string {
  switch (state.toUpperCase()) {
    case "OPEN":
      return "var(--pr-open)";
    case "MERGED":
      return "var(--pr-merged)";
    case "CLOSED":
      return "var(--pr-closed)";
    default:
      return "var(--text-faint)";
  }
}

function CheckDot({ status }: { status: string }) {
  const color = CHECK_STATUS_COLORS[status] ?? "var(--text-faint)";
  return (
    <span
      className="pr-panel__check-dot"
      aria-label={status}
      style={{ backgroundColor: color }}
    />
  );
}

function CommentKindBadge({ kind, state }: { kind: string; state: string | null }) {
  const label = state ? `${kind} · ${state}` : kind;
  return <span className="pr-panel__comment-badge">{label}</span>;
}

export function PrPanel({ taskId, refreshToken }: PrPanelProps) {
  const tasks = useVigieStore((s) => s.tasks);
  const repos = useVigieStore((s) => s.repos);

  const task = tasks.find((t) => t.id === taskId);
  const repo = repos.find((r) => r.id === task?.repoId);

  const [ghStatusData, setGhStatusData] = useState<GhStatus | null>(null);
  const [prStatus, setPrStatus] = useState<PrStatus | null | undefined>(undefined);
  const [comments, setComments] = useState<PrComment[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Create form state
  const [createTitle, setCreateTitle] = useState(task?.title ?? "");
  const [createBody, setCreateBody] = useState("");
  const [createDraft, setCreateDraft] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const gh = await ghStatus();
      setGhStatusData(gh);

      if (!gh.available || !gh.authenticated) {
        setLoading(false);
        return;
      }

      if (!repo?.remoteUrl) {
        setLoading(false);
        return;
      }

      const status = await getPrStatus(taskId);
      setPrStatus(status);

      if (status) {
        const coms = await getPrComments(taskId);
        setComments(coms);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [taskId, repo?.remoteUrl]);

  useEffect(() => {
    load();
  }, [load]);

  // Re-fetch when the shared Review "Refresh" control is triggered.
  useEffect(() => {
    if (refreshToken) load();
  }, [refreshToken]); // eslint-disable-line react-hooks/exhaustive-deps

  // Prefill create title whenever task.title becomes available and the field is still empty
  useEffect(() => {
    if (task?.title && !createTitle) {
      setCreateTitle(task.title);
    }
  }, [task?.title]); // eslint-disable-line react-hooks/exhaustive-deps

  // Keep create title in sync with task title when transitioning to "no PR" state
  useEffect(() => {
    if (prStatus === null && task?.title) {
      setCreateTitle(task.title);
    }
  }, [prStatus, task?.title]);

  const handleCreate = async () => {
    setCreateError(null);
    setCreating(true);
    try {
      await createPr(taskId, createTitle, createBody, createDraft);
      setCreateBody("");
      setCreateDraft(false);
      await load();
    } catch (e) {
      setCreateError(String(e));
    } finally {
      setCreating(false);
    }
  };

  // ── State 1: gh not available or not authenticated ────────────────────────
  if (ghStatusData !== null && (!ghStatusData.available || !ghStatusData.authenticated)) {
    return (
      <div className="pr-panel pr-panel--notice">
        {!ghStatusData.available ? (
          <p>GitHub CLI not found. Please install <code>gh</code> and try again.</p>
        ) : (
          <p>Not authenticated. Run <code>gh auth login</code> to connect.</p>
        )}
        <button type="button" className="btn" onClick={load} disabled={loading}>
          Refresh
        </button>
      </div>
    );
  }

  // ── State 2: no remote ────────────────────────────────────────────────────
  if (ghStatusData?.available && ghStatusData.authenticated && !repo?.remoteUrl) {
    return (
      <div className="pr-panel pr-panel--notice">
        <p>Add a remote to create a PR.</p>
        <button type="button" className="btn" onClick={load} disabled={loading}>
          Refresh
        </button>
      </div>
    );
  }

  // ── Loading / initial ─────────────────────────────────────────────────────
  if (ghStatusData === null || prStatus === undefined) {
    return (
      <div className="pr-panel pr-panel--notice">
        {loading && <p>Loading…</p>}
        {error && (
          <p role="alert" className="pr-panel__error">
            {error}
          </p>
        )}
      </div>
    );
  }

  // ── State 3: no PR — create form ──────────────────────────────────────────
  if (prStatus === null) {
    return (
      <div className="pr-panel">
        <h3 className="pr-panel__heading">Create Pull Request</h3>
        <div className="pr-panel__field">
          <label htmlFor="pr-title">Title</label>
          <input
            id="pr-title"
            type="text"
            className="field"
            value={createTitle}
            onChange={(e) => setCreateTitle(e.target.value)}
          />
        </div>
        <div className="pr-panel__field">
          <label htmlFor="pr-body">Body</label>
          <textarea
            id="pr-body"
            className="field pr-panel__textarea"
            value={createBody}
            onChange={(e) => setCreateBody(e.target.value)}
            rows={6}
          />
        </div>
        <label className="pr-panel__checkbox-row" htmlFor="pr-draft">
          <input
            id="pr-draft"
            type="checkbox"
            checked={createDraft}
            onChange={(e) => setCreateDraft(e.target.checked)}
          />
          Create as draft
        </label>
        {createError && (
          <p role="alert" className="pr-panel__error">
            {createError}
          </p>
        )}
        {error && (
          <p role="alert" className="pr-panel__error">
            {error}
          </p>
        )}
        <div className="pr-panel__actions">
          <button
            type="button"
            className="btn btn--primary"
            onClick={handleCreate}
            disabled={creating || !createTitle.trim()}
          >
            Create
          </button>
          <button type="button" className="btn" onClick={load} disabled={loading}>
            Refresh
          </button>
        </div>
      </div>
    );
  }

  // ── State 4: PR exists ────────────────────────────────────────────────────
  return (
    <div className="pr-panel">
      {/* Status header */}
      <div className="pr-panel__status-row">
        <span
          className="pr-panel__state"
          style={{ backgroundColor: prStateColor(prStatus.state) }}
        >
          {prStatus.state}
        </span>
        {prStatus.isDraft && <span className="pr-panel__draft">draft</span>}
        <span className="pr-panel__meta">Mergeable: {prStatus.mergeable}</span>
        {prStatus.reviewDecision && (
          <span className="pr-panel__meta">Review: {prStatus.reviewDecision}</span>
        )}
      </div>

      <h3 className="pr-panel__heading">
        #{prStatus.number} {prStatus.title}
      </h3>

      {/* Checks */}
      {prStatus.checks.length > 0 && (
        <div className="pr-panel__section">
          <strong className="pr-panel__section-title">Checks</strong>
          <ul className="pr-panel__checks">
            {prStatus.checks.map((check) => (
              <li key={check.name} className="pr-panel__check">
                <CheckDot status={check.status} />
                <span>{check.name}</span>
                <span className="pr-panel__check-status">{check.status}</span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {/* Actions */}
      <div className="pr-panel__actions">
        <button
          type="button"
          className="btn"
          onClick={() => openUrl(prStatus.url).catch((e) => setError(String(e)))}
        >
          Open in browser
        </button>
        <button type="button" className="btn" onClick={load} disabled={loading}>
          Refresh
        </button>
      </div>

      {error && (
        <p role="alert" className="pr-panel__error">
          {error}
        </p>
      )}

      {/* Comments */}
      {comments.length > 0 && (
        <div className="pr-panel__section">
          <strong className="pr-panel__section-title">Comments</strong>
          <ul className="pr-panel__comments">
            {comments.map((comment) => (
              <li
                key={`${comment.author}|${comment.createdAt}|${comment.kind}|${comment.path ?? ""}`}
                className="pr-panel__comment"
              >
                <div className="pr-panel__comment-head">
                  <strong className="pr-panel__comment-author">{comment.author}</strong>
                  <CommentKindBadge kind={comment.kind} state={comment.state} />
                  <span className="pr-panel__comment-date">
                    {new Date(comment.createdAt).toLocaleString()}
                  </span>
                </div>
                {comment.path && (
                  <span className="pr-panel__comment-path">
                    {comment.line != null
                      ? `${comment.path}:${comment.line}`
                      : comment.path}
                  </span>
                )}
                <p className="pr-panel__comment-body">{comment.body}</p>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
