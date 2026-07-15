//! System-tray menu (TASK-204): a macOS menu-bar item whose dropdown lists the
//! in-progress tasks grouped by repo, so the user can jump back to any active
//! task even when the window is hidden. Clicking a task focuses the main window
//! and selects that task in the frontend.
//!
//! Split by testability: [`build_tray_model`] is the pure core (repos+tasks →
//! grouped display model) and is unit-tested; the tray/menu/window wiring below
//! needs a running app + macOS menu bar and is verified by a human (GUI-only,
//! per the project's "AI agents cannot verify the GUI" rule).

use tauri::{
    menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter as _, Manager as _, Wry,
};

use crate::store::{Repo, Task, TaskStatus};
use crate::AppState;

/// Stable id so [`refresh`] can retrieve the tray via `app.tray_by_id`.
const TRAY_ID: &str = "lavigie-tray";
const ID_SHOW: &str = "tray:show";
const ID_QUIT: &str = "tray:quit";
const ID_EMPTY: &str = "tray:empty";
/// Task menu-item ids are `tray:task:<task_id>`; the click handler strips this.
const TASK_PREFIX: &str = "tray:task:";

/// One in-progress task as shown in the tray menu.
#[derive(Debug, Clone, PartialEq)]
pub struct TrayTaskItem {
    pub task_id: String,
    pub ticket_key: Option<String>,
    pub title: String,
    pub status: TaskStatus,
}

/// In-progress tasks for one repo. Only non-empty groups are produced.
#[derive(Debug, Clone, PartialEq)]
pub struct TrayRepoGroup {
    pub repo_name: String,
    pub items: Vec<TrayTaskItem>,
}

/// A task belongs in the tray when it is actively being worked: a non-terminal
/// status and not hidden. `Done` (finished) and `Pending` (queued/blocked — no
/// worktree yet) are excluded; `Pending` may earn its own group in a later pass.
pub fn is_in_progress(t: &Task) -> bool {
    !t.hidden
        && matches!(
            t.status,
            TaskStatus::Idle | TaskStatus::Working | TaskStatus::NeedsAttention | TaskStatus::Error
        )
}

/// Pure core: group in-progress tasks by repo, preserving the input order of
/// both repos and tasks (which is the sidebar order). Repos with no in-progress
/// task are omitted. UNIT-TESTED — the menu built from this is untested glue.
pub fn build_tray_model(repos: &[Repo], tasks: &[Task]) -> Vec<TrayRepoGroup> {
    repos
        .iter()
        .filter_map(|repo| {
            let items: Vec<TrayTaskItem> = tasks
                .iter()
                .filter(|t| t.repo_id == repo.id && is_in_progress(t))
                .map(|t| TrayTaskItem {
                    task_id: t.id.clone(),
                    ticket_key: t.ticket_key.clone(),
                    title: t.title.clone(),
                    status: t.status,
                })
                .collect();
            (!items.is_empty()).then(|| TrayRepoGroup {
                repo_name: repo.name.clone(),
                items,
            })
        })
        .collect()
}

/// A small status glyph for the menu label (working / needs-attention / …).
pub fn status_glyph(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Working => "🟢",
        TaskStatus::NeedsAttention => "🔴",
        TaskStatus::Error => "⚠️",
        TaskStatus::Idle => "⚪",
        // Filtered out of the tray, but keep the match total.
        TaskStatus::Done | TaskStatus::Pending => "•",
    }
}

/// The menu-item label for a task: glyph + ticket key (if any) + title.
pub fn task_label(item: &TrayTaskItem) -> String {
    let glyph = status_glyph(item.status);
    match &item.ticket_key {
        Some(key) if !key.trim().is_empty() => format!("{glyph}  {key}  {}", item.title),
        _ => format!("{glyph}  {}", item.title),
    }
}

/// Read repos+tasks under one brief store lock (no await) and build the tray
/// menu from the grouped model. Errors are surfaced with `{e:#}` so the caller
/// can log the underlying store/menu error.
fn build_menu(app: &AppHandle) -> Result<Menu<Wry>, String> {
    let (repos, tasks) = {
        let st = app.state::<AppState>();
        let store = st.store.lock().map_err(|e| format!("{e:#}"))?;
        let repos = store.list_repos().map_err(|e| format!("{e:#}"))?;
        let tasks = store.list_tasks().map_err(|e| format!("{e:#}"))?;
        (repos, tasks)
    };
    let groups = build_tray_model(&repos, &tasks);

    let mut menu = MenuBuilder::new(app);
    if groups.is_empty() {
        // Empty state: a disabled cue that nothing is in progress.
        let empty = MenuItemBuilder::with_id(ID_EMPTY, "No tasks in progress")
            .enabled(false)
            .build(app)
            .map_err(|e| format!("{e:#}"))?;
        menu = menu.item(&empty);
    } else {
        for (i, group) in groups.iter().enumerate() {
            if i > 0 {
                menu = menu.separator();
            }
            // Repo name as a disabled section header; tasks listed beneath it.
            let header = MenuItemBuilder::with_id(format!("tray:repo:{i}"), &group.repo_name)
                .enabled(false)
                .build(app)
                .map_err(|e| format!("{e:#}"))?;
            menu = menu.item(&header);
            for item in &group.items {
                let mi = MenuItemBuilder::with_id(
                    format!("{TASK_PREFIX}{}", item.task_id),
                    task_label(item),
                )
                .build(app)
                .map_err(|e| format!("{e:#}"))?;
                menu = menu.item(&mi);
            }
        }
    }

    let show = MenuItemBuilder::with_id(ID_SHOW, "Show La Vigie")
        .build(app)
        .map_err(|e| format!("{e:#}"))?;
    let quit = MenuItemBuilder::with_id(ID_QUIT, "Quit La Vigie")
        .build(app)
        .map_err(|e| format!("{e:#}"))?;
    let sep = PredefinedMenuItem::separator(app).map_err(|e| format!("{e:#}"))?;

    menu.item(&sep)
        .item(&show)
        .item(&quit)
        .build()
        .map_err(|e| format!("{e:#}"))
}

/// Stand up the tray icon with its initial menu. Call once at startup, on the
/// main thread (tray/menu creation is main-thread-only on macOS — `setup` runs
/// there).
pub fn init(app: &AppHandle) -> Result<(), String> {
    let menu = build_menu(app)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("La Vigie")
        .menu(&menu)
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()));
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app).map_err(|e| format!("{e:#}"))?;
    Ok(())
}

/// Rebuild the tray menu from current state. Best-effort: a store/menu error
/// leaves the previous menu in place rather than crashing a background event
/// handler. Called on each task/status change; `agent_status` only fires on
/// real transitions, so this stays bounded (no tight timer).
///
/// Marshals to the main thread: callers include the hook-server and teardown
/// threads, and macOS menu mutation must happen on the main thread.
pub fn refresh(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(tray) = handle.tray_by_id(TRAY_ID) else {
            return;
        };
        match build_menu(&handle) {
            Ok(menu) => {
                if let Err(e) = tray.set_menu(Some(menu)) {
                    eprintln!("tray: failed to set menu: {e:#}");
                }
            }
            Err(e) => eprintln!("tray: failed to build menu: {e}"),
        }
    });
}

/// Route a tray menu click. Runs on the main thread (Tauri delivers menu events
/// there), so window ops are safe here.
fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        ID_QUIT => app.exit(0),
        ID_SHOW => focus_main_window(app),
        _ if id.starts_with(TASK_PREFIX) => {
            let task_id = id[TASK_PREFIX.len()..].to_string();
            focus_main_window(app);
            // The frontend selects the task; `setSelectedTask` also clears its
            // attention cue (mirrors how `task_launched` is bridged, TASK-89).
            let _ = app.emit("tray_select_task", TraySelectPayload { task_id });
        }
        // Disabled headers / empty-state item: no-op.
        _ => {}
    }
}

/// Bring the main window to the foreground: show (if hidden), unminimize, focus.
fn focus_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        if let Err(e) = win.set_focus() {
            eprintln!("tray: failed to focus main window: {e:#}");
        }
    }
}

/// Payload for the `tray_select_task` event (camelCase across IPC).
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TraySelectPayload {
    task_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SetupStatus;

    fn repo(id: &str, name: &str) -> Repo {
        Repo {
            id: id.into(),
            name: name.into(),
            path: format!("/repos/{id}"),
            default_branch: "main".into(),
            remote_url: None,
            worktree_root: None,
            setup_command: None,
            default_agent: None,
            auto_start_agent: false,
            initial_prompt: None,
            default_model: None,
            sound_settings: None,
            fetch_remote_base: None,
            auto_approve: None,
            in_place_default: false,
        }
    }

    fn task(id: &str, repo_id: &str, status: TaskStatus) -> Task {
        Task {
            id: id.into(),
            repo_id: repo_id.into(),
            title: format!("Task {id}"),
            worktree_path: format!("/wt/{id}"),
            branch: format!("feat/{id}"),
            base_branch: "main".into(),
            status,
            created_at: 0,
            updated_at: 0,
            pr_number: None,
            pr_url: None,
            ticket_key: None,
            agent: None,
            model: None,
            setup_status: None as Option<SetupStatus>,
            hidden: false,
            pending_prompt: None,
            auto_approve: None,
            in_place: false,
        }
    }

    #[test]
    fn groups_in_progress_tasks_by_repo_in_input_order() {
        let repos = vec![repo("r1", "alpha"), repo("r2", "beta")];
        let tasks = vec![
            task("t1", "r1", TaskStatus::Working),
            task("t2", "r2", TaskStatus::NeedsAttention),
            task("t3", "r1", TaskStatus::Idle),
        ];
        let model = build_tray_model(&repos, &tasks);
        assert_eq!(model.len(), 2);
        assert_eq!(model[0].repo_name, "alpha");
        // Task order within a repo preserves input order (t1 then t3).
        assert_eq!(
            model[0]
                .items
                .iter()
                .map(|i| i.task_id.as_str())
                .collect::<Vec<_>>(),
            vec!["t1", "t3"]
        );
        assert_eq!(model[1].repo_name, "beta");
        assert_eq!(model[1].items.len(), 1);
        assert_eq!(model[1].items[0].task_id, "t2");
    }

    #[test]
    fn omits_repos_with_no_in_progress_tasks() {
        let repos = vec![repo("r1", "alpha"), repo("r2", "beta")];
        // r2 has only a Done task → dropped entirely.
        let tasks = vec![
            task("t1", "r1", TaskStatus::Working),
            task("t2", "r2", TaskStatus::Done),
        ];
        let model = build_tray_model(&repos, &tasks);
        assert_eq!(model.len(), 1);
        assert_eq!(model[0].repo_name, "alpha");
    }

    #[test]
    fn excludes_done_pending_and_hidden_but_keeps_error_and_idle() {
        assert!(is_in_progress(&task("a", "r", TaskStatus::Idle)));
        assert!(is_in_progress(&task("a", "r", TaskStatus::Working)));
        assert!(is_in_progress(&task("a", "r", TaskStatus::NeedsAttention)));
        assert!(is_in_progress(&task("a", "r", TaskStatus::Error)));
        assert!(!is_in_progress(&task("a", "r", TaskStatus::Done)));
        assert!(!is_in_progress(&task("a", "r", TaskStatus::Pending)));

        let mut hidden = task("a", "r", TaskStatus::Working);
        hidden.hidden = true;
        assert!(!is_in_progress(&hidden), "hidden tasks are excluded");
    }

    #[test]
    fn empty_when_nothing_in_progress() {
        let repos = vec![repo("r1", "alpha")];
        let tasks = vec![task("t1", "r1", TaskStatus::Done)];
        assert!(build_tray_model(&repos, &tasks).is_empty());
    }

    #[test]
    fn task_label_includes_ticket_key_when_present() {
        let mut item = TrayTaskItem {
            task_id: "t1".into(),
            ticket_key: Some("TASK-204".into()),
            title: "Tray menu".into(),
            status: TaskStatus::Working,
        };
        let with_key = task_label(&item);
        assert!(with_key.contains("TASK-204"), "got: {with_key}");
        assert!(with_key.contains("Tray menu"));

        // Blank/absent ticket key falls back to glyph + title only.
        item.ticket_key = Some("   ".into());
        assert!(!task_label(&item).contains("TASK-204"));
        item.ticket_key = None;
        let no_key = task_label(&item);
        assert!(no_key.contains("Tray menu"));
        assert!(!no_key.contains("  TASK-204"));
    }
}
