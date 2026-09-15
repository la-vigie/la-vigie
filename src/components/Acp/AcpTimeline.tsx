import type { PlanEntryEvent } from "../../api";
import type { ThoughtItem, TimelineItem, ToolContentBlock } from "../../acp/timeline";
import { parseToolContent } from "../../acp/timeline";

// Pure presentational renderer for the ACP bubbles surface. Takes
// the reducer output from `../../acp/timeline` and renders it — no store, no
// IPC, so it can be exercised directly in tests.

export interface AcpTimelineProps {
  items: TimelineItem[];
  plan: PlanEntryEvent[] | null;
}

const TOOL_ICONS: Record<string, string> = {
  read: "📖",
  edit: "✏️",
  execute: "⚡",
  fetch: "🌐",
  search: "🔍",
  think: "💭",
};

function toolIcon(kind: string): string {
  return TOOL_ICONS[kind] ?? "🔧";
}

const KNOWN_TOOL_STATUSES = new Set(["pending", "in_progress", "completed", "failed"]);

function toolStatusModifier(status: string): string {
  return KNOWN_TOOL_STATUSES.has(status) ? status : "unknown";
}

function planGlyph(status: string): string {
  switch (status) {
    case "pending":
      return "○";
    case "in_progress":
      return "◐";
    case "completed":
      return "●";
    default:
      return "○";
  }
}

function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max)}…` : text;
}

export function AcpTimeline({ items, plan }: AcpTimelineProps) {
  return (
    <div className="acp-timeline">
      {plan && plan.length > 0 && <PlanCard plan={plan} />}
      {items.map((item) => (
        <TimelineItemView key={item.key} item={item} />
      ))}
    </div>
  );
}

function PlanCard({ plan }: { plan: PlanEntryEvent[] }) {
  return (
    <details className="acp-plan" open data-testid="acp-plan">
      <summary className="acp-plan__summary">Plan</summary>
      <ul className="acp-plan__list">
        {plan.map((entry, i) => (
          <li key={i} className="acp-plan__entry" data-status={entry.status}>
            <span className="acp-plan__glyph" aria-hidden="true">
              {planGlyph(entry.status)}
            </span>
            <span className="acp-plan__content">{entry.content}</span>
          </li>
        ))}
      </ul>
    </details>
  );
}

function TimelineItemView({ item }: { item: TimelineItem }) {
  switch (item.kind) {
    case "message":
      return (
        <div
          className={[
            "acp-message",
            `acp-message--${item.role}`,
            item.replay ? "acp-message--replay" : "",
          ]
            .filter(Boolean)
            .join(" ")}
          data-testid="acp-item-message"
        >
          <div className="acp-message__bubble">{item.text}</div>
        </div>
      );
    case "thought":
      return <ThoughtView item={item} />;
    case "tool":
      return (
        <div className="acp-tool" data-testid="acp-item-tool">
          <div className="acp-tool__head">
            <span className="acp-tool__icon" aria-hidden="true">
              {toolIcon(item.toolKind)}
            </span>
            <span className="acp-tool__title">{item.title}</span>
            <span className={`acp-tool__status acp-tool__status--${toolStatusModifier(item.status)}`}>
              {item.status}
            </span>
          </div>
          {item.content.length > 0 && (
            <details className="acp-tool__details">
              <summary>Details</summary>
              <div className="acp-tool__content">
                {item.content.map((raw, i) => (
                  <ToolContentBlockView key={i} block={parseToolContent(raw)} />
                ))}
              </div>
            </details>
          )}
        </div>
      );
    case "error":
      return (
        <div className="acp-error" data-testid="acp-item-error">
          {item.message}
        </div>
      );
  }
}

function ThoughtView({ item }: { item: ThoughtItem }) {
  return (
    <details
      className={["acp-thought", item.replay ? "acp-thought--replay" : ""].filter(Boolean).join(" ")}
      data-testid="acp-item-thought"
    >
      <summary>Thoughts</summary>
      <div className="acp-thought__body">{item.text}</div>
    </details>
  );
}

function ToolContentBlockView({ block }: { block: ToolContentBlock }) {
  switch (block.type) {
    case "text":
      return <pre className="acp-tool__text">{block.text}</pre>;
    case "diff":
      return (
        <div className="acp-diff">
          <div className="acp-diff__path">{block.path}</div>
          {block.oldText !== null && <pre className="acp-diff__old">{block.oldText}</pre>}
          <pre className="acp-diff__new">{block.newText}</pre>
        </div>
      );
    case "raw":
      return <pre className="acp-tool__raw">{truncate(block.json, 300)}</pre>;
  }
}
