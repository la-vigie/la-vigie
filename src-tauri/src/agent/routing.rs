//! Automatic agent/provider routing.
//!
//! When a task is created WITHOUT an explicit agent and its repo has an enabled
//! routing policy, La Vigie picks an engine (and optionally a model) from a quick
//! classification of the task, instead of always deferring to the static
//! repo/global default.
//!
//! This module is the **pure core**: the policy data model
//! (`RoutingPolicy`/`RoutingRule`/`RuleCondition`/`RouteTarget`), the
//! classification types (`TaskClassification`), the strict-JSON classifier-output
//! parser (`parse_classification`), and the deterministic `route()` matcher. It
//! performs no I/O and is fully unit-tested. The async classifier shell-out and
//! the launch-path wiring live in `agent::classifier` and the command layer.
//!
//! Precedence & override: routing only runs when no explicit agent was chosen, so
//! a manual per-task agent pick always wins by construction. A `route()` decision
//! is layered *before* the static `resolve_for_task` fallback — if routing yields
//! nothing (disabled, no match, or a target naming an unresolvable agent), the
//! caller keeps today's behaviour.

use serde::{Deserialize, Serialize};

use crate::agent::spec::{resolve_agent, AgentSpec};

/// Estimated difficulty of a task, produced by the classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
}

/// Coarse task kind, produced by the classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Refactor,
    Greenfield,
    Debug,
    Ui,
    Docs,
    Other,
}

/// The classifier's structured verdict about a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskClassification {
    pub difficulty: Difficulty,
    pub kind: TaskKind,
}

/// A routing target: which engine to use, and optionally which model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteTarget {
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// One rule's match condition. Every present clause must hold (AND); within a
/// clause the listed values are alternatives (any-of). An all-`None` condition
/// matches everything — a catch-all rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleCondition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<Vec<Difficulty>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Vec<TaskKind>>,
    /// Case-insensitive substring match against the task title. Substring (not
    /// regex) keeps the policy injection-safe and dependency-free.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_contains: Option<String>,
}

impl RuleCondition {
    /// True when no clause is set — a catch-all condition. Used to omit an empty
    /// `when` from the canonical JSON so round-tripped policies stay tidy.
    fn is_empty(&self) -> bool {
        self.difficulty.is_none() && self.kind.is_none() && self.title_contains.is_none()
    }
}

/// One ordered routing rule: match `when`, dispatch to `target`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoutingRule {
    #[serde(default, skip_serializing_if = "RuleCondition::is_empty")]
    pub when: RuleCondition,
    pub target: RouteTarget,
}

/// A repo's routing policy, persisted as JSON in `repos.routing_policy`. `None`
/// column ⇒ no policy ⇒ routing off (today's behaviour, zero change).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoutingPolicy {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<RoutingRule>,
    /// Used when no rule matches. `None` ⇒ fall through to the static default
    /// (`resolve_for_task`), i.e. routing declines rather than forcing a choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<RouteTarget>,
}

impl RoutingPolicy {
    /// Parse a policy from the stored JSON column. A malformed policy is treated
    /// as "no policy" (`None`) so a bad hand-edit can never break task creation.
    pub fn from_json(raw: Option<&str>) -> Option<RoutingPolicy> {
        let raw = raw?.trim();
        if raw.is_empty() {
            return None;
        }
        serde_json::from_str(raw).ok()
    }

    /// Whether any rule references a classifier output. When false, the caller
    /// can skip the async, costly classifier entirely and route on title alone.
    pub fn needs_classification(&self) -> bool {
        self.enabled
            && self
                .rules
                .iter()
                .any(|r| r.when.difficulty.is_some() || r.when.kind.is_some())
    }
}

/// Non-classifier signals available at the routing seam.
pub struct RouteSignals<'a> {
    pub title: &'a str,
}

/// The outcome of routing: the chosen engine (+ optional model) and a
/// human-readable reason recorded on the task for observability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDecision {
    pub agent: String,
    pub model: Option<String>,
    pub reason: String,
}

impl RuleCondition {
    /// True when every present clause is satisfied. A clause requiring a
    /// classifier output fails when no classification is available (e.g. the
    /// classifier errored) — so such a rule is skipped and routing degrades to
    /// the next rule / fallback / static default.
    fn matches(&self, class: Option<&TaskClassification>, signals: &RouteSignals) -> bool {
        if let Some(diffs) = &self.difficulty {
            match class {
                Some(c) if diffs.contains(&c.difficulty) => {}
                _ => return false,
            }
        }
        if let Some(kinds) = &self.kind {
            match class {
                Some(c) if kinds.contains(&c.kind) => {}
                _ => return false,
            }
        }
        if let Some(sub) = &self.title_contains {
            let needle = sub.trim().to_lowercase();
            // An empty/whitespace needle is NOT a match-everything wildcard
            // (`str::contains("")` is always true); treat it as an inert clause
            // so a blank `titleContains` can't silently turn a rule into a
            // catch-all. The validator also rejects it at save time.
            if !needle.is_empty() && !signals.title.to_lowercase().contains(&needle) {
                return false;
            }
        }
        true
    }
}

/// Decide an engine (+ model) for a task under `policy`.
///
/// Returns `None` when the policy is disabled, or when neither a rule nor the
/// fallback yields a **resolvable** agent — in which case the caller keeps the
/// existing static default (`resolve_for_task`). A target naming an unknown agent
/// (a deleted custom engine, a typo in a hand-edited policy) is skipped rather
/// than honoured, so a stale policy can never strand a task on a dead engine.
pub fn route(
    policy: &RoutingPolicy,
    class: Option<&TaskClassification>,
    signals: &RouteSignals,
    custom: &[AgentSpec],
) -> Option<RouteDecision> {
    if !policy.enabled {
        return None;
    }
    for (i, rule) in policy.rules.iter().enumerate() {
        if rule.when.matches(class, signals) && resolve_agent(&rule.target.agent, custom).is_some() {
            return Some(RouteDecision {
                agent: rule.target.agent.clone(),
                model: rule.target.model.clone(),
                reason: rule_reason(i, &rule.target, class),
            });
        }
    }
    if let Some(fb) = &policy.fallback {
        if resolve_agent(&fb.agent, custom).is_some() {
            return Some(RouteDecision {
                agent: fb.agent.clone(),
                model: fb.model.clone(),
                reason: format!("policy fallback → {}", target_label(fb)),
            });
        }
    }
    None
}

/// Human-readable reason for a rule match, e.g.
/// `auto-routed by rule #2 (hard/debug) → codex/gpt-5.5`.
fn rule_reason(idx: usize, target: &RouteTarget, class: Option<&TaskClassification>) -> String {
    let signal = match class {
        Some(c) => format!(" ({}/{})", difficulty_word(c.difficulty), kind_word(c.kind)),
        None => String::new(),
    };
    format!(
        "auto-routed by rule #{}{} → {}",
        idx + 1,
        signal,
        target_label(target)
    )
}

fn target_label(target: &RouteTarget) -> String {
    match &target.model {
        Some(m) if !m.trim().is_empty() => format!("{}/{}", target.agent, m),
        _ => target.agent.clone(),
    }
}

fn difficulty_word(d: Difficulty) -> &'static str {
    match d {
        Difficulty::Easy => "easy",
        Difficulty::Medium => "medium",
        Difficulty::Hard => "hard",
    }
}

fn kind_word(k: TaskKind) -> &'static str {
    match k {
        TaskKind::Refactor => "refactor",
        TaskKind::Greenfield => "greenfield",
        TaskKind::Debug => "debug",
        TaskKind::Ui => "ui",
        TaskKind::Docs => "docs",
        TaskKind::Other => "other",
    }
}

/// Parse the classifier CLI's stdout into a `TaskClassification`.
///
/// The classifier is asked for a single strict-JSON object
/// `{"difficulty": "...", "kind": "..."}`, but models routinely wrap it in prose
/// or ```json fences, so we extract the first balanced `{...}` span and parse it
/// leniently: the difficulty/kind strings are matched case-insensitively with a
/// few common synonyms. Any failure (no JSON, missing/unknown field) ⇒ `None`,
/// which downgrades routing to the non-classifier path — the classifier can
/// never block or mis-fail task creation.
pub fn parse_classification(stdout: &str) -> Option<TaskClassification> {
    let json = extract_first_json_object(stdout)?;

    #[derive(Deserialize)]
    struct Raw {
        difficulty: Option<String>,
        kind: Option<String>,
    }
    let raw: Raw = serde_json::from_str(&json).ok()?;
    Some(TaskClassification {
        difficulty: parse_difficulty(raw.difficulty.as_deref()?)?,
        kind: parse_kind(raw.kind.as_deref()?)?,
    })
}

/// Extract the first balanced `{...}` object from arbitrary text. Respects JSON
/// string literals (so a `}` inside a quoted value doesn't close the object) and
/// backslash escapes. Returns `None` if no balanced object is present.
fn extract_first_json_object(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let start = s.find('{')?;
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_difficulty(s: &str) -> Option<Difficulty> {
    match s.trim().to_lowercase().as_str() {
        "easy" | "simple" | "trivial" | "low" => Some(Difficulty::Easy),
        "medium" | "moderate" | "mid" | "normal" => Some(Difficulty::Medium),
        "hard" | "difficult" | "complex" | "high" => Some(Difficulty::Hard),
        _ => None,
    }
}

fn parse_kind(s: &str) -> Option<TaskKind> {
    match s.trim().to_lowercase().as_str() {
        "refactor" | "refactoring" | "cleanup" => Some(TaskKind::Refactor),
        "greenfield" | "feature" | "new" => Some(TaskKind::Greenfield),
        "debug" | "bug" | "bugfix" | "fix" => Some(TaskKind::Debug),
        "ui" | "ux" | "frontend" | "design" => Some(TaskKind::Ui),
        "docs" | "doc" | "documentation" => Some(TaskKind::Docs),
        "other" | "misc" => Some(TaskKind::Other),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(enabled: bool, rules: Vec<RoutingRule>, fallback: Option<RouteTarget>) -> RoutingPolicy {
        RoutingPolicy { enabled, rules, fallback }
    }

    fn target(agent: &str, model: Option<&str>) -> RouteTarget {
        RouteTarget { agent: agent.to_string(), model: model.map(|m| m.to_string()) }
    }

    fn class(difficulty: Difficulty, kind: TaskKind) -> TaskClassification {
        TaskClassification { difficulty, kind }
    }

    fn sig(title: &str) -> RouteSignals<'_> {
        RouteSignals { title }
    }

    // ---- route() -----------------------------------------------------------

    #[test]
    fn disabled_policy_declines() {
        let p = policy(false, vec![RoutingRule { when: RuleCondition::default(), target: target("codex", None) }], None);
        assert_eq!(route(&p, None, &sig("x"), &[]), None);
    }

    #[test]
    fn first_matching_rule_wins_and_carries_model() {
        let p = policy(
            true,
            vec![
                RoutingRule {
                    when: RuleCondition { difficulty: Some(vec![Difficulty::Hard]), ..Default::default() },
                    target: target("claude", Some("opus")),
                },
                RoutingRule {
                    when: RuleCondition { difficulty: Some(vec![Difficulty::Easy]), ..Default::default() },
                    target: target("codex", None),
                },
            ],
            None,
        );
        let d = route(&p, Some(&class(Difficulty::Hard, TaskKind::Debug)), &sig("x"), &[]).unwrap();
        assert_eq!(d.agent, "claude");
        assert_eq!(d.model.as_deref(), Some("opus"));
        assert!(d.reason.contains("rule #1"));
        assert!(d.reason.contains("hard/debug"));
    }

    #[test]
    fn falls_back_when_no_rule_matches() {
        let p = policy(
            true,
            vec![RoutingRule {
                when: RuleCondition { kind: Some(vec![TaskKind::Ui]), ..Default::default() },
                target: target("cursor", None),
            }],
            Some(target("claude", None)),
        );
        let d = route(&p, Some(&class(Difficulty::Easy, TaskKind::Debug)), &sig("x"), &[]).unwrap();
        assert_eq!(d.agent, "claude");
        assert!(d.reason.contains("fallback"));
    }

    #[test]
    fn declines_when_no_rule_and_no_fallback() {
        let p = policy(
            true,
            vec![RoutingRule {
                when: RuleCondition { kind: Some(vec![TaskKind::Ui]), ..Default::default() },
                target: target("cursor", None),
            }],
            None,
        );
        assert_eq!(route(&p, Some(&class(Difficulty::Easy, TaskKind::Debug)), &sig("x"), &[]), None);
    }

    #[test]
    fn skips_rule_targeting_unknown_agent() {
        // First rule points at a non-existent engine; router must skip it and
        // honour the next resolvable rule rather than strand the task.
        let p = policy(
            true,
            vec![
                RoutingRule { when: RuleCondition::default(), target: target("ghost-engine", None) },
                RoutingRule { when: RuleCondition::default(), target: target("codex", None) },
            ],
            None,
        );
        let d = route(&p, None, &sig("x"), &[]).unwrap();
        assert_eq!(d.agent, "codex");
    }

    #[test]
    fn declines_when_only_target_is_unknown_agent() {
        let p = policy(true, vec![RoutingRule { when: RuleCondition::default(), target: target("ghost", None) }], Some(target("ghost2", None)));
        assert_eq!(route(&p, None, &sig("x"), &[]), None);
    }

    #[test]
    fn empty_condition_is_catch_all() {
        let p = policy(true, vec![RoutingRule { when: RuleCondition::default(), target: target("codex", None) }], None);
        let d = route(&p, None, &sig("anything"), &[]).unwrap();
        assert_eq!(d.agent, "codex");
    }

    #[test]
    fn classifier_clause_without_classification_does_not_match() {
        // A rule requiring difficulty must not match when classification is None
        // (classifier failed) — it falls through to the fallback.
        let p = policy(
            true,
            vec![RoutingRule {
                when: RuleCondition { difficulty: Some(vec![Difficulty::Hard]), ..Default::default() },
                target: target("claude", None),
            }],
            Some(target("codex", None)),
        );
        let d = route(&p, None, &sig("x"), &[]).unwrap();
        assert_eq!(d.agent, "codex");
    }

    #[test]
    fn title_contains_is_case_insensitive_substring() {
        let p = policy(
            true,
            vec![RoutingRule {
                when: RuleCondition { title_contains: Some("CSS".to_string()), ..Default::default() },
                target: target("cursor", None),
            }],
            None,
        );
        assert_eq!(route(&p, None, &sig("Tweak the css grid"), &[]).unwrap().agent, "cursor");
        assert_eq!(route(&p, None, &sig("backend fix"), &[]), None);
    }

    #[test]
    fn empty_title_contains_is_inert_not_catch_all() {
        // A blank titleContains must NOT match every title. With no other clause
        // the rule's condition is effectively empty → catch-all (that's fine),
        // but a blank needle paired with a real clause must not widen it.
        let p = policy(
            true,
            vec![RoutingRule {
                when: RuleCondition {
                    title_contains: Some("   ".to_string()),
                    kind: Some(vec![TaskKind::Ui]),
                    ..Default::default()
                },
                target: target("cursor", None),
            }],
            None,
        );
        // kind clause still gates: a non-UI task must not match.
        assert!(route(&p, Some(&class(Difficulty::Easy, TaskKind::Debug)), &sig("anything"), &[]).is_none());
        assert!(route(&p, Some(&class(Difficulty::Easy, TaskKind::Ui)), &sig("anything"), &[]).is_some());
    }

    #[test]
    fn all_clauses_must_hold_and_any_of_within_clause() {
        let p = policy(
            true,
            vec![RoutingRule {
                when: RuleCondition {
                    difficulty: Some(vec![Difficulty::Easy, Difficulty::Medium]),
                    kind: Some(vec![TaskKind::Refactor]),
                    title_contains: None,
                },
                target: target("codex", Some("gpt-5.5")),
            }],
            None,
        );
        // easy+refactor → matches (easy is one of the any-of set)
        assert!(route(&p, Some(&class(Difficulty::Easy, TaskKind::Refactor)), &sig("x"), &[]).is_some());
        // hard+refactor → difficulty clause fails
        assert!(route(&p, Some(&class(Difficulty::Hard, TaskKind::Refactor)), &sig("x"), &[]).is_none());
        // easy+debug → kind clause fails
        assert!(route(&p, Some(&class(Difficulty::Easy, TaskKind::Debug)), &sig("x"), &[]).is_none());
    }

    // ---- needs_classification ---------------------------------------------

    #[test]
    fn needs_classification_only_when_a_rule_uses_a_classifier_field() {
        let title_only = policy(true, vec![RoutingRule { when: RuleCondition { title_contains: Some("ui".into()), ..Default::default() }, target: target("cursor", None) }], None);
        assert!(!title_only.needs_classification());

        let with_diff = policy(true, vec![RoutingRule { when: RuleCondition { difficulty: Some(vec![Difficulty::Hard]), ..Default::default() }, target: target("claude", None) }], None);
        assert!(with_diff.needs_classification());

        let disabled = policy(false, vec![RoutingRule { when: RuleCondition { kind: Some(vec![TaskKind::Ui]), ..Default::default() }, target: target("cursor", None) }], None);
        assert!(!disabled.needs_classification());
    }

    // ---- from_json ---------------------------------------------------------

    #[test]
    fn from_json_none_and_blank_and_bad_are_none() {
        assert!(RoutingPolicy::from_json(None).is_none());
        assert!(RoutingPolicy::from_json(Some("   ")).is_none());
        assert!(RoutingPolicy::from_json(Some("{not json")).is_none());
    }

    #[test]
    fn canonical_json_omits_none_and_empty_fields() {
        // The command re-serializes the parsed policy to store a canonical form;
        // that output must NOT contain `null`s / empty clauses, else reopening
        // settings shows `"difficulty":null` which the validator then rejects.
        let raw = r#"{"enabled":true,"rules":[{"when":{"titleContains":"css"},"target":{"agent":"cursor"}}],"fallback":{"agent":"claude","model":"sonnet"}}"#;
        let p = RoutingPolicy::from_json(Some(raw)).unwrap();
        let canonical = serde_json::to_string(&p).unwrap();
        assert!(!canonical.contains("null"), "canonical JSON leaked a null: {canonical}");
        assert!(!canonical.contains("difficulty"), "empty difficulty should be omitted: {canonical}");
        assert!(!canonical.contains(r#""when":{}"#), "empty when should be omitted: {canonical}");
        // And it must re-parse to the same policy.
        assert_eq!(RoutingPolicy::from_json(Some(&canonical)).unwrap(), p);
    }

    #[test]
    fn unknown_fields_are_rejected_not_silently_dropped() {
        // A misspelled `rulez` must fail the parse (deny_unknown_fields) rather
        // than deserialize to an empty rule set that silently disables routing.
        assert!(RoutingPolicy::from_json(Some(r#"{"enabled":true,"rulez":[]}"#)).is_none());
        assert!(serde_json::from_str::<RoutingPolicy>(r#"{"enabled":true,"rulez":[]}"#).is_err());
    }

    #[test]
    fn from_json_roundtrip() {
        let raw = r#"{
          "enabled": true,
          "rules": [
            { "when": { "difficulty": ["hard"], "kind": ["debug"] }, "target": { "agent": "claude", "model": "opus" } },
            { "when": { "titleContains": "css" }, "target": { "agent": "cursor" } }
          ],
          "fallback": { "agent": "codex", "model": "gpt-5.5" }
        }"#;
        let p = RoutingPolicy::from_json(Some(raw)).unwrap();
        assert!(p.enabled);
        assert_eq!(p.rules.len(), 2);
        assert_eq!(p.rules[0].target.agent, "claude");
        assert_eq!(p.rules[0].when.difficulty, Some(vec![Difficulty::Hard]));
        assert_eq!(p.rules[1].when.title_contains.as_deref(), Some("css"));
        assert_eq!(p.fallback.unwrap().model.as_deref(), Some("gpt-5.5"));
    }

    // ---- parse_classification ---------------------------------------------

    #[test]
    fn parses_clean_json() {
        let c = parse_classification(r#"{"difficulty":"hard","kind":"debug"}"#).unwrap();
        assert_eq!(c, class(Difficulty::Hard, TaskKind::Debug));
    }

    #[test]
    fn parses_json_wrapped_in_prose_and_fences() {
        let out = "Sure! Here is the classification:\n```json\n{\n  \"difficulty\": \"Easy\",\n  \"kind\": \"Refactor\"\n}\n```\nHope that helps.";
        let c = parse_classification(out).unwrap();
        assert_eq!(c, class(Difficulty::Easy, TaskKind::Refactor));
    }

    #[test]
    fn tolerates_synonyms_and_case() {
        let c = parse_classification(r#"{"difficulty":"COMPLEX","kind":"frontend"}"#).unwrap();
        assert_eq!(c, class(Difficulty::Hard, TaskKind::Ui));
    }

    #[test]
    fn ignores_brace_inside_string_value() {
        let c = parse_classification(r#"{"note":"a } brace","difficulty":"medium","kind":"greenfield"}"#).unwrap();
        assert_eq!(c, class(Difficulty::Medium, TaskKind::Greenfield));
    }

    #[test]
    fn none_on_missing_or_unknown_fields_or_no_json() {
        assert!(parse_classification("no json here").is_none());
        assert!(parse_classification(r#"{"difficulty":"hard"}"#).is_none());
        assert!(parse_classification(r#"{"difficulty":"???","kind":"debug"}"#).is_none());
    }
}
