import { useEffect, useState } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { listTaskDocs, openUrl, readTaskDoc } from "../../api";
import type { DocRef } from "../../api";
import "./SpecDock.css";

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}

// Open markdown links in the user's browser via the opener plugin instead of
// letting them navigate the app's webview away (mirrors PrDock/PrPanel). Only
// absolute http(s) links are opened; relative links are inert so a stray
// `docs/...` href can't replace the SPA.
export function handleDocLinkClick(
  e: { preventDefault: () => void },
  href: string | undefined,
) {
  e.preventDefault();
  if (href && /^https?:\/\//i.test(href)) {
    openUrl(href).catch(() => {});
  }
}

const markdownComponents: Components = {
  a({ href, children }) {
    return (
      <a href={href} onClick={(e) => handleDocLinkClick(e, href)}>
        {children}
      </a>
    );
  },
};

export interface SpecDockProps {
  taskId: string;
  refreshToken?: number;
  /** When true the dock fills the whole review pane (terminal collapsed). The
   *  state lives in TaskDetail because maximizing also resizes the body split;
   *  the dock just renders the affordance and the filled layout. */
  maximized?: boolean;
  onToggleMaximize?: () => void;
}

/**
 * "Option D" sibling of PrDock — a collapsible bottom dock that shows the task's
 * spec/design/plan markdown read-only. Collapsed it is a one-line bar; expanded
 * it shows a doc picker (when >1) and the rendered markdown, drag-resizable via
 * the top grip. It is purely additive inside ReviewPanel and never touches the
 * terminal area, so the KEEP-ALIVE invariant is unaffected.
 */
export function SpecDock({ taskId, refreshToken, maximized = false, onToggleMaximize }: SpecDockProps) {
  const [collapsed, setCollapsed] = useState<boolean>(
    () => localStorage.getItem("vigie.specDockCollapsed") !== "false",
  );
  const [height, setHeight] = useState<number>(
    () => Number(localStorage.getItem("vigie.specDockHeight")) || 240,
  );
  const [docs, setDocs] = useState<DocRef[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [content, setContent] = useState<string>("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    localStorage.setItem("vigie.specDockCollapsed", String(collapsed));
  }, [collapsed]);

  useEffect(() => {
    localStorage.setItem("vigie.specDockHeight", String(height));
  }, [height]);

  // Resolve the docs for this task; keep the current selection if it survives,
  // else fall back to the first doc. Failures fall back to an empty list.
  useEffect(() => {
    let cancelled = false;
    listTaskDocs(taskId)
      .then((list) => {
        if (cancelled) return;
        const arr = list ?? [];
        setDocs(arr);
        setActiveId((prev) =>
          prev && arr.some((d) => d.id === prev) ? prev : (arr[0]?.id ?? null),
        );
      })
      .catch(() => {
        if (!cancelled) {
          setDocs([]);
          setActiveId(null);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [taskId, refreshToken]);

  // Load the active doc's markdown.
  useEffect(() => {
    if (!activeId) {
      setContent("");
      setError(null);
      return;
    }
    let cancelled = false;
    readTaskDoc(taskId, activeId)
      .then((md) => {
        if (!cancelled) {
          setContent(md);
          setError(null);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setContent("");
          setError(String(e));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [taskId, activeId, refreshToken]);

  const handleGripMouseDown = (e: React.MouseEvent) => {
    const startY = e.clientY;
    const startH = height;
    const onMove = (ev: MouseEvent) => {
      setHeight(clamp(startH + (startY - ev.clientY), 120, 560));
    };
    const onUp = () => {
      document.body.style.userSelect = "";
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    document.body.style.userSelect = "none";
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  // ── Collapsed: one-line bar ───────────────────────────────────────────────
  // Maximized forces the expanded view (a collapsed-but-maximized dock makes no
  // sense — the terminal is already hidden behind it).
  if (collapsed && !maximized) {
    return (
      <button
        type="button"
        className="spec-dock__bar"
        onClick={() => setCollapsed(false)}
        aria-expanded={false}
        aria-label="Expand spec and docs dock"
      >
        <span className="spec-dock__bar-title">Spec / Docs</span>
        {docs.length > 0 ? (
          <span className="spec-dock__count">{docs.length}</span>
        ) : (
          <span className="spec-dock__muted">none</span>
        )}
        <span className="spec-dock__spacer" />
        <span className="spec-dock__chevron" aria-hidden>⌃</span>
      </button>
    );
  }

  // ── Expanded: grip + header + rendered markdown ───────────────────────────
  // Maximized fills the review pane (flex:1), so the fixed height and the
  // resize grip are dropped — resizing a full-pane dock is meaningless.
  return (
    <section
      className={"spec-dock" + (maximized ? " spec-dock--max" : "")}
      style={maximized ? undefined : { height }}
    >
      {!maximized && (
        <div
          className="spec-dock__grip"
          role="separator"
          aria-orientation="horizontal"
          aria-label="Resize spec and docs dock"
          onMouseDown={handleGripMouseDown}
        >
          <span className="spec-dock__grip-handle" aria-hidden />
        </div>
      )}
      <div className="spec-dock__header">
        <span className="spec-dock__bar-title">Spec / Docs</span>
        {docs.length > 1 && (
          <select
            className="spec-dock__picker"
            aria-label="Select document"
            value={activeId ?? ""}
            onChange={(e) => setActiveId(e.target.value)}
          >
            {docs.map((d) => (
              <option key={d.id} value={d.id}>
                {d.label}
              </option>
            ))}
          </select>
        )}
        {docs.length === 1 && (
          <span className="spec-dock__single">{docs[0].label}</span>
        )}
        <span className="spec-dock__spacer" />
        {onToggleMaximize && (
          <button
            type="button"
            className="icon-btn"
            aria-label={maximized ? "Restore spec and docs dock" : "Maximize spec and docs dock"}
            title={maximized ? "Restore split" : "Fill side panel"}
            onClick={onToggleMaximize}
          >
            {maximized ? "⤡" : "⤢"}
          </button>
        )}
        {!maximized && (
          <button
            type="button"
            className="icon-btn"
            aria-label="Collapse spec and docs dock"
            onClick={() => setCollapsed(true)}
          >
            ⌄
          </button>
        )}
      </div>
      <div className="spec-dock__body">
        {docs.length === 0 ? (
          <p className="spec-dock__empty">No spec or docs found for this task.</p>
        ) : error ? (
          <p className="spec-dock__empty">{error}</p>
        ) : content ? (
          <div className="spec-dock__md">
            <ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>
              {content}
            </ReactMarkdown>
          </div>
        ) : null}
      </div>
    </section>
  );
}
