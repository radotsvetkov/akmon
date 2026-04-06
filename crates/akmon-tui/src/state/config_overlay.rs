#![allow(dead_code)]
//! Full-screen settings UI (/config, Ctrl+S).

/// Top-level configuration tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfigTab {
    /// Model picker and local/cloud lists.
    #[default]
    Model,
    /// Provider connectivity.
    Providers,
    /// Session policy toggles (display only until wired to disk).
    Permissions,
    /// Version and paths.
    About,
}

/// Field being edited inline (e.g. API key).
#[derive(Debug, Clone)]
pub enum ConfigField {
    /// Provider id / label.
    ProviderKey {
        /// Display name.
        label: String,
        /// Masked buffer.
        buffer: String,
    },
}

/// Stateful settings overlay.
#[derive(Debug, Clone, Default)]
pub struct ConfigOverlay {
    /// Active tab.
    pub tab: ConfigTab,
    /// Row index within the tab panel.
    pub selected_row: usize,
    /// Inline editor when set.
    pub editing: Option<ConfigField>,
}

impl ConfigOverlay {
    /// New overlay on the About tab (safe default).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cycles tab forward.
    pub fn next_tab(&mut self) {
        self.tab = match self.tab {
            ConfigTab::Model => ConfigTab::Providers,
            ConfigTab::Providers => ConfigTab::Permissions,
            ConfigTab::Permissions => ConfigTab::About,
            ConfigTab::About => ConfigTab::Model,
        };
        self.selected_row = 0;
        self.editing = None;
    }

    /// Cycles tab backward.
    pub fn prev_tab(&mut self) {
        self.tab = match self.tab {
            ConfigTab::Model => ConfigTab::About,
            ConfigTab::Providers => ConfigTab::Model,
            ConfigTab::Permissions => ConfigTab::Providers,
            ConfigTab::About => ConfigTab::Permissions,
        };
        self.selected_row = 0;
        self.editing = None;
    }
}
