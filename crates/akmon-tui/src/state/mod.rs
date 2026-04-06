//! Sub-state for viewport scrolling, input, spinner, and overlays.

pub mod agent_display;
pub mod config_overlay;
pub mod dialog;
pub mod input;
pub mod spinner;
pub mod viewport;

pub use agent_display::AgentDisplayState;
pub use dialog::{ConfirmChoice, ConfirmationDialog, OperationType};
