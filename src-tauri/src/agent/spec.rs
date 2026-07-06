//! Pluggable agent definitions (AC2-21): the data describing how to launch a
//! coding agent in a worktree, the code-defined built-in presets, the pure
//! launch-command builder, and the pure registry resolver.
//!
//! Everything here is pure and unit-tested. Wiring into the PTY supervisor
//! (`agent/mod.rs`) and the store registry is separate.

// Inert Phase-1 model/registry code: Phase 2 wires these into the PTY
// supervisor and the store registry. Allow dead code until then rather than
// reach prematurely into live code paths.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// How an agent receives its initial prompt at launch. Forward-looking: no
/// spawn path uses this yet (tracked as AC2-49); it is carried on the spec so
/// definitions are complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PromptMode {
    /// Write the prompt to the agent's stdin.
    Stdin,
    /// Pass the prompt as a command-line argument.
    Arg,
    /// The agent takes no initial prompt.
    None,
}

/// How an agent reports out-of-band status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StatusMechanism {
    /// Claude Code: inject `--settings` hooks that POST to the HookBridge
    /// (rich working/idle/needs-attention).
    ClaudeHooks,
    /// Process liveness only (running/stopped); no hook pipeline.
    Lifecycle,
}

/// A definition of a launchable coding agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSpec {
    /// Unique identifier, e.g. "claude", "aider", "my-codex".
    pub name: String,
    /// Human-facing label.
    pub display_name: String,
    /// Binary name (resolved via the binary resolver) or absolute path.
    pub binary: String,
    /// Args always passed.
    pub base_args: Vec<String>,
    /// Args appended when resuming. Empty ⇒ resume unsupported.
    pub resume_args: Vec<String>,
    /// Raw extra args, appended last (power-user escape hatch).
    pub extra_args: Vec<String>,
    /// How an initial prompt is delivered (forward-looking; see AC2-49).
    pub prompt_mode: PromptMode,
    /// How status is reported.
    pub status: StatusMechanism,
    /// Flag used to pass a selected model id, e.g. `--model`. `None` ⇒ the
    /// agent takes no model selection (no Model control shown for it).
    #[serde(default)]
    pub model_arg: Option<String>,
    /// Argv appended to `binary` to enumerate available models (one id per
    /// stdout line), e.g. `["models"]`. `None` ⇒ the agent lists no models.
    #[serde(default)]
    pub models_list_args: Option<Vec<String>>,
    /// True for code-defined presets (read-only in the UI).
    pub builtin: bool,
}

/// The id of the global default agent, used when neither task nor repo selects one.
pub const DEFAULT_AGENT: &str = "claude";

/// The code-defined, always-present agent presets. `claude` is the only preset
/// with `StatusMechanism::ClaudeHooks`; the others are lifecycle-only.
pub fn builtin_specs() -> Vec<AgentSpec> {
    vec![
        AgentSpec {
            name: "claude".into(),
            display_name: "Claude Code".into(),
            binary: "claude".into(),
            base_args: vec![],
            resume_args: vec!["--continue".into()],
            extra_args: vec![],
            prompt_mode: PromptMode::Arg,
            status: StatusMechanism::ClaudeHooks,
            model_arg: None,
            models_list_args: None,
            builtin: true,
        },
        AgentSpec {
            name: "aider".into(),
            display_name: "Aider".into(),
            binary: "aider".into(),
            base_args: vec![],
            resume_args: vec![],
            extra_args: vec![],
            prompt_mode: PromptMode::Arg,
            status: StatusMechanism::Lifecycle,
            model_arg: None,
            models_list_args: None,
            builtin: true,
        },
        AgentSpec {
            name: "codex".into(),
            display_name: "Codex".into(),
            binary: "codex".into(),
            base_args: vec![],
            resume_args: vec![],
            extra_args: vec![],
            prompt_mode: PromptMode::Arg,
            status: StatusMechanism::Lifecycle,
            model_arg: None,
            models_list_args: None,
            builtin: true,
        },
        AgentSpec {
            name: "antigravity".into(),
            display_name: "Antigravity".into(),
            binary: "agy".into(),
            base_args: vec![],
            resume_args: vec![],
            extra_args: vec![],
            prompt_mode: PromptMode::Arg,
            status: StatusMechanism::Lifecycle,
            model_arg: None,
            models_list_args: None,
            builtin: true,
        },
        AgentSpec {
            name: "cursor".into(),
            display_name: "Cursor".into(),
            binary: "cursor-agent".into(),
            base_args: vec![],
            resume_args: vec![],
            extra_args: vec![],
            prompt_mode: PromptMode::Arg,
            status: StatusMechanism::Lifecycle,
            model_arg: None,
            models_list_args: None,
            builtin: true,
        },
        AgentSpec {
            name: "opencode".into(),
            display_name: "OpenCode".into(),
            binary: "opencode".into(),
            base_args: vec![],
            resume_args: vec![],
            extra_args: vec![],
            prompt_mode: PromptMode::Arg,
            status: StatusMechanism::Lifecycle,
            model_arg: Some("--model".into()),
            models_list_args: Some(vec!["models".into()]),
            builtin: true,
        },
        AgentSpec {
            name: "mistral".into(),
            display_name: "Mistral Vibe".into(),
            binary: "vibe".into(),
            base_args: vec!["--trust".into()],
            resume_args: vec!["--continue".into()],
            extra_args: vec![],
            prompt_mode: PromptMode::Arg,
            status: StatusMechanism::Lifecycle,
            model_arg: None,
            models_list_args: None,
            builtin: true,
        },
    ]
}

/// Build the `(binary, args)` to launch `spec`.
///
/// Arg order: `base_args`, then `resume_args` when `resume` and they are
/// non-empty, then (for `ClaudeHooks` specs) `--mcp-config <mcp_config>` when
/// provided followed by `--settings <hook_settings>` when provided — `--mcp-config`
/// is variadic and must precede the single-value `--settings` so it never sits
/// last before the positional prompt the caller appends — then model args when
/// `spec.model_arg` is `Some(flag)` and `model` is `Some(m)` (non-empty),
/// then `extra_args`. The binary is returned unresolved; the caller resolves
/// it via the binary resolver at spawn time. `hook_settings` and `mcp_config` are
/// the inline JSON from `crate::agent::build_hook_settings` and
/// `crate::agent::build_mcp_config` respectively; both are ignored for `Lifecycle`
/// specs.
pub fn build_agent_command(
    spec: &AgentSpec,
    resume: bool,
    hook_settings: Option<&str>,
    mcp_config: Option<&str>,
    model: Option<&str>,
) -> (String, Vec<String>) {
    let mut args: Vec<String> = Vec::new();
    args.extend(spec.base_args.iter().cloned());
    if resume && !spec.resume_args.is_empty() {
        args.extend(spec.resume_args.iter().cloned());
    }
    if spec.status == StatusMechanism::ClaudeHooks {
        // `--mcp-config` is variadic in the claude CLI: it greedily consumes
        // following non-option tokens. It must therefore NOT be the last option
        // before the positional prompt (appended later by start_agent), or the
        // prompt is parsed as a second config path and claude exits 1. Emit it
        // before the single-value `--settings`, so `--settings` is the last
        // option before the prompt.
        if let Some(cfg) = mcp_config {
            args.push("--mcp-config".to_string());
            args.push(cfg.to_string());
        }
        if let Some(settings) = hook_settings {
            args.push("--settings".to_string());
            args.push(settings.to_string());
        }
    }
    if let (Some(flag), Some(m)) = (spec.model_arg.as_deref(), model) {
        if !m.trim().is_empty() {
            args.push(flag.to_string());
            args.push(m.to_string());
        }
    }
    args.extend(spec.extra_args.iter().cloned());
    (spec.binary.clone(), args)
}

/// The result of delivering an initial prompt: extra positional args to append
/// to the launch command, plus an optional payload to write to the agent's
/// stdin after spawn. For a non-empty prompt exactly one is populated (by mode);
/// for `PromptMode::None` or an absent/blank prompt, both are empty.
#[derive(Debug, Default, PartialEq)]
pub struct PromptDelivery {
    pub args: Vec<String>,
    pub stdin: Option<String>,
}

/// Decide how to deliver an optional initial `prompt` to a freshly launched
/// agent given its `mode`. A `None` mode, or a missing/whitespace-only prompt,
/// yields an empty (no-op) delivery. The prompt's internal formatting is
/// preserved (only outer blankness is checked) so a multi-line composed prompt
/// is delivered verbatim.
pub fn initial_prompt_delivery(mode: PromptMode, prompt: Option<&str>) -> PromptDelivery {
    let prompt = match prompt {
        Some(p) if !p.trim().is_empty() => p,
        _ => return PromptDelivery::default(),
    };
    match mode {
        PromptMode::Arg => PromptDelivery { args: vec![prompt.to_string()], stdin: None },
        PromptMode::Stdin => PromptDelivery { args: Vec::new(), stdin: Some(prompt.to_string()) },
        PromptMode::None => PromptDelivery::default(),
    }
}

/// Resolve an agent by `name`: built-in presets first (so a custom agent can
/// never shadow a built-in name), then the provided custom definitions.
pub fn resolve_agent(name: &str, custom: &[AgentSpec]) -> Option<AgentSpec> {
    builtin_specs()
        .into_iter()
        .find(|s| s.name == name)
        .or_else(|| custom.iter().find(|s| s.name == name).cloned())
}

/// Resolve the effective agent for a session using the precedence
/// task → repo default → global `DEFAULT_AGENT`. A name that does not resolve
/// (e.g. a deleted custom agent) falls through to the next level. Always
/// returns a spec — the `claude` built-in is the final guarantee.
pub fn resolve_for_task(
    task_agent: Option<&str>,
    repo_default: Option<&str>,
    custom: &[AgentSpec],
) -> AgentSpec {
    task_agent
        .and_then(|n| resolve_agent(n, custom))
        .or_else(|| repo_default.and_then(|n| resolve_agent(n, custom)))
        .or_else(|| resolve_agent(DEFAULT_AGENT, custom))
        .expect("DEFAULT_AGENT must be a built-in")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_now_include_cursor_opencode_and_mistral() {
        let specs = builtin_specs();
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["claude", "aider", "codex", "antigravity", "cursor", "opencode", "mistral"]);
        assert!(specs.iter().all(|s| s.builtin));
    }

    #[test]
    fn only_opencode_advertises_a_model_capability() {
        let specs = builtin_specs();
        for s in &specs {
            if s.name == "opencode" {
                assert_eq!(s.model_arg.as_deref(), Some("--model"));
                assert_eq!(s.models_list_args, Some(vec!["models".to_string()]));
            } else {
                assert_eq!(s.model_arg, None, "{} must not take a model yet", s.name);
                assert_eq!(s.models_list_args, None, "{} must not list models yet", s.name);
            }
        }
    }

    #[test]
    fn cursor_and_opencode_are_lifecycle_no_resume() {
        let specs = builtin_specs();
        for name in ["cursor", "opencode"] {
            let s = specs.iter().find(|s| s.name == name).unwrap();
            assert_eq!(s.status, StatusMechanism::Lifecycle);
            assert!(s.resume_args.is_empty());
            assert_eq!(s.binary, if name == "cursor" { "cursor-agent" } else { "opencode" });
        }
    }

    #[test]
    fn claude_is_the_only_hooks_agent_and_resumes_with_continue() {
        let specs = builtin_specs();
        let claude = specs.iter().find(|s| s.name == "claude").unwrap();
        assert_eq!(claude.status, StatusMechanism::ClaudeHooks);
        assert_eq!(claude.resume_args, vec!["--continue".to_string()]);
        for s in specs.iter().filter(|s| s.name != "claude" && s.name != "mistral") {
            assert_eq!(s.status, StatusMechanism::Lifecycle, "{} should be lifecycle", s.name);
            assert!(s.resume_args.is_empty(), "{} should not resume", s.name);
        }
        // Mistral Vibe uses --continue for resume, like Claude
        let mistral = specs.iter().find(|s| s.name == "mistral").unwrap();
        assert_eq!(mistral.status, StatusMechanism::Lifecycle);
        assert_eq!(mistral.resume_args, vec!["--continue".to_string()]);
    }

    #[test]
    fn enums_serialize_camel_case() {
        assert_eq!(serde_json::to_string(&PromptMode::None).unwrap(), "\"none\"");
        assert_eq!(serde_json::to_string(&PromptMode::Stdin).unwrap(), "\"stdin\"");
        assert_eq!(serde_json::to_string(&PromptMode::Arg).unwrap(), "\"arg\"");
        assert_eq!(serde_json::to_string(&StatusMechanism::ClaudeHooks).unwrap(), "\"claudeHooks\"");
        assert_eq!(serde_json::to_string(&StatusMechanism::Lifecycle).unwrap(), "\"lifecycle\"");
    }

    fn lifecycle_spec(name: &str) -> AgentSpec {
        AgentSpec {
            name: name.into(),
            display_name: name.into(),
            binary: name.into(),
            base_args: vec![],
            resume_args: vec![],
            extra_args: vec![],
            prompt_mode: PromptMode::None,
            status: StatusMechanism::Lifecycle,
            model_arg: None,
            models_list_args: None,
            builtin: false,
        }
    }

    #[test]
    fn claude_command_injects_settings_and_continue_on_resume() {
        let claude = builtin_specs().into_iter().find(|s| s.name == "claude").unwrap();
        let (bin, args) = build_agent_command(&claude, true, Some("{\"hooks\":{}}"), None, None);
        assert_eq!(bin, "claude");
        assert_eq!(args, vec!["--continue", "--settings", "{\"hooks\":{}}"]);
    }

    #[test]
    fn claude_command_without_resume_has_settings_only() {
        let claude = builtin_specs().into_iter().find(|s| s.name == "claude").unwrap();
        let (_bin, args) = build_agent_command(&claude, false, Some("{\"hooks\":{}}"), None, None);
        assert_eq!(args, vec!["--settings", "{\"hooks\":{}}"]);
    }

    #[test]
    fn lifecycle_command_ignores_hook_settings_and_resume_args() {
        let mut spec = lifecycle_spec("aider");
        spec.base_args = vec!["--no-auto-commit".into()];
        // resume=true but resume_args empty ⇒ nothing added; hook_settings ignored for lifecycle.
        let (bin, args) = build_agent_command(&spec, true, Some("{\"hooks\":{}}"), None, None);
        assert_eq!(bin, "aider");
        assert_eq!(args, vec!["--no-auto-commit"]);
    }

    #[test]
    fn extra_args_are_appended_last() {
        let mut spec = lifecycle_spec("custom");
        spec.base_args = vec!["--base".into()];
        spec.extra_args = vec!["--model".into(), "gpt-4o".into()];
        let (_bin, args) = build_agent_command(&spec, false, None, None, None);
        assert_eq!(args, vec!["--base", "--model", "gpt-4o"]);
    }

    #[test]
    fn resolve_agent_prefers_builtin_over_custom_same_name() {
        // A custom agent that tries to shadow "claude" must not win.
        let mut shadow = lifecycle_spec("claude");
        shadow.display_name = "Evil".into();
        let resolved = resolve_agent("claude", &[shadow]).unwrap();
        assert_eq!(resolved.display_name, "Claude Code");
        assert_eq!(resolved.status, StatusMechanism::ClaudeHooks);
    }

    #[test]
    fn resolve_agent_finds_custom_by_name() {
        let custom = lifecycle_spec("my-agent");
        assert_eq!(resolve_agent("my-agent", std::slice::from_ref(&custom)), Some(custom));
        assert_eq!(resolve_agent("missing", &[]), None);
    }

    #[test]
    fn resolve_for_task_uses_task_then_repo_then_default() {
        let custom = vec![lifecycle_spec("aider"), lifecycle_spec("codex")];
        // task wins
        assert_eq!(resolve_for_task(Some("aider"), Some("codex"), &custom).name, "aider");
        // repo default when task is None
        assert_eq!(resolve_for_task(None, Some("codex"), &custom).name, "codex");
        // global default when both None
        assert_eq!(resolve_for_task(None, None, &custom).name, "claude");
    }

    #[test]
    fn resolve_for_task_falls_through_unresolvable_names() {
        // task names a now-deleted agent ⇒ fall through to repo, then default.
        assert_eq!(resolve_for_task(Some("ghost"), Some("aider"), &[lifecycle_spec("aider")]).name, "aider");
        assert_eq!(resolve_for_task(Some("ghost"), Some("also-ghost"), &[]).name, "claude");
    }

    #[test]
    fn arg_mode_appends_prompt_as_positional_arg() {
        let d = initial_prompt_delivery(PromptMode::Arg, Some("do the thing"));
        assert_eq!(d.args, vec!["do the thing".to_string()]);
        assert_eq!(d.stdin, None);
    }

    #[test]
    fn stdin_mode_routes_prompt_to_stdin() {
        let d = initial_prompt_delivery(PromptMode::Stdin, Some("hello"));
        assert!(d.args.is_empty());
        assert_eq!(d.stdin, Some("hello".to_string()));
    }

    #[test]
    fn none_mode_ignores_prompt() {
        let d = initial_prompt_delivery(PromptMode::None, Some("ignored"));
        assert_eq!(d, PromptDelivery::default());
    }

    #[test]
    fn blank_or_absent_prompt_is_a_noop_in_any_mode() {
        for mode in [PromptMode::Arg, PromptMode::Stdin, PromptMode::None] {
            assert_eq!(initial_prompt_delivery(mode, None), PromptDelivery::default());
            assert_eq!(initial_prompt_delivery(mode, Some("   ")), PromptDelivery::default());
        }
    }

    #[test]
    fn opencode_command_appends_model_when_selected() {
        let oc = builtin_specs().into_iter().find(|s| s.name == "opencode").unwrap();
        let (bin, args) = build_agent_command(&oc, false, None, None, Some("zhipuai-coding-plan/glm-5.2"));
        assert_eq!(bin, "opencode");
        assert_eq!(args, vec!["--model", "zhipuai-coding-plan/glm-5.2"]);
    }

    #[test]
    fn model_ignored_when_spec_has_no_model_arg() {
        let claude = builtin_specs().into_iter().find(|s| s.name == "claude").unwrap();
        // model passed but claude has model_arg=None ⇒ ignored.
        let (_b, args) = build_agent_command(&claude, false, Some("{\"hooks\":{}}"), None, Some("opus"));
        assert_eq!(args, vec!["--settings", "{\"hooks\":{}}"]);
    }

    #[test]
    fn empty_or_absent_model_adds_nothing() {
        let oc = builtin_specs().into_iter().find(|s| s.name == "opencode").unwrap();
        assert_eq!(build_agent_command(&oc, false, None, None, None).1, Vec::<String>::new());
        assert_eq!(build_agent_command(&oc, false, None, None, Some("   ")).1, Vec::<String>::new());
    }

    #[test]
    fn build_agent_command_appends_mcp_config_for_claude() {
        let spec = resolve_agent("claude", &[]).unwrap();
        let (_program, args) =
            build_agent_command(&spec, false, Some("{hooks}"), Some("{mcp}"), None);
        let i = args.iter().position(|a| a == "--mcp-config").expect("has --mcp-config");
        assert_eq!(args[i + 1], "{mcp}");
    }

    #[test]
    fn build_agent_command_omits_mcp_config_when_none() {
        let spec = resolve_agent("claude", &[]).unwrap();
        let (_program, args) = build_agent_command(&spec, false, Some("{hooks}"), None, None);
        assert!(!args.iter().any(|a| a == "--mcp-config"));
    }

    #[test]
    fn build_agent_command_emits_mcp_config_before_settings() {
        // claude's `--mcp-config` is variadic: if it is the last option before
        // the positional prompt (appended later by start_agent), it swallows the
        // prompt as a second config path and claude exits 1. Keep the
        // single-value `--settings` last by emitting `--mcp-config` first.
        let spec = resolve_agent("claude", &[]).unwrap();
        let (_program, args) =
            build_agent_command(&spec, false, Some("{hooks}"), Some("{mcp}"), None);
        let mcp_i = args.iter().position(|a| a == "--mcp-config").expect("has --mcp-config");
        let settings_i = args.iter().position(|a| a == "--settings").expect("has --settings");
        assert!(mcp_i < settings_i, "--mcp-config must precede --settings; args: {args:?}");
    }
}
