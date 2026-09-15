import { useLayoutEffect, useRef, useState } from "react";
import { useVigieStore } from "../../store";
import { emptyTimeline, permissionToolTitle } from "../../acp/timeline";
import { AcpTimeline } from "./AcpTimeline";
import "./Acp.css";

// The ACP bubbles surface: a pure view over the store's per-task
// timeline (`acpByTask`). The live session Channel is owned by the store, so
// this component may mount/unmount freely (task switches, shell tabs) without
// touching the session — the PTY KEEP-ALIVE invariant does not extend to it,
// but it must stay a SIBLING of <TerminalHost/>, never wrap it.

export interface AcpSurfaceProps {
  taskId: string;
}

export function AcpSurface({ taskId }: AcpSurfaceProps) {
  const timeline = useVigieStore((s) => s.acpByTask[taskId]) ?? emptyTimeline();
  const session = useVigieStore((s) =>
    s.sessionsByTask[taskId]?.find((x) => x.kind === "agent"),
  );
  const setTaskError = useVigieStore((s) => s.setTaskError);

  const [draft, setDraft] = useState("");
  const scrollRef = useRef<HTMLDivElement>(null);
  // Follow the stream only while the user is at (or near) the bottom; a manual
  // scroll-up pins the viewport until they return to the bottom themselves.
  const followRef = useRef(true);

  // A resumed session is still "restoring" until its SessionStarted lands
  // (`sessionId` set) — for the session/load path that spans the whole
  // reconnect handshake (agent cold-start + native history replay), during
  // which `status` is already "running" but no history has arrived yet. Gate
  // the composer on it so we show "restoring", not a misleading "ready", and
  // don't accept a prompt mid-reconnect. This always resolves: every establish
  // path (fresh, reconnect, reconnect-fallback) emits SessionStarted on
  // success, and a hard failure emits Exit (backend fail-notify) which tears
  // the session down — so "restoring" never wedges the composer.
  const restoring =
    !!session?.resume && session?.status === "running" && timeline.sessionId === null;
  const ready = session?.status === "running" && !!session.backendId && !restoring;
  const busy = timeline.turnActive;

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el && followRef.current) el.scrollTop = el.scrollHeight;
  }, [timeline.items, timeline.plan, timeline.permission]);

  const handleScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    followRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
  };

  const surfaceError = (err: unknown) =>
    setTaskError(taskId, err instanceof Error ? err.message : String(err));

  const handleSend = () => {
    const text = draft.trim();
    if (!text || !ready) return;
    setDraft("");
    followRef.current = true;
    useVigieStore.getState().sendAcpPrompt(taskId, text).catch(surfaceError);
  };

  const handleCancel = () => {
    useVigieStore.getState().cancelAcpTurn(taskId).catch(surfaceError);
  };

  const handlePermission = (optionId?: string) => {
    if (!timeline.permission) return;
    useVigieStore
      .getState()
      .respondAcpPermission(taskId, timeline.permission.requestId, optionId)
      .catch(surfaceError);
  };

  const handleMode = (modeId: string) => {
    useVigieStore.getState().setAcpMode(taskId, modeId).catch(surfaceError);
  };

  const permissionTitle = timeline.permission
    ? permissionToolTitle(timeline.permission.toolCall)
    : null;

  return (
    <div className="acp-surface" data-testid="acp-surface">
      <div
        className="acp-surface__scroll"
        ref={scrollRef}
        onScroll={handleScroll}
        data-testid="acp-scroll"
      >
        <AcpTimeline items={timeline.items} plan={timeline.plan} />
        {timeline.items.length === 0 && (
          <p className="acp-surface__empty">
            {ready
              ? "Session ready — send a prompt below."
              : restoring
                ? "Restoring session history…"
                : "Starting agent session…"}
          </p>
        )}
      </div>

      {timeline.permission && (
        <div className="acp-permission" role="alertdialog" aria-label="Permission request" data-testid="acp-permission">
          <div className="acp-permission__title">
            Permission requested{permissionTitle ? `: ${permissionTitle}` : ""}
          </div>
          <div className="acp-permission__options">
            {timeline.permission.options.map((o) => (
              <button
                key={o.optionId}
                type="button"
                className={`btn acp-permission__option acp-permission__option--${o.kind}`}
                onClick={() => handlePermission(o.optionId)}
              >
                {o.name}
              </button>
            ))}
            <button
              type="button"
              className="btn btn--ghost acp-permission__option"
              onClick={() => handlePermission(undefined)}
            >
              Dismiss
            </button>
          </div>
        </div>
      )}

      <div className="acp-composer">
        <textarea
          className="acp-composer__input"
          aria-label="Message the agent"
          placeholder={
            ready
              ? "Message the agent — Enter to send, Shift+Enter for a newline"
              : restoring
                ? "Restoring…"
                : "Starting…"
          }
          value={draft}
          disabled={!ready}
          rows={2}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              handleSend();
            }
          }}
        />
        <div className="acp-composer__footer">
          <div className="acp-composer__meta">
            {timeline.modes && timeline.modes.availableModes.length > 0 && (
              <label className="acp-composer__mode">
                <span>Mode</span>
                <select
                  aria-label="Session mode"
                  value={timeline.modes.currentModeId ?? ""}
                  onChange={(e) => handleMode(e.target.value)}
                >
                  {timeline.modes.currentModeId === null && (
                    <option value="" disabled>
                      —
                    </option>
                  )}
                  {timeline.modes.availableModes.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name}
                    </option>
                  ))}
                </select>
              </label>
            )}
            {timeline.usage && timeline.usage.size > 0 && (
              <span className="acp-composer__usage" title="Model · context used / window · cost">
                {timeline.currentModel && `${timeline.currentModel} · `}
                ctx {Math.round((timeline.usage.used / timeline.usage.size) * 100)}%
                {timeline.usage.cost &&
                  ` · ${timeline.usage.cost.amount.toFixed(2)} ${timeline.usage.cost.currency}`}
                {timeline.usage.rateLimit &&
                  timeline.usage.rateLimit.status !== "allowed" &&
                  ` · ⚠ ${timeline.usage.rateLimit.status}`}
              </span>
            )}
          </div>
          <div className="acp-composer__actions">
            {busy && (
              <button type="button" className="btn btn--danger" onClick={handleCancel}>
                Cancel turn
              </button>
            )}
            <button
              type="button"
              className="btn btn--primary"
              disabled={!ready || draft.trim().length === 0}
              onClick={handleSend}
            >
              Send
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
