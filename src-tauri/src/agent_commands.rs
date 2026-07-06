//! Tauri command glue for the pluggable agent runtime (AC2-21): listing agents,
//! custom-agent CRUD, and per-task / per-repo agent selection. Thin glue over
//! the store and the pure registry in `agent::spec`; not unit-tested (needs a
//! running app), per project convention.

use tauri::State;

use crate::agent::spec::{builtin_specs, AgentSpec, StatusMechanism};
use crate::state::AppState;

/// All selectable agents: built-in presets first, then custom definitions.
#[tauri::command]
pub fn list_agents(state: State<'_, AppState>) -> Result<Vec<AgentSpec>, String> {
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

/// Enumerate the models the named agent advertises (empty when it advertises
/// none). Shells the agent's `models_list_args` via the resolved binary; argv
/// only (no shell). Used to populate the Model pane in the picker.
#[tauri::command]
pub fn list_agent_models(
    state: State<'_, AppState>,
    agent_name: String,
) -> Result<Vec<String>, String> {
    use crate::agent::spec::resolve_agent;
    let custom = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store.list_custom_agents().map_err(|e| format!("{e:#}"))?
    };
    let spec = match resolve_agent(&agent_name, &custom) {
        Some(s) => s,
        None => return Ok(vec![]),
    };
    let Some(list_args) = spec.models_list_args else { return Ok(vec![]); };
    let bin = crate::claude_path::find_binary(&spec.binary);
    let output = std::process::Command::new(&bin)
        .args(&list_args)
        .output()
        .map_err(|e| format!("running {} {list_args:?}: {e:#}", bin.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} {list_args:?} failed: {}",
            bin.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(crate::agent::models::parse_model_ids(&String::from_utf8_lossy(&output.stdout)))
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
