//! Recurring-schedule engine (TASK-173): pure cron math plus the background
//! poller that fires due schedules. Timing is local-time (so `0 7 * * 1`
//! means Monday 07:00 local).

use chrono::{DateTime, Local};
use croner::Cron;

/// Validate a cron expression (5/6-field standard cron). Used at create/update
/// time so an unparseable expression is rejected before it is stored.
pub fn validate_cron(expr: &str) -> Result<(), String> {
    expr.parse::<Cron>()
        .map(|_| ())
        .map_err(|e| format!("invalid cron expression: {e}"))
}

/// The next occurrence of `expr` strictly after `after`, as unix seconds.
/// `Ok(None)` if the expression has no upcoming occurrence; `Err` if it is
/// unparseable. Strict (`inclusive = false`) so a schedule sitting exactly on
/// its due minute advances to the *next* slot — collapsing every slot missed
/// while the app was closed into a single catch-up fire.
pub fn next_run_after(expr: &str, after: DateTime<Local>) -> Result<Option<i64>, String> {
    let cron = expr
        .parse::<Cron>()
        .map_err(|e| format!("invalid cron expression: {e}"))?;
    match cron.find_next_occurrence(&after, false) {
        Ok(dt) => Ok(Some(dt.timestamp())),
        Err(_) => Ok(None),
    }
}

/// Trim and drop-if-empty an optional string field (agent/model/base_branch).
pub fn normalize_opt(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// The normalized+validated fields a schedule create/update shares across the
/// Tauri and MCP surfaces (TASK-178). Single source of truth so the two paths
/// can't drift on trims, empty-checks, or cron validation.
#[derive(Debug)]
pub struct ScheduleFields {
    pub name: String,
    pub prompt: String,
    pub cron: String,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub base_branch: Option<String>,
}

/// Trim name/prompt/cron, reject empty name/prompt, validate the cron, and
/// normalize the optional fields. Errors are user-facing strings.
pub fn validate_schedule_fields(
    name: &str,
    prompt: &str,
    cron: &str,
    agent: Option<String>,
    model: Option<String>,
    base_branch: Option<String>,
) -> Result<ScheduleFields, String> {
    let name = name.trim().to_string();
    let prompt = prompt.trim().to_string();
    let cron = cron.trim().to_string();
    if name.is_empty() {
        return Err("Schedule name cannot be empty.".to_string());
    }
    if prompt.is_empty() {
        return Err("Schedule prompt cannot be empty.".to_string());
    }
    validate_cron(&cron)?;
    Ok(ScheduleFields {
        name,
        prompt,
        cron,
        agent: normalize_opt(agent),
        model: normalize_opt(model),
        base_branch: normalize_opt(base_branch),
    })
}

/// Resolve a one-shot's absolute fire time (unix seconds) from an optional
/// relative offset and/or an optional absolute time. An absolute `at_unix`
/// wins when present; otherwise `now + in_seconds`. A negative offset is
/// rejected; supplying neither is an error. A past `at_unix` is allowed — it
/// simply fires on the next poll (the catch-up path). TASK-179.
pub fn resolve_fire_at(
    now: i64,
    in_seconds: Option<i64>,
    at_unix: Option<i64>,
) -> Result<i64, String> {
    if let Some(at) = at_unix {
        return Ok(at);
    }
    match in_seconds {
        Some(secs) if secs < 0 => Err("delay must be non-negative".to_string()),
        Some(secs) => now
            .checked_add(secs)
            .ok_or_else(|| "delay is too far in the future".to_string()),
        None => Err("provide a delay (inSeconds/inHours) or an absolute time".to_string()),
    }
}

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::launch::LaunchArgs;
use crate::state::AppState;
use crate::store::Schedule;

/// Current unix time in seconds.
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Public wrapper around `now_secs` for the command layer.
pub fn now_secs_pub() -> i64 {
    now_secs()
}

/// Spawn the background schedule poller: every 60s, fire any due schedules.
/// Runs for the app's lifetime on the Tauri runtime. Mirrors
/// `concierge::spawn_reaper`. TASK-173.
pub fn spawn_scheduler(app: tauri::AppHandle) {
    use tauri::Manager as _;
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let state = app.state::<AppState>();
            tick(state.inner(), &app).await;
        }
    });
}

/// One poll: claim every due schedule UNDER the store lock, drop the lock,
/// then launch each claimed schedule. Recurring schedules are claimed by
/// advancing `next_run_at` to the next occurrence strictly after now
/// (stamping `last_run_at`); one-shot schedules (TASK-179) are retired instead
/// (disabled, `next_run_at` cleared, `last_run_at` stamped) since there is no
/// next occurrence. A schedule is launched only after its claim persists: if
/// the claim call fails, the schedule is skipped this tick (logged) rather
/// than launched, so a failed claim can't double-fire on the next tick.
/// Claiming before launching makes a slow launch unable to double-fire on the
/// next tick; per-schedule errors are isolated (logged, never propagated).
async fn tick(state: &AppState, app: &tauri::AppHandle) {
    let now = now_secs();

    // Phase 1: select + claim under the lock (no `.await` while held).
    let claimed: Vec<Schedule> = {
        let store = match state.store.lock() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("scheduler: store lock poisoned: {e}");
                return;
            }
        };
        let due = match store.due_schedules(now) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("scheduler: due_schedules failed: {e:#}");
                return;
            }
        };
        let mut claimed = Vec::with_capacity(due.len());
        for s in due {
            let claim = if s.one_shot {
                // One-shot: fire once, then retire (no next occurrence).
                store.retire_schedule(&s.id, now, now)
            } else {
                let next = next_run_after(&s.cron, Local::now()).ok().flatten();
                store.advance_schedule(&s.id, next, now, now)
            };
            match claim {
                Ok(()) => claimed.push(s),
                Err(e) => eprintln!("scheduler: claim {} failed: {e:#}", s.id),
            }
        }
        claimed
    };

    // Phase 2: launch each claimed schedule (lock released).
    for s in claimed {
        if let Err(e) = launch_scheduled_run(state, app, &s).await {
            eprintln!("scheduler: launch for schedule {} failed: {e}", s.id);
        }
    }
}

/// Launch one fresh worktree-backed task from a schedule and start its agent.
/// Reuses the shared launch core, then emits `task_launched` (the runner has no
/// frontend caller) exactly as the MCP `start_task` path does.
async fn launch_scheduled_run(
    state: &AppState,
    app: &tauri::AppHandle,
    s: &Schedule,
) -> Result<(), String> {
    let args = LaunchArgs {
        repo_id: s.repo_id.clone(),
        title: s.name.clone(),
        base_branch: s.base_branch.clone(),
        ticket_key: None,
        agent: s.agent.clone(),
        model: s.model.clone(),
        after_merge_of: Vec::new(),
        prompt: Some(s.prompt.clone()),
        auto_approve: None,
        // TASK-163: placeholder default — scheduled runs don't launch in-place tasks.
        in_place: false,
        branch_name: None,
    };
    let task = crate::commands::launch_and_kickoff_setup(state, app, args).await?;
    // TASK-181: route the schedule's skip-repo-prompt flag to the frontend so it
    // reuses TASK-160's combineInitialPrompts(null, …) skip path.
    crate::mcp::emit_task_launched(app, task.id, Some(s.prompt.clone()), s.skip_repo_prompt);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn validate_accepts_standard_5_field_cron() {
        assert!(validate_cron("0 7 * * 1").is_ok());
    }

    #[test]
    fn validate_rejects_garbage() {
        assert!(validate_cron("not a cron").is_err());
    }

    #[test]
    fn next_run_after_finds_next_monday_7am() {
        // 2026-07-12 is a Sunday; next "Mon 07:00" is 2026-07-13 07:00 local.
        let after = chrono::Local.with_ymd_and_hms(2026, 7, 12, 9, 0, 0).unwrap();
        let expected = chrono::Local.with_ymd_and_hms(2026, 7, 13, 7, 0, 0).unwrap();
        let got = next_run_after("0 7 * * 1", after).unwrap();
        assert_eq!(got, Some(expected.timestamp()));
    }

    #[test]
    fn next_run_after_is_strictly_after() {
        // `after` sits exactly on a matching minute — must return the NEXT one,
        // not the same instant (this is what gives catch-up-once its single fire).
        let after = chrono::Local.with_ymd_and_hms(2026, 7, 13, 7, 0, 0).unwrap();
        let got = next_run_after("0 7 * * 1", after).unwrap();
        assert!(got.unwrap() > after.timestamp());
    }

    #[test]
    fn next_run_after_errors_on_bad_expr() {
        let after = chrono::Local.with_ymd_and_hms(2026, 7, 12, 9, 0, 0).unwrap();
        assert!(next_run_after("nope", after).is_err());
    }

    #[test]
    fn validate_schedule_fields_trims_and_normalizes() {
        let f = validate_schedule_fields(
            "  nightly  ", "  /security-scan  ", "  0 2 * * *  ",
            Some("  claude  ".into()), Some("   ".into()), None,
        )
        .expect("valid");
        assert_eq!(f.name, "nightly");
        assert_eq!(f.prompt, "/security-scan");
        assert_eq!(f.cron, "0 2 * * *");
        assert_eq!(f.agent.as_deref(), Some("claude"));
        assert_eq!(f.model, None); // whitespace-only → None
        assert_eq!(f.base_branch, None);
    }

    #[test]
    fn validate_schedule_fields_rejects_empty_name_and_prompt() {
        assert!(validate_schedule_fields("  ", "p", "0 2 * * *", None, None, None)
            .unwrap_err()
            .contains("name cannot be empty"));
        assert!(validate_schedule_fields("n", "  ", "0 2 * * *", None, None, None)
            .unwrap_err()
            .contains("prompt cannot be empty"));
    }

    #[test]
    fn validate_schedule_fields_rejects_bad_cron() {
        assert!(validate_schedule_fields("n", "p", "not a cron", None, None, None)
            .unwrap_err()
            .contains("invalid cron expression"));
    }

    #[test]
    fn normalize_opt_drops_blank() {
        assert_eq!(normalize_opt(Some("  x ".into())).as_deref(), Some("x"));
        assert_eq!(normalize_opt(Some("   ".into())), None);
        assert_eq!(normalize_opt(None), None);
    }

    #[test]
    fn resolve_fire_at_uses_absolute_when_present() {
        assert_eq!(resolve_fire_at(1_000, Some(60), Some(5_000)), Ok(5_000));
    }

    #[test]
    fn resolve_fire_at_adds_relative_offset() {
        assert_eq!(resolve_fire_at(1_000, Some(3_600), None), Ok(4_600));
    }

    #[test]
    fn resolve_fire_at_allows_past_absolute() {
        assert_eq!(resolve_fire_at(1_000, None, Some(10)), Ok(10));
    }

    #[test]
    fn resolve_fire_at_rejects_negative_offset() {
        assert!(resolve_fire_at(1_000, Some(-5), None).is_err());
    }

    #[test]
    fn resolve_fire_at_requires_one_input() {
        assert!(resolve_fire_at(1_000, None, None).is_err());
    }

    #[test]
    fn resolve_fire_at_rejects_overflowing_offset() {
        assert!(resolve_fire_at(i64::MAX, Some(1), None).is_err());
    }
}
