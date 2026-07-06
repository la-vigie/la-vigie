// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
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
mod remote;
mod session;
mod concierge;
mod setup;
mod shell_env;
mod sound;
mod sound_commands;
mod state;
mod store;

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

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
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

            // Start the HookBridge server. Use block_on so setup remains sync.
            let sink: Arc<dyn hooks::StatusSink> =
                Arc::new(hooks::TauriSink::new(app.handle().clone()));
            let hook_port = tauri::async_runtime::block_on(hooks::start_hook_server(sink))
                .map_err(|e| format!("failed to start hook server: {e}"))?;

            // Start the MCP self-dispatch server (AC2-89). block_on keeps setup sync.
            let mcp_port = tauri::async_runtime::block_on(mcp::start_mcp_server(app.handle().clone()))
                .map_err(|e| format!("failed to start mcp server: {e}"))?;

            app.manage(AppState {
                store: Mutex::new(store),
                worktrees_root,
                sounds_root,
                concierge_root,
                sessions: Mutex::new(std::collections::HashMap::new()),
                hook_port,
                agent_states: Mutex::new(std::collections::HashMap::new()),
                agent_tasks: Mutex::new(std::collections::HashMap::new()),
                setups: Mutex::new(std::collections::HashMap::new()),
                mcp_port,
                mcp_tokens: Mutex::new(std::collections::HashMap::new()),
                remote: std::sync::Mutex::new(remote::RemoteState::default()),
                transcripts: Mutex::new(std::collections::HashMap::new()),
                concierge_spawn: Mutex::new(()),
            });

            // AC2-112: reap idle concierge sessions — the poll-based remote
            // transport gives no disconnect signal, so silence is the only cue.
            concierge::spawn_reaper(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            commands::add_repo,
            commands::update_repo,
            commands::set_sound_settings,
            commands::is_meeting_active,
            commands::set_fetch_remote_base,
            commands::remove_repo,
            commands::list_repo_branches,
            commands::create_task,
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
            agent_commands::list_agents,
            agent_commands::upsert_custom_agent,
            agent_commands::delete_custom_agent,
            agent_commands::set_task_agent,
            agent_commands::set_repo_default_model,
            agent_commands::set_task_model,
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

    /// Regression guard for AC2-74: the custom HTML title bar drags the window
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
             or the custom title bar cannot drag the window (AC2-74)"
        );
    }

    /// AC2-81: custom sounds play from a `blob:` URL built in the webview.
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
