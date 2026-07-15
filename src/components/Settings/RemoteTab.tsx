import { useEffect, useState } from "react";
import { QRCodeSVG } from "qrcode.react";
import { listRemoteSessions, stopSession, openOrchestrator, type RemoteSession } from "../../api";
import { useVigieStore } from "../../store";

export function RemoteTab() {
  const remote = useVigieStore((s) => s.remote);
  const repos = useVigieStore((s) => s.repos);
  const refreshRemote = useVigieStore((s) => s.refreshRemote);
  const enableRemoteControl = useVigieStore((s) => s.enableRemoteControl);
  const disableRemoteControl = useVigieStore((s) => s.disableRemoteControl);
  const [remoteSessions, setRemoteSessions] = useState<RemoteSession[]>([]);

  useEffect(() => {
    if (!remote.active) {
      setRemoteSessions([]);
      return;
    }
    let alive = true;
    const refresh = () => {
      void listRemoteSessions()
        .then((s) => { if (alive) setRemoteSessions(Array.isArray(s) ? s : []); })
        .catch(() => {});
    };
    refresh();
    const t = setInterval(refresh, 5000);
    return () => { alive = false; clearInterval(t); };
  }, [remote.active]);

  useEffect(() => { void refreshRemote(); }, [refreshRemote]);

  // Disambiguate remote sessions in the list: a per-repo orchestrator is labeled
  // by its repo (name if known, else the raw id); the legacy global concierge
  // keeps its bare kind label (TASK-180).
  const sessionLabel = (s: RemoteSession): string => {
    if (s.kind === "orchestrator") {
      // Orchestrator sessions always carry a repoId (per remote_kind_label); the
      // legacy concierge reports kind:"concierge". Label by repo name when known.
      const name = repos.find((r) => r.id === s.repoId)?.name ?? s.repoId;
      return `orchestrator · ${name}`;
    }
    return s.kind;
  };

  return (
    <section className="settings__section">
      <div className="settings__section-header">
        <h3 className="settings__section-title">Remote access</h3>
      </div>
      <p className="settings__remote-hint">
        Drive La Vigie from another device on your tailnet. Off by default;
        reachable only over Tailscale (never the public internet).
      </p>
      {remote.active ? (
        <div className="settings__remote-active">
          <p>Remote is <strong>active</strong>.</p>
          <label className="settings__label">Pairing token</label>
          <code className="settings__remote-token">{remote.token}</code>
          {remote.url && (
            <p className="settings__remote-hint">Open <code>{remote.url}</code> on your phone.</p>
          )}
          {remote.url && remote.token && (
            <div className="settings__remote-qr">
              <QRCodeSVG
                value={`${remote.url}#token=${remote.token}`}
                size={176}
                marginSize={2}
                aria-label="Pairing QR code"
              />
              <p className="settings__remote-hint">
                Scan with your phone camera to open the remote client already paired — no token
                typing. The token is in the link fragment, so it never reaches the server or its
                logs. <strong>Anyone who can see this screen (or a screenshot of it) can pair</strong>;
                keep it private and disable remote when you're done.
              </p>
            </div>
          )}
          <p className="settings__remote-hint">
            {remote.sleepInhibited ? (
              <>System sleep is <strong>prevented</strong> while remote is active, so the host
              stays reachable. Works on AC power only — on battery (especially with the lid
              closed) the Mac may still sleep.</>
            ) : (
              <>⚠️ Couldn’t prevent system sleep — the host may become unreachable if the Mac
              goes to sleep while idle.</>
            )}
          </p>
          <div className="settings__remote-sessions">
            <label className="settings__label">Remote sessions</label>
            {remoteSessions.length === 0 ? (
              <p className="settings__remote-hint">No remote sessions running.</p>
            ) : (
              <ul className="settings__remote-session-list">
                {remoteSessions.map((s) => (
                  <li key={s.id} className="settings__remote-session">
                    <span>{sessionLabel(s)} · idle {Math.floor(s.idleSecs / 60)}m</span>
                    <button
                      type="button"
                      className="btn btn--danger"
                      onClick={() =>
                        void stopSession(s.id).then(() =>
                          setRemoteSessions((cur) => cur.filter((x) => x.id !== s.id)),
                        )
                      }
                    >
                      Stop
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
          {repos.length > 0 && (
            <div className="settings__remote-orchestrators">
              <label className="settings__label">Orchestrators</label>
              <p className="settings__remote-hint">
                Open a repo-scoped orchestrator session you can drive remotely.
              </p>
              <ul className="settings__remote-session-list">
                {repos.map((r) => (
                  <li key={r.id} className="settings__remote-session">
                    <span>{r.name}</span>
                    <button
                      type="button"
                      className="btn"
                      onClick={() =>
                        void openOrchestrator(r.id).then(() => {
                          void listRemoteSessions()
                            .then((s) => setRemoteSessions(Array.isArray(s) ? s : []))
                            .catch(() => {});
                        })
                      }
                    >
                      Open orchestrator
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          )}
          <button type="button" className="btn btn--danger" onClick={() => void disableRemoteControl()}>
            Disable remote
          </button>
        </div>
      ) : (
        <button type="button" className="btn btn--primary" onClick={() => void enableRemoteControl()}>
          Enable remote
        </button>
      )}
    </section>
  );
}
