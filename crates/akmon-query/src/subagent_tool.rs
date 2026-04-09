//! `spawn_subagent` — nested session with compact prompts and tight limits.

use std::sync::Arc;

use akmon_core::{
    AgentConfig, AgentEvent, InteractivePolicyReply, PolicyEngine, PolicyVerdict, Sandbox,
};
use akmon_models::LlmProvider;
use akmon_tools::{Tool, ToolContext, ToolOutput};
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::session::AgentSession;

/// Builds a fresh tool list for each nested run (excludes `spawn_subagent`).
pub type SubagentToolFactory = Arc<dyn Fn() -> Vec<Box<dyn Tool>> + Send + Sync + 'static>;

/// Shared dependencies for [`SpawnSubagentTool`].
pub struct SubagentRuntime {
    /// Same model backend as the parent session.
    pub provider: Arc<dyn LlmProvider>,
    /// Nested sessions use interactive approvals pre-filled on a channel (see [`SpawnSubagentTool`]).
    pub policy: Arc<PolicyEngine>,
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

    async fn execute(&self, args: JsonValue, _ctx: &ToolContext) -> ToolOutput {
        let task = match args.get("task").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => {
                return ToolOutput::Error {
                    code: akmon_tools::ToolErrorCode::InvalidArgs,
                    message: "missing non-empty `task` string".into(),
                };
            }
        };

        let tools = (self.rt.tool_factory)();
        let sub_config = AgentConfig {
            max_iterations: 15,
            confirmation_timeout_secs: self.rt.confirmation_timeout_secs,
            session_id: Uuid::new_v4(),
            auto_commit: false,
            max_completion_tokens: Some(2000),
            subagent_style: true,
            max_budget_usd: None,
            fallback_model: None,
        };

        let (seed_tx, seed_rx) = mpsc::channel::<InteractivePolicyReply>(256);
        let seed = InteractivePolicyReply {
            verdict: PolicyVerdict::Allow,
            remember_for_session: true,
            allow_all_writes_session: true,
            shell_allow_prefix: None,
        };
        for _ in 0..256 {
            if seed_tx.try_send(seed.clone()).is_err() {
                break;
            }
        }
        drop(seed_tx);

        let mut session = AgentSession::new(
            sub_config,
            Arc::clone(&self.rt.policy),
            Arc::clone(&self.rt.provider),
            tools,
            Arc::clone(&self.rt.sandbox),
            self.rt.akmon_md.clone(),
            self.rt.plan_mode,
        );

        let (ev_tx, mut ev_rx) = mpsc::channel::<AgentEvent>(32);
        let drain = tokio::spawn(async move { while ev_rx.recv().await.is_some() {} });

        let mut policy_in = Some(seed_rx);
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
