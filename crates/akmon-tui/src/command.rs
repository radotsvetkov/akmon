//! Messages sent from the TUI to the agent task (policy verdicts and flow control).

/// User-driven control sent asynchronously to the agent orchestrator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiCommand {
    /// Answer a pending interactive policy prompt.
    Confirm {
        /// When `true`, the policy engine records an allow verdict; otherwise deny.
        allow: bool,
        /// When `true` with `allow`, identical permissions auto-approve for the rest of this session.
        remember_for_session: bool,
        /// When `true` with `allow`, all file writes are auto-approved until the session ends.
        allow_all_writes_session: bool,
        /// When `allow` and set, shell commands starting with this prefix skip further prompts.
        shell_allow_prefix: Option<String>,
    },
    /// Request a graceful stop after the in-flight tool batch completes.
    Interrupt,
}
