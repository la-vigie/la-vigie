//! Tauri command glue for the pluggable agent runtime: listing agents,
//! custom-agent CRUD, and per-task / per-repo agent selection. Thin glue over
//! the store and the pure registry in `agent::spec`; not unit-tested (needs a
//! running app), per project convention.

use tauri::State;

use crate::agent::spec::{builtin_specs, AgentSpec, StatusMechanism};
use crate::state::AppState;

/// All selectable agents: built-in presets first, then custom definitions.
#[tauri::command]
pub fn list_agents(state: State<'_, AppState>) -> Result<Vec<AgentSpec>, String> {
    agents_list(state.inner())
}

/// Core for `list_agents`, taking `&AppState` so the remote server's
/// `GET /api/agents` reuses the exact same list.
pub fn agents_list(state: &AppState) -> Result<Vec<AgentSpec>, String> {
    let custom = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store.list_custom_agents().map_err(|e| format!("{e:#}"))?
    };
    let mut out = builtin_specs();
    out.extend(custom);
    Ok(out)
}

/// Create or update a custom agent. Custom agents are always lifecycle-only and
/// non-builtin, and may not reuse a built-in name.
#[tauri::command]
pub fn upsert_custom_agent(state: State<'_, AppState>, spec: AgentSpec) -> Result<(), String> {
    let name = spec.name.trim().to_string();
    if name.is_empty() {
        return Err("agent name is required".to_string());
    }
    if builtin_specs().iter().any(|b| b.name == name) {
        return Err(format!("'{name}' is a built-in agent name"));
    }
    let normalized = AgentSpec {
        name,
        builtin: false,
        status: StatusMechanism::Lifecycle,
        skill_injection: crate::agent::spec::SkillInjection::None,
        // Generic user-defined ACP engines ride the same `ExecutionMode::Acp`
        // machinery in principle, but are unwired/untested in v1 (see the ACP
        // backend engine design doc) — custom agents stay PTY-only for now.
        execution: crate::agent::spec::ExecutionMode::Pty,
        ..spec
    };
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.upsert_agent(&normalized).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn delete_custom_agent(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.delete_agent(&name).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn set_task_agent(
    state: State<'_, AppState>,
    task_id: String,
    agent: Option<String>,
) -> Result<(), String> {
    let agent = agent.map(|a| a.trim().to_string()).filter(|a| !a.is_empty());
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .set_task_agent(&task_id, agent.as_deref())
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn set_task_model(
    state: State<'_, AppState>,
    task_id: String,
    model: Option<String>,
) -> Result<(), String> {
    let model = model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.set_task_model(&task_id, model.as_deref()).map_err(|e| format!("{e:#}"))
}

/// Set (or clear) a task's auto-approve override. `None` ⇒ inherit the repo.
#[tauri::command]
pub fn set_task_auto_approve(
    state: State<'_, AppState>,
    task_id: String,
    auto_approve: Option<bool>,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .set_task_auto_approve(&task_id, auto_approve)
        .map_err(|e| format!("{e:#}"))
}

/// Hard ceiling on the model-enumeration round-trip. Bounds a slow/hung engine
/// CLI so it can never wedge the caller — critically the desktop UI thread,
/// which awaits this command when the picker's Model pane opens. Past the
/// deadline we surface an error and the frontend degrades to the "Default
/// model" row rather than hanging. A `models` listing normally returns in well
/// under a second; this is the stuck-CLI cap.
const MODELS_LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Enumerate the models the named agent advertises (empty when it advertises
/// none). Shells the agent's `models_list_args` via the resolved binary; argv
/// only (no shell). Used to populate the Model pane in the picker.
///
/// `async` on purpose: a synchronous Tauri command runs on the main (UI) thread,
/// so the blocking subprocess froze the whole WebView until the engine CLI
/// returned. As an `async fn` it runs on the async runtime instead, and the
/// enumeration is spawned off-thread + timeout-bounded below.
#[tauri::command]
pub async fn list_agent_models(
    state: State<'_, AppState>,
    agent_name: String,
) -> Result<Vec<String>, String> {
    agent_models(state.inner(), &agent_name).await
}

/// Core for `list_agent_models`, taking `&AppState` so the remote server's
/// `GET /api/agents/{name}/models` reuses the exact same enumeration.
pub async fn agent_models(state: &AppState, agent_name: &str) -> Result<Vec<String>, String> {
    use crate::agent::spec::resolve_agent;
    // Resolve the spec under a short store lock, dropped before any `.await`
    // (never hold the store Mutex across an await — project invariant).
    let (bin, list_args, provider_slash_ids_only) = {
        let custom = {
            let store = state.store.lock().map_err(|e| e.to_string())?;
            store.list_custom_agents().map_err(|e| format!("{e:#}"))?
        };
        let spec = match resolve_agent(agent_name, &custom) {
            Some(s) => s,
            None => return Ok(vec![]),
        };
        let Some(list_args) = spec.models_list_args else { return Ok(vec![]); };
        let bin = crate::claude_path::find_binary(&spec.binary);
        // opencode's `models` output is `provider/model` ids mixed with
        // banner/help lines to filter out; every other enumerable builtin
        // (currently just antigravity's `agy models`) prints one plain display
        // label per line with nothing else, so no filtering is safe to apply.
        (bin, list_args, spec.name == "opencode")
    };
    // Shell the enumeration off the caller's thread (`tokio::process`), bounded
    // by `MODELS_LIST_TIMEOUT`. `kill_on_drop` reaps the child if we time out.
    let run = tokio::process::Command::new(&bin)
        .args(&list_args)
        .kill_on_drop(true)
        .output();
    let output = match tokio::time::timeout(MODELS_LIST_TIMEOUT, run).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(format!("running {} {list_args:?}: {e:#}", bin.display())),
        Err(_) => {
            return Err(format!(
                "{} {list_args:?} timed out after {}s",
                bin.display(),
                MODELS_LIST_TIMEOUT.as_secs()
            ))
        }
    };
    if !output.status.success() {
        return Err(format!(
            "{} {list_args:?} failed: {}",
            bin.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(crate::agent::models::parse_model_ids(&String::from_utf8_lossy(&output.stdout), provider_slash_ids_only))
}

#[tauri::command]
pub fn set_repo_default_model(
    state: State<'_, AppState>,
    repo_id: String,
    model: Option<String>,
) -> Result<(), String> {
    let model = model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .set_repo_default_model(&repo_id, model.as_deref())
        .map_err(|e| format!("{e:#}"))
}

/// Set (or clear) a repo's auto-routing policy. A blank/`None` value
/// clears it. A non-blank value must parse as a `RoutingPolicy` — an invalid
/// policy is rejected with an error (not silently dropped) so the settings UI
/// can surface it; the canonicalised JSON is what gets stored.
#[tauri::command]
pub fn set_repo_routing_policy(
    state: State<'_, AppState>,
    repo_id: String,
    policy: Option<String>,
) -> Result<(), String> {
    let trimmed = policy.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    let canonical = match trimmed {
        None => None,
        Some(raw) => {
            let parsed: crate::agent::routing::RoutingPolicy = serde_json::from_str(&raw)
                .map_err(|e| format!("invalid routing policy: {e}"))?;
            Some(serde_json::to_string(&parsed).map_err(|e| format!("{e}"))?)
        }
    };
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .set_repo_routing_policy(&repo_id, canonical.as_deref())
        .map_err(|e| format!("{e:#}"))
}
