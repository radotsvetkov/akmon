//! Agent query cycle: session loop connecting the FSM, policy engine, LLM provider, and tools.

#![warn(missing_docs)]

mod context;
mod error;
mod session;

pub use context::{
    build_followup_messages, build_messages, AKMON_MD_END, AKMON_MD_START, PROJECT_CONTEXT_END,
    PROJECT_CONTEXT_START,
};
pub use error::SessionError;
pub use session::{AgentSession, ToolCallSummary};

#[cfg(test)]
mod tests {
    use akmon_core::AgentError;
    use akmon_models::ModelError;

    use super::SessionError;

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
