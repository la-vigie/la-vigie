// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
pub mod acp;
pub mod agent;
mod agent_commands;
mod claude_path;
mod commands;
mod docs;
mod git;
mod github;
mod launch;
mod meeting;
pub mod hooks;
mod mcp;
pub mod teardown;
mod remote;
mod session;
mod concierge;
pub mod schedule;
mod schedule_commands;
mod setup;
mod lavigie_plugin;
mod lavigie_skills;
mod shell_env;
mod sound;
mod sound_commands;
mod state;
mod store;
mod tray;

use std::sync::{Arc, Mutex};

use tauri::Manager;

pub use state::AppState;
pub use store::{Repo, Task, TaskStatus, TaskStore};

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Repair the process environment BEFORE any threads spawn, so the claude
    // PTY and all git/gh subprocesses inherit the user's real PATH, TERM, etc.
    // even when launched as a bundled .app. (set_var is not thread-safe.)
    shell_env::hydrate();

    // Single-instance guard, registered FIRST and ONLY in the packaged
    // (release) build. The data dir is a fixed path (~/Library/Application
    // Support/com.lavigie/), so two *installed* instances would co-mutate one
    // SQLite DB (only a per-process store Mutex guards it) and both run the 60s
    // schedule poller, double-firing crons. On a second launch the plugin hands
    // its argv to the primary and exits before reaching `setup`, where the
    // schedule poller and MCP loopback server start — so they only ever run in
    // the one primary. The callback raises+focuses the existing window instead
    // of hard-refusing.
    //
    // Dev builds (`tauri dev`) deliberately SKIP the guard so an in-development
    // instance can run side-by-side with an installed copy. They never collide:
    // a debug build redirects its data dir to a sibling `<identifier>.dev` tree
    // (see the `setup` closure), so the two share neither the DB nor worktrees.
    // The dual `#[cfg]` on the initial binding (not `mut`) keeps both profiles
    // warning-free — exactly one arm is compiled in, leaving no unused-mut.
    #[cfg(not(debug_assertions))]
    let builder = tauri::Builder::default().plugin(tauri_plugin_single_instance::init(
        |app, _argv, _cwd| {
            tray::focus_main_window(app);
        },
    ));
    #[cfg(debug_assertions)]
    let builder = tauri::Builder::default();

    let builder = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init());

    // Embedded WebDriver server for GUI-verification e2e. Registered
    // under #[cfg(debug_assertions)] only, so it never ships in a release build.
    #[cfg(debug_assertions)]
    let builder = builder.plugin(tauri_plugin_wdio_webdriver::init());

    builder
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            // A dev build uses a SEPARATE data dir (`<identifier>.dev`)
            // so `tauri dev` and an installed release never share one DB /
            // worktrees tree. Release builds are unchanged. Paired with the
            // release-only single-instance guard above, this lets both run at
            // once, each isolated. This is the single `app_data_dir()` call site,
            // and every derived root (DB, worktrees, sounds, concierge, acp_logs)
            // hangs off it — so forking it here forks all of them.
            #[cfg(debug_assertions)]
            let app_data_dir = {
                let name = app_data_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("com.lavigie");
                app_data_dir.with_file_name(format!("{name}.dev"))
            };
            std::fs::create_dir_all(&app_data_dir)?;

            let db_path = app_data_dir.join("vigie.db");
            let store = TaskStore::open(&db_path)?;

            // Any task left as 'running' at startup had its background job killed
            // by a crash or machine sleep — flip it to 'failed' so the UI is truthful.
            let _ = store.reconcile_interrupted_setups();

            let worktrees_root = app_data_dir.join("worktrees");
            std::fs::create_dir_all(&worktrees_root)?;

            let sounds_root = app_data_dir.join("sounds");
            std::fs::create_dir_all(&sounds_root)?;

            let concierge_root = app_data_dir.join("concierge");
            std::fs::create_dir_all(&concierge_root)?;

            let acp_logs_root = app_data_dir.join("acp_logs");
            std::fs::create_dir_all(&acp_logs_root)?;

            // Start the HookBridge server. Use block_on so setup remains sync.
            let tauri_sink = Arc::new(hooks::TauriSink::new(app.handle().clone()));
            let sink: Arc<dyn hooks::StatusSink> = tauri_sink.clone();
            let teardown: Arc<dyn hooks::TaskTeardown> = tauri_sink;
            let hook_port = tauri::async_runtime::block_on(hooks::start_hook_server(sink, teardown))
                .map_err(|e| format!("failed to start hook server: {e}"))?;

            // Start the MCP self-dispatch server. block_on keeps setup sync.
            let mcp_port = tauri::async_runtime::block_on(mcp::start_mcp_server(app.handle().clone()))
                .map_err(|e| format!("failed to start mcp server: {e}"))?;

            app.manage(AppState {
                store: Mutex::new(store),
                worktrees_root,
                sounds_root,
                concierge_root,
                acp_logs_root,
                sessions: Mutex::new(std::collections::HashMap::new()),
                hook_port,
                agent_states: Mutex::new(std::collections::HashMap::new()),
                agent_tasks: Mutex::new(std::collections::HashMap::new()),
                setups: Mutex::new(std::collections::HashMap::new()),
                mcp_port,
                mcp_tokens: Mutex::new(std::collections::HashMap::new()),
                remote: std::sync::Mutex::new(remote::RemoteState::default()),
                transcripts: Mutex::new(std::collections::HashMap::new()),
                pending_questions: Mutex::new(std::collections::HashMap::new()),
                task_errors: Mutex::new(std::collections::HashMap::new()),
                concierge_spawn: Mutex::new(()),
                base_fetch_at: Mutex::new(std::collections::HashMap::new()),
            });

            // Drop orchestrator resume-markers for repos deleted while
            // the app was closed, so we never resurrect a gone repo's orchestrator.
            concierge::prune_orphan_orchestrator_markers(app.state::<AppState>().inner());

            // Reap idle concierge sessions — the poll-based remote
            // transport gives no disconnect signal, so silence is the only cue.
            concierge::spawn_reaper(app.handle().clone());

            // Fire recurring schedules when due.
            schedule::spawn_scheduler(app.handle().clone());

            // Stand up the system-tray menu (in-progress tasks by repo).
            // Main-thread-only on macOS — `setup` runs on the main thread.
            tray::init(app.handle()).map_err(|e| format!("failed to init tray: {e}"))?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            commands::add_repo,
            commands::update_repo,
            commands::set_sound_settings,
            commands::is_meeting_active,
            commands::set_fetch_remote_base,
            commands::set_inject_lavigie_skills,
            commands::remove_repo,
            commands::list_repo_branches,
            commands::create_task,
            commands::check_worktree_path,
            commands::get_setup_state,
            commands::delete_task,
            commands::finish_task,
            commands::list_state,
            remote::commands::enable_remote,
            remote::commands::disable_remote,
            remote::commands::remote_status,
            commands::get_diff,
            commands::get_changed_files,
            commands::stage_files,
            commands::commit_task,
            commands::list_task_docs,
            commands::read_task_doc,
            commands::gh_status,
            commands::create_pr,
            commands::get_pr_status,
            commands::get_pr_comments,
            commands::set_task_hidden,
            agent::start_agent,
            agent::start_shell,
            agent::write_session,
            agent::resize_session,
            agent::stop_session,
            acp::start_acp_agent,
            acp::acp_prompt,
            acp::acp_cancel,
            acp::acp_set_mode,
            acp::acp_respond_permission,
            acp::get_acp_usage_summary,
            agent_commands::list_agents,
            agent_commands::upsert_custom_agent,
            agent_commands::delete_custom_agent,
            agent_commands::set_task_agent,
            agent_commands::set_repo_default_model,
            agent_commands::set_repo_routing_policy,
            agent_commands::set_task_model,
            agent_commands::set_task_auto_approve,
            agent_commands::list_agent_models,
            sound_commands::import_custom_sound,
            sound_commands::list_custom_sounds,
            sound_commands::read_sound_bytes,
            sound_commands::delete_custom_sound,
            commands::list_prompts,
            commands::create_prompt,
            commands::update_prompt,
            commands::delete_prompt,
            commands::reorder_prompts,
            concierge::list_remote_sessions,
            concierge::open_orchestrator,
            concierge::open_orchestrator_terminal,
            schedule_commands::list_schedules,
            schedule_commands::create_schedule,
            schedule_commands::create_one_shot_schedule,
            schedule_commands::update_schedule,
            schedule_commands::set_schedule_enabled,
            schedule_commands::delete_schedule,
            schedule_commands::preview_next_run,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greet_includes_name() {
        assert_eq!(
            greet("World"),
            "Hello, World! You've been greeted from Rust!"
        );
    }

    /// Regression guard: the custom HTML title bar drags the window
    /// via `data-tauri-drag-region`, which invokes the `start_dragging` command.
    /// That command is NOT part of `core:window:default`, so without an explicit
    /// grant every drag is silently denied and the window can't be moved.
    #[test]
    fn capabilities_grant_window_start_dragging() {
        let caps = include_str!("../capabilities/default.json");
        let json: serde_json::Value =
            serde_json::from_str(caps).expect("capabilities/default.json must be valid JSON");
        let permissions = json["permissions"]
            .as_array()
            .expect("capabilities must have a permissions array");
        assert!(
            permissions
                .iter()
                .any(|p| p.as_str() == Some("core:window:allow-start-dragging")),
            "capabilities/default.json must grant core:window:allow-start-dragging \
             or the custom title bar cannot drag the window (TASK-74)"
        );
    }

    /// The single-instance guard only works if its plugin is registered
    /// FIRST — a second launch must be detected and redirected to the primary
    /// before this process reaches `.setup(...)`, which is where the schedule
    /// poller and MCP loopback server are started. If someone reorders the
    /// plugins so `single_instance` no longer precedes the others (and `setup`),
    /// a second instance could boot far enough to double-run the poller. Guard
    /// the ordering at the source level.
    #[test]
    fn single_instance_plugin_registered_first() {
        let src = include_str!("lib.rs");
        let si = src
            .find("tauri_plugin_single_instance::init")
            .expect("single-instance plugin must be registered (TASK-225)");
        let opener = src
            .find("tauri_plugin_opener::init")
            .expect("opener plugin registration should exist");
        let setup = src
            .find(".setup(")
            .expect("setup closure should exist");
        assert!(
            si < opener && si < setup,
            "tauri_plugin_single_instance::init must be the FIRST plugin and precede .setup() \
             (TASK-225) — otherwise a second instance can boot far enough to double-run the \
             schedule poller / MCP server"
        );
    }

    /// The guard must be RELEASE-ONLY, and a dev build must redirect its
    /// data dir to a `<identifier>.dev` sibling. Together these let `tauri dev`
    /// run beside an installed copy without sharing the DB / worktrees. If either
    /// cfg gate is dropped, dev and prod would collide again (blocked launch, or
    /// two processes co-mutating one DB) — so pin both at the source level.
    #[test]
    fn dev_build_uses_separate_identity() {
        let src = include_str!("lib.rs");
        // The single-instance plugin sits under #[cfg(not(debug_assertions))],
        // so dev builds skip it. `find` returns the first cfg-gate, which is the
        // plugin's (the data-dir fork's #[cfg(debug_assertions)] comes later).
        let release_gate = src
            .find("#[cfg(not(debug_assertions))]")
            .expect("single-instance plugin must be gated to release builds (TASK-225)");
        let si = src
            .find("tauri_plugin_single_instance::init")
            .expect("single-instance plugin must be registered (TASK-225)");
        assert!(
            release_gate < si,
            "the single-instance plugin must sit under #[cfg(not(debug_assertions))] so \
             `tauri dev` can run beside an installed release (TASK-225)"
        );
        // A dev build forks app_data_dir to a `.dev` sibling.
        assert!(
            src.contains("#[cfg(debug_assertions)]")
                && src.contains("format!(\"{name}.dev\")"),
            "a dev build must redirect app_data_dir to a `<identifier>.dev` sibling so it \
             never shares the installed build's DB / worktrees (TASK-225)"
        );
    }

    /// Custom sounds play from a `blob:` URL built in the webview.
    /// Without `media-src blob:` in the CSP, <audio>/Audio falls back to
    /// default-src 'self' and the blob is blocked — so custom sounds go silent.
    #[test]
    fn csp_allows_blob_media() {
        let conf = include_str!("../tauri.conf.json");
        let json: serde_json::Value =
            serde_json::from_str(conf).expect("tauri.conf.json must be valid JSON");
        let media = json["app"]["security"]["csp"]["media-src"]
            .as_str()
            .expect("CSP must define media-src");
        assert!(
            media.contains("blob:"),
            "CSP media-src must allow blob: for custom-sound playback, got: {media}"
        );
    }
}
