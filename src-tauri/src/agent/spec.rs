//! Pluggable agent definitions (TASK-21): the data describing how to launch a
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
/// spawn path uses this yet (tracked as TASK-49); it is carried on the spec so
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

/// How La Vigie injects its bundled way-of-working skills into a launched
/// agent (TASK-35). Distinct from `StatusMechanism`: skill injection and
/// status/hook wiring are independent concerns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillInjection {
    /// No injection (default; custom agents and engines without a bundle).
    None,
    /// Claude Code: pass `--plugin-dir <resolved lavigie-plugin>` — out-of-tree,
    /// namespaced `/lavigie:*`.
    PluginDir,
    /// Provider that discovers project-local skills from the worktree: the
    /// vendored per-provider bundle `resources/lavigie-skills/<provider>/` is
    /// copied into the worktree at spawn, git-excluded. `provider` is the
    /// bundle subdirectory name.
    WorktreeBundle { provider: String },
}

impl Default for SkillInjection {
    fn default() -> Self {
        SkillInjection::None
    }
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
    /// Flag(s) that enable the agent's auto-approve/skip-confirmation mode,
    /// appended after `base_args` (before `resume_args`) only when the resolved
    /// per-task/per-repo setting is on. Empty ⇒ the agent has no auto-approve
    /// concept, so the setting is a no-op for it. (TASK-135)
    #[serde(default)]
    pub auto_approve_args: Vec<String>,
    /// How an initial prompt is delivered (forward-looking; see TASK-49).
    pub prompt_mode: PromptMode,
    /// How status is reported.
    pub status: StatusMechanism,
    /// Flag used to pass a selected model id, e.g. `--model`. `None` ⇒ the
    /// agent takes no model selection (no Model control shown for it). Set but
    /// with `models_list_args` `None` ⇒ the agent takes a model but can't
    /// enumerate them, so the picker offers free-text entry (TASK-209).
    #[serde(default)]
    pub model_arg: Option<String>,
    /// Argv appended to `binary` to enumerate available models (one id per
    /// stdout line), e.g. `["models"]`. `None` ⇒ the agent lists no models (the
    /// picker falls back to free-text when `model_arg` is set).
    #[serde(default)]
    pub models_list_args: Option<Vec<String>>,
    /// True for code-defined presets (read-only in the UI).
    pub builtin: bool,
    /// How La Vigie's bundled skills are injected for this agent (TASK-35).
    /// Defaults to `None` so stored/custom specs and older rows decode safely.
    #[serde(default)]
    pub skill_injection: SkillInjection,
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
            auto_approve_args: vec![],
            prompt_mode: PromptMode::Arg,
            status: StatusMechanism::ClaudeHooks,
            // TASK-209: Claude Code accepts `--model <id>` but has no `models`
            // subcommand to enumerate ids, so `models_list_args` stays `None` and
            // the picker offers free-text entry (see `AgentModelPicker`).
            model_arg: Some("--model".into()),
            models_list_args: None,
            builtin: true,
            skill_injection: SkillInjection::PluginDir,
        },
        AgentSpec {
            name: "aider".into(),
            display_name: "Aider".into(),
            binary: "aider".into(),
            base_args: vec![],
            resume_args: vec![],
            extra_args: vec![],
            auto_approve_args: vec![],
            prompt_mode: PromptMode::Arg,
            status: StatusMechanism::Lifecycle,
            model_arg: None,
            models_list_args: None,
            builtin: true,
            skill_injection: SkillInjection::None,
        },
        AgentSpec {
            name: "codex".into(),
            display_name: "Codex".into(),
            binary: "codex".into(),
            base_args: vec![],
            resume_args: vec![],
            extra_args: vec![],
            auto_approve_args: vec![],
            prompt_mode: PromptMode::Arg,
            status: StatusMechanism::Lifecycle,
            model_arg: None,
            models_list_args: None,
            builtin: true,
            skill_injection: SkillInjection::WorktreeBundle { provider: "codex".into() },
        },
        AgentSpec {
            name: "antigravity".into(),
            display_name: "Antigravity".into(),
            binary: "agy".into(),
            base_args: vec![],
            resume_args: vec![],
            extra_args: vec![],
            auto_approve_args: vec![],
            prompt_mode: PromptMode::Arg,
            status: StatusMechanism::Lifecycle,
            model_arg: None,
            models_list_args: None,
            builtin: true,
            skill_injection: SkillInjection::WorktreeBundle { provider: "antigravity".into() },
        },
        AgentSpec {
            name: "cursor".into(),
            display_name: "Cursor".into(),
            binary: "cursor-agent".into(),
            base_args: vec![],
            resume_args: vec![],
            extra_args: vec![],
            auto_approve_args: vec![],
            prompt_mode: PromptMode::Arg,
            status: StatusMechanism::Lifecycle,
            model_arg: None,
            models_list_args: None,
            builtin: true,
            skill_injection: SkillInjection::None,
        },
        AgentSpec {
            name: "opencode".into(),
            display_name: "OpenCode".into(),
            binary: "opencode".into(),
            base_args: vec![],
            resume_args: vec![],
            extra_args: vec![],
            auto_approve_args: vec![],
            prompt_mode: PromptMode::Arg,
            status: StatusMechanism::Lifecycle,
            model_arg: Some("--model".into()),
            models_list_args: Some(vec!["models".into()]),
            builtin: true,
            skill_injection: SkillInjection::WorktreeBundle { provider: "opencode".into() },
        },
        AgentSpec {
            name: "mistral".into(),
            display_name: "Mistral Vibe".into(),
            binary: "vibe".into(),
            base_args: vec!["--trust".into()],
            resume_args: vec!["--continue".into()],
            extra_args: vec![],
            auto_approve_args: vec!["--auto-approve".into()],
            prompt_mode: PromptMode::Arg,
            status: StatusMechanism::Lifecycle,
            model_arg: None,
            models_list_args: None,
            builtin: true,
            skill_injection: SkillInjection::WorktreeBundle { provider: "mistral".into() },
        },
    ]
}

/// Build the `(binary, args)` to launch `spec`.
///
/// Arg order: `base_args`, then `auto_approve_args` when `auto_approve`, then
/// `resume_args` when `resume` and they are non-empty, then (for
/// `SkillInjection::PluginDir` specs) `--plugin-dir <plugin_dir>` when provided, followed by
/// `--mcp-config <mcp_config>` when provided, followed by `--settings
/// <hook_settings>` when provided — `--plugin-dir` takes a single path so it is
/// safe ahead of the variadic `--mcp-config`, and `--mcp-config` in turn must
/// precede the single-value `--settings` so it never sits last before the
/// positional prompt the caller appends — then model args when `spec.model_arg`
/// is `Some(flag)` and `model` is `Some(m)` (non-empty), then `extra_args`. The
/// binary is returned unresolved; the caller resolves it via the binary resolver
/// at spawn time. `hook_settings` and `mcp_config` are the inline JSON from
/// `crate::agent::build_hook_settings` and `crate::agent::build_mcp_config`
/// respectively; `plugin_dir` is the resolved La Vigie skill plugin path
/// (TASK-153); all three are ignored for `Lifecycle` specs.
pub fn build_agent_command(
    spec: &AgentSpec,
    resume: bool,
    hook_settings: Option<&str>,
    mcp_config: Option<&str>,
    model: Option<&str>,
    plugin_dir: Option<&str>,
    auto_approve: bool,
) -> (String, Vec<String>) {
    let mut args: Vec<String> = Vec::new();
    args.extend(spec.base_args.iter().cloned());
    if auto_approve {
        args.extend(spec.auto_approve_args.iter().cloned());
    }
    if resume && !spec.resume_args.is_empty() {
        args.extend(spec.resume_args.iter().cloned());
    }
    // TASK-35: `--plugin-dir` is driven by the skill-injection strategy now, not
    // the status mechanism. `claude` is both `PluginDir` and `ClaudeHooks`, so
    // argv order (plugin-dir → mcp-config → settings) is unchanged for it.
    if spec.skill_injection == SkillInjection::PluginDir {
        if let Some(dir) = plugin_dir {
            args.push("--plugin-dir".to_string());
            args.push(dir.to_string());
        }
    }
    if spec.status == StatusMechanism::ClaudeHooks {
        // `--mcp-config` is variadic in the claude CLI: it greedily consumes
        // following non-option tokens. Emit it before the single-value
        // `--settings`, so `--settings` is the last option before the prompt.
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

/// The effective auto-approve setting for a session, using the precedence
/// task override → repo default → global default. The global default is `true`,
/// preserving the historical always-on behavior for agents that support
/// auto-approve (an agent with empty `auto_approve_args` ignores it). (TASK-135)
pub fn effective_auto_approve(task: Option<bool>, repo: Option<bool>) -> bool {
    task.or(repo).unwrap_or(true)
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
    fn model_capability_matrix_across_builtins() {
        let specs = builtin_specs();
        for s in &specs {
            match s.name.as_str() {
                // OpenCode both takes a model and can enumerate them (list picker).
                "opencode" => {
                    assert_eq!(s.model_arg.as_deref(), Some("--model"));
                    assert_eq!(s.models_list_args, Some(vec!["models".to_string()]));
                }
                // Claude Code takes `--model` but has no `models` subcommand, so it
                // lists none — the picker offers free-text entry (TASK-209).
                "claude" => {
                    assert_eq!(s.model_arg.as_deref(), Some("--model"));
                    assert_eq!(s.models_list_args, None, "claude cannot enumerate models");
                }
                // Every other built-in takes no model yet.
                _ => {
                    assert_eq!(s.model_arg, None, "{} must not take a model yet", s.name);
                    assert_eq!(s.models_list_args, None, "{} must not list models yet", s.name);
                }
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
            auto_approve_args: vec![],
            prompt_mode: PromptMode::None,
            status: StatusMechanism::Lifecycle,
            model_arg: None,
            models_list_args: None,
            builtin: false,
            skill_injection: SkillInjection::None,
        }
    }

    #[test]
    fn claude_command_injects_settings_and_continue_on_resume() {
        let claude = builtin_specs().into_iter().find(|s| s.name == "claude").unwrap();
        let (bin, args) = build_agent_command(&claude, true, Some("{\"hooks\":{}}"), None, None, None, true);
        assert_eq!(bin, "claude");
        assert_eq!(args, vec!["--continue", "--settings", "{\"hooks\":{}}"]);
    }

    #[test]
    fn claude_command_without_resume_has_settings_only() {
        let claude = builtin_specs().into_iter().find(|s| s.name == "claude").unwrap();
        let (_bin, args) = build_agent_command(&claude, false, Some("{\"hooks\":{}}"), None, None, None, false);
        assert_eq!(args, vec!["--settings", "{\"hooks\":{}}"]);
    }

    #[test]
    fn lifecycle_command_ignores_hook_settings_and_resume_args() {
        let mut spec = lifecycle_spec("aider");
        spec.base_args = vec!["--no-auto-commit".into()];
        // resume=true but resume_args empty ⇒ nothing added; hook_settings ignored for lifecycle.
        let (bin, args) = build_agent_command(&spec, true, Some("{\"hooks\":{}}"), None, None, None, true);
        assert_eq!(bin, "aider");
        assert_eq!(args, vec!["--no-auto-commit"]);
    }

    #[test]
    fn extra_args_are_appended_last() {
        let mut spec = lifecycle_spec("custom");
        spec.base_args = vec!["--base".into()];
        spec.extra_args = vec!["--model".into(), "gpt-4o".into()];
        let (_bin, args) = build_agent_command(&spec, false, None, None, None, None, false);
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
        let (bin, args) = build_agent_command(&oc, false, None, None, Some("zhipuai-coding-plan/glm-5.2"), None, false);
        assert_eq!(bin, "opencode");
        assert_eq!(args, vec!["--model", "zhipuai-coding-plan/glm-5.2"]);
    }

    #[test]
    fn model_ignored_when_spec_has_no_model_arg() {
        // aider has model_arg=None, so a passed model is ignored.
        let aider = builtin_specs().into_iter().find(|s| s.name == "aider").unwrap();
        let (_b, args) = build_agent_command(&aider, false, None, None, Some("gpt-4o"), None, false);
        assert!(args.is_empty(), "aider takes no model; args: {args:?}");
    }

    #[test]
    fn claude_command_appends_model_when_selected() {
        // TASK-209: claude now takes `--model <id>`; the selected model reaches spawn.
        let claude = builtin_specs().into_iter().find(|s| s.name == "claude").unwrap();
        let (_b, args) = build_agent_command(&claude, false, Some("{\"hooks\":{}}"), None, Some("opus"), None, false);
        assert_eq!(args, vec!["--settings", "{\"hooks\":{}}", "--model", "opus"]);
    }

    #[test]
    fn claude_command_omits_model_when_unset() {
        // No model chosen ⇒ no `--model` flag (Claude's own default).
        let claude = builtin_specs().into_iter().find(|s| s.name == "claude").unwrap();
        let (_b, args) = build_agent_command(&claude, false, Some("{\"hooks\":{}}"), None, None, None, false);
        assert_eq!(args, vec!["--settings", "{\"hooks\":{}}"]);
    }

    #[test]
    fn empty_or_absent_model_adds_nothing() {
        let oc = builtin_specs().into_iter().find(|s| s.name == "opencode").unwrap();
        assert_eq!(build_agent_command(&oc, false, None, None, None, None, true).1, Vec::<String>::new());
        assert_eq!(build_agent_command(&oc, false, None, None, Some("   "), None, true).1, Vec::<String>::new());
    }

    #[test]
    fn build_agent_command_appends_mcp_config_for_claude() {
        let spec = resolve_agent("claude", &[]).unwrap();
        let (_program, args) =
            build_agent_command(&spec, false, Some("{hooks}"), Some("{mcp}"), None, None, false);
        let i = args.iter().position(|a| a == "--mcp-config").expect("has --mcp-config");
        assert_eq!(args[i + 1], "{mcp}");
    }

    #[test]
    fn build_agent_command_omits_mcp_config_when_none() {
        let spec = resolve_agent("claude", &[]).unwrap();
        let (_program, args) = build_agent_command(&spec, false, Some("{hooks}"), None, None, None, false);
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
            build_agent_command(&spec, false, Some("{hooks}"), Some("{mcp}"), None, None, false);
        let mcp_i = args.iter().position(|a| a == "--mcp-config").expect("has --mcp-config");
        let settings_i = args.iter().position(|a| a == "--settings").expect("has --settings");
        assert!(mcp_i < settings_i, "--mcp-config must precede --settings; args: {args:?}");
    }

    #[test]
    fn mistral_spec_splits_trust_and_auto_approve() {
        let mistral = resolve_agent("mistral", &[]).unwrap();
        assert!(mistral.base_args.contains(&"--trust".to_string()), "Mistral keeps --trust in base_args");
        assert!(!mistral.base_args.contains(&"--auto-approve".to_string()), "--auto-approve moved out of base_args");
        assert_eq!(mistral.auto_approve_args, vec!["--auto-approve".to_string()]);
    }

    #[test]
    fn build_agent_command_appends_auto_approve_when_enabled() {
        let mistral = resolve_agent("mistral", &[]).unwrap();
        let (_prog, args) = build_agent_command(&mistral, false, None, None, None, None, true);
        assert!(args.contains(&"--trust".to_string()));
        assert!(args.contains(&"--auto-approve".to_string()));
    }

    #[test]
    fn build_agent_command_omits_auto_approve_when_disabled() {
        let mistral = resolve_agent("mistral", &[]).unwrap();
        let (_prog, args) = build_agent_command(&mistral, false, None, None, None, None, false);
        assert!(args.contains(&"--trust".to_string()));
        assert!(!args.contains(&"--auto-approve".to_string()));
    }

    #[test]
    fn build_agent_command_auto_approve_precedes_resume() {
        let mistral = resolve_agent("mistral", &[]).unwrap();
        let (_prog, args) = build_agent_command(&mistral, true, None, None, None, None, true);
        assert_eq!(args, vec!["--trust", "--auto-approve", "--continue"]);
    }

    #[test]
    fn build_agent_command_no_auto_approve_flag_for_agent_without_any() {
        let claude = resolve_agent("claude", &[]).unwrap();
        let (_p1, on) = build_agent_command(&claude, false, None, None, None, None, true);
        let (_p2, off) = build_agent_command(&claude, false, None, None, None, None, false);
        assert_eq!(on, off, "claude has no auto_approve_args, so the flag is a no-op");
        assert!(!on.contains(&"--auto-approve".to_string()));
    }

    #[test]
    fn effective_auto_approve_precedence() {
        assert!(effective_auto_approve(Some(true), Some(false)));   // task wins
        assert!(!effective_auto_approve(Some(false), Some(true)));  // task wins
        assert!(effective_auto_approve(None, Some(true)));          // repo used
        assert!(!effective_auto_approve(None, Some(false)));        // repo used
        assert!(effective_auto_approve(None, None));                // default true
    }

    #[test]
    fn plugin_dir_injected_before_mcp_config_for_claude() {
        let spec = builtin_specs().into_iter().find(|s| s.name == "claude").unwrap();
        let (_prog, args) = build_agent_command(
            &spec,
            false,
            Some("{\"settings\":1}"),
            Some("{\"mcp\":1}"),
            None,
            Some("/tmp/lavigie-plugin"),
            false,
        );
        let pd = args.iter().position(|a| a == "--plugin-dir").expect("--plugin-dir present");
        assert_eq!(args[pd + 1], "/tmp/lavigie-plugin");
        let mcp = args.iter().position(|a| a == "--mcp-config").unwrap();
        assert!(pd < mcp, "--plugin-dir must precede --mcp-config");
    }

    #[test]
    fn plugin_dir_absent_when_none() {
        let spec = builtin_specs().into_iter().find(|s| s.name == "claude").unwrap();
        let (_prog, args) = build_agent_command(&spec, false, None, None, None, None, false);
        assert!(!args.iter().any(|a| a == "--plugin-dir"));
    }

    #[test]
    fn builtins_carry_expected_skill_injection() {
        let specs = builtin_specs();
        let get = |n: &str| specs.iter().find(|s| s.name == n).unwrap().skill_injection.clone();
        assert_eq!(get("claude"), SkillInjection::PluginDir);
        assert_eq!(get("codex"), SkillInjection::WorktreeBundle { provider: "codex".into() });
        assert_eq!(get("antigravity"), SkillInjection::WorktreeBundle { provider: "antigravity".into() });
        assert_eq!(get("opencode"), SkillInjection::WorktreeBundle { provider: "opencode".into() });
        assert_eq!(get("mistral"), SkillInjection::WorktreeBundle { provider: "mistral".into() });
        assert_eq!(get("aider"), SkillInjection::None);
        assert_eq!(get("cursor"), SkillInjection::None);
    }

    #[test]
    fn skill_injection_defaults_to_none_when_absent_in_json() {
        // A stored/custom AgentSpec JSON without the field decodes to None.
        let json = r#"{"name":"x","displayName":"x","binary":"x","baseArgs":[],"resumeArgs":[],
            "extraArgs":[],"promptMode":"none","status":"lifecycle","builtin":false}"#;
        let spec: AgentSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.skill_injection, SkillInjection::None);
    }

    #[test]
    fn plugin_dir_gated_on_skill_injection_not_status() {
        // A WorktreeBundle engine must NOT receive --plugin-dir even if a dir is passed.
        let mut spec = lifecycle_spec("codex");
        spec.skill_injection = SkillInjection::WorktreeBundle { provider: "codex".into() };
        let (_b, args) = build_agent_command(&spec, false, None, None, None, Some("/tmp/x"), false);
        assert!(!args.iter().any(|a| a == "--plugin-dir"));
    }
}
