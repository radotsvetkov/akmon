//! Full-screen settings UI (`/config`, Ctrl+S).

use akmon_config::AkmonGlobalConfig;
use akmon_core::ModelCostEstimateRow;
use akmon_core::{context_window_tokens_hint, match_model_cost_row};

use crate::app::TuiApp;

/// Top-level configuration tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfigTab {
    /// Model selection notes.
    #[default]
    Model,
    /// Provider / API key hints.
    Providers,
    /// Context window & USD hints for the status bar and cost estimate.
    Estimates,
    /// Session policy (display / future wiring).
    Permissions,
    /// Version and paths.
    About,
}

impl ConfigTab {
    /// Label for the tab strip.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            ConfigTab::Model => "Model",
            ConfigTab::Providers => "Providers",
            ConfigTab::Estimates => "Estimates",
            ConfigTab::Permissions => "Permissions",
            ConfigTab::About => "About",
        }
    }

    /// Cycle forward.
    pub fn next(self) -> Self {
        match self {
            ConfigTab::Model => ConfigTab::Providers,
            ConfigTab::Providers => ConfigTab::Estimates,
            ConfigTab::Estimates => ConfigTab::Permissions,
            ConfigTab::Permissions => ConfigTab::About,
            ConfigTab::About => ConfigTab::Model,
        }
    }

    /// Cycle backward.
    pub fn prev(self) -> Self {
        match self {
            ConfigTab::Model => ConfigTab::About,
            ConfigTab::Providers => ConfigTab::Model,
            ConfigTab::Estimates => ConfigTab::Providers,
            ConfigTab::Permissions => ConfigTab::Estimates,
            ConfigTab::About => ConfigTab::Permissions,
        }
    }
}

/// Row index for **[Save]** in the estimates form.
pub const ESTIMATE_ROW_SAVE: usize = 6;
/// Row index for **[Cancel]**.
pub const ESTIMATE_ROW_CANCEL: usize = 7;

/// Draft row for `[[model_estimates]]` tied to the current session model.
#[derive(Debug, Clone)]
pub struct ModelEstimateEditorState {
    /// Pattern of the row being edited (`None` if this was a new row).
    pub original_pattern: Option<String>,
    /// Match substring (stored in config).
    pub pattern: String,
    pub context_window: String,
    pub input_m: String,
    pub output_m: String,
    pub cache_read_m: String,
    pub note: String,
    /// 0..=5 editable fields; [`ESTIMATE_ROW_SAVE`]; [`ESTIMATE_ROW_CANCEL`].
    pub selected: usize,
    pub editing: bool,
    pub status_line: Option<String>,
}

impl ModelEstimateEditorState {
    /// Pre-fill from the active model and existing `model_estimates` match (if any).
    #[must_use]
    pub fn from_app(app: &TuiApp) -> Self {
        let matched = match_model_cost_row(&app.model_name, &app.model_estimates);
        let original_pattern = matched.map(|r| r.pattern.clone());
        let pattern = matched
            .map(|r| r.pattern.clone())
            .unwrap_or_else(|| app.model_name.clone());
        let context_window = matched
            .and_then(|r| r.context_window_tokens)
            .map(|n| n.to_string())
            .unwrap_or_default();
        let fmt_opt_f64 = |v: Option<f64>| -> String {
            v.map(|x| {
                if x.fract() == 0.0 {
                    format!("{}", x as i64)
                } else {
                    format!("{x}")
                }
            })
            .unwrap_or_default()
        };
        let input_m = matched
            .map(|r| fmt_opt_f64(r.input_per_million_usd))
            .unwrap_or_default();
        let output_m = matched
            .map(|r| fmt_opt_f64(r.output_per_million_usd))
            .unwrap_or_default();
        let cache_read_m = matched
            .map(|r| fmt_opt_f64(r.cache_read_per_million_usd))
            .unwrap_or_default();
        let note = matched.and_then(|r| r.note.clone()).unwrap_or_default();
        Self {
            original_pattern,
            pattern,
            context_window,
            input_m,
            output_m,
            cache_read_m,
            note,
            selected: 0,
            editing: false,
            status_line: None,
        }
    }

    /// Hint used when context field is blank.
    #[must_use]
    pub fn builtin_context_hint(&self, app: &TuiApp) -> u64 {
        context_window_tokens_hint(&app.model_name, &app.model_estimates)
    }

    fn field_mut(&mut self, row: usize) -> Option<&mut String> {
        match row {
            0 => Some(&mut self.pattern),
            1 => Some(&mut self.context_window),
            2 => Some(&mut self.input_m),
            3 => Some(&mut self.output_m),
            4 => Some(&mut self.cache_read_m),
            5 => Some(&mut self.note),
            _ => None,
        }
    }

    /// Apply a typed character to the active field when `editing` is set.
    pub fn insert_char(&mut self, c: char) {
        if !self.editing {
            return;
        }
        let Some(buf) = self.field_mut(self.selected) else {
            return;
        };
        buf.push(c);
    }

    pub fn backspace(&mut self) {
        if !self.editing {
            return;
        }
        let Some(buf) = self.field_mut(self.selected) else {
            return;
        };
        buf.pop();
    }

    /// Build a [`ModelCostEstimateRow`] or validation error.
    pub fn parse_row(&self) -> Result<ModelCostEstimateRow, String> {
        let pattern = self.pattern.trim().to_string();
        if pattern.is_empty() {
            return Err("Pattern must not be empty.".into());
        }
        let context_window_tokens = {
            let t = self.context_window.trim();
            if t.is_empty() {
                None
            } else {
                Some(
                    t.parse::<u64>()
                        .map_err(|_| format!("Invalid context window: {t}"))?,
                )
            }
        };
        let parse_f = |s: &str, label: &str| -> Result<Option<f64>, String> {
            let t = s.trim();
            if t.is_empty() {
                return Ok(None);
            }
            t.parse::<f64>()
                .map(Some)
                .map_err(|_| format!("Invalid {label}: {t}"))
        };
        let input_per_million_usd = parse_f(&self.input_m, "input USD/M")?;
        let output_per_million_usd = parse_f(&self.output_m, "output USD/M")?;
        let cache_read_per_million_usd = parse_f(&self.cache_read_m, "cache-read USD/M")?;
        let note = {
            let t = self.note.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        };
        Ok(ModelCostEstimateRow {
            pattern,
            context_window_tokens,
            input_per_million_usd,
            output_per_million_usd,
            cache_read_per_million_usd,
            note,
        })
    }
}

/// Stateful settings overlay (`/config`).
#[derive(Debug, Clone)]
pub struct SettingsOverlayState {
    /// Active tab.
    pub tab: ConfigTab,
    /// Editor for the Estimates tab.
    pub estimate: ModelEstimateEditorState,
}

impl SettingsOverlayState {
    /// Opens settings focused on **Estimates** for the current model.
    #[must_use]
    pub fn open_estimates(app: &TuiApp) -> Self {
        Self {
            tab: ConfigTab::Estimates,
            estimate: ModelEstimateEditorState::from_app(app),
        }
    }
}

/// Merge one estimate row into user global config (replace by `original_pattern` and/or new `pattern`).
pub fn merge_model_estimate_into_global(
    global: &mut AkmonGlobalConfig,
    original_pattern: Option<&str>,
    row: ModelCostEstimateRow,
) {
    if let Some(p) = original_pattern {
        global.model_estimates.retain(|r| r.pattern != p);
    }
    global.model_estimates.retain(|r| r.pattern != row.pattern);
    global.model_estimates.push(row);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_replaces_by_original_and_new_pattern() {
        let mut g = AkmonGlobalConfig::default();
        g.model_estimates.push(ModelCostEstimateRow {
            pattern: "a".into(),
            context_window_tokens: Some(1),
            input_per_million_usd: None,
            output_per_million_usd: None,
            cache_read_per_million_usd: None,
            note: None,
        });
        let row = ModelCostEstimateRow {
            pattern: "b".into(),
            context_window_tokens: Some(2),
            input_per_million_usd: None,
            output_per_million_usd: None,
            cache_read_per_million_usd: None,
            note: None,
        };
        merge_model_estimate_into_global(&mut g, Some("a"), row);
        assert_eq!(g.model_estimates.len(), 1);
        assert_eq!(g.model_estimates[0].pattern, "b");
    }
}
