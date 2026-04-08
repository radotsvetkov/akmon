//! Permission / confirmation overlay model.

/// Classified operation for the permission dialog title and layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationType {
    /// Overwrite or write a file.
    WriteFile {
        /// Sandbox-relative path.
        path: String,
        /// Unified diff preview.
        diff: String,
    },
    /// Patch-style edit.
    EditFile {
        /// Path being edited.
        path: String,
        /// Unified diff.
        diff: String,
    },
    /// Shell invocation.
    RunShell {
        /// Full command string.
        command: String,
    },
    /// Git commit (future).
    GitCommit {
        /// Proposed message.
        message: String,
        /// Staged paths.
        files: Vec<String>,
    },
    /// HTTP fetch.
    WebFetch {
        /// Request URL.
        url: String,
    },
    /// Fallback.
    Generic {
        /// Policy description line(s).
        description: String,
    },
}

/// Focusable answers in the confirmation dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfirmChoice {
    /// Allow this action once.
    #[default]
    Allow,
    /// Remember for this session.
    AllowAlways,
    /// Allow broadly for the session (all writes, or shell prefix — see dialog copy).
    AllowBroad,
    /// Reject.
    Deny,
    /// Only used when diff is tall (scroll).
    ViewMore,
}

/// Stateful centered permission dialog.
#[derive(Debug, Clone)]
pub struct ConfirmationDialog {
    /// Window title.
    pub title: String,
    /// Structured operation.
    pub operation: OperationType,
    /// Raw diff or preview string.
    pub diff_or_preview: Option<String>,
    /// Keyboard- / mouse-focused option.
    pub selected_option: ConfirmChoice,
    /// Vertical scroll within the diff viewport.
    pub scroll_offset: u16,
    /// When `true`, [`ConfirmChoice::AllowBroad`] is part of the cycle (file/shell prompts).
    pub broad_choice_enabled: bool,
    /// Short label for the broad-allow row (includes leading space for alignment).
    pub broad_choice_label: String,
}

impl ConfirmationDialog {
    /// Moves focus to the next primary choice (Tab).
    pub fn cycle_choice(&mut self) {
        self.selected_option = match self.selected_option {
            ConfirmChoice::Allow => ConfirmChoice::AllowAlways,
            ConfirmChoice::AllowAlways => {
                if self.broad_choice_enabled {
                    ConfirmChoice::AllowBroad
                } else {
                    ConfirmChoice::Deny
                }
            }
            ConfirmChoice::AllowBroad => ConfirmChoice::Deny,
            ConfirmChoice::Deny | ConfirmChoice::ViewMore => ConfirmChoice::Allow,
        };
    }

    /// Moves focus to the previous choice (Shift-Tab).
    pub fn cycle_choice_back(&mut self) {
        self.selected_option = match self.selected_option {
            ConfirmChoice::Allow => ConfirmChoice::Deny,
            ConfirmChoice::AllowAlways => ConfirmChoice::Allow,
            ConfirmChoice::AllowBroad => ConfirmChoice::AllowAlways,
            ConfirmChoice::Deny | ConfirmChoice::ViewMore => {
                if self.broad_choice_enabled {
                    ConfirmChoice::AllowBroad
                } else {
                    ConfirmChoice::AllowAlways
                }
            }
        };
    }
}
