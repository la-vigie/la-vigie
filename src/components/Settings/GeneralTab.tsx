import { useVigieStore } from "../../store";

export function GeneralTab() {
  const fetchRemoteBase = useVigieStore((s) => s.fetchRemoteBase);
  const setFetchRemoteBase = useVigieStore((s) => s.setFetchRemoteBase);
  const injectLavigieSkills = useVigieStore((s) => s.injectLavigieSkills);
  const setInjectLavigieSkills = useVigieStore((s) => s.setInjectLavigieSkills);

  return (
    <>
      <section className="settings__section">
        <div className="settings__section-header">
          <h3 className="settings__section-title">New worktrees</h3>
        </div>
        <label className="settings__master">
          <input
            type="checkbox"
            role="switch"
            checked={fetchRemoteBase}
            aria-label="Base new worktrees on the latest remote base branch"
            onChange={(e) => setFetchRemoteBase(e.target.checked)}
          />
          <span>Base new worktrees on the latest remote base branch (git fetch first)</span>
        </label>
      </section>

      <section className="settings__section">
        <div className="settings__section-header">
          <h3 className="settings__section-title">Agent skills</h3>
        </div>
        <label className="settings__master">
          <input
            type="checkbox"
            role="switch"
            checked={injectLavigieSkills}
            aria-label="Load La Vigie's default skills into launched agents"
            onChange={(e) => setInjectLavigieSkills(e.target.checked)}
          />
          <span>
            Load La Vigie's default skills into launched agents
            (adds <code>/lavigie:rename</code>, <code>/lavigie:finished</code>,
            <code>/lavigie:spec-init</code>, <code>/lavigie:verify-claims</code>,
            <code>/lavigie:await-merge</code>)
          </span>
        </label>
      </section>
    </>
  );
}
