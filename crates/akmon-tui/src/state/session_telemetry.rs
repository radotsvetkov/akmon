//! Session-level counters and touched-file telemetry.

use akmon_core::ModelCostEstimateRow;

use crate::render::context_usage_percent;

/// Session telemetry and counters reflected in status bars and summaries.
#[derive(Debug, Clone, Default)]
pub struct SessionTelemetryState {
    /// Cumulative provider input tokens.
    pub total_input_tokens: u32,
    /// Cumulative provider cache-read tokens.
    pub total_cache_read_tokens: u32,
    /// Cumulative provider output tokens.
    pub total_output_tokens: u32,
    /// Whether 80% context warning was already shown.
    pub context_warn_80_shown: bool,
    /// Whether 90% context warning was already shown.
    pub context_warn_90_shown: bool,
    /// Cumulative cache-write tokens.
    pub total_cache_write_tokens: u32,
    /// Estimated tokens reclaimed by micro-compact.
    pub total_microcompact_cleared: u32,
    /// User messages submitted.
    pub message_count: u32,
    /// Total completed tool calls.
    pub total_tool_calls: u32,
    /// Successful tool calls.
    pub successful_tool_calls: u32,
    /// Failed tool calls.
    pub failed_tool_calls: u32,
    /// Distinct files read.
    pub files_read: Vec<String>,
    /// Distinct files written/edited.
    pub files_written: Vec<String>,
    /// Distinct touched files (read or write).
    pub session_touched_files: Vec<String>,
}

impl SessionTelemetryState {
    fn push_unique(list: &mut Vec<String>, path: String) {
        if path.is_empty() || list.iter().any(|p| p == &path) {
            return;
        }
        list.push(path);
    }

    /// Records one touched file.
    pub fn note_touched_file(&mut self, path: &str) {
        Self::push_unique(&mut self.session_touched_files, path.to_string());
    }

    /// Tracks a successful read for `path`.
    pub fn note_file_read(&mut self, path: &str) {
        Self::push_unique(&mut self.files_read, path.to_string());
        self.note_touched_file(path);
    }

    /// Tracks a successful write/edit for `path`.
    pub fn note_file_written(&mut self, path: &str) {
        Self::push_unique(&mut self.files_written, path.to_string());
        self.note_touched_file(path);
    }

    /// Applies a usage report and returns an optional context warning flash.
    pub fn apply_usage(
        &mut self,
        input_tokens: u32,
        output_tokens: u32,
        cache_creation_tokens: u32,
        cache_read_tokens: u32,
        model_name: &str,
        model_estimates: &[ModelCostEstimateRow],
    ) -> Option<String> {
        self.total_input_tokens = self.total_input_tokens.saturating_add(input_tokens);
        self.total_output_tokens = self.total_output_tokens.saturating_add(output_tokens);
        self.total_cache_read_tokens = self
            .total_cache_read_tokens
            .saturating_add(cache_read_tokens);
        self.total_cache_write_tokens = self
            .total_cache_write_tokens
            .saturating_add(cache_creation_tokens);
        let pct = context_usage_percent(
            self.total_input_tokens,
            self.total_cache_read_tokens,
            model_name,
            model_estimates,
        );
        if pct >= 90 && !self.context_warn_90_shown {
            self.context_warn_90_shown = true;
            self.context_warn_80_shown = true;
            return Some("─ context at 90% — auto-compact will trigger soon ─".into());
        }
        if pct >= 80 && !self.context_warn_80_shown {
            self.context_warn_80_shown = true;
            return Some("─ context at 80% — consider /compact soon ─".into());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::SessionTelemetryState;

    #[test]
    fn usage_accumulates_and_warns_at_thresholds() {
        let mut s = SessionTelemetryState::default();
        let flash_80 = s.apply_usage(7_000, 0, 0, 0, "llama3.2", &[]);
        assert!(flash_80.is_some());
        let flash_90 = s.apply_usage(1_000, 0, 0, 0, "llama3.2", &[]);
        assert!(flash_90.is_some());
        assert!(s.context_warn_80_shown);
        assert!(s.context_warn_90_shown);
    }

    #[test]
    fn touched_files_are_deduped() {
        let mut s = SessionTelemetryState::default();
        s.note_file_read("src/main.rs");
        s.note_file_read("src/main.rs");
        s.note_file_written("src/lib.rs");
        assert_eq!(s.session_touched_files, vec!["src/main.rs", "src/lib.rs"]);
    }
}
