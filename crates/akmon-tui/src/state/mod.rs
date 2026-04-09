//! Sub-state for viewport scrolling, input, spinner, and overlays.

pub mod agent_display;
pub mod config_overlay;
pub mod dialog;
pub mod input;
pub mod spinner;
pub mod viewport;

pub use agent_display::AgentDisplayState;
pub use config_overlay::{
    ConfigTab, ESTIMATE_ROW_CANCEL, ESTIMATE_ROW_SAVE, ModelEstimateEditorState,
    SettingsOverlayState, merge_model_estimate_into_global,
};
pub use dialog::{ConfirmChoice, ConfirmationDialog, OperationType};
