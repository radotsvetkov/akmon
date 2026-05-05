/// Replay execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayMode {
    /// Semantic replay behavior: tolerate selected divergences and continue.
    Default,
    /// Strict replay behavior: enforce strict mismatch handling.
    Strict,
}
