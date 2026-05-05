//! Replay primitives for deterministic Akmon session playback.

#![warn(missing_docs)]

mod divergence;
mod engine;
mod error;
mod mode;
mod provider;
mod report;
mod tool;

pub use divergence::{ReplayDivergence, ReplayDivergenceCollector, ReplayDivergenceKind};
pub use engine::{
    ReplayEngine, ReplayEngineConfig, ReplayRunOutput, SourceSession, compare,
    compare_default_mode, compare_strict_mode, load_source_session_from_journal,
};
pub use error::ReplayError;
pub use mode::ReplayMode;
pub use provider::{PlaybackProvider, PlaybackProviderConfig};
pub use report::{ReplayInfraError, ReplayReportV1, assemble_report};
pub use tool::{PlaybackTool, PlaybackToolConfig};
