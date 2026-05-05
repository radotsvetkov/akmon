//! Diff primitives for deterministic Akmon session comparison.

#![warn(missing_docs)]

mod comparison;
mod divergence;
mod engine;
mod error;
mod mode;
mod report;
mod resolve;

pub use comparison::{
    DiffComparison, StructuralBreak, compare_assistant_turn, compare_permission_gate,
    compare_provider_call, compare_retrieval_call, compare_session_end, compare_session_start,
    compare_tool_call, compare_user_turn,
};
pub use divergence::{DiffDivergence, DiffDivergenceKind, ResolvedContent};
pub use engine::{DiffEngine, SourceSession, load_source_session_from_journal};
pub use error::DiffError;
pub use mode::DiffMode;
pub use report::DiffReportV1;
pub use resolve::{
    RESOLVE_READ_CAP_BYTES, RESOLVE_SKIP_EXCEEDS_CAP, RESOLVE_SKIP_NOT_DEREFERENCABLE,
    RESOLVE_SKIP_OBJECT_MISSING, ResolveContext,
};
