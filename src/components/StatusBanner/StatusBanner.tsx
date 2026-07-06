import type { ConsoleStatus, Task } from "../../store";
import { useVigieStore } from "../../store";
import "./StatusBanner.css";

export interface BannerSegment {
  key: string;
  node: React.ReactNode;
}

const EMPTY_CONSOLE: ConsoleStatus = {};

// Permission-mode → banner label. `default` (and absent) shows no segment.
const MODE_LABELS: Record<string, string> = {
  auto: "auto mode on",
  plan: "plan mode",
  acceptEdits: "accept edits",
  dontAsk: "don't ask",
  bypassPermissions: "bypass",
};

function CtxMeter({ percent }: { percent: number }) {
  return (
    <span className="status-banner__ctx" title={`${percent}% context left`}>
      <span className="status-banner__meter" aria-hidden>
        <span className="status-banner__meter-fill" style={{ width: `${percent}%` }} />
      </span>
      <span className="status-banner__ctx-label">{percent}% ctx left</span>
    </span>
  );
}

export function buildSegments(task: Task, consoleStatus: ConsoleStatus): BannerSegment[] {
  const segments: BannerSegment[] = [];

  if (consoleStatus.contextRemainingPercent != null) {
    segments.push({ key: "ctx", node: <CtxMeter percent={Math.round(consoleStatus.contextRemainingPercent)} /> });
  }
  if (consoleStatus.model) {
    segments.push({ key: "model", node: <span className="status-banner__mono">{consoleStatus.model}</span> });
  }
  const modeLabel =
    consoleStatus.mode && consoleStatus.mode !== "default"
      ? MODE_LABELS[consoleStatus.mode] ?? consoleStatus.mode
      : undefined;
  if (modeLabel) {
    segments.push({
      key: "mode",
      node: (
        <span className="status-banner__auto">
          <span aria-hidden>⚡</span> {modeLabel}
        </span>
      ),
    });
  }
  if (task.prNumber != null) {
    segments.push({ key: "pr", node: <span className="status-banner__mono">PR #{task.prNumber}</span> });
  }
  return segments;
}

export interface StatusBannerProps {
  task: Task;
}

export function StatusBanner({ task }: StatusBannerProps) {
  const agentBackendId = useVigieStore(
    (s) => s.sessionsByTask[task.id]?.find((x) => x.kind === "agent")?.backendId,
  );
  const consoleStatus = useVigieStore(
    (s) => (agentBackendId ? s.consoleByAgentId[agentBackendId] : undefined),
  );
  const segments = buildSegments(task, consoleStatus ?? EMPTY_CONSOLE);
  return (
    <div className="status-banner" role="status">
      {segments.map((seg, i) => (
        <span className="status-banner__segment" key={seg.key}>
          {i > 0 && <span className="status-banner__dot" aria-hidden>·</span>}
          {seg.node}
        </span>
      ))}
    </div>
  );
}
