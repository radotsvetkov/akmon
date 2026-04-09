//! Agent query cycle: session loop connecting the FSM, policy engine, LLM provider, and tools.

#![warn(missing_docs)]

mod akmon_md_gen;
mod context;
mod context_manager;
mod error;
mod microcompact;
mod session;
mod specs_and_handoff;
mod subagent_tool;
mod tools_filter;

pub use akmon_md_gen::{AKMON_MD_SYSTEM_PROMPT, generate_akmon_md_markdown};

pub use akmon_models::UsageReport;
pub use context::{
    AKMON_MD_END, AKMON_MD_START, LOCAL_MODEL_SYSTEM_PROMPT, PLAN_MODE_SYSTEM_ADDON,
    PROJECT_CONTEXT_END, PROJECT_CONTEXT_START, RESEARCH_PLAN_IMPLEMENT_WORKFLOW,
    SUBAGENT_SYSTEM_PROMPT, build_followup_messages, build_messages,
    build_subagent_followup_messages, build_subagent_task_messages, context_limit_for_model,
    is_openai_native_chat_model, system_prompt_for_model,
};
pub use context_manager::{COMPACT_RESERVED_BUFFER, COMPACT_TRIGGER, ContextManager};
pub use error::SessionError;
pub use session::{
    AgentSession, PendingToolCall, SessionRunExit, ToolCallResult, ToolCallSummary,
    execute_single_tool_call,
};
pub use specs_and_handoff::{
    MIN_USER_TURNS_FOR_HANDOFF, clear_specs_dir, handoff_path, load_handoff_block_for_prompt,
    load_specs_block_for_prompt, should_write_handoff, write_handoff_file,
};
pub use subagent_tool::{SpawnSubagentTool, SubagentRuntime, SubagentToolFactory};

#[cfg(test)]
mod tests {
    use akmon_core::AgentError;
    use akmon_models::{Message, MessageRole, ModelError};

    use super::SessionError;

    #[test]
    fn approximate_tokens_sensible_via_models_crate() {
        let m = vec![Message {
            role: MessageRole::User,
            content: "abc".into(),
        }];
        let n = akmon_models::approximate_tokens(&m);
        assert!(n >= 1, "expected positive token estimate");
    }

    #[test]
    fn session_error_variants_non_empty_display() {
        let cases = [
            SessionError::ProviderError(ModelError::AuthError),
            SessionError::PolicyError("denied".into()),
            SessionError::ToolError("bad tool".into()),
            SessionError::StateMachineError(AgentError::ResponseTruncated),
        ];
        for e in cases {
            assert!(!e.to_string().trim().is_empty());
        }
    }
}
