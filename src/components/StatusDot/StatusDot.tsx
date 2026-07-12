import type { TaskStatus } from "../../store";

const COLOR_BY_STATUS: Record<TaskStatus, string> = {
  idle: "#9ca3af",
  working: "#3b82f6",
  needs_attention: "#f59e0b",
  done: "#22c55e",
  error: "#ef4444",
  pending: "#a855f7",
};

export interface StatusDotProps {
  status: TaskStatus;
}

export function StatusDot({ status }: StatusDotProps) {
  return (
    <span
      className="status-dot"
      role="status"
      aria-label={`status: ${status}`}
      style={{
        display: "inline-block",
        width: 8,
        height: 8,
        borderRadius: "50%",
        backgroundColor: COLOR_BY_STATUS[status],
      }}
    />
  );
}
