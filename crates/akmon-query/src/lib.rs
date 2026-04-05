//! Agent query cycle: session loop connecting the FSM, policy engine, LLM provider, and tools.

#![warn(missing_docs)]

mod context;
mod context_manager;
mod error;
mod session;

pub use context::{
    build_followup_messages, build_messages, AKMON_MD_END, AKMON_MD_START, PROJECT_CONTEXT_END,
    PROJECT_CONTEXT_START,
};
pub use context_manager::ContextManager;
pub use error::SessionError;
pub use session::{AgentSession, ToolCallSummary};

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
        assert_eq!(akmon_models::approximate_tokens(&m), 1);
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
