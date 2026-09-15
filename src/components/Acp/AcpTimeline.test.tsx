import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { AcpTimeline } from "./AcpTimeline";
import type { PlanEntryEvent } from "../../api";
import type { MessageItem, ThoughtItem, ToolCallItem, ErrorItem, TimelineItem } from "../../acp/timeline";

function message(over: Partial<MessageItem> = {}): MessageItem {
  return {
    kind: "message",
    key: "m:1",
    role: "assistant",
    text: "hello there",
    replay: false,
    open: false,
    ...over,
  };
}

function thought(over: Partial<ThoughtItem> = {}): ThoughtItem {
  return {
    kind: "thought",
    key: "t:1",
    text: "thinking it over",
    replay: false,
    open: false,
    ...over,
  };
}

function tool(over: Partial<ToolCallItem> = {}): ToolCallItem {
  return {
    kind: "tool",
    key: "tool:1",
    toolKind: "read",
    title: "read src/main.rs",
    status: "pending",
    content: [],
    ...over,
  };
}

function errorItem(over: Partial<ErrorItem> = {}): ErrorItem {
  return { kind: "error", key: "err:1", message: "session crashed", ...over };
}

describe("AcpTimeline", () => {
  it("renders user and assistant bubbles with distinct classes and text", () => {
    const items: TimelineItem[] = [
      message({ key: "m:user", role: "user", text: "hi agent" }),
      message({ key: "m:assistant", role: "assistant", text: "hi there" }),
    ];
    const { container } = render(<AcpTimeline items={items} plan={null} />);

    const bubbles = container.querySelectorAll(".acp-message");
    expect(bubbles).toHaveLength(2);
    expect(bubbles[0]).toHaveClass("acp-message--user");
    expect(bubbles[1]).toHaveClass("acp-message--assistant");
    expect(screen.getByText("hi agent")).toBeInTheDocument();
    expect(screen.getByText("hi there")).toBeInTheDocument();
  });

  it("marks replay items with the --replay modifier", () => {
    const items: TimelineItem[] = [message({ replay: true })];
    const { container } = render(<AcpTimeline items={items} plan={null} />);
    expect(container.querySelector(".acp-message")).toHaveClass("acp-message--replay");
  });

  it("renders a thought inside a collapsed details element", () => {
    const items: TimelineItem[] = [thought({ text: "pondering" })];
    render(<AcpTimeline items={items} plan={null} />);

    const details = screen.getByText("Thoughts").closest("details");
    expect(details).not.toBeNull();
    expect(details).not.toHaveAttribute("open");
    expect(screen.getByText("pondering")).toBeInTheDocument();
  });

  it("dims a replayed thought via the --replay modifier", () => {
    const items: TimelineItem[] = [thought({ replay: true })];
    const { container } = render(<AcpTimeline items={items} plan={null} />);
    expect(container.querySelector(".acp-thought")).toHaveClass("acp-thought--replay");
  });

  it("shows a tool card with title and status badge modifiers", () => {
    for (const status of ["pending", "in_progress", "completed", "failed"] as const) {
      const items: TimelineItem[] = [tool({ status, title: `tool-${status}` })];
      const { container, unmount } = render(<AcpTimeline items={items} plan={null} />);
      expect(screen.getByText(`tool-${status}`)).toBeInTheDocument();
      const badge = container.querySelector(".acp-tool__status");
      expect(badge).toHaveClass(`acp-tool__status--${status}`);
      unmount();
    }
  });

  it("renders text and diff content blocks inside a collapsed Details section", () => {
    const items: TimelineItem[] = [
      tool({
        content: [
          { type: "content", content: { type: "text", text: "some output" } },
          { type: "diff", path: "src/foo.ts", oldText: "old line", newText: "new line" },
        ],
      }),
    ];
    render(<AcpTimeline items={items} plan={null} />);

    const details = screen.getByText("Details").closest("details");
    expect(details).not.toBeNull();
    expect(details).not.toHaveAttribute("open");

    expect(within(details as HTMLElement).getByText("some output")).toBeInTheDocument();
    expect(within(details as HTMLElement).getByText("src/foo.ts")).toBeInTheDocument();
    expect(within(details as HTMLElement).getByText("old line")).toBeInTheDocument();
    expect(within(details as HTMLElement).getByText("new line")).toBeInTheDocument();
  });

  it("falls back to the generic icon for an unknown toolKind", () => {
    const items: TimelineItem[] = [tool({ toolKind: "mystery" })];
    const { container } = render(<AcpTimeline items={items} plan={null} />);
    const item = container.querySelector('[data-testid="acp-item-tool"]');
    expect(item).not.toBeNull();
    expect(within(item as HTMLElement).getByText("🔧")).toBeInTheDocument();
  });

  it("renders a plan card with per-entry status", () => {
    const plan: PlanEntryEvent[] = [
      { content: "step one", priority: "high", status: "completed" },
      { content: "step two", priority: "medium", status: "in_progress" },
      { content: "step three", priority: "low", status: "pending" },
    ];
    render(<AcpTimeline items={[]} plan={plan} />);

    const planCard = screen.getByTestId("acp-plan");
    expect(within(planCard).getByText("Plan")).toBeInTheDocument();

    const entries = planCard.querySelectorAll(".acp-plan__entry");
    expect(entries).toHaveLength(3);
    expect(entries[0]).toHaveAttribute("data-status", "completed");
    expect(entries[1]).toHaveAttribute("data-status", "in_progress");
    expect(entries[2]).toHaveAttribute("data-status", "pending");
    expect(within(planCard).getByText("step one")).toBeInTheDocument();
  });

  it("omits the plan card when plan is null or empty", () => {
    const { rerender } = render(<AcpTimeline items={[]} plan={null} />);
    expect(screen.queryByTestId("acp-plan")).not.toBeInTheDocument();
    rerender(<AcpTimeline items={[]} plan={[]} />);
    expect(screen.queryByTestId("acp-plan")).not.toBeInTheDocument();
  });

  it("renders an error item as a message banner", () => {
    render(<AcpTimeline items={[errorItem({ message: "boom" })]} plan={null} />);
    expect(screen.getByTestId("acp-item-error")).toHaveTextContent("boom");
  });

  it("renders items in array order via testids", () => {
    const items: TimelineItem[] = [
      message({ key: "m:1", text: "first" }),
      tool({ key: "tool:1" }),
      thought({ key: "t:1" }),
      errorItem({ key: "err:1" }),
    ];
    const { container } = render(<AcpTimeline items={items} plan={null} />);

    const kinds = Array.from(container.querySelectorAll(".acp-timeline > *")).map((el) =>
      el.getAttribute("data-testid"),
    );
    expect(kinds).toEqual(["acp-item-message", "acp-item-tool", "acp-item-thought", "acp-item-error"]);
  });
});
