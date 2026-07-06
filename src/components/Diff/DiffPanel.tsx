import React, { useCallback, useEffect, useMemo, useState } from "react";
import { commitTask, getDiff, getChangedFiles, stageFiles } from "../../api";
import type { DiffScope, FileChange } from "../../api";
import "./DiffPanel.css";
import {
  Decoration, Diff, Hunk, parseDiff, tokenize,
  getChangeKey, computeNewLineNumber, computeOldLineNumber, isDelete,
} from "react-diff-view";
import type { ChangeData } from "react-diff-view";
import { useDiffComments } from "./useDiffComments";
import type { DiffComments } from "./useDiffComments";
import { useCollapsedFiles } from "./useCollapsedFiles";
import type { CollapsedFiles } from "./useCollapsedFiles";
import { displayPath, fileChangeLabel } from "./diffHeader";
import type { ComposerAnchor } from "./comments";
import { CommentComposer } from "./CommentComposer";
import { InlineComment } from "./InlineComment";
import { ReviewFooter } from "./ReviewFooter";
import { sendToAgent } from "./sendToAgent";
import refractor from "refractor/core";
import typescript from "refractor/lang/typescript";
import tsx from "refractor/lang/tsx";
import javascript from "refractor/lang/javascript";
import jsx from "refractor/lang/jsx";
import json from "refractor/lang/json";
import css from "refractor/lang/css";
import python from "refractor/lang/python";
import rust from "refractor/lang/rust";
import go from "refractor/lang/go";
import bash from "refractor/lang/bash";
import markdown from "refractor/lang/markdown";
import yaml from "refractor/lang/yaml";
import "react-diff-view/style/index.css";
import "./diff-dark.css";

// Register all languages once (module-level, safe to call multiple times)
refractor.register(typescript);
refractor.register(tsx);
refractor.register(javascript);
refractor.register(jsx);
refractor.register(json);
refractor.register(css);
refractor.register(python);
refractor.register(rust);
refractor.register(go);
refractor.register(bash);
refractor.register(markdown);
refractor.register(yaml);

const EXT_TO_LANG: Record<string, string> = {
  ts: "typescript",
  tsx: "tsx",
  js: "javascript",
  jsx: "jsx",
  json: "json",
  css: "css",
  py: "python",
  rs: "rust",
  go: "go",
  sh: "bash",
  bash: "bash",
  md: "markdown",
  yaml: "yaml",
  yml: "yaml",
};

function getLang(fileName: string): string {
  const ext = fileName.split(".").pop() ?? "";
  return EXT_TO_LANG[ext] ?? "";
}

function GitHubDiffRenderer({
  diffText,
  comments,
  collapse,
}: {
  diffText: string;
  comments?: DiffComments;
  collapse: CollapsedFiles;
}) {
  if (!diffText) return null;
  const files = parseDiff(diffText);
  if (files.length === 0) return null;

  const allPaths = files.map((f) => displayPath(f));
  const allCollapsed = allPaths.length > 0 && allPaths.every((p) => collapse.isCollapsed(p));

  return (
    <div className="diff-wrapper">
      <div className="diff-panel__diff-toolbar">
        <button
          type="button"
          className="diff-panel__collapse-all"
          onClick={() =>
            allCollapsed ? collapse.expandAll() : collapse.collapseAll(allPaths)
          }
        >
          {allCollapsed ? "Expand all" : "Collapse all"}
        </button>
      </div>
      {files.map((file) => {
        const filePath = displayPath(file);
        const collapsed = collapse.isCollapsed(filePath);
        const label = fileChangeLabel(file.type);

        // Per-file +/- counts, from the hunks we already parsed.
        let additions = 0;
        let deletions = 0;
        for (const hunk of file.hunks) {
          for (const change of hunk.changes) {
            if (change.type === "insert") additions += 1;
            else if (change.type === "delete") deletions += 1;
          }
        }

        const header = (
          <button
            type="button"
            className="diff-panel__diff-file-header"
            aria-expanded={!collapsed}
            onClick={() => collapse.toggle(filePath)}
          >
            <span className="diff-panel__diff-file-caret" aria-hidden="true">
              {collapsed ? "▶" : "▼"}
            </span>
            <span className="diff-panel__diff-file-path">{filePath}</span>
            {label && (
              <span
                className={
                  "diff-panel__diff-file-badge diff-panel__diff-file-badge--" + file.type
                }
              >
                {label}
              </span>
            )}
            {(additions > 0 || deletions > 0) && (
              <span className="diff-panel__diff-file-counts">
                <span className="diff-panel__count-add">+{additions}</span>
                <span className="diff-panel__count-del">−{deletions}</span>
              </span>
            )}
          </button>
        );

        if (collapsed) {
          return (
            <div
              key={filePath}
              className="diff-panel__diff-file diff-panel__diff-file--collapsed"
            >
              {header}
            </div>
          );
        }

        const lang = getLang(filePath);
        const tokens = (() => {
          if (!lang) return undefined;
          try {
            return tokenize(file.hunks, { highlight: true, refractor, language: lang });
          } catch {
            return undefined;
          }
        })();

        // Build the per-file widgets map (composer + saved comments) keyed by
        // change key (unique within a file).
        const widgets: Record<string, React.ReactNode> = {};
        if (comments) {
          for (const c of comments.comments) {
            if (c.filePath !== filePath) continue;
            widgets[c.changeKey] =
              comments.editingId === c.id ? (
                <CommentComposer
                  filePath={c.filePath}
                  line={c.line}
                  initialBody={c.body}
                  onSave={(body) => comments.updateComment(c.id, body)}
                  onCancel={() => comments.cancelComposer()}
                />
              ) : (
                <InlineComment
                  comment={c}
                  onEdit={() => comments.startEdit(c.id)}
                  onDelete={() => comments.removeComment(c.id)}
                />
              );
          }
          const composer = comments.composer;
          if (composer && composer.filePath === filePath && !widgets[composer.changeKey]) {
            widgets[composer.changeKey] = (
              <CommentComposer
                filePath={composer.filePath}
                line={composer.line}
                onSave={(body) => comments.addComment(body)}
                onCancel={() => comments.cancelComposer()}
              />
            );
          }
        }

        const codeEvents = comments
          ? {
              onClick: (args: { change: ChangeData | null }) => {
                const change = args.change;
                if (!change) return;
                const anchor: ComposerAnchor = {
                  filePath,
                  changeKey: getChangeKey(change),
                  side: isDelete(change) ? "old" : "new",
                  line: isDelete(change)
                    ? computeOldLineNumber(change)
                    : computeNewLineNumber(change),
                  lineText: change.content,
                };
                comments.openComposer(anchor);
              },
            }
          : undefined;

        return (
          <div key={filePath} className="diff-panel__diff-file">
            {header}
            <Diff
              viewType="unified"
              diffType={file.type}
              hunks={file.hunks}
              tokens={tokens}
              widgets={widgets}
              codeEvents={codeEvents}
            >
              {(hunks) =>
                hunks.flatMap((hunk) => [
                  <Decoration key={`deco-${hunk.content}`}>
                    <span className="diff-decoration-content">{hunk.content}</span>
                  </Decoration>,
                  <Hunk key={hunk.content} hunk={hunk} />,
                ])
              }
            </Diff>
          </div>
        );
      })}
    </div>
  );
}

interface CheckedFile extends FileChange {
  checked: boolean;
}

interface LineCounts {
  additions: number;
  deletions: number;
}

const CHANGE_LABEL: Record<string, string> = {
  added: "added",
  modified: "modified",
  deleted: "deleted",
  renamed: "renamed",
  copied: "copied",
  type_changed: "type changed",
  unknown: "changed",
};

// Per-file +/- line counts, derived from the unified diff we already fetch.
function countsByPath(diffText: string): Record<string, LineCounts> {
  if (!diffText) return {};
  const result: Record<string, LineCounts> = {};
  try {
    for (const file of parseDiff(diffText)) {
      const path = displayPath(file);
      const counts: LineCounts = { additions: 0, deletions: 0 };
      for (const hunk of file.hunks) {
        for (const change of hunk.changes) {
          if (change.type === "insert") counts.additions += 1;
          else if (change.type === "delete") counts.deletions += 1;
        }
      }
      result[path] = counts;
    }
  } catch {
    return result;
  }
  return result;
}

export interface DiffPanelProps {
  taskId: string;
  refreshToken?: number;
  scope?: DiffScope;
  /** Read-only review (no file selection / commit) — used by the "base" scope. */
  readOnly?: boolean;
  /** Enable inline commenting + Submit-to-Claude (Overall/base scope only). */
  commentable?: boolean;
}

export function DiffPanel({
  taskId,
  refreshToken,
  scope = "uncommitted",
  readOnly = false,
  commentable = false,
}: DiffPanelProps) {
  const comments = useDiffComments(taskId);
  const collapse = useCollapsedFiles(taskId);
  const [diffText, setDiffText] = useState<string>("");
  const [files, setFiles] = useState<CheckedFile[]>([]);
  const [commitMessage, setCommitMessage] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const [, setLoading] = useState<boolean>(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [diff, changed] = await Promise.all([
        getDiff(taskId, scope),
        getChangedFiles(taskId, scope),
      ]);
      setDiffText(diff);
      setFiles(changed.map((f) => ({ ...f, checked: true })));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [taskId, scope]);

  useEffect(() => {
    refresh();
  }, [refresh, refreshToken]);

  const counts = useMemo(() => countsByPath(diffText), [diffText]);

  const handleToggleFile = (path: string) => {
    setFiles((prev) =>
      prev.map((f) => (f.path === path ? { ...f, checked: !f.checked } : f)),
    );
  };

  const handleStageAndCommit = async () => {
    const checkedPaths = files.filter((f) => f.checked).map((f) => f.path);
    if (checkedPaths.length === 0 || !commitMessage.trim()) return;

    try {
      await stageFiles(taskId, checkedPaths);
      await commitTask(taskId, commitMessage);
      setCommitMessage("");
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const checkedCount = files.filter((f) => f.checked).length;
  const canCommit = commitMessage.trim().length > 0 && checkedCount > 0;
  const isEmpty = files.length === 0 && !diffText;

  return (
    <div className="diff-panel">
      {error && (
        <div className="diff-panel__error" role="alert">
          {error}
        </div>
      )}

      <div className="diff-panel__scroll">
        {isEmpty ? (
          <div className="diff-panel__empty">
            {readOnly ? "No changes vs base" : "No uncommitted changes"}
          </div>
        ) : (
          <>
            <ul className="diff-panel__file-list" aria-label="changed files">
              {files.map((file) => {
                const c = counts[file.path];
                return (
                  <li key={file.path} className="diff-panel__file-item">
                    <label className="diff-panel__file-label">
                      {!readOnly && (
                        <input
                          type="checkbox"
                          checked={file.checked}
                          onChange={() => handleToggleFile(file.path)}
                          className="diff-panel__file-checkbox"
                        />
                      )}
                      <span className="diff-panel__file-path">{file.path}</span>
                      <span
                        className={
                          "diff-panel__file-change-badge diff-panel__file-change-badge--" +
                          file.change
                        }
                      >
                        {CHANGE_LABEL[file.change] ?? file.change}
                      </span>
                      {c && (c.additions > 0 || c.deletions > 0) && (
                        <span className="diff-panel__file-counts">
                          <span className="diff-panel__count-add">+{c.additions}</span>
                          <span className="diff-panel__count-del">−{c.deletions}</span>
                        </span>
                      )}
                    </label>
                  </li>
                );
              })}
            </ul>

            <GitHubDiffRenderer
              diffText={diffText}
              comments={commentable ? comments : undefined}
              collapse={collapse}
            />
          </>
        )}
      </div>

      {!readOnly && (
        <div className="diff-panel__commit-box">
          <input
            type="text"
            className="diff-panel__commit-message field"
            placeholder="Commit message"
            value={commitMessage}
            onChange={(e) => setCommitMessage(e.target.value)}
          />
          <button
            type="button"
            className="btn btn--primary diff-panel__stage-commit-button"
            onClick={handleStageAndCommit}
            disabled={!canCommit}
          >
            Commit
          </button>
        </div>
      )}

      {commentable && (
        <ReviewFooter
          comments={comments.comments}
          onDiscard={comments.clear}
          onSubmit={async (prompt) => {
            await sendToAgent(taskId, prompt);
            comments.clear();
          }}
        />
      )}
    </div>
  );
}
