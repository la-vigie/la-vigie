//! Agent process management: spawn `claude` agent sessions inside a PTY in a
//! task's worktree, and stream their output to the frontend over a Tauri
//! `Channel`.
//!
//! The low-level PTY mechanics (`spawn_pty`) are kept free of any Tauri
//! types so they can be unit-tested directly (real PTY, no app harness
//! needed). The `#[tauri::command]` layer above it is thin glue that wires
//! `spawn_pty` to a `Channel<PtyEvent>` and a `SessionHandle` registry on
//! `AppState`; that layer is not unit-tested here (see module docs on the
//! commands below) because it needs a running Tauri app to construct a
//! `Channel`.

pub mod mcp_bundle;
pub mod models;
pub mod skill_bundle;
pub mod spec;
pub mod status;

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tauri::ipc::Channel;
use tauri::State;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

use crate::state::AppState;

/// Event streamed to the frontend over the `Channel` for a running agent
/// session.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PtyEvent {
    /// Base64-encoded raw PTY output bytes.
    Data { data: String },
    /// The agent process has exited with this code.
    Exit { code: i32 },
}

/// A freshly spawned PTY session: the master side (for resize), a writer
/// (for input), the child process (for wait/kill), and the reader — taken
/// exactly once by the output-streaming thread.
pub struct PtySession {
    pub master: Box<dyn MasterPty + Send>,
    pub writer: Box<dyn std::io::Write + Send>,
    pub child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    pub reader: Option<Box<dyn Read + Send>>,
}

/// Mark every La Vigie-spawned process so cooperating tools (e.g. the user's
/// dotfiles sound hooks) can detect they're running inside the app and defer.
fn apply_lavigie_env(cmd: &mut CommandBuilder) {
    cmd.env("LAVIGIE", "1");
    // Advertise OSC 8 hyperlink support to CLIs that gate it on terminal
    // detection (e.g. Claude Code's `supports-hyperlinks` check). Our embedded
    // xterm.js genuinely supports OSC 8 (see TerminalView's linkHandler), but it
    // sets no recognized TERM_PROGRAM, so tools would otherwise fall back to
    // plain text — leaving footer badges like the "PR #123" link unclickable
    // (TASK-170). FORCE_HYPERLINK=1 is the documented opt-in override.
    cmd.env("FORCE_HYPERLINK", "1");
}

/// Spawn `program` with `args` in a new PTY, optionally with working
/// directory `cwd`, sized `cols` x `rows`. `extra_env` sets additional
/// environment variables on the child (on top of `LAVIGIE=1`) — used to hand
/// an agent its HookBridge coordinates so a skill can call back (TASK-40).
pub fn spawn_pty(
    program: impl AsRef<std::ffi::OsStr>,
    args: &[String],
    cwd: Option<&Path>,
    cols: u16,
    rows: u16,
    extra_env: &[(&str, String)],
) -> anyhow::Result<PtySession> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let writer = pair.master.take_writer()?;
    let reader = pair.master.try_clone_reader()?;

    let mut cmd = CommandBuilder::new(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.cwd(dir);
    }
    apply_lavigie_env(&mut cmd);
    for (key, value) in extra_env {
        cmd.env(key, value);
    }

    let child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);

    Ok(PtySession {
        master: pair.master,
        writer,
        child: Arc::new(Mutex::new(child)),
        reader: Some(reader),
    })
}

/// What a registered PTY session represents. Drives the desktop "Remote
/// sessions" surface and the idle reaper (rootless `Concierge`/`Orchestrator`
/// sessions are reaped; `Task`/`Shell` are not).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    /// A task-bound agent (worktree + DB Task row).
    Task,
    /// A plain interactive shell.
    Shell,
    /// The worktree-less mobile concierge (TASK-112).
    Concierge,
    /// A worktree-less, repo-scoped orchestrator session (TASK-180). Like the
    /// concierge but bound to one repo via its MCP token; the repo id lives on
    /// `SessionHandle.repo_id`.
    Orchestrator,
}

/// A registered, running PTY session: everything needed to write input,
/// resize, and stop it. The output-streaming thread holds its own clone of
/// `child` (for wait/exit-code) and the `reader` taken from the `PtySession`
/// at spawn time; neither is stored here.
pub struct SessionHandle {
    pub master: Box<dyn MasterPty + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    /// The agent's MCP bearer token, if any (None for shells / non-Claude
    /// agents). Removed from `AppState.mcp_tokens` when the session stops.
    pub mcp_token: Option<String>,
    /// What this session is (task agent / shell / concierge / orchestrator).
    pub kind: SessionKind,
    /// The repo this rootless session is scoped to, when applicable. `Some` for
    /// a per-repo `Orchestrator` (TASK-180); `None` for tasks, shells, and the
    /// legacy global concierge.
    pub repo_id: Option<String>,
    /// Last client activity (poll/reply). Updated via `bump_activity`; read by
    /// the idle reaper and the desktop "Remote sessions" list.
    pub last_activity: std::time::Instant,
    /// True when a live frontend `Channel` is attached — a directly-attached
    /// desktop terminal, exempt from the idle reaper (TASK-126). Task/Shell
    /// sessions also pass a channel, but they're already exempt by `kind`; this
    /// field only changes reaper behavior for rootless (`Concierge`/
    /// `Orchestrator`) sessions.
    pub has_frontend_channel: bool,
}

/// Resolve the user's interactive login shell: `$SHELL` if set, else
/// `/bin/zsh` (macOS default). Always launched as a login shell (`-l`) so the
/// user's profile (PATH, aliases) is loaded.
pub fn resolve_shell() -> (String, Vec<String>) {
    let program = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    (program, vec!["-l".to_string()])
}

/// Take the session's reader, spawn the background thread that streams its PTY
/// output over `on_event` when present (emitting Exit on EOF), and register the
/// live handle in `state.sessions` under `session_id`. When `on_event` is `None`
/// (the rootless concierge has no desktop terminal) the reader is still drained
/// to a sink so the PTY never blocks. Shared by start_agent, start_shell, and
/// the concierge spawn path.
pub(crate) fn register_streaming_session(
    state: &AppState,
    session_id: &str,
    mut session: PtySession,
    on_event: Option<Channel<PtyEvent>>,
    mcp_token: Option<String>,
    kind: SessionKind,
    repo_id: Option<String>,
) -> Result<(), String> {
    let reader = session.reader.take().expect("freshly spawned PtySession always has a reader");
    let child_for_thread = std::sync::Arc::clone(&session.child);
    let on_event_for_thread = on_event.clone();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 16 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if let Some(ch) = on_event_for_thread.as_ref() {
                        let _ = ch.send(PtyEvent::Data { data: B64.encode(&buf[..n]) });
                    }
                    // on_event == None (concierge): bytes are drained and dropped.
                }
            }
        }
        let code = child_for_thread.lock().ok().and_then(|mut c| c.wait().ok()).map(|s| s.exit_code() as i32).unwrap_or(-1);
        if let Some(ch) = on_event_for_thread.as_ref() {
            let _ = ch.send(PtyEvent::Exit { code });
        }
    });
    let has_frontend_channel = on_event.is_some();
    let handle = SessionHandle {
        master: session.master,
        writer: session.writer,
        child: session.child,
        mcp_token,
        kind,
        repo_id,
        last_activity: std::time::Instant::now(),
        has_frontend_channel,
    };
    state.sessions.lock().map_err(|e| e.to_string())?.insert(session_id.to_string(), handle);
    Ok(())
}

/// Build the inline `--settings` JSON that injects hook callbacks for a
/// specific agent id. Each supported hook event POSTs the raw stdin payload to
/// the local HookBridge server so status can be derived out-of-band.
///
/// The returned string is passed verbatim to `--settings`; Claude Code accepts
/// inline JSON for that flag and merges the hooks additively with the user's
/// own settings.
pub fn build_hook_settings(port: u16, agent_id: &str) -> String {
    let curl_cmd = format!(
        "curl -s -X POST http://127.0.0.1:{port}/hook/{agent_id} --data-binary @-"
    );

    let hook_entry = serde_json::json!([{
        "hooks": [{
            "type": "command",
            "command": curl_cmd
        }]
    }]);

    // statusLine: POST the JSON to the bridge and print nothing (-o /dev/null),
    // so it contributes no visible text to Claude Code's status row.
    let status_cmd = format!(
        "curl -s -o /dev/null -X POST http://127.0.0.1:{port}/status/{agent_id} --data-binary @-"
    );

    let settings = serde_json::json!({
        "hooks": {
            "UserPromptSubmit": hook_entry,
            "Notification":     hook_entry,
            "Stop":             hook_entry,
            "StopFailure":      hook_entry,
            "PreToolUse":       hook_entry,
            "SubagentStart":    hook_entry,
            "SubagentStop":     hook_entry
        },
        "statusLine": { "type": "command", "command": status_cmd }
    });

    settings.to_string()
}

/// Build the inline `--mcp-config` JSON registering La Vigie's loopback MCP
/// server for a specific agent. The per-agent bearer token both authenticates
/// the call and (server-side) resolves the originating task/repo context.
///
/// Returned verbatim to `--mcp-config`; Claude Code accepts inline JSON for that
/// flag and an `http`-type server with static `Authorization` headers.
pub fn build_mcp_config(port: u16, token: &str) -> String {
    serde_json::json!({
        "mcpServers": {
            "lavigie": {
                "type": "http",
                "url": format!("http://127.0.0.1:{port}/mcp"),
                "headers": { "Authorization": format!("Bearer {token}") }
            }
        }
    })
    .to_string()
}

/// TASK-174: the optional `LAVIGIE_TASK_REF` env entry for a launched agent —
/// the provider ticket ref (e.g. `TASK-174`, `#123`, `owner/repo#123`) passed
/// through verbatim so a `task-provider` adapter skill can route/act on it.
/// `None` when the task has no ticket key (the adapter then degrades to reading
/// the launch prompt). The ref is handed over rather than recovered from the
/// branch because `slugify` is lossy for issue ids (`#123` → `123`).
fn lavigie_task_ref_env(ticket_key: Option<&str>) -> Option<(&'static str, String)> {
    match ticket_key {
        Some(key) if !key.trim().is_empty() => Some(("LAVIGIE_TASK_REF", key.to_string())),
        _ => None,
    }
}

/// Spawn the task's resolved agent in its worktree and stream its PTY output
/// to the frontend over `on_event`. The agent is determined by the task →
/// repo default → global default (`claude`) precedence chain. Returns the new
/// agent's id, used to address it in `write_session`/`resize_session`/`stop_session`.
#[tauri::command]
pub fn start_agent(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    task_id: String,
    resume: bool,
    initial_prompt: Option<String>,
    on_event: Channel<PtyEvent>,
) -> Result<String, String> {
    let (
        worktree_path,
        repo_id,
        task_agent,
        task_model,
        task_auto_approve,
        repo_default,
        repo_auto_approve,
        custom_agents,
        app_inject_skills,
        task_ticket_key,
    ) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let task = store
            .get_task(&task_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("task not found: {task_id}"))?;
        let repo = store.get_repo(&task.repo_id).map_err(|e| e.to_string())?;
        let repo_default = repo.as_ref().and_then(|r| r.default_agent.clone());
        let repo_auto_approve = repo.as_ref().and_then(|r| r.auto_approve);
        let custom = store.list_custom_agents().map_err(|e| e.to_string())?;
        let app_inject_skills = crate::commands::effective_inject_lavigie_skills(
            store
                .get_app_setting("inject_lavigie_skills")
                .map_err(|e| format!("{e:#}"))?
                .as_deref()
                .and_then(crate::commands::parse_bool_setting),
        );
        (
            task.worktree_path,
            task.repo_id,
            task.agent,
            task.model,
            task.auto_approve,
            repo_default,
            repo_auto_approve,
            custom,
            app_inject_skills,
            task.ticket_key,
        )
    };

    // Generate agent_id before building args so it can go in the hook URL.
    let agent_id = uuid::Uuid::new_v4().to_string();

    // Record agent_id → task_id so hook-driven status updates can persist to the
    // owning task. (Lingering entries after an un-stopped resume are harmless —
    // ids are unique; stop_session removes them.)
    state
        .agent_tasks
        .lock()
        .map_err(|e| e.to_string())?
        .insert(agent_id.clone(), task_id.clone());

    use crate::agent::spec::{build_agent_command, resolve_for_task, StatusMechanism};

    let spec = resolve_for_task(task_agent.as_deref(), repo_default.as_deref(), &custom_agents);
    let auto_approve = crate::agent::spec::effective_auto_approve(task_auto_approve, repo_auto_approve);

    // Hooks (and the HookBridge status pipeline) are Claude-Code-specific; only
    // a ClaudeHooks agent gets `--settings`. Lifecycle agents launch hook-free.
    let hook_settings = if spec.status == StatusMechanism::ClaudeHooks {
        Some(build_hook_settings(state.hook_port, &agent_id))
    } else {
        None
    };

    use crate::agent::spec::SkillInjection;

    // TASK-89 / TASK-193: mint a per-agent Agent-tier MCP token (auth + originating
    // task/repo carrier). Claude consumes it via an inline `--mcp-config`; the four
    // WorktreeBundle engines consume it via a materialized project-local MCP config
    // (below). Mint for either path; the token's lifecycle is owned by
    // register_streaming_session/stop_session_inner regardless of engine.
    let wants_mcp_injection =
        app_inject_skills && matches!(spec.skill_injection, SkillInjection::WorktreeBundle { .. });
    let mcp_token = if spec.status == StatusMechanism::ClaudeHooks || wants_mcp_injection {
        let token = uuid::Uuid::new_v4().to_string();
        state
            .mcp_tokens
            .lock()
            .map_err(|e| e.to_string())?
            .insert(
                token.clone(),
                crate::state::McpToken::Agent(crate::state::AgentLaunchContext {
                    task_id: task_id.clone(),
                    repo_id: repo_id.clone(),
                }),
            );
        Some(token)
    } else {
        None
    };
    // Inline `--mcp-config` is Claude-only; Lifecycle engines get a materialized file.
    let mcp_config = if spec.status == StatusMechanism::ClaudeHooks {
        mcp_token.as_deref().map(|t| build_mcp_config(state.mcp_port, t))
    } else {
        None
    };

    // TASK-153: Claude gets La Vigie's bundled skills out-of-tree via `--plugin-dir`.
    // Unresolvable/invalid bundle → omit the flag; launch never breaks.
    let plugin_dir: Option<String> = if app_inject_skills
        && spec.skill_injection == SkillInjection::PluginDir
    {
        crate::lavigie_plugin::resolve_plugin_dir(&app).map(|p| p.to_string_lossy().into_owned())
    } else {
        None
    };

    // TASK-35: providers that discover project-local skills from the worktree get
    // their vendored per-provider bundle materialized into the worktree
    // (git-excluded, so it never shows in the Diff). Best-effort: any failure
    // logs and the agent still launches, skill-free. Runs before spawn_pty so it
    // never races the PTY (KEEP-ALIVE safe).
    if app_inject_skills {
        if let SkillInjection::WorktreeBundle { provider } = &spec.skill_injection {
            match crate::lavigie_skills::resolve_skills_bundle_dir(&app, provider) {
                Some(bundle) => {
                    match crate::agent::skill_bundle::materialize(&bundle, Path::new(&worktree_path)) {
                        Ok(dirs) if dirs.is_empty() => {
                            eprintln!("TASK-35: bundle for {provider} had no skill dirs; launching skill-free");
                        }
                        Ok(_dirs) => {}
                        Err(e) => eprintln!("TASK-35: skill injection for {provider} failed: {e:#}"),
                    }
                }
                None => {
                    eprintln!("TASK-35: no vendored skill bundle for {provider}; launching skill-free");
                }
            }
        }
    }

    // TASK-193: the same WorktreeBundle engines get La Vigie's loopback MCP server
    // registered via a project-local config materialized into the worktree, with
    // the ephemeral port + this agent's bearer token substituted in. Git-excluded
    // like the skills, never overwriting a repo-tracked config. Best-effort: any
    // failure logs and the agent launches without the injected server. Runs before
    // spawn_pty (KEEP-ALIVE safe).
    if let (SkillInjection::WorktreeBundle { provider }, Some(token)) =
        (&spec.skill_injection, mcp_token.as_deref())
    {
        if wants_mcp_injection {
            match crate::lavigie_skills::resolve_mcp_bundle_dir(&app, provider) {
                Some(bundle) => {
                    match crate::agent::mcp_bundle::materialize_mcp(
                        &bundle,
                        Path::new(&worktree_path),
                        state.mcp_port,
                        token,
                    ) {
                        Ok(dirs) if dirs.is_empty() => {
                            eprintln!("TASK-193: MCP bundle for {provider} was empty; launching without lavigie server");
                        }
                        Ok(_dirs) => {}
                        Err(e) => eprintln!("TASK-193: MCP injection for {provider} failed: {e:#}"),
                    }
                }
                None => {
                    eprintln!("TASK-193: no vendored MCP bundle for {provider}; launching without lavigie server");
                }
            }
        }
    }

    let (program, mut args) = build_agent_command(
        &spec,
        resume,
        hook_settings.as_deref(),
        mcp_config.as_deref(),
        task_model.as_deref(),
        plugin_dir.as_deref(),
        auto_approve,
    );
    let resolved = crate::claude_path::find_binary(&program);

    // Deliver an optional initial prompt using the resolved agent's prompt_mode
    // (TASK-49 delivery + TASK-21 resolution). A prompt is only seeded on a fresh
    // start, never on resume.
    let delivery = crate::agent::spec::initial_prompt_delivery(
        spec.prompt_mode,
        if resume { None } else { initial_prompt.as_deref() },
    );
    args.extend(delivery.args);

    // Hand the agent its HookBridge coordinates so a skill it runs can call back.
    // LAVIGIE_TASK_ID is the durable key for task-scoped callbacks — POST
    // /rename/{task_id} and /finish/{task_id} (TASK-40/TASK-139/TASK-151) — so they
    // resolve even after an app restart empties the in-memory agent→task map.
    // LAVIGIE_AGENT_ID stays for the per-agent hook URLs (status/transcript).
    // TASK-174: also hand over LAVIGIE_TASK_REF — the provider ticket ref
    // verbatim — so a task-provider adapter skill can route/act on it. Only
    // present when the task carries a ticket key.
    let mut agent_env: Vec<(&str, String)> = vec![
        ("LAVIGIE_HOOK_PORT", state.hook_port.to_string()),
        ("LAVIGIE_AGENT_ID", agent_id.clone()),
        ("LAVIGIE_TASK_ID", task_id.clone()),
    ];
    if let Some(entry) = lavigie_task_ref_env(task_ticket_key.as_deref()) {
        agent_env.push(entry);
    }
    // TASK-193: Codex reads its MCP bearer token from an env var (it has no
    // static-header auth for HTTP MCP; its vendored config names LAVIGIE_MCP_TOKEN
    // via `bearer_token_env_var`), so hand it the minted token when we injected an
    // MCP config. Harmless for the other engines, which read the token from the
    // static `Authorization` header in their own materialized config.
    if wants_mcp_injection {
        if let Some(t) = mcp_token.as_deref() {
            agent_env.push(("LAVIGIE_MCP_TOKEN", t.to_string()));
        }
    }

    // If spawn or the initial write fails we return early, before
    // register_streaming_session (which owns the token's lifecycle on success)
    // ever runs — so revoke the just-minted MCP token here, or a failed launch
    // orphans a live credential (for a WorktreeBundle engine it was also just
    // written into a worktree config file on disk). TASK-193 review.
    let mut session = match spawn_pty(&resolved, &args, Some(Path::new(&worktree_path)), 80, 24, &agent_env) {
        Ok(s) => s,
        Err(e) => {
            if let Some(t) = mcp_token.as_deref() {
                let _ = state.mcp_tokens.lock().map(|mut m| m.remove(t));
            }
            return Err(format!("{e:#}"));
        }
    };

    // PromptMode::Stdin path (not exercised by the claude-only live route, but
    // wired so the TASK-21 thread inherits a complete delivery).
    if let Some(stdin) = delivery.stdin {
        if let Err(e) = session
            .writer
            .write_all(stdin.as_bytes())
            .and_then(|_| session.writer.flush())
        {
            if let Some(t) = mcp_token.as_deref() {
                let _ = state.mcp_tokens.lock().map(|mut m| m.remove(t));
            }
            return Err(format!("{e:#}"));
        }
    }

    register_streaming_session(&state, &agent_id, session, Some(on_event), mcp_token, SessionKind::Task, None)?;

    // Emit initial console status with permission mode for agents whose auto-approve
    // is resolved on. Lifecycle agents (e.g. Mistral Vibe) don't use hooks. (TASK-135)
    if auto_approve && !spec.auto_approve_args.is_empty() {
        use tauri::Emitter as _;
        let _ = app.emit(
            "agent_console",
            crate::hooks::AgentConsolePayload {
                agent_id: agent_id.clone(),
                console: crate::hooks::ConsoleStatus {
                    mode: Some("auto".to_string()),
                    ..Default::default()
                },
            },
        );
    }

    Ok(agent_id)
}

/// Spawn a plain interactive login shell in the given task's worktree and
/// stream its PTY output over `on_event`. Returns the new session id used to
/// address it in `write_session`/`resize_session`/`stop_session`. No hooks,
/// no status wiring (those are agent-only).
#[tauri::command]
pub fn start_shell(
    state: State<'_, AppState>,
    task_id: String,
    on_event: Channel<PtyEvent>,
) -> Result<String, String> {
    let worktree_path = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let task = store
            .get_task(&task_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("task not found: {task_id}"))?;
        task.worktree_path
    };

    let session_id = uuid::Uuid::new_v4().to_string();
    let (program, args) = resolve_shell();

    let session = spawn_pty(&program, &args, Some(Path::new(&worktree_path)), 80, 24, &[])
        .map_err(|e| format!("{e:#}"))?;

    register_streaming_session(&state, &session_id, session, Some(on_event), None, SessionKind::Shell, None)?;

    Ok(session_id)
}

/// Write raw bytes to a registered session's PTY. Shared by the `write_session`
/// command and the remote reply handler (TASK-108). Errors if the id is unknown.
pub fn write_to_session(state: &AppState, session_id: &str, data: &str) -> Result<(), String> {
    let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    let handle = sessions
        .get_mut(session_id)
        .ok_or_else(|| format!("session not found: {session_id}"))?;

    handle.writer.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
    handle.writer.flush().map_err(|e| e.to_string())?;
    Ok(())
}

/// Write raw input bytes (a UTF-8 string of keystrokes from xterm) to the
/// session's PTY.
#[tauri::command]
pub fn write_session(state: State<'_, AppState>, session_id: String, data: String) -> Result<(), String> {
    write_to_session(state.inner(), &session_id, &data)
}

/// Update a session's last-activity timestamp to now (no-op if the id is
/// unknown). Called by the remote read/reply path so the concierge reaper sees
/// client polls. Guarded by the existing `sessions` Mutex.
pub fn bump_activity(state: &AppState, session_id: &str) {
    if let Ok(mut sessions) = state.sessions.lock() {
        if let Some(h) = sessions.get_mut(session_id) {
            h.last_activity = std::time::Instant::now();
        }
    }
}

/// Resize the session's PTY (e.g. when the xterm container is resized).
#[tauri::command]
pub fn resize_session(
    state: State<'_, AppState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    let handle = sessions
        .get(&session_id)
        .ok_or_else(|| format!("session not found: {session_id}"))?;

    handle
        .master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())
}

/// Stop a session by id: kill its process, drop its PTY handles, and clear all
/// per-session bookkeeping (status, agent→task map, MCP token). Removing the
/// handle drops `master`/`writer`, closing the PTY so the streaming thread sees
/// EOF. Shared by the `stop_session` command and the concierge idle reaper.
pub fn stop_session_inner(state: &AppState, session_id: &str) -> Result<(), String> {
    let handle = {
        let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
        sessions
            .remove(session_id)
            .ok_or_else(|| format!("session not found: {session_id}"))?
    };

    // Ignore kill errors: the process may have already exited on its own.
    let _ = handle.child.lock().map_err(|e| e.to_string())?.kill();

    // Drop status-machine bookkeeping (no-op for shells/concierge).
    let _ = state.agent_states.lock().map(|mut m| m.remove(session_id));
    let _ = state.agent_tasks.lock().map(|mut m| m.remove(session_id));

    // Drop this session's MCP token so its scope is gone.
    if let Some(token) = handle.mcp_token.as_ref() {
        let _ = state.mcp_tokens.lock().map(|mut m| m.remove(token));
    }

    Ok(())
}

/// Stop the session: kill its process and drop its PTY handles (see
/// `stop_session_inner`).
#[tauri::command]
pub fn stop_session(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    stop_session_inner(state.inner(), &session_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    static SHELL_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn resolve_shell_uses_shell_env_when_set() {
        let _guard = SHELL_TEST_LOCK.lock().unwrap();
        std::env::set_var("SHELL", "/bin/bash");
        let (program, args) = resolve_shell();
        assert_eq!(program, "/bin/bash");
        assert_eq!(args, vec!["-l".to_string()]);
        std::env::remove_var("SHELL");
    }

    #[test]
    fn resolve_shell_falls_back_when_env_missing() {
        let _guard = SHELL_TEST_LOCK.lock().unwrap();
        std::env::remove_var("SHELL");
        let (program, _args) = resolve_shell();
        assert_eq!(program, "/bin/zsh");
    }

    #[test]
    fn apply_lavigie_env_marks_process_and_forces_hyperlinks() {
        let mut cmd = CommandBuilder::new("true");
        apply_lavigie_env(&mut cmd);
        assert_eq!(cmd.get_env("LAVIGIE"), Some(std::ffi::OsStr::new("1")));
        // FORCE_HYPERLINK=1 makes hyperlink-gating CLIs (e.g. Claude Code) emit
        // OSC 8 links our xterm.js terminal can render clickably (TASK-170).
        assert_eq!(
            cmd.get_env("FORCE_HYPERLINK"),
            Some(std::ffi::OsStr::new("1"))
        );
    }

    fn read_to_end(mut reader: Box<dyn Read + Send>) -> Vec<u8> {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).expect("read_to_end failed");
        buf
    }

    #[test]
    fn spawn_pty_runs_program_and_captures_stdout() {
        let mut session = spawn_pty(
            "/bin/sh",
            &["-c".to_string(), "printf hello".to_string()],
            None,
            80,
            24,
            &[],
        )
        .expect("spawn_pty failed");

        let reader = session.reader.take().expect("reader already taken");
        let output = read_to_end(reader);
        let output = String::from_utf8_lossy(&output);

        assert!(
            output.contains("hello"),
            "expected output to contain 'hello', got: {output:?}"
        );
    }

    #[test]
    fn spawn_pty_propagates_force_hyperlink_to_child() {
        // Runtime check that FORCE_HYPERLINK=1 actually reaches the spawned
        // process (not just the CommandBuilder) — this is what makes Claude Code
        // emit OSC 8 footer links our terminal can render clickably (TASK-170).
        let mut session = spawn_pty(
            "/bin/sh",
            &["-c".to_string(), "printf %s \"$FORCE_HYPERLINK\"".to_string()],
            None,
            80,
            24,
            &[],
        )
        .expect("spawn_pty failed");

        let reader = session.reader.take().expect("reader already taken");
        let output = read_to_end(reader);
        let output = String::from_utf8_lossy(&output);

        assert!(
            output.contains('1'),
            "expected child to see FORCE_HYPERLINK=1, got: {output:?}"
        );
    }

    #[test]
    fn spawn_pty_honors_cwd() {
        let tmp = tempfile::TempDir::new().expect("tempdir failed");

        let mut session = spawn_pty(
            "/bin/sh",
            &["-c".to_string(), "pwd".to_string()],
            Some(tmp.path()),
            80,
            24,
            &[],
        )
        .expect("spawn_pty failed");

        let reader = session.reader.take().expect("reader already taken");
        let output = read_to_end(reader);
        let output = String::from_utf8_lossy(&output);

        // Canonicalize to handle e.g. /tmp -> /private/tmp symlinks on macOS.
        let expected = tmp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| tmp.path().to_path_buf());
        let expected = expected.to_string_lossy().to_string();

        assert!(
            output.contains(&expected) || output.trim().contains(tmp.path().to_str().unwrap()),
            "expected output to contain cwd {expected:?}, got: {output:?}"
        );
    }

    #[test]
    fn spawn_pty_round_trips_input_through_cat() {
        let mut session =
            spawn_pty("/bin/cat", &[], None, 80, 24, &[]).expect("spawn_pty failed");

        let mut reader = session.reader.take().expect("reader already taken");
        session
            .writer
            .write_all(b"ping\n")
            .expect("write_all failed");
        // Let cat echo the line back, then close stdin so cat exits and the
        // reader hits EOF.
        drop(session.writer);

        let mut buf = [0u8; 1024];
        let n = reader.read(&mut buf).expect("read failed");
        let output = String::from_utf8_lossy(&buf[..n]);

        assert!(
            output.contains("ping"),
            "expected echoed output to contain 'ping', got: {output:?}"
        );
    }

    #[test]
    fn spawn_pty_sets_extra_env_on_child() {
        let mut session = spawn_pty(
            "/bin/sh",
            &["-c".to_string(), "printf %s \"$LAVIGIE_AGENT_ID\"".to_string()],
            None,
            80,
            24,
            &[("LAVIGIE_AGENT_ID", "agent-xyz".to_string())],
        )
        .expect("spawn_pty failed");

        let reader = session.reader.take().expect("reader already taken");
        let output = read_to_end(reader);
        let output = String::from_utf8_lossy(&output);
        assert!(
            output.contains("agent-xyz"),
            "expected child to see LAVIGIE_AGENT_ID, got: {output:?}"
        );
    }

    // ── lavigie_task_ref_env (TASK-174) ────────────────────────────────────────

    #[test]
    fn lavigie_task_ref_env_present_when_keyed_absent_when_not() {
        // Present verbatim for every provider ID shape (ticket keys + GitHub issues).
        for key in ["TASK-174", "#123", "owner/repo#123"] {
            assert_eq!(
                lavigie_task_ref_env(Some(key)),
                Some(("LAVIGIE_TASK_REF", key.to_string())),
                "expected LAVIGIE_TASK_REF for {key:?}"
            );
        }
        // Absent when there is no ticket key, or it is blank after trimming.
        assert_eq!(lavigie_task_ref_env(None), None);
        assert_eq!(lavigie_task_ref_env(Some("")), None);
        assert_eq!(lavigie_task_ref_env(Some("   ")), None);
    }

    // ── build_hook_settings ───────────────────────────────────────────────────

    #[test]
    fn build_hook_settings_is_valid_json_with_all_event_keys() {
        let json_str = build_hook_settings(54321, "test-agent");
        let v: serde_json::Value =
            serde_json::from_str(&json_str).expect("must be valid JSON");

        let hooks = v["hooks"].as_object().expect("hooks must be an object");
        // TASK-85 adds SubagentStart/SubagentStop to the existing five.
        for key in &[
            "UserPromptSubmit",
            "Notification",
            "Stop",
            "StopFailure",
            "PreToolUse",
            "SubagentStart",
            "SubagentStop",
        ] {
            assert!(
                hooks.contains_key(*key),
                "missing hook key: {key}"
            );
        }
    }

    #[test]
    fn build_hook_settings_commands_contain_port_and_agent_id() {
        let json_str = build_hook_settings(12345, "my-agent-42");
        let v: serde_json::Value =
            serde_json::from_str(&json_str).expect("must be valid JSON");

        let expected_url = "http://127.0.0.1:12345/hook/my-agent-42";

        let hooks = v["hooks"].as_object().unwrap();
        for (key, val) in hooks {
            let command = val[0]["hooks"][0]["command"]
                .as_str()
                .unwrap_or_else(|| panic!("command not a string for key {key}"));
            assert!(
                command.contains(expected_url),
                "command for {key} missing URL {expected_url:?}, got: {command:?}"
            );
        }
    }

    #[test]
    fn build_hook_settings_resume_still_works_conceptually() {
        // Verify that build_hook_settings does not panic with port 0 or empty id.
        let json_str = build_hook_settings(0, "");
        let v: serde_json::Value = serde_json::from_str(&json_str).expect("must be valid JSON");
        assert!(v["hooks"].is_object());
    }

    #[test]
    fn build_hook_settings_includes_pretooluse_and_statusline() {
        let json_str = build_hook_settings(8080, "agent-x");
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(v["hooks"]["PreToolUse"].is_array(), "PreToolUse hook missing");
        let cmd = v["statusLine"]["command"].as_str().expect("statusLine.command");
        assert_eq!(v["statusLine"]["type"], "command");
        assert!(cmd.contains("http://127.0.0.1:8080/status/agent-x"), "statusLine must POST to /status: {cmd}");
    }

    #[test]
    fn spawned_command_sets_lavigie_env() {
        let mut cmd = CommandBuilder::new("claude");
        apply_lavigie_env(&mut cmd);
        let found = cmd
            .iter_extra_env_as_str()
            .any(|(k, v)| k == "LAVIGIE" && v == "1");
        assert!(found, "spawned command must set LAVIGIE=1");
    }

    #[test]
    fn build_mcp_config_contains_port_token_and_http_type() {
        let cfg = build_mcp_config(54321, "tok-abc");
        let v: serde_json::Value = serde_json::from_str(&cfg).expect("valid JSON");
        let server = &v["mcpServers"]["lavigie"];
        assert_eq!(server["type"], "http");
        assert_eq!(server["url"], "http://127.0.0.1:54321/mcp");
        assert_eq!(server["headers"]["Authorization"], "Bearer tok-abc");
    }
}
