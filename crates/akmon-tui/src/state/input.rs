#![allow(dead_code)]
//! Multi-line compose buffer with optional history navigation.

/// Editor state for the bottom compose area.
#[derive(Debug, Clone, Default)]
pub struct InputState {
    /// Draft text (may include newlines).
    pub buffer: String,
    /// Caret as a UTF-8 byte index into [`Self::buffer`].
    pub cursor_pos: usize,
    /// Submitted lines for ↑/↓ recall (newest last).
    pub history: Vec<String>,
    /// Index into `history` when navigating, if any.
    pub history_idx: Option<usize>,
}

impl InputState {
    /// Empty buffer at column zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a submitted line into history and clears the draft.
    pub fn push_history(&mut self, line: String) {
        if !line.trim().is_empty() {
            self.history.push(line);
        }
        self.history_idx = None;
        self.buffer.clear();
        self.cursor_pos = 0;
    }

    /// Navigates backward through history when the caret is on the first line.
    pub fn history_prev(&mut self, on_first_line: bool) -> bool {
        if !on_first_line || self.history.is_empty() {
            return false;
        }
        let idx = match self.history_idx {
            None => self.history.len().saturating_sub(1),
            Some(i) => i.saturating_sub(1),
        };
        if let Some(entry) = self.history.get(idx) {
            self.history_idx = Some(idx);
            self.buffer.clone_from(entry);
            self.cursor_pos = self.buffer.len();
            return true;
        }
        false
    }

    /// Navigates forward through history toward the empty draft.
    pub fn history_next(&mut self) -> bool {
        let Some(cur) = self.history_idx else {
            return false;
        };
        let next = cur.saturating_add(1);
        if next >= self.history.len() {
            self.history_idx = None;
            self.buffer.clear();
            self.cursor_pos = 0;
            return true;
        }
        if let Some(entry) = self.history.get(next) {
            self.history_idx = Some(next);
            self.buffer.clone_from(entry);
            self.cursor_pos = self.buffer.len();
            return true;
        }
        false
    }
}
