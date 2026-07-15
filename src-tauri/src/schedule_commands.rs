//! Tauri commands for managing recurring schedules (TASK-173). Thin glue over the
//! store CRUD + the pure cron engine; validation lives in `schedule`.

use tauri::State;

use crate::schedule::{next_run_after, normalize_opt, now_secs_pub, validate_schedule_fields};
use crate::state::AppState;
use crate::store::Schedule;

/// Compute `next_run_at` from a validated cron, as of now (local time).
fn compute_next(cron: &str) -> Result<Option<i64>, String> {
    next_run_after(cron, chrono::Local::now())
}

/// Core: list a repo's schedules. Shared by the Tauri command and the remote
/// axum handler (TASK-196) so both go through one store path.
pub fn list_schedules_core(state: &AppState, repo_id: &str) -> Result<Vec<Schedule>, String> {
    let store = state.store.lock().map_err(|e| format!("{e}"))?;
    store.list_schedules(repo_id).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn list_schedules(state: State<'_, AppState>, repo_id: String) -> Result<Vec<Schedule>, String> {
    list_schedules_core(state.inner(), &repo_id)
}

/// Core: create a recurring schedule. Validates fields + computes the next fire
/// via the shared cron engine. Shared by the command and the remote handler (TASK-196).
#[allow(clippy::too_many_arguments)]
pub fn create_schedule_core(
    state: &AppState,
    repo_id: String,
    name: String,
    prompt: String,
    cron: String,
    agent: Option<String>,
    model: Option<String>,
    base_branch: Option<String>,
    // TASK-181: skip prepending the repo's initial prompt when this schedule fires.
    // Defaults to `true` when the caller omits it (frontend always passes it).
    skip_repo_prompt: Option<bool>,
) -> Result<Schedule, String> {
    let fields = validate_schedule_fields(&name, &prompt, &cron, agent, model, base_branch)?;

    let now = now_secs_pub();
    let schedule = Schedule {
        id: uuid::Uuid::new_v4().to_string(),
        repo_id,
        name: fields.name,
        prompt: fields.prompt,
        cron: fields.cron.clone(),
        agent: fields.agent,
        model: fields.model,
        base_branch: fields.base_branch,
        enabled: true,
        one_shot: false,
        skip_repo_prompt: skip_repo_prompt.unwrap_or(true),
        next_run_at: compute_next(&fields.cron)?,
        last_run_at: None,
        created_at: now,
        updated_at: now,
    };

    let store = state.store.lock().map_err(|e| format!("{e}"))?;
    store.insert_schedule(&schedule).map_err(|e| format!("{e:#}"))?;
    Ok(schedule)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn create_schedule(
    state: State<'_, AppState>,
    repo_id: String,
    name: String,
    prompt: String,
    cron: String,
    agent: Option<String>,
    model: Option<String>,
    base_branch: Option<String>,
    skip_repo_prompt: Option<bool>,
) -> Result<Schedule, String> {
    create_schedule_core(
        state.inner(), repo_id, name, prompt, cron, agent, model, base_branch, skip_repo_prompt,
    )
}

/// Core: create a one-time (non-recurring) schedule that fires once at an absolute
/// time (`at_unix`) or after a relative delay (`in_seconds`), then retires.
/// One-shots carry an empty `cron` (never cron-parsed). TASK-179. Shared by the
/// command and the remote handler (TASK-196).
#[allow(clippy::too_many_arguments)]
pub fn create_one_shot_core(
    state: &AppState,
    repo_id: String,
    name: String,
    prompt: String,
    in_seconds: Option<i64>,
    at_unix: Option<i64>,
    agent: Option<String>,
    model: Option<String>,
    base_branch: Option<String>,
    // TASK-181: skip the repo initial prompt on fire; defaults to `true` when omitted.
    skip_repo_prompt: Option<bool>,
) -> Result<Schedule, String> {
    let name = name.trim().to_string();
    let prompt = prompt.trim().to_string();
    if name.is_empty() {
        return Err("Schedule name cannot be empty.".to_string());
    }
    let now = now_secs_pub();
    let fire_at = crate::schedule::resolve_fire_at(now, in_seconds, at_unix)?;

    let schedule = Schedule {
        id: uuid::Uuid::new_v4().to_string(),
        repo_id,
        name,
        prompt,
        cron: String::new(),
        agent: normalize_opt(agent),
        model: normalize_opt(model),
        base_branch: normalize_opt(base_branch),
        enabled: true,
        one_shot: true,
        skip_repo_prompt: skip_repo_prompt.unwrap_or(true),
        next_run_at: Some(fire_at),
        last_run_at: None,
        created_at: now,
        updated_at: now,
    };

    let store = state.store.lock().map_err(|e| format!("{e}"))?;
    store.insert_schedule(&schedule).map_err(|e| format!("{e:#}"))?;
    Ok(schedule)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn create_one_shot_schedule(
    state: State<'_, AppState>,
    repo_id: String,
    name: String,
    prompt: String,
    in_seconds: Option<i64>,
    at_unix: Option<i64>,
    agent: Option<String>,
    model: Option<String>,
    base_branch: Option<String>,
    skip_repo_prompt: Option<bool>,
) -> Result<Schedule, String> {
    create_one_shot_core(
        state.inner(), repo_id, name, prompt, in_seconds, at_unix, agent, model, base_branch,
        skip_repo_prompt,
    )
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn update_schedule(
    state: State<'_, AppState>,
    id: String,
    name: String,
    prompt: String,
    cron: String,
    agent: Option<String>,
    model: Option<String>,
    base_branch: Option<String>,
    enabled: bool,
    // TASK-181: skip the repo initial prompt on fire; defaults to `true` when omitted.
    skip_repo_prompt: Option<bool>,
) -> Result<Schedule, String> {
    let fields = validate_schedule_fields(&name, &prompt, &cron, agent, model, base_branch)?;
    // Disabled ⇒ never fires; enabled ⇒ recompute the next fire from now.
    let next_run_at = if enabled { compute_next(&fields.cron)? } else { None };
    let now = now_secs_pub();

    let store = state.store.lock().map_err(|e| format!("{e}"))?;
    store
        .update_schedule_fields(
            &id, &fields.name, &fields.prompt, &fields.cron,
            fields.agent.as_deref(), fields.model.as_deref(), fields.base_branch.as_deref(),
            enabled, skip_repo_prompt.unwrap_or(true), next_run_at, now,
        )
        .map_err(|e| format!("{e:#}"))?;
    store
        .get_schedule(&id)
        .map_err(|e| format!("{e:#}"))?
        .ok_or_else(|| "schedule not found after update".to_string())
}

#[tauri::command]
pub fn set_schedule_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<Schedule, String> {
    set_schedule_enabled_core(state.inner(), id, enabled)
}

/// Core: arm/disarm a schedule. Recurring schedules recompute their next fire from
/// cron; one-shots keep their absolute fire time (the enabled flag alone arms them).
/// Shared by the command and the remote handler (TASK-196).
pub fn set_schedule_enabled_core(
    state: &AppState,
    id: String,
    enabled: bool,
) -> Result<Schedule, String> {
    let store = state.store.lock().map_err(|e| format!("{e}"))?;
    let current = store
        .get_schedule(&id)
        .map_err(|e| format!("{e:#}"))?
        .ok_or_else(|| "schedule not found".to_string())?;
    // One-shots keep their absolute fire time regardless of enable state — the
    // enabled flag alone arms/disarms them (due_schedules also checks enabled=1),
    // so disabling then re-enabling a pending one-shot re-arms it rather than
    // permanently cancelling it. Recurring schedules recompute next fire from cron.
    let next_run_at = if current.one_shot {
        current.next_run_at
    } else if !enabled {
        None
    } else {
        compute_next(&current.cron)?
    };
    let now = now_secs_pub();
    store
        .update_schedule_fields(
            &id, &current.name, &current.prompt, &current.cron,
            current.agent.as_deref(), current.model.as_deref(), current.base_branch.as_deref(),
            enabled, current.skip_repo_prompt, next_run_at, now,
        )
        .map_err(|e| format!("{e:#}"))?;
    store
        .get_schedule(&id)
        .map_err(|e| format!("{e:#}"))?
        .ok_or_else(|| "schedule not found after update".to_string())
}

/// Core: delete a schedule (idempotent at the store level). Shared by the command
/// and the remote handler (TASK-196).
pub fn delete_schedule_core(state: &AppState, id: &str) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| format!("{e}"))?;
    store.delete_schedule(id).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn delete_schedule(state: State<'_, AppState>, id: String) -> Result<(), String> {
    delete_schedule_core(state.inner(), &id)
}

/// Validate a cron and return its next fire time (unix seconds) for a live UI
/// preview. Errors if the expression is unparseable or has no upcoming run.
#[tauri::command]
pub fn preview_next_run(cron: String) -> Result<i64, String> {
    match next_run_after(cron.trim(), chrono::Local::now())? {
        Some(ts) => Ok(ts),
        None => Err("cron has no upcoming occurrence".to_string()),
    }
}
