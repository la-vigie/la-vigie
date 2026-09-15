//! Pure aggregation of persisted ACP usage samples into a per-model / per-task
//! spend + rate-limit summary — the "what did this week cost, by model?" view.
//! No I/O: `store::acp_usage_events_since` fetches the raw rows, this collapses
//! them, and the Tauri command (`acp/mod.rs`) glues the two.
//!
//! The subtlety: `cost_amount` on each row is the session's **cumulative** cost
//! (monotonic), not a per-event delta, so SUMming raw rows would multiply a
//! session's cost by its sample count. Collapse to one record per session
//! (latest cumulative cost, peak context gauge) first, then group. A session
//! that switches models mid-run is attributed wholesale to its latest model —
//! ACP exposes only one cumulative session cost, not per-model segments.
//!
//! Because the cost is whole-session cumulative, a window counts a session's
//! *entire* lifetime cost as long as one sample falls inside it — a session
//! that started before the window still contributes its full cost. ACP gives no
//! pre-window baseline to subtract, so the window bounds which sessions count,
//! not how much of each session's cost accrued within it.

use std::collections::BTreeMap;

use crate::store::AcpUsageEvent;

/// Per-(provider, model) rollup over the window.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub provider: String,
    /// `None` when the agent exposes no model selector (provider-only attribution).
    pub model: Option<String>,
    /// Summed cumulative cost across the sessions in this group.
    pub cost: f64,
    pub currency: Option<String>,
    /// Peak context-window occupancy seen (a gauge, not cumulative tokens).
    pub context_peak: i64,
    /// Context-window size (tokens) for this model, if seen.
    pub context_size: i64,
    /// Number of distinct sessions rolled up here.
    pub sessions: u64,
    /// Most-recent best-effort rate-limit status seen for this group.
    pub rate_status: Option<String>,
}

/// Per-task spend rollup over the window.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskUsage {
    pub task_id: String,
    pub cost: f64,
    pub currency: Option<String>,
}

/// The full summary returned to the frontend Usage panel.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpUsageSummary {
    /// Sum of every session's cumulative cost in the window. Mixed currencies
    /// are summed naively (a local single-user tool is effectively single
    /// currency); `currency` names the first one seen.
    pub total_cost: f64,
    pub currency: Option<String>,
    /// Distinct ACP sessions contributing to the window.
    pub sessions: u64,
    pub by_model: Vec<ModelUsage>,
    pub by_task: Vec<TaskUsage>,
}

/// One session collapsed from its many usage samples.
struct SessionAgg {
    provider: String,
    model: Option<String>,
    task_id: String,
    cost: Option<f64>,
    currency: Option<String>,
    peak_used: i64,
    size: i64,
    rate_status: Option<String>,
    last_ts: i64,
}

fn max_cost(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

/// Collapse raw samples to per-session records, then group by model and task.
/// Deterministic (input-ordered via `BTreeMap` + stable sort) so it's
/// unit-testable without a clock.
pub fn summarize_usage(events: &[AcpUsageEvent]) -> AcpUsageSummary {
    // Session key: prefer the ACP session id; fall back to agent_id for the
    // brief pre-`SessionStarted` window where session_id is still null.
    let mut sessions: BTreeMap<String, SessionAgg> = BTreeMap::new();
    for e in events {
        let key = e.session_id.clone().unwrap_or_else(|| format!("agent:{}", e.agent_id));
        let entry = sessions.entry(key).or_insert_with(|| SessionAgg {
            provider: e.provider.clone(),
            model: e.model.clone(),
            task_id: e.task_id.clone(),
            cost: None,
            currency: None,
            peak_used: 0,
            size: 0,
            rate_status: None,
            last_ts: i64::MIN,
        });
        entry.cost = max_cost(entry.cost, e.cost_amount);
        entry.peak_used = entry.peak_used.max(e.used);
        entry.size = entry.size.max(e.size);
        // Latest-sample-wins for the mutable-over-time fields.
        if e.ts >= entry.last_ts {
            entry.last_ts = e.ts;
            entry.provider = e.provider.clone();
            entry.model = e.model.clone();
            entry.task_id = e.task_id.clone();
            if e.currency.is_some() {
                entry.currency = e.currency.clone();
            }
            if e.rate_status.is_some() {
                entry.rate_status = e.rate_status.clone();
            }
        }
    }

    let session_count = sessions.len() as u64;
    let mut total_cost = 0.0_f64;
    let mut summary_currency: Option<String> = None;

    // Group key: (provider, model) for by_model; task_id for by_task.
    let mut by_model: BTreeMap<(String, Option<String>), ModelUsage> = BTreeMap::new();
    let mut by_task: BTreeMap<String, TaskUsage> = BTreeMap::new();
    // Track the newest rate_status per model group.
    let mut model_rate_ts: BTreeMap<(String, Option<String>), i64> = BTreeMap::new();

    for s in sessions.values() {
        let cost = s.cost.unwrap_or(0.0);
        total_cost += cost;
        if summary_currency.is_none() {
            summary_currency = s.currency.clone();
        }

        let mkey = (s.provider.clone(), s.model.clone());
        let m = by_model.entry(mkey.clone()).or_insert_with(|| ModelUsage {
            provider: s.provider.clone(),
            model: s.model.clone(),
            cost: 0.0,
            currency: s.currency.clone(),
            context_peak: 0,
            context_size: 0,
            sessions: 0,
            rate_status: None,
        });
        m.cost += cost;
        m.context_peak = m.context_peak.max(s.peak_used);
        m.context_size = m.context_size.max(s.size);
        m.sessions += 1;
        if m.currency.is_none() {
            m.currency = s.currency.clone();
        }
        // Newest rate status for this group wins.
        if s.rate_status.is_some() && s.last_ts >= *model_rate_ts.get(&mkey).unwrap_or(&i64::MIN) {
            m.rate_status = s.rate_status.clone();
            model_rate_ts.insert(mkey, s.last_ts);
        }

        let t = by_task.entry(s.task_id.clone()).or_insert_with(|| TaskUsage {
            task_id: s.task_id.clone(),
            cost: 0.0,
            currency: s.currency.clone(),
        });
        t.cost += cost;
        if t.currency.is_none() {
            t.currency = s.currency.clone();
        }
    }

    let mut by_model: Vec<ModelUsage> = by_model.into_values().collect();
    by_model.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));
    let mut by_task: Vec<TaskUsage> = by_task.into_values().collect();
    by_task.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));

    AcpUsageSummary {
        total_cost,
        currency: summary_currency,
        sessions: session_count,
        by_model,
        by_task,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(session: &str, task: &str, provider: &str, model: Option<&str>, used: i64, cost: Option<f64>, ts: i64) -> AcpUsageEvent {
        AcpUsageEvent {
            agent_id: format!("a-{session}"),
            task_id: task.to_string(),
            session_id: Some(session.to_string()),
            provider: provider.to_string(),
            model: model.map(str::to_string),
            used,
            size: 1_000_000,
            cost_amount: cost,
            currency: cost.map(|_| "USD".to_string()),
            rate_status: None,
            ts,
        }
    }

    #[test]
    fn cumulative_cost_collapses_per_session_not_summed() {
        // One session, three monotonic cumulative samples: total is the LAST
        // cumulative value (0.30), NOT their sum (0.60).
        let events = vec![
            ev("s1", "TASK-1", "claude-acp", Some("opus"), 100, Some(0.10), 1),
            ev("s1", "TASK-1", "claude-acp", Some("opus"), 300, Some(0.20), 2),
            ev("s1", "TASK-1", "claude-acp", Some("opus"), 250, Some(0.30), 3),
        ];
        let s = summarize_usage(&events);
        assert!((s.total_cost - 0.30).abs() < 1e-9, "cost = {}", s.total_cost);
        assert_eq!(s.sessions, 1);
        assert_eq!(s.by_model.len(), 1);
        assert_eq!(s.by_model[0].cost, 0.30);
        // Peak context gauge, not the last value.
        assert_eq!(s.by_model[0].context_peak, 300);
    }

    #[test]
    fn groups_by_provider_model_and_by_task_sorted_desc() {
        let events = vec![
            ev("s1", "TASK-1", "claude-acp", Some("opus"), 100, Some(6.0), 1),
            ev("s2", "TASK-2", "claude-acp", Some("opus"), 100, Some(4.0), 2),
            ev("s3", "TASK-1", "mistral-acp", Some("large"), 100, Some(1.0), 3),
        ];
        let s = summarize_usage(&events);
        assert_eq!(s.total_cost, 11.0);
        assert_eq!(s.sessions, 3);
        // by_model: opus (10.0 across 2 sessions) before large (1.0).
        assert_eq!(s.by_model[0].model.as_deref(), Some("opus"));
        assert_eq!(s.by_model[0].cost, 10.0);
        assert_eq!(s.by_model[0].sessions, 2);
        assert_eq!(s.by_model[1].model.as_deref(), Some("large"));
        // by_task: TASK-1 (6.0 + 1.0 = 7.0) before TASK-2 (4.0).
        assert_eq!(s.by_task[0].task_id, "TASK-1");
        assert_eq!(s.by_task[0].cost, 7.0);
        assert_eq!(s.by_task[1].task_id, "TASK-2");
    }

    #[test]
    fn missing_cost_and_model_degrade_without_fabrication() {
        // No cost reported, no model selector: contributes 0 to spend and is
        // grouped under the provider with model = None.
        let events = vec![ev("s1", "TASK-9", "mistral-acp", None, 500, None, 1)];
        let s = summarize_usage(&events);
        assert_eq!(s.total_cost, 0.0);
        assert_eq!(s.by_model.len(), 1);
        assert_eq!(s.by_model[0].model, None);
        assert_eq!(s.by_model[0].context_peak, 500);
    }

    #[test]
    fn latest_rate_status_wins_per_model() {
        let mut early = ev("s1", "TASK-1", "claude-acp", Some("opus"), 100, Some(0.1), 1);
        early.rate_status = Some("allowed".into());
        let mut late = ev("s1", "TASK-1", "claude-acp", Some("opus"), 100, Some(0.2), 5);
        late.rate_status = Some("rejected".into());
        let s = summarize_usage(&[early, late]);
        assert_eq!(s.by_model[0].rate_status.as_deref(), Some("rejected"));
    }

    #[test]
    fn empty_input_yields_zero_summary() {
        let s = summarize_usage(&[]);
        assert_eq!(s.total_cost, 0.0);
        assert_eq!(s.sessions, 0);
        assert!(s.by_model.is_empty());
        assert!(s.by_task.is_empty());
    }
}
