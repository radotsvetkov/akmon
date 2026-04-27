//! `spawn_subagent` — nested session with compact prompts and tight limits.

use std::sync::Arc;

use akmon_core::{
    AgentConfig, AgentEvent, Permission, PolicyEngine, PolicyEngineError, PolicyEngineMode, Sandbox,
};
use akmon_models::LlmProvider;
use akmon_tools::{Tool, ToolContext, ToolOutput};
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::open_default_journal_handle;
use crate::session::AgentSession;

/// Builds a fresh tool list for each nested run (excludes `spawn_subagent`).
pub type SubagentToolFactory = Arc<dyn Fn() -> Vec<Box<dyn Tool>> + Send + Sync + 'static>;

/// Shared dependencies for [`SpawnSubagentTool`].
pub struct SubagentRuntime {
    /// Same model backend as the parent session.
    pub provider: Arc<dyn LlmProvider>,
    /// Same project [`Sandbox`] as the parent (path resolution, roots).
    pub sandbox: Arc<Sandbox>,
    /// Optional `AKMON.md` body forwarded into nested prompts.
    pub akmon_md: Option<String>,
    /// When `true`, nested tool registry matches read-only plan mode.
    pub plan_mode: bool,
    /// Confirmation timeout forwarded into nested [`AgentConfig`].
    pub confirmation_timeout_secs: u64,
    /// Produces a fresh tool list per nested run (excludes `spawn_subagent`).
    pub tool_factory: SubagentToolFactory,
}

/// Runs a bounded nested agent turn; results are returned as tool output text.
pub struct SpawnSubagentTool {
    rt: Arc<SubagentRuntime>,
}

impl SpawnSubagentTool {
    /// Wraps a [`SubagentRuntime`] (one per parent session).
    pub fn new(rt: Arc<SubagentRuntime>) -> Self {
        Self { rt }
    }
}

#[async_trait]
impl Tool for SpawnSubagentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Run a nested research agent with a fresh transcript, compact prompts, max 15 iterations, ~2000 output tokens per completion. Same sandbox and AKMON.md as the parent. Use for deep exploration without bloating the main context; summarize results for the user in your own words. Never call this tool from nested work."
    }

    fn required_permissions(&self) -> &[akmon_core::Permission] {
        &[]
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "description": "Instructions or questions for the nested agent." }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, args: JsonValue, ctx: &ToolContext) -> ToolOutput {
        let task = match args.get("task").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => {
                return ToolOutput::Error {
                    code: akmon_tools::ToolErrorCode::InvalidArgs,
                    message: "missing non-empty `task` string".into(),
                };
            }
        };

        let parent_policy = ctx.policy_engine();
        let effective_mode = derive_subagent_policy_mode(parent_policy.mode());
        let effective_policy = Arc::new(PolicyEngine::new(effective_mode));
        let tools = (self.rt.tool_factory)();
        let (tools, blocked_reasons) =
            filter_tools_by_policy_ceiling(tools, effective_policy.as_ref());
        if tools.is_empty() {
            let mut details = blocked_reasons
                .into_iter()
                .take(6)
                .collect::<Vec<_>>()
                .join("\n- ");
            if !details.is_empty() {
                details = format!("- {details}");
            }
            return ToolOutput::Error {
                code: akmon_tools::ToolErrorCode::PermissionDenied,
                message: format!(
                    "subagent blocked: no tools available under parent permission ceiling\n{details}"
                ),
            };
        }
        let sub_config = AgentConfig {
            max_iterations: 15,
            confirmation_timeout_secs: self.rt.confirmation_timeout_secs,
            session_id: Uuid::new_v4(),
            auto_commit: false,
            max_completion_tokens: Some(2000),
            subagent_style: true,
            max_budget_usd: None,
            fallback_model: None,
            model_estimates: Vec::new(),
        };

        let journal = match open_default_journal_handle(sub_config.session_id) {
            Ok(j) => j,
            Err(e) => {
                return ToolOutput::Error {
                    code: akmon_tools::ToolErrorCode::InvalidArgs,
                    message: format!("subagent journal: {e}"),
                };
            }
        };

        let mut session = match AgentSession::new(
            sub_config,
            effective_policy,
            Arc::clone(&self.rt.provider),
            tools,
            Arc::clone(&self.rt.sandbox),
            self.rt.akmon_md.clone(),
            self.rt.plan_mode,
            journal,
        ) {
            Ok(s) => s,
            Err(e) => {
                return ToolOutput::Error {
                    code: akmon_tools::ToolErrorCode::InvalidArgs,
                    message: format!("subagent session: {e}"),
                };
            }
        };

        let (ev_tx, mut ev_rx) = mpsc::channel::<AgentEvent>(32);
        let drain = tokio::spawn(async move { while ev_rx.recv().await.is_some() {} });

        let mut policy_in = None;
        let outcome = session
            .run(task, ev_tx, &mut policy_in, &mut None, None)
            .await;

        let _ = drain.await;

        match outcome {
            Ok(()) => ToolOutput::Success {
                content: format!(
                    "--- subagent finished ---\n\n{}",
                    session.result_text().trim_end()
                ),
            },
            Err(e) => ToolOutput::Error {
                code: akmon_tools::ToolErrorCode::InvalidArgs,
                message: format!("subagent: {e}"),
            },
        }
    }
}

fn derive_subagent_policy_mode(parent_mode: &PolicyEngineMode) -> PolicyEngineMode {
    match parent_mode {
        // Fail closed in nested execution: no implicit write/shell/network prompts.
        PolicyEngineMode::Interactive => PolicyEngineMode::AutoApproveReads {
            confirm_writes: true,
        },
        PolicyEngineMode::DenyAll => PolicyEngineMode::DenyAll,
        PolicyEngineMode::AutoApproveReads { confirm_writes } => {
            PolicyEngineMode::AutoApproveReads {
                confirm_writes: *confirm_writes,
            }
        }
        PolicyEngineMode::AutoApproveReadsAndFetch { confirm_writes } => {
            PolicyEngineMode::AutoApproveReadsAndFetch {
                confirm_writes: *confirm_writes,
            }
        }
        PolicyEngineMode::Configured(cfg) => PolicyEngineMode::Configured(cfg.clone()),
    }
}

fn filter_tools_by_policy_ceiling(
    tools: Vec<Box<dyn Tool>>,
    policy: &PolicyEngine,
) -> (Vec<Box<dyn Tool>>, Vec<String>) {
    let mut allowed = Vec::new();
    let mut blocked = Vec::new();
    let probe_session = "subagent-ceiling-probe";
    for tool in tools {
        let name = tool.name().to_string();
        let required = tool.required_permissions();
        if required.is_empty() {
            allowed.push(tool);
            continue;
        }
        let mut denied_reason: Option<String> = None;
        for perm in required {
            let probe_perm = normalize_permission_probe(perm);
            match policy.evaluate_automatic_for_tool(
                probe_session,
                probe_perm.clone(),
                Some(name.as_str()),
            ) {
                Ok(decision) => {
                    if !decision.allowed {
                        denied_reason = Some(format!(
                            "tool `{name}` denied by parent ceiling for permission `{probe_perm:?}`: {}",
                            decision.reason
                        ));
                        break;
                    }
                }
                Err(PolicyEngineError::InteractiveRequiresCaller) => {
                    denied_reason = Some(format!(
                        "tool `{name}` denied by parent ceiling for permission `{probe_perm:?}`: nested session cannot satisfy interactive confirmation"
                    ));
                    break;
                }
                Err(e) => {
                    denied_reason = Some(format!(
                        "tool `{name}` denied by parent ceiling due to policy error: {e}"
                    ));
                    break;
                }
            }
        }
        if let Some(reason) = denied_reason {
            blocked.push(reason);
        } else {
            allowed.push(tool);
        }
    }
    (allowed, blocked)
}

fn normalize_permission_probe(permission: &Permission) -> Permission {
    match permission {
        Permission::ReadFile { path } => Permission::ReadFile { path: path.clone() },
        Permission::ListDirectory { path } => Permission::ListDirectory { path: path.clone() },
        Permission::WriteFile { path } => Permission::WriteFile { path: path.clone() },
        Permission::ExecuteCommand { cwd, .. } => Permission::ExecuteCommand {
            command: "echo subagent-ceiling-probe".into(),
            cwd: cwd.clone(),
        },
        Permission::NetworkFetch { .. } => Permission::NetworkFetch {
            url: "https://example.com".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        derive_subagent_policy_mode, filter_tools_by_policy_ceiling, normalize_permission_probe,
    };
    use std::path::PathBuf;

    use akmon_core::{Permission, PolicyConfig, PolicyEngine, PolicyEngineMode};
    use akmon_tools::{Tool, ToolContext, ToolOutput};
    use async_trait::async_trait;
    use serde_json::Value as JsonValue;

    struct TestTool {
        name: &'static str,
        perms: Vec<Permission>,
    }

    #[async_trait]
    impl Tool for TestTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "test tool"
        }

        fn required_permissions(&self) -> &[Permission] {
            &self.perms
        }

        fn parameters_schema(&self) -> JsonValue {
            serde_json::json!({"type":"object"})
        }

        async fn execute(&self, _args: JsonValue, _ctx: &ToolContext) -> ToolOutput {
            ToolOutput::Success {
                content: "ok".to_string(),
            }
        }
    }

    fn tool(name: &'static str, perms: Vec<Permission>) -> Box<dyn Tool> {
        Box::new(TestTool { name, perms })
    }

    #[test]
    fn interactive_parent_is_downgraded_to_read_only_ceiling() {
        let effective = derive_subagent_policy_mode(&PolicyEngineMode::Interactive);
        assert!(matches!(
            effective,
            PolicyEngineMode::AutoApproveReads {
                confirm_writes: true
            }
        ));
    }

    #[test]
    fn subagent_cannot_escalate_write_when_parent_disallows() {
        let tools: Vec<Box<dyn Tool>> = vec![
            tool(
                "write_file",
                vec![Permission::WriteFile {
                    path: PathBuf::from("out.txt"),
                }],
            ),
            tool(
                "read_file",
                vec![Permission::ReadFile {
                    path: PathBuf::from("input.txt"),
                }],
            ),
        ];
        let policy = PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
            confirm_writes: true,
        });
        let (allowed, blocked) = filter_tools_by_policy_ceiling(tools, &policy);
        let names = allowed.iter().map(|t| t.name()).collect::<Vec<_>>();
        assert_eq!(names, vec!["read_file"]);
        assert!(blocked.iter().any(|r| r.contains("write_file")));
    }

    #[test]
    fn subagent_cannot_escalate_shell_or_network_when_parent_disallows() {
        let tools: Vec<Box<dyn Tool>> = vec![
            tool(
                "shell",
                vec![Permission::ExecuteCommand {
                    command: "ls".into(),
                    cwd: PathBuf::from("."),
                }],
            ),
            tool(
                "web_fetch",
                vec![Permission::NetworkFetch {
                    url: "https://example.com".into(),
                }],
            ),
        ];
        let policy = PolicyEngine::new(PolicyEngineMode::AutoApproveReads {
            confirm_writes: true,
        });
        let (allowed, blocked) = filter_tools_by_policy_ceiling(tools, &policy);
        assert!(allowed.is_empty());
        assert!(blocked.iter().any(|r| r.contains("shell")));
        assert!(blocked.iter().any(|r| r.contains("web_fetch")));
    }

    #[test]
    fn configured_parent_allows_explicit_grants_without_regression() {
        let tools: Vec<Box<dyn Tool>> = vec![
            tool(
                "write_file",
                vec![Permission::WriteFile {
                    path: PathBuf::from("allowed.txt"),
                }],
            ),
            tool(
                "shell",
                vec![Permission::ExecuteCommand {
                    command: "echo test".into(),
                    cwd: PathBuf::from("."),
                }],
            ),
        ];
        let mut cfg = PolicyConfig::default();
        cfg.filesystem.write.allow = vec!["allowed.txt".into()];
        cfg.shell.allow_prefixes = vec!["echo ".into()];
        let policy = PolicyEngine::new(PolicyEngineMode::Configured(cfg));
        let (allowed, blocked) = filter_tools_by_policy_ceiling(tools, &policy);
        let names = allowed.iter().map(|t| t.name()).collect::<Vec<_>>();
        assert_eq!(names, vec!["write_file", "shell"]);
        assert!(blocked.is_empty());
    }

    #[test]
    fn ceiling_filter_uses_tool_context_policy_rules() {
        let tools: Vec<Box<dyn Tool>> = vec![tool(
            "shell",
            vec![Permission::ExecuteCommand {
                command: "echo hi".into(),
                cwd: PathBuf::from("."),
            }],
        )];
        let mut cfg = PolicyConfig::default();
        cfg.shell.allow_prefixes = vec!["echo ".into()];
        cfg.tools.deny = vec!["shell".into()];
        let policy = PolicyEngine::new(PolicyEngineMode::Configured(cfg));
        let (allowed, blocked) = filter_tools_by_policy_ceiling(tools, &policy);
        assert!(allowed.is_empty());
        assert_eq!(blocked.len(), 1);
        assert!(blocked[0].contains("tool `shell`"));
        assert!(blocked[0].contains("denied by parent ceiling"));
    }

    #[test]
    fn ambiguous_nested_policy_context_fails_closed() {
        let tools: Vec<Box<dyn Tool>> = vec![tool(
            "write_file",
            vec![Permission::WriteFile {
                path: PathBuf::from("sensitive.txt"),
            }],
        )];
        let policy = PolicyEngine::new(PolicyEngineMode::Interactive);
        let (allowed, blocked) = filter_tools_by_policy_ceiling(tools, &policy);
        assert!(allowed.is_empty());
        assert_eq!(blocked.len(), 1);
        assert!(blocked[0].contains("nested session cannot satisfy interactive confirmation"));
    }

    #[test]
    fn probe_normalization_uses_safe_shell_and_network_samples() {
        let shell = normalize_permission_probe(&Permission::ExecuteCommand {
            command: "rm -rf /".into(),
            cwd: PathBuf::from("."),
        });
        let fetch = normalize_permission_probe(&Permission::NetworkFetch {
            url: "https://internal.example/secret".into(),
        });
        assert!(matches!(
            shell,
            Permission::ExecuteCommand { ref command, .. }
                if command == "echo subagent-ceiling-probe"
        ));
        assert!(matches!(
            fetch,
            Permission::NetworkFetch { ref url } if url == "https://example.com"
        ));
    }
}
