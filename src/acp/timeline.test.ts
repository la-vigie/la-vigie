import { describe, expect, it } from "vitest";
import {
  appendLocalUserMessage,
  clearPermission,
  emptyTimeline,
  parseModes,
  parseToolContent,
  permissionToolTitle,
  reduceAcpEvent,
  type MessageItem,
  type ThoughtItem,
  type ToolCallItem,
} from "./timeline";

describe("reduceAcpEvent — chunk assembly", () => {
  it("concatenates messageChunks with the same messageId+role into one item; a different messageId starts a new item", () => {
    let state = emptyTimeline();
    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "assistant",
      text: "Hel",
      messageId: "1",
    });
    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "assistant",
      text: "lo",
      messageId: "1",
    });
    expect(state.items).toHaveLength(1);
    expect((state.items[0] as MessageItem).text).toBe("Hello");

    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "assistant",
      text: "New message",
      messageId: "2",
    });
    expect(state.items).toHaveLength(2);
    expect((state.items[1] as MessageItem).text).toBe("New message");
  });

  it("appends null-id chunks to the open tail only of the same role; a toolCall breaks the run and starts a new bubble", () => {
    let state = emptyTimeline();
    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "assistant",
      text: "A",
      messageId: null,
    });
    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "assistant",
      text: "B",
      messageId: null,
    });
    expect(state.items).toHaveLength(1);
    expect((state.items[0] as MessageItem).text).toBe("AB");
    expect((state.items[0] as MessageItem).open).toBe(true);

    state = reduceAcpEvent(state, {
      type: "toolCall",
      id: "t1",
      kind: "execute",
      title: "Run",
      status: "pending",
      content: [],
      rawInput: null,
    });
    expect(state.items).toHaveLength(2);
    expect((state.items[0] as MessageItem).open).toBe(false);

    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "assistant",
      text: "C",
      messageId: null,
    });
    expect(state.items).toHaveLength(3);
    expect((state.items[2] as MessageItem).text).toBe("C");
  });

  it("a role switch on a null-id chunk starts a new bubble instead of merging", () => {
    let state = emptyTimeline();
    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "assistant",
      text: "A",
      messageId: null,
    });
    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "user",
      text: "B",
      messageId: null,
    });
    expect(state.items).toHaveLength(2);
    expect((state.items[0] as MessageItem).role).toBe("assistant");
    expect((state.items[0] as MessageItem).text).toBe("A");
    expect((state.items[1] as MessageItem).role).toBe("user");
    expect((state.items[1] as MessageItem).text).toBe("B");
  });

  it("thoughtChunk assembles in its own key space, never merging into a message with the same messageId", () => {
    let state = emptyTimeline();
    state = reduceAcpEvent(state, { type: "thoughtChunk", text: "thinking", messageId: "x" });
    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "assistant",
      text: "reply",
      messageId: "x",
    });
    expect(state.items).toHaveLength(2);
    expect(state.items[0].kind).toBe("thought");
    expect((state.items[0] as ThoughtItem).text).toBe("thinking");
    expect(state.items[1].kind).toBe("message");
    expect((state.items[1] as MessageItem).text).toBe("reply");
  });

  it("replay chunks assemble the same way, mark the item replay:true, and leave turnActive alone; a live chunk sets turnActive", () => {
    let state = emptyTimeline();
    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "assistant",
      text: "Old ",
      messageId: "r1",
      replay: true,
    });
    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "assistant",
      text: "reply",
      messageId: "r1",
      replay: true,
    });
    expect(state.items).toHaveLength(1);
    expect((state.items[0] as MessageItem).text).toBe("Old reply");
    expect((state.items[0] as MessageItem).replay).toBe(true);
    expect(state.turnActive).toBe(false);

    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "assistant",
      text: "!",
      messageId: "r2",
      replay: false,
    });
    expect(state.turnActive).toBe(true);
    expect((state.items[1] as MessageItem).replay).toBe(false);
  });

  it("turnEnded closes open items, clears turnActive/permission; a null-id chunk after it starts a new bubble", () => {
    let state = emptyTimeline();
    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "assistant",
      text: "A",
      messageId: null,
    });
    state = reduceAcpEvent(state, {
      type: "permissionRequest",
      requestId: "p1",
      options: [],
      toolCall: null,
    });
    expect(state.permission).not.toBeNull();

    state = reduceAcpEvent(state, { type: "turnEnded", stopReason: "end_turn" });
    expect(state.turnActive).toBe(false);
    expect(state.permission).toBeNull();
    expect((state.items[0] as MessageItem).open).toBe(false);

    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "assistant",
      text: "B",
      messageId: null,
    });
    expect(state.items).toHaveLength(2);
    expect((state.items[0] as MessageItem).text).toBe("A");
    expect((state.items[1] as MessageItem).text).toBe("B");
    expect((state.items[1] as MessageItem).open).toBe(true);
  });
});

describe("optimistic local user bubble", () => {
  it("appendLocalUserMessage appends a closed, local:true user message and sets turnActive", () => {
    const state = appendLocalUserMessage(emptyTimeline(), "hello");
    expect(state.items).toHaveLength(1);
    const item = state.items[0] as MessageItem;
    expect(item.kind).toBe("message");
    expect(item.role).toBe("user");
    expect(item.text).toBe("hello");
    expect(item.local).toBe(true);
    expect(item.open).toBe(false);
    expect(state.turnActive).toBe(true);
  });

  it("dedupes an exact live echo of the local bubble's text, but not a different text", () => {
    const state = appendLocalUserMessage(emptyTimeline(), "hello");

    const echoed = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "user",
      text: "hello",
      messageId: null,
    });
    expect(echoed.items).toHaveLength(1);
    expect((echoed.items[0] as MessageItem).text).toBe("hello");
    expect(echoed.items).toBe(state.items); // untouched — no new item, no growth

    const different = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "user",
      text: "hello world",
      messageId: null,
    });
    expect(different.items).toHaveLength(2);
    expect((different.items[1] as MessageItem).text).toBe("hello world");
  });

  it("dedup walks back to the nearest preceding user bubble, not merely the last item", () => {
    // Local user bubble, then an (unrelated, open) assistant message — the
    // dedup check still walks back to find the preceding user bubble rather
    // than only looking at the last item in the list.
    let state = appendLocalUserMessage(emptyTimeline(), "hello");
    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "assistant",
      text: "thinking...",
      messageId: null,
    });
    expect(state.items).toHaveLength(2);

    const echoed = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "user",
      text: "hello",
      messageId: null,
    });
    // Skipped: no third item was added for the echo.
    expect(echoed.items).toHaveLength(2);
  });

  it("dedupes a live echo of the backend kickoff bubble even though it is not local (TASK-250)", () => {
    // The kickoff prompt arrives as a backend-emitted user chunk (messageId
    // null, `local` unset — see drive_connection in acp/mod.rs), which creates a
    // plain non-local user bubble. Agents that live-echo the prompt (mistral-acp)
    // then send an identical user chunk; it must not render a second bubble.
    const kickoff = reduceAcpEvent(emptyTimeline(), {
      type: "messageChunk",
      role: "user",
      text: "do the thing",
      messageId: null,
    });
    expect(kickoff.items).toHaveLength(1);
    expect((kickoff.items[0] as MessageItem).local).toBeUndefined();

    const echoed = reduceAcpEvent(kickoff, {
      type: "messageChunk",
      role: "user",
      text: "do the thing",
      messageId: null,
    });
    expect(echoed.items).toHaveLength(1);
    expect(echoed.items).toBe(kickoff.items); // untouched — echo dropped
    expect(echoed.turnActive).toBe(true);
  });

  it("dedupes a live echo of the kickoff bubble even when the agent keys it with a messageId (TASK-250)", () => {
    // The dedupe must not depend on the echo being null-keyed: an echoing agent
    // may attach its own messageId, which would otherwise take the keyed-append
    // path and mint a second bubble.
    const kickoff = reduceAcpEvent(emptyTimeline(), {
      type: "messageChunk",
      role: "user",
      text: "do the thing",
      messageId: null,
    });

    const echoed = reduceAcpEvent(kickoff, {
      type: "messageChunk",
      role: "user",
      text: "do the thing",
      messageId: "msg_echo_1",
    });
    expect(echoed.items).toHaveLength(1);
    expect(echoed.items).toBe(kickoff.items);
  });

  it("does not dedupe a replayed user chunk that repeats the last user text", () => {
    // Replayed history (session resume) must render verbatim; the dedupe only
    // targets live echoes (`replay !== true`). Close the first bubble via
    // `turnEnded` so the second chunk can't merely be appended to an open tail.
    let state = reduceAcpEvent(emptyTimeline(), {
      type: "messageChunk",
      role: "user",
      text: "do the thing",
      messageId: null,
    });
    state = reduceAcpEvent(state, { type: "turnEnded", stopReason: "end_turn" });

    const replayed = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "user",
      text: "do the thing",
      messageId: null,
      replay: true,
    });
    expect(replayed.items).toHaveLength(2);
  });
});

describe("tool calls", () => {
  it("toolCall appends a tool item and closes open bubbles; a repeat id replaces in place", () => {
    let state = emptyTimeline();
    state = reduceAcpEvent(state, {
      type: "messageChunk",
      role: "assistant",
      text: "thinking",
      messageId: null,
    });
    state = reduceAcpEvent(state, {
      type: "toolCall",
      id: "t1",
      kind: "read",
      title: "Read file",
      status: "pending",
      content: [],
      rawInput: null,
    });
    expect(state.items).toHaveLength(2);
    expect((state.items[0] as MessageItem).open).toBe(false);
    expect(state.items[1]).toMatchObject({ kind: "tool", key: "tool:t1", status: "pending" });

    state = reduceAcpEvent(state, {
      type: "toolCall",
      id: "t1",
      kind: "read",
      title: "Read file",
      status: "completed",
      content: [{ type: "text", text: "done" }],
      rawInput: null,
    });
    expect(state.items).toHaveLength(2); // replaced, not duplicated
    const tool = state.items[1] as ToolCallItem;
    expect(tool.status).toBe("completed");
    expect(tool.content).toEqual([{ type: "text", text: "done" }]);
  });

  it("toolCallUpdate merges only non-null fields, preserving previous values for nulls", () => {
    let state = emptyTimeline();
    state = reduceAcpEvent(state, {
      type: "toolCall",
      id: "t1",
      kind: "read",
      title: "T",
      status: "pending",
      content: ["c1"],
      rawInput: null,
    });
    state = reduceAcpEvent(state, {
      type: "toolCallUpdate",
      id: "t1",
      kind: null,
      title: null,
      status: "completed",
      content: null,
    });
    const item = state.items[0] as ToolCallItem;
    expect(item.toolKind).toBe("read"); // preserved (null in update)
    expect(item.title).toBe("T"); // preserved (null in update)
    expect(item.status).toBe("completed"); // overwritten
    expect(item.content).toEqual(["c1"]); // preserved (null in update)
  });

  it("toolCallUpdate for an unknown id creates a degraded card", () => {
    const state = reduceAcpEvent(emptyTimeline(), {
      type: "toolCallUpdate",
      id: "unknown",
      kind: null,
      title: null,
      status: null,
      content: null,
    });
    expect(state.items).toHaveLength(1);
    const item = state.items[0] as ToolCallItem;
    expect(item.kind).toBe("tool");
    expect(item.toolKind).toBe("other");
    expect(item.title).toBe("(tool call)");
    expect(item.status).toBe("pending");
    expect(item.content).toEqual([]);
  });
});

describe("session / plan / permission / mode / usage / lifecycle", () => {
  describe("parseModes", () => {
    it("parses a valid SessionModeState shape, falling back to id when name is missing", () => {
      const modes = parseModes({
        currentModeId: "a",
        availableModes: [
          { id: "a", name: "Alpha" },
          { id: "b" },
        ],
      });
      expect(modes).toEqual({
        currentModeId: "a",
        availableModes: [
          { id: "a", name: "Alpha" },
          { id: "b", name: "b" },
        ],
      });
    });

    it("returns null for garbage input", () => {
      expect(parseModes("nope")).toBeNull();
      expect(parseModes(null)).toBeNull();
      expect(parseModes(42)).toBeNull();
      expect(parseModes({})).toBeNull();
    });

    it("yields a state for empty availableModes with a string currentModeId", () => {
      expect(parseModes({ currentModeId: "x", availableModes: [] })).toEqual({
        currentModeId: "x",
        availableModes: [],
      });
    });
  });

  it("sessionStarted sets sessionId and parses modes", () => {
    const state = reduceAcpEvent(emptyTimeline(), {
      type: "sessionStarted",
      sessionId: "s1",
      modes: { currentModeId: "a", availableModes: [{ id: "a", name: "Alpha" }] },
      models: null,
    });
    expect(state.sessionId).toBe("s1");
    expect(state.modes).toEqual({
      currentModeId: "a",
      availableModes: [{ id: "a", name: "Alpha" }],
    });
    expect(state.exited).toBe(false);
  });

  it("plan replaces entries wholesale — latest wins", () => {
    let state = reduceAcpEvent(emptyTimeline(), {
      type: "plan",
      entries: [{ content: "a", priority: "high", status: "pending" }],
    });
    expect(state.plan).toEqual([{ content: "a", priority: "high", status: "pending" }]);

    state = reduceAcpEvent(state, {
      type: "plan",
      entries: [{ content: "b", priority: "low", status: "completed" }],
    });
    expect(state.plan).toEqual([{ content: "b", priority: "low", status: "completed" }]);
  });

  it("permissionRequest stores requestId/options/toolCall; clearPermission and exit both clear it", () => {
    const withPerm = reduceAcpEvent(emptyTimeline(), {
      type: "permissionRequest",
      requestId: "p1",
      options: [{ optionId: "o1", name: "Allow", kind: "allow" }],
      toolCall: { title: "Edit file" },
    });
    expect(withPerm.permission).toEqual({
      requestId: "p1",
      options: [{ optionId: "o1", name: "Allow", kind: "allow" }],
      toolCall: { title: "Edit file" },
    });

    expect(clearPermission(withPerm).permission).toBeNull();

    const exited = reduceAcpEvent(withPerm, { type: "exit", code: 0 });
    expect(exited.permission).toBeNull();
    expect(exited.exited).toBe(true);
    expect(exited.turnActive).toBe(false);
  });

  it("modeChanged updates currentModeId while preserving availableModes", () => {
    const withModes = reduceAcpEvent(emptyTimeline(), {
      type: "sessionStarted",
      sessionId: "s1",
      modes: {
        currentModeId: "a",
        availableModes: [
          { id: "a", name: "Alpha" },
          { id: "b", name: "Beta" },
        ],
      },
      models: null,
    });
    const changed = reduceAcpEvent(withModes, { type: "modeChanged", modeId: "b" });
    expect(changed.modes).toEqual({
      currentModeId: "b",
      availableModes: [
        { id: "a", name: "Alpha" },
        { id: "b", name: "Beta" },
      ],
    });
  });

  it("modeChanged with no prior modes creates a fresh state with empty availableModes", () => {
    const state = reduceAcpEvent(emptyTimeline(), { type: "modeChanged", modeId: "x" });
    expect(state.modes).toEqual({ currentModeId: "x", availableModes: [] });
  });

  it("usage replaces the usage snapshot wholesale, including cost:null", () => {
    let state = reduceAcpEvent(emptyTimeline(), {
      type: "usage",
      used: 100,
      size: 1000,
      cost: { amount: 0.5, currency: "USD" },
    });
    expect(state.usage).toEqual({
      used: 100,
      size: 1000,
      cost: { amount: 0.5, currency: "USD" },
      rateLimit: null,
    });

    state = reduceAcpEvent(state, { type: "usage", used: 200, size: 1000, cost: null });
    expect(state.usage).toEqual({ used: 200, size: 1000, cost: null, rateLimit: null });
  });

  it("usage carries a rate-limit snapshot when present", () => {
    const state = reduceAcpEvent(emptyTimeline(), {
      type: "usage",
      used: 100,
      size: 1000,
      cost: null,
      rateLimit: { status: "allowed", resetAt: 123, limitType: "five_hour", usingOverage: false },
    });
    expect(state.usage?.rateLimit).toEqual({
      status: "allowed",
      resetAt: 123,
      limitType: "five_hour",
      usingOverage: false,
    });
  });

  it("modelSelected tracks the active model id", () => {
    let state = reduceAcpEvent(emptyTimeline(), { type: "modelSelected", modelId: "opus-4" });
    expect(state.currentModel).toBe("opus-4");
    // A later switch replaces it.
    state = reduceAcpEvent(state, { type: "modelSelected", modelId: "sonnet-4" });
    expect(state.currentModel).toBe("sonnet-4");
  });

  it("error appends an error item and clears turnActive", () => {
    const state = reduceAcpEvent(
      { ...emptyTimeline(), turnActive: true },
      { type: "error", message: "boom" },
    );
    expect(state.turnActive).toBe(false);
    expect(state.items).toHaveLength(1);
    expect(state.items[0]).toMatchObject({ kind: "error", message: "boom" });
  });

  it("exit sets exited:true and clears turnActive", () => {
    const state = reduceAcpEvent(
      { ...emptyTimeline(), turnActive: true },
      { type: "exit", code: null },
    );
    expect(state.exited).toBe(true);
    expect(state.turnActive).toBe(false);
  });
});

describe("parseToolContent", () => {
  it("parses a text content block", () => {
    expect(parseToolContent({ type: "content", content: { type: "text", text: "hi" } })).toEqual({
      type: "text",
      text: "hi",
    });
  });

  it("parses a diff block", () => {
    expect(
      parseToolContent({ type: "diff", path: "a.ts", oldText: "old", newText: "new" }),
    ).toEqual({ type: "diff", path: "a.ts", oldText: "old", newText: "new" });
  });

  it("a diff block missing oldText yields oldText: null", () => {
    expect(parseToolContent({ type: "diff", path: "a.ts", newText: "new" })).toEqual({
      type: "diff",
      path: "a.ts",
      oldText: null,
      newText: "new",
    });
  });

  it("anything else falls back to raw with the stringified input", () => {
    const input = { foo: "bar" };
    expect(parseToolContent(input)).toEqual({ type: "raw", json: JSON.stringify(input) });
    expect(parseToolContent("plain string")).toEqual({
      type: "raw",
      json: JSON.stringify("plain string"),
    });
    expect(parseToolContent(null)).toEqual({ type: "raw", json: JSON.stringify(null) });
  });
});

describe("permissionToolTitle", () => {
  it("returns the title for an object with a non-empty string title", () => {
    expect(permissionToolTitle({ title: "Edit file" })).toBe("Edit file");
  });

  it("returns null for empty string, missing title, or non-object input", () => {
    expect(permissionToolTitle({ title: "" })).toBeNull();
    expect(permissionToolTitle({})).toBeNull();
    expect(permissionToolTitle("nope")).toBeNull();
    expect(permissionToolTitle(null)).toBeNull();
    expect(permissionToolTitle(42)).toBeNull();
  });
});

describe("purity", () => {
  function deepFreeze<T>(obj: T): T {
    if (obj !== null && typeof obj === "object" && !Object.isFrozen(obj)) {
      for (const name of Object.getOwnPropertyNames(obj)) {
        const value = (obj as Record<string, unknown>)[name];
        if (value !== null && typeof value === "object") {
          deepFreeze(value);
        }
      }
      Object.freeze(obj);
    }
    return obj;
  }

  it("reduceAcpEvent never mutates its input state", () => {
    let seed = emptyTimeline();
    seed = reduceAcpEvent(seed, {
      type: "messageChunk",
      role: "assistant",
      text: "A",
      messageId: "1",
    });
    seed = reduceAcpEvent(seed, {
      type: "toolCall",
      id: "t1",
      kind: "read",
      title: "T",
      status: "pending",
      content: [],
      rawInput: null,
    });
    seed = reduceAcpEvent(seed, {
      type: "permissionRequest",
      requestId: "p1",
      options: [],
      toolCall: null,
    });

    const frozen = deepFreeze(structuredClone(seed));
    const before = structuredClone(frozen);

    expect(() => {
      reduceAcpEvent(frozen, {
        type: "messageChunk",
        role: "assistant",
        text: "B",
        messageId: "1",
      });
      reduceAcpEvent(frozen, {
        type: "toolCallUpdate",
        id: "t1",
        kind: null,
        title: null,
        status: "completed",
        content: null,
      });
      reduceAcpEvent(frozen, { type: "turnEnded", stopReason: "end_turn" });
      reduceAcpEvent(frozen, { type: "error", message: "oops" });
      reduceAcpEvent(frozen, { type: "exit", code: 0 });
      appendLocalUserMessage(frozen, "hi");
      clearPermission(frozen);
    }).not.toThrow();

    expect(frozen).toEqual(before);
  });
});
