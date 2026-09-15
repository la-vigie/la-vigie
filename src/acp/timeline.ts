import type {
  AcpEvent,
  CostEvent,
  MessageRole,
  PermissionOptionEvent,
  PlanEntryEvent,
  RateLimitEvent,
} from "../api";

// Pure timeline model for the ACP bubbles surface. The store owns an
// `AcpTimelineState` per task and folds every `AcpEvent` from the backend
// Channel through `reduceAcpEvent`; components only ever render this state.
// Keeping the reducer pure (no store, no IPC) makes chunk assembly, tool-call
// merging and permission lifecycle unit-testable — the repo's testable-core
// convention.

export interface MessageItem {
  kind: "message";
  key: string;
  role: MessageRole;
  text: string;
  replay: boolean;
  /** Chunks may still be appended (closed by `turnEnded`). */
  open: boolean;
  /** True for the optimistic bubble added locally on a composer send (see
   *  `appendLocalUserMessage`). Provenance only — the live-echo dedupe matches
   *  any preceding user bubble by text, not this flag. */
  local?: boolean;
}

export interface ThoughtItem {
  kind: "thought";
  key: string;
  text: string;
  replay: boolean;
  open: boolean;
}

export interface ToolCallItem {
  kind: "tool";
  key: string;
  /** ACP tool-call kind (read/edit/execute/fetch/…), styled per kind. */
  toolKind: string;
  title: string;
  status: string;
  content: unknown[];
}

export interface ErrorItem {
  kind: "error";
  key: string;
  message: string;
}

export type TimelineItem = MessageItem | ThoughtItem | ToolCallItem | ErrorItem;

export interface AcpMode {
  id: string;
  name: string;
}

export interface AcpModes {
  currentModeId: string | null;
  availableModes: AcpMode[];
}

export interface PendingPermission {
  requestId: string;
  options: PermissionOptionEvent[];
  toolCall: unknown;
}

export interface AcpUsage {
  used: number;
  size: number;
  cost: CostEvent | null;
  /** Best-effort rate-limit snapshot, when the agent reports one. */
  rateLimit: RateLimitEvent | null;
}

export interface AcpTimelineState {
  sessionId: string | null;
  items: TimelineItem[];
  plan: PlanEntryEvent[] | null;
  modes: AcpModes | null;
  permission: PendingPermission | null;
  usage: AcpUsage | null;
  /** The session's active model id (from `modelSelected`), null until the agent
   *  reports a model selector. */
  currentModel: string | null;
  turnActive: boolean;
  exited: boolean;
  /** Monotonic counter used to mint keys for items without a protocol id. */
  seq: number;
}

export function emptyTimeline(): AcpTimelineState {
  return {
    sessionId: null,
    items: [],
    plan: null,
    modes: null,
    permission: null,
    usage: null,
    currentModel: null,
    turnActive: false,
    exited: false,
    seq: 0,
  };
}

// ── Defensive parsers ────────────────────────────────────────────────────────
// `sessionStarted.modes` and tool-call `content` are `unknown` in the frozen
// IPC contract on purpose (the Rust side forwards raw ACP JSON). Parse them
// leniently: unexpected shapes degrade to null/fallback, never throw.

/** Parse the ACP `SessionModeState` shape:
 *  `{ currentModeId, availableModes: [{id, name}] }`. */
export function parseModes(raw: unknown): AcpModes | null {
  if (typeof raw !== "object" || raw === null) return null;
  const obj = raw as Record<string, unknown>;
  const available = Array.isArray(obj.availableModes) ? obj.availableModes : [];
  const modes: AcpMode[] = [];
  for (const m of available) {
    if (typeof m === "object" && m !== null) {
      const mo = m as Record<string, unknown>;
      if (typeof mo.id === "string") {
        modes.push({ id: mo.id, name: typeof mo.name === "string" ? mo.name : mo.id });
      }
    }
  }
  const current = typeof obj.currentModeId === "string" ? obj.currentModeId : null;
  if (current === null && modes.length === 0) return null;
  return { currentModeId: current, availableModes: modes };
}

/** One renderable block of a tool call's `content` array. */
export type ToolContentBlock =
  | { type: "text"; text: string }
  | { type: "diff"; path: string; oldText: string | null; newText: string }
  | { type: "raw"; json: string };

/** Parse one ACP `ToolCallContent` entry (text / diff / anything else → raw). */
export function parseToolContent(raw: unknown): ToolContentBlock {
  if (typeof raw === "object" && raw !== null) {
    const obj = raw as Record<string, unknown>;
    if (obj.type === "content" && typeof obj.content === "object" && obj.content !== null) {
      const inner = obj.content as Record<string, unknown>;
      if (inner.type === "text" && typeof inner.text === "string") {
        return { type: "text", text: inner.text };
      }
    }
    if (obj.type === "diff" && typeof obj.path === "string" && typeof obj.newText === "string") {
      return {
        type: "diff",
        path: obj.path,
        oldText: typeof obj.oldText === "string" ? obj.oldText : null,
        newText: obj.newText,
      };
    }
  }
  return { type: "raw", json: JSON.stringify(raw) };
}

/** Best-effort human title for the tool call a permission request is about
 *  (`permissionRequest.toolCall` is raw ACP JSON). */
export function permissionToolTitle(toolCall: unknown): string | null {
  if (typeof toolCall === "object" && toolCall !== null) {
    const t = (toolCall as Record<string, unknown>).title;
    if (typeof t === "string" && t.length > 0) return t;
  }
  return null;
}

// ── Reducer ──────────────────────────────────────────────────────────────────

function closeOpenItems(items: TimelineItem[]): TimelineItem[] {
  if (!items.some((i) => (i.kind === "message" || i.kind === "thought") && i.open)) return items;
  return items.map((i) =>
    (i.kind === "message" || i.kind === "thought") && i.open ? { ...i, open: false } : i,
  );
}

/** Find the index of the open bubble a null-id chunk should append to: the
 *  LAST item overall, only if it is an open item of the wanted shape — a
 *  chunk stream interrupted by a tool call or another role starts a new
 *  bubble, mirroring how agents interleave output. */
function openTailIndex(
  items: TimelineItem[],
  kind: "message" | "thought",
  role?: MessageRole,
): number {
  const last = items[items.length - 1];
  if (!last) return -1;
  if (last.kind !== kind || !("open" in last) || !last.open) return -1;
  if (kind === "message" && (last as MessageItem).role !== role) return -1;
  return items.length - 1;
}

function appendChunk(
  state: AcpTimelineState,
  kind: "message" | "thought",
  text: string,
  messageId: string | null,
  replay: boolean,
  role?: MessageRole,
): AcpTimelineState {
  const items = [...state.items];
  let seq = state.seq;

  // Dedupe the live echo of the user bubble we already show. Two paths produce
  // that bubble: an optimistic composer send (`local: true`, added by
  // `appendLocalUserMessage`) and the backend-emitted kickoff bubble (`local`
  // unset, `messageId: null`). Agents that echo the prompt back as a live user
  // chunk — claude-acp does not, mistral-acp does — would otherwise render a
  // second identical bubble. Skip a non-replay user chunk whose text exactly
  // matches the nearest preceding user bubble, regardless of `local` or of
  // whether the echo carries a `messageId`.
  if (kind === "message" && role === "user" && !replay) {
    const lastUser = [...items].reverse().find((i) => i.kind === "message" && i.role === "user");
    if (lastUser && (lastUser as MessageItem).text === text) {
      return { ...state, turnActive: true };
    }
  }

  if (messageId !== null) {
    const prefix = kind === "message" ? "m:" : "t:";
    const key = prefix + messageId + (kind === "message" ? `:${role}` : "");
    const idx = items.findIndex((i) => i.kind === kind && i.key === key);
    if (idx >= 0) {
      const item = items[idx] as MessageItem | ThoughtItem;
      items[idx] = { ...item, text: item.text + text };
    } else if (kind === "message") {
      items.push({ kind, key, role: role as MessageRole, text, replay, open: true });
    } else {
      items.push({ kind, key, text, replay, open: true });
    }
  } else {
    const idx = openTailIndex(items, kind, role);
    if (idx >= 0) {
      const item = items[idx] as MessageItem | ThoughtItem;
      items[idx] = { ...item, text: item.text + text };
    } else {
      const key = `${kind === "message" ? "m" : "t"}:auto-${seq++}`;
      if (kind === "message") {
        items.push({ kind, key, role: role as MessageRole, text, replay, open: true });
      } else {
        items.push({ kind, key, text, replay, open: true });
      }
    }
  }
  // Replayed history must not resurrect the "turn running" affordances.
  return { ...state, items, seq, turnActive: replay ? state.turnActive : true };
}

/** Append the optimistic user bubble for a locally-sent prompt. */
export function appendLocalUserMessage(state: AcpTimelineState, text: string): AcpTimelineState {
  const key = `m:local-${state.seq}`;
  return {
    ...state,
    seq: state.seq + 1,
    turnActive: true,
    items: [
      ...closeOpenItems(state.items),
      { kind: "message", key, role: "user", text, replay: false, open: false, local: true },
    ],
  };
}

/** Clear the pending permission request (called once it has been answered). */
export function clearPermission(state: AcpTimelineState): AcpTimelineState {
  return state.permission === null ? state : { ...state, permission: null };
}

export function reduceAcpEvent(state: AcpTimelineState, event: AcpEvent): AcpTimelineState {
  switch (event.type) {
    case "sessionStarted":
      return {
        ...state,
        sessionId: event.sessionId,
        modes: parseModes(event.modes),
        exited: false,
      };
    case "messageChunk":
      return appendChunk(
        state,
        "message",
        event.text,
        event.messageId,
        event.replay === true,
        event.role,
      );
    case "thoughtChunk":
      return appendChunk(state, "thought", event.text, event.messageId, event.replay === true);
    case "toolCall": {
      const key = `tool:${event.id}`;
      const next: ToolCallItem = {
        kind: "tool",
        key,
        toolKind: event.kind,
        title: event.title,
        status: event.status,
        content: event.content,
      };
      const idx = state.items.findIndex((i) => i.kind === "tool" && i.key === key);
      const items =
        idx >= 0
          ? state.items.map((i, n) => (n === idx ? next : i))
          : [...closeOpenItems(state.items), next];
      return { ...state, items, turnActive: true };
    }
    case "toolCallUpdate": {
      const key = `tool:${event.id}`;
      const idx = state.items.findIndex((i) => i.kind === "tool" && i.key === key);
      if (idx < 0) {
        // An update for a call we never saw — degrade to creating it.
        const item: ToolCallItem = {
          kind: "tool",
          key,
          toolKind: event.kind ?? "other",
          title: event.title ?? "(tool call)",
          status: event.status ?? "pending",
          content: event.content ?? [],
        };
        return { ...state, items: [...closeOpenItems(state.items), item], turnActive: true };
      }
      const prev = state.items[idx] as ToolCallItem;
      const merged: ToolCallItem = {
        ...prev,
        toolKind: event.kind ?? prev.toolKind,
        title: event.title ?? prev.title,
        status: event.status ?? prev.status,
        content: event.content ?? prev.content,
      };
      return {
        ...state,
        items: state.items.map((i, n) => (n === idx ? merged : i)),
        turnActive: true,
      };
    }
    case "plan":
      return { ...state, plan: event.entries, turnActive: true };
    case "permissionRequest":
      return {
        ...state,
        permission: {
          requestId: event.requestId,
          options: event.options,
          toolCall: event.toolCall,
        },
      };
    case "modeChanged":
      return {
        ...state,
        modes: state.modes
          ? { ...state.modes, currentModeId: event.modeId }
          : { currentModeId: event.modeId, availableModes: [] },
      };
    case "modelSelected":
      return { ...state, currentModel: event.modelId };
    case "usage":
      return {
        ...state,
        usage: {
          used: event.used,
          size: event.size,
          cost: event.cost,
          rateLimit: event.rateLimit ?? null,
        },
      };
    case "turnEnded":
      return {
        ...state,
        items: closeOpenItems(state.items),
        turnActive: false,
        permission: null,
      };
    case "error":
      return {
        ...state,
        items: [
          ...closeOpenItems(state.items),
          { kind: "error", key: `err-${state.seq}`, message: event.message },
        ],
        seq: state.seq + 1,
        turnActive: false,
      };
    case "exit":
      return { ...state, exited: true, turnActive: false, permission: null };
  }
}
