//! Pure authorization core for the MCP surface (TASK-180). Every mutating MCP
//! tool routes through `decide()`; the choke-point resolves the target repo
//! from storage and returns an `AuthorizedContext`, never raw token claims.
//!
//! NOTE: these value types are consumed by the policy fn + choke-point wiring
//! landed in the following tasks (A2/A3); until then they are dead by design,
//! so the module carries a scoped `dead_code` allowance to stay warning-free
//! under CI's `-D warnings`.
#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    StartTask,
    FinishTask,
    ScheduleTask,
    ReadControlPlane,
    ManageSchedule,
    StirTask,
}

/// How the dispatch layer must derive the *target repo* for a call. The repo is
/// resolved from storage for id-addressed resources — never trusted from a
/// caller-supplied `repoId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionStrategy {
    /// Repo named in the args (create/list) — validated against the token scope.
    RepoArg,
    /// Resolve `target_repo` from the task row (`get_task(id).repo_id`).
    FromTaskId,
    /// Resolve `target_repo` from the schedule row (`get_schedule(id).repo_id`).
    FromScheduleId,
    /// Use the caller token's own repo (Agent/Orchestrator self-scope default).
    CallerRepo,
}

/// Proof handed to a handler that authorization passed. Handlers receive this,
/// never the raw `CallContext`, so they cannot act outside the resolved repo.
#[derive(Debug, Clone)]
pub struct AuthorizedContext {
    pub repo_id: String,
    pub caller_task_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum AuthzError {
    Denied(String),
    NotFound(String),
}

impl AuthzError {
    pub fn into_message(self) -> String {
        match self {
            AuthzError::Denied(what) => format!("not authorized: {what}"),
            AuthzError::NotFound(what) => format!("not found: {what}"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Principal {
    Agent { repo_id: String },
    Orchestrator { repo_id: String },
    Concierge,
}

pub fn principal_of(ctx: &crate::mcp::CallContext) -> Principal {
    match ctx {
        crate::mcp::CallContext::Agent { repo_id, .. } => Principal::Agent {
            repo_id: repo_id.clone(),
        },
        crate::mcp::CallContext::Orchestrator { repo_id } => Principal::Orchestrator {
            repo_id: repo_id.clone(),
        },
        crate::mcp::CallContext::Concierge => Principal::Concierge,
    }
}

pub fn decide(principal: &Principal, cap: Capability, target_repo: &str) -> Result<(), AuthzError> {
    let own = match principal {
        Principal::Agent { repo_id } | Principal::Orchestrator { repo_id } => repo_id.as_str(),
        Principal::Concierge => {
            // Legacy global concierge: cross-repo READS only.
            return match cap {
                Capability::ReadControlPlane => Ok(()),
                _ => Err(AuthzError::Denied(format!(
                    "{cap:?} (concierge is read-only)"
                ))),
            };
        }
    };
    if own == target_repo {
        Ok(())
    } else {
        Err(AuthzError::Denied(format!(
            "{cap:?} on repo {target_repo} (token scoped to {own})"
        )))
    }
}

/// The single source of truth mapping each mutating / read-gated MCP tool name
/// to its capability and how the dispatch layer must resolve the target repo.
/// `list_repos` is intentionally absent — it is the only ungated tool. Any new
/// act tool that fails to appear here is caught by the exhaustiveness test
/// (`every_mutating_tool_is_registered`), preserving deny-by-default (TASK-180).
pub fn registry() -> &'static [(&'static str, Capability, ResolutionStrategy)] {
    &[
        ("start_task", Capability::StartTask, ResolutionStrategy::RepoArg),
        // queue_dependency (TASK-164) is start_task with a required dependency
        // list — same capability + repo resolution, own-repo only.
        ("queue_dependency", Capability::StartTask, ResolutionStrategy::RepoArg),
        ("finish_task", Capability::FinishTask, ResolutionStrategy::FromTaskId),
        ("schedule_task", Capability::ScheduleTask, ResolutionStrategy::RepoArg),
        ("list_tasks", Capability::ReadControlPlane, ResolutionStrategy::CallerRepo),
        ("task_status", Capability::ReadControlPlane, ResolutionStrategy::FromTaskId),
        ("get_task_activity", Capability::ReadControlPlane, ResolutionStrategy::FromTaskId),
        ("create_schedule", Capability::ManageSchedule, ResolutionStrategy::RepoArg),
        ("list_schedules", Capability::ManageSchedule, ResolutionStrategy::RepoArg),
        ("update_schedule", Capability::ManageSchedule, ResolutionStrategy::FromScheduleId),
        ("set_schedule_enabled", Capability::ManageSchedule, ResolutionStrategy::FromScheduleId),
        ("delete_schedule", Capability::ManageSchedule, ResolutionStrategy::FromScheduleId),
        ("send_task_message", Capability::StirTask, ResolutionStrategy::FromTaskId),
    ]
}

/// Look up the capability + resolution strategy for a tool, or `None` if the
/// tool is ungated (`list_repos`).
pub fn capability_for(tool: &str) -> Option<(Capability, ResolutionStrategy)> {
    registry()
        .iter()
        .find(|(t, _, _)| *t == tool)
        .map(|(_, c, s)| (*c, *s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authz_error_messages_are_human_readable() {
        assert_eq!(
            AuthzError::Denied("start_task".into()).into_message(),
            "not authorized: start_task"
        );
        assert_eq!(
            AuthzError::NotFound("schedule s1".into()).into_message(),
            "not found: schedule s1"
        );
    }

    fn agent(repo: &str) -> Principal {
        Principal::Agent {
            repo_id: repo.into(),
        }
    }
    fn orch(repo: &str) -> Principal {
        Principal::Orchestrator {
            repo_id: repo.into(),
        }
    }

    #[test]
    fn orchestrator_confined_to_its_repo() {
        // same repo: allowed for every act capability
        for cap in [
            Capability::StartTask,
            Capability::FinishTask,
            Capability::ScheduleTask,
            Capability::ManageSchedule,
            Capability::ReadControlPlane,
            Capability::StirTask,
        ] {
            assert!(decide(&orch("r1"), cap, "r1").is_ok(), "{cap:?} same-repo");
        }
        // cross repo: denied for every capability (the confused-deputy case:
        // a caller passing its own repo but a foreign resource resolves to r2)
        for cap in [
            Capability::StartTask,
            Capability::FinishTask,
            Capability::ScheduleTask,
            Capability::ManageSchedule,
            Capability::ReadControlPlane,
            Capability::StirTask,
        ] {
            assert!(
                matches!(decide(&orch("r1"), cap, "r2"), Err(AuthzError::Denied(_))),
                "{cap:?} cross-repo must be denied"
            );
        }
    }

    #[test]
    fn agent_confined_to_its_repo() {
        assert!(decide(&agent("r1"), Capability::StartTask, "r1").is_ok());
        assert!(matches!(
            decide(&agent("r1"), Capability::StartTask, "r2"),
            Err(AuthzError::Denied(_))
        ));
        assert!(decide(&agent("r1"), Capability::StirTask, "r1").is_ok());
        assert!(matches!(
            decide(&agent("r1"), Capability::StirTask, "r2"),
            Err(AuthzError::Denied(_))
        ));
    }

    /// Every mutating/read-gated MCP tool MUST appear in the registry. This
    /// guards against a new act tool silently bypassing the boundary (TASK-180
    /// deny-by-default). `list_repos` is the only ungated tool.
    #[test]
    fn every_mutating_tool_is_registered() {
        // The full advertised tool set (keep in sync with tools_list_result).
        let all = [
            "start_task",
            "queue_dependency",
            "finish_task",
            "schedule_task",
            "list_repos",
            "list_tasks",
            "task_status",
            "get_task_activity",
            "create_schedule",
            "list_schedules",
            "update_schedule",
            "set_schedule_enabled",
            "delete_schedule",
            "send_task_message",
        ];
        let ungated = ["list_repos"];
        for tool in all {
            let registered = capability_for(tool).is_some();
            if ungated.contains(&tool) {
                assert!(!registered, "{tool} must NOT be gated");
            } else {
                assert!(
                    registered,
                    "{tool} must be in the authz registry (deny-by-default)"
                );
            }
        }
    }

    #[test]
    fn concierge_is_read_only_any_repo() {
        assert!(decide(&Principal::Concierge, Capability::ReadControlPlane, "r1").is_ok());
        assert!(decide(&Principal::Concierge, Capability::ReadControlPlane, "r2").is_ok());
        for cap in [
            Capability::StartTask,
            Capability::ScheduleTask,
            Capability::ManageSchedule,
            Capability::StirTask,
        ] {
            assert!(
                matches!(
                    decide(&Principal::Concierge, cap, "r1"),
                    Err(AuthzError::Denied(_))
                ),
                "{cap:?} concierge must be denied"
            );
        }
    }
}
