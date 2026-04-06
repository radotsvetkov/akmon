//! Messages sent from the TUI to the agent task (policy verdicts and flow control).

/// User-driven control sent asynchronously to the agent orchestrator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiCommand {
    /// Answer a pending interactive policy prompt.
    Confirm {
        /// When `true`, the policy engine records an allow verdict; otherwise deny.
        allow: bool,
    },
    /// Request a graceful stop after the in-flight tool batch completes.
    Interrupt,
}
