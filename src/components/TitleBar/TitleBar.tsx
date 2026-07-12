import { useVigieStore } from "../../store";
import { SettingsModal } from "../Settings/SettingsModal";
import "./TitleBar.css";

/**
 * Custom overlay title bar (matches the La Vigie mockup): the native macOS
 * traffic lights sit at the far left over this dark bar (Tauri
 * `titleBarStyle: Overlay`), so we reserve space for them and make the bar
 * draggable. Contents: brand mark, breadcrumb, and the theme toggle.
 */
export function TitleBar() {
  const repos = useVigieStore((s) => s.repos);
  const tasks = useVigieStore((s) => s.tasks);
  const selectedTaskId = useVigieStore((s) => s.selectedTaskId);
  const theme = useVigieStore((s) => s.theme);
  const toggleTheme = useVigieStore((s) => s.toggleTheme);
  const soundSettings = useVigieStore((s) => s.soundSettings);
  const setSoundSettings = useVigieStore((s) => s.setSoundSettings);
  const remote = useVigieStore((s) => s.remote);
  const settingsOpen = useVigieStore((s) => s.settingsOpen);
  const openSettings = useVigieStore((s) => s.openSettings);
  const closeSettings = useVigieStore((s) => s.closeSettings);
  const muted = soundSettings?.muted ?? false;
  const toggleMute = () => {
    const cur = useVigieStore.getState().soundSettings;
    setSoundSettings({ ...cur, muted: !cur.muted });
  };

  const task = tasks.find((t) => t.id === selectedTaskId);
  const repo = repos.find((r) => r.id === task?.repoId);

  return (
    <header className="titlebar" data-tauri-drag-region>
      {/*
        Tauri only starts a window drag when the mousedown target itself carries
        `data-tauri-drag-region` (it doesn't bubble to ancestors), so every
        non-interactive element that fills the bar must opt in. Interactive
        controls below deliberately omit it so clicks don't drag the window.
      */}
      <div className="titlebar__brand" data-tauri-drag-region>
        <img
          className="titlebar__logo"
          src="/logo.png"
          alt=""
          aria-hidden
          data-tauri-drag-region
        />
        <span className="titlebar__wordmark" data-tauri-drag-region>
          La Vigie
        </span>
      </div>

      <div className="titlebar__right" data-tauri-drag-region>
        {(repo || task) && (
          <nav className="titlebar__crumbs" aria-label="Location">
            {repo && <span className="titlebar__crumb">{repo.name}</span>}
            {repo && task && <span className="titlebar__crumb-sep">/</span>}
            {task && (
              <span className="titlebar__crumb titlebar__crumb--current">
                {task.branch}
              </span>
            )}
          </nav>
        )}

        {remote.active && (
          <span
            className="titlebar__remote-dot"
            role="img"
            aria-label="Remote active"
            title="Remote control is active (tailnet)"
          >
            📡
          </span>
        )}
        <button
          type="button"
          className="titlebar__icon-btn"
          onClick={toggleMute}
          aria-label={muted ? "Unmute notification sounds" : "Mute notification sounds"}
          title={muted ? "Sounds muted" : "Mute sounds"}
        >
          <span aria-hidden>{muted ? "🔇" : "🔊"}</span>
        </button>
        <button
          type="button"
          className="titlebar__theme"
          onClick={toggleTheme}
          aria-label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
        >
          <span aria-hidden>{theme === "dark" ? "☾" : "☀"}</span>
          {theme === "dark" ? "Dark" : "Light"}
        </button>

        {/*
          Gear / settings button — deliberately omits data-tauri-drag-region so
          a click does not drag the window (TASK-74 drag rule).
        */}
        <button
          type="button"
          className="icon-btn"
          aria-label="Settings"
          onClick={openSettings}
        >
          <svg viewBox="0 0 24 24" width="15" height="15" fill="none" aria-hidden="true">
            <path
              d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
            <path
              d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>
      </div>

      {settingsOpen && (
        <SettingsModal onClose={closeSettings} />
      )}
    </header>
  );
}
