//! Diff primitives for deterministic Akmon session comparison.

#![warn(missing_docs)]

mod comparison;
mod divergence;
mod engine;
mod error;
mod mode;
mod report;

pub use comparison::{
    DiffComparison, StructuralBreak, compare_assistant_turn, compare_permission_gate,
    compare_provider_call, compare_session_end, compare_session_start, compare_tool_call,
    compare_user_turn,
};
pub use divergence::{DiffDivergence, DiffDivergenceKind};
pub use engine::DiffEngine;
pub use error::DiffError;
pub use mode::DiffMode;
pub use report::DiffReportV1;
