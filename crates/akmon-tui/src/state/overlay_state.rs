//! Overlay/modal and slash-autocomplete state.

use crate::slash::SlashCommand;
use crate::state::ConfirmationDialog;

/// One line in model-picker overlays.
#[derive(Debug, Clone)]
pub struct ModelPickerRow {
    /// Header rows are not selectable.
    pub section_header: bool,
    /// Selectable row marker.
    pub selectable: bool,
    /// Raw model id / label.
    pub label: String,
    /// Optional custom display text.
    pub display: Option<String>,
}

/// State for an `ask_followup` prompt.
#[derive(Debug, Clone)]
pub struct QuestionPromptState {
    /// Tool call id matching the transcript row.
    pub call_id: String,
    /// Prompt text.
    pub question: String,
    /// Suggested quick replies.
    pub suggestions: Vec<String>,
}

/// Modal overlay drawn over transcript/input.
#[derive(Debug)]
pub enum Overlay {
    /// No overlay.
    None,
    /// Help popup.
    Help,
    /// Session picker.
    SessionList {
        /// Session rows.
        sessions: Vec<crate::session_persist::SessionSummary>,
        /// Selected row.
        selected: usize,
        /// Scroll offset.
        scroll: usize,
    },
    /// Audit viewer.
    AuditLog {
        /// Pre-rendered lines.
        lines: Vec<String>,
        /// Scroll offset.
        scroll: usize,
    },
    /// Generic scroll-text overlay.
    ScrollText {
        /// Overlay title.
        title: String,
        /// Source lines.
        lines: Vec<String>,
        /// Scroll offset.
        scroll: usize,
    },
    /// Cost summary panel.
    CostSummary,
    /// Model picker panel.
    ModelPicker {
        /// Rows.
        rows: Vec<ModelPickerRow>,
        /// Selectable row indices.
        selectable: Vec<usize>,
        /// Index within selectable rows.
        selected: usize,
        /// First visible row.
        scroll: usize,
    },
    /// Slash command autocomplete popup.
    SlashAutocomplete {
        /// Candidate commands.
        matches: Vec<&'static SlashCommand>,
        /// Selected candidate.
        selected: usize,
    },
    /// Settings overlay.
    Settings(crate::state::SettingsOverlayState),
}

/// Overlay composition state owned by [`crate::TuiApp`].
#[derive(Debug)]
pub struct OverlayState {
    /// Active overlay.
    pub overlay: Overlay,
    /// Confirmation gate active.
    pub awaiting_confirmation: bool,
    /// Optional confirmation dialog details.
    pub confirmation_dialog: Option<ConfirmationDialog>,
    /// Ask-followup answer mode active.
    pub awaiting_question: bool,
    /// Pending question payload.
    pub question_prompt: Option<QuestionPromptState>,
    /// Slash autocomplete selected index.
    pub slash_ac_selected: usize,
    /// Signature for stable slash selection.
    pub slash_ac_sig: String,
    /// Hide slash autocomplete until next buffer edit.
    pub slash_ac_suppress: bool,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            overlay: Overlay::None,
            awaiting_confirmation: false,
            confirmation_dialog: None,
            awaiting_question: false,
            question_prompt: None,
            slash_ac_selected: 0,
            slash_ac_sig: String::new(),
            slash_ac_suppress: false,
        }
    }
}

impl OverlayState {
    /// Creates an empty overlay state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks confirmation as answered and clears modal gate state.
    pub fn mark_confirmation_answered(&mut self) {
        self.awaiting_confirmation = false;
        self.confirmation_dialog = None;
    }

    /// Resets slash-autocomplete suppression and filtering signatures.
    pub fn reset_slash_autocomplete_state(&mut self) {
        self.slash_ac_suppress = false;
    }
}

#[cfg(test)]
mod tests {
    use super::{Overlay, OverlayState};

    #[test]
    fn confirmation_open_then_answered_clears_gate() {
        let mut s = OverlayState::new();
        s.awaiting_confirmation = true;
        s.mark_confirmation_answered();
        assert!(!s.awaiting_confirmation);
        assert!(s.confirmation_dialog.is_none());
    }

    #[test]
    fn slash_selection_and_reset_behavior() {
        let mut s = OverlayState::new();
        s.slash_ac_selected = 3;
        s.slash_ac_sig = "abc".into();
        s.slash_ac_suppress = true;
        s.overlay = Overlay::None;
        s.reset_slash_autocomplete_state();
        assert!(!s.slash_ac_suppress);
        assert_eq!(s.slash_ac_selected, 3);
        assert_eq!(s.slash_ac_sig, "abc");
    }
}
