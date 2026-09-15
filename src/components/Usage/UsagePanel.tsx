import { useCallback, useEffect, useState } from "react";
import { getAcpUsageSummary, type AcpUsageSummary } from "../../api";
import "./UsagePanel.css";

interface UsagePanelProps {
  onClose: () => void;
}

const WINDOWS: { days: number; label: string }[] = [
  { days: 1, label: "24h" },
  { days: 7, label: "7 days" },
  { days: 30, label: "30 days" },
  { days: 90, label: "90 days" },
];

function money(cost: number, currency: string | null): string {
  return `${cost.toFixed(2)}${currency ? ` ${currency}` : ""}`;
}

function contextLabel(peak: number, size: number): string {
  if (size <= 0) return peak > 0 ? peak.toLocaleString() : "—";
  return `${Math.round((peak / size) * 100)}% (${peak.toLocaleString()})`;
}

/**
 * The ACP per-model usage / cost / rate-limit panel — "what did this window of
 * agents cost, by model?". Reads persisted usage via `get_acp_usage_summary`
 * (the pure session-collapse + grouping). A global overlay opened from the
 * title bar; rendered at the header level like SettingsModal, so it never wraps
 * a live terminal/ACP host (KEEP-ALIVE).
 *
 * Reflects ACP-backend sessions only — the PTY/hooks path emits no usage.
 */
export function UsagePanel({ onClose }: UsagePanelProps) {
  const [windowDays, setWindowDays] = useState(7);
  const [summary, setSummary] = useState<AcpUsageSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(async (days: number) => {
    setLoading(true);
    setError(null);
    try {
      setSummary(await getAcpUsageSummary(days));
    } catch (e) {
      setError(String(e));
      setSummary(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh(windowDays);
  }, [refresh, windowDays]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const empty = summary !== null && summary.byModel.length === 0;

  return (
    <div className="usage__backdrop" role="presentation" onClick={onClose}>
      <div
        className="usage"
        role="dialog"
        aria-label="Usage & cost"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="usage__header">
          <h2 className="usage__title">Usage &amp; cost</h2>
          <div className="usage__windows" role="tablist" aria-label="Time window">
            {WINDOWS.map(({ days, label }) => (
              <button
                key={days}
                type="button"
                role="tab"
                aria-selected={windowDays === days}
                className={`usage__window${windowDays === days ? " usage__window--active" : ""}`}
                onClick={() => setWindowDays(days)}
              >
                {label}
              </button>
            ))}
          </div>
          <button type="button" className="icon-btn usage__close" aria-label="Close" onClick={onClose}>
            ✕
          </button>
        </header>

        <div className="usage__body">
          {error && <p className="usage__error">Couldn’t load usage: {error}</p>}
          {loading && summary === null && <p className="usage__muted">Loading…</p>}

          {summary && (
            <>
              <div className="usage__summary">
                <span className="usage__total">{money(summary.totalCost, summary.currency)}</span>
                <span className="usage__muted">
                  across {summary.sessions} ACP session{summary.sessions === 1 ? "" : "s"}
                </span>
              </div>

              {empty ? (
                <p className="usage__muted usage__empty">
                  No ACP usage recorded in this window. Usage &amp; cost are captured for ACP-backend
                  agents (e.g. claude-acp, mistral-acp) — run one and its spend shows up here.
                </p>
              ) : (
                <>
                  <h3 className="usage__section">By model</h3>
                  <table className="usage__table">
                    <thead>
                      <tr>
                        <th>Provider</th>
                        <th>Model</th>
                        <th className="usage__num">Sessions</th>
                        <th className="usage__num">Context peak</th>
                        <th className="usage__num">Cost</th>
                        <th>Rate limit</th>
                      </tr>
                    </thead>
                    <tbody>
                      {summary.byModel.map((m, i) => (
                        <tr key={`${m.provider}:${m.model ?? ""}:${i}`}>
                          <td>{m.provider}</td>
                          <td>{m.model ?? <span className="usage__muted">—</span>}</td>
                          <td className="usage__num">{m.sessions}</td>
                          <td className="usage__num">{contextLabel(m.contextPeak, m.contextSize)}</td>
                          <td className="usage__num">{money(m.cost, m.currency)}</td>
                          <td>
                            {m.rateStatus ? (
                              <span
                                className={`usage__rate usage__rate--${
                                  m.rateStatus === "allowed" ? "ok" : "warn"
                                }`}
                              >
                                {m.rateStatus}
                              </span>
                            ) : (
                              <span className="usage__muted">—</span>
                            )}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>

                  <h3 className="usage__section">By task</h3>
                  <table className="usage__table">
                    <thead>
                      <tr>
                        <th>Task</th>
                        <th className="usage__num">Cost</th>
                      </tr>
                    </thead>
                    <tbody>
                      {summary.byTask.map((t) => (
                        <tr key={t.taskId}>
                          <td>{t.taskId}</td>
                          <td className="usage__num">{money(t.cost, t.currency)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
