/// Replay execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayMode {
    /// Semantic replay behavior: tolerate selected divergences and continue.
    Default,
    /// Strict replay behavior: enforce strict mismatch handling.
    Strict,
}

impl std::fmt::Display for ReplayMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => f.write_str("default"),
            Self::Strict => f.write_str("strict"),
        }
    }
}
