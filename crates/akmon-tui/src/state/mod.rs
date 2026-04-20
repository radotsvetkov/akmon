//! Sub-state for viewport scrolling, input, spinner, and overlays.

pub mod agent_display;
pub mod composer;
pub mod config_overlay;
pub mod dialog;
pub mod input;
pub mod overlay_state;
pub mod provider_runtime;
pub mod session_telemetry;
pub mod spinner;
pub mod viewport;

pub use agent_display::AgentDisplayState;
pub use composer::ComposerState;
pub use config_overlay::{
    ConfigTab, ESTIMATE_ROW_CANCEL, ESTIMATE_ROW_SAVE, ModelEstimateEditorState,
    SettingsOverlayState, merge_model_estimate_into_global,
};
pub use dialog::{ConfirmChoice, ConfirmationDialog, OperationType};
pub use overlay_state::{ModelPickerRow, Overlay, OverlayState, QuestionPromptState};
pub use provider_runtime::ProviderRuntimeState;
pub use session_telemetry::SessionTelemetryState;
