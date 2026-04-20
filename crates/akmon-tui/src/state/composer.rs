//! Input/composer state for the bottom draft area.

/// Editor state for the compose input.
#[derive(Debug, Clone, Default)]
pub struct ComposerState {
    /// Draft text (may include newlines).
    pub buffer: String,
    /// Caret byte index within [`Self::buffer`].
    pub cursor: usize,
}

impl ComposerState {
    /// Empty composer at cursor zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears the draft and resets the caret.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
    }

    /// Takes the current buffer and returns trimmed content when non-empty.
    pub fn submit_trimmed(&mut self) -> Option<String> {
        if self.buffer.trim().is_empty() {
            return None;
        }
        let raw = std::mem::take(&mut self.buffer);
        self.cursor = 0;
        Some(raw.trim().to_string())
    }

    /// Inserts one character at the caret.
    pub fn insert(&mut self, ch: char) -> bool {
        const MAX_INPUT_BYTES: usize = 512 * 1024;
        if self.buffer.len() >= MAX_INPUT_BYTES {
            return false;
        }
        let idx = self.cursor.min(self.buffer.len());
        self.buffer.insert(idx, ch);
        self.cursor = self.cursor.saturating_add(ch.len_utf8());
        true
    }

    /// Inserts text at the caret, capped by max input bytes.
    pub fn paste(&mut self, text: &str) {
        const MAX_INPUT_BYTES: usize = 512 * 1024;
        if self.buffer.len() >= MAX_INPUT_BYTES {
            return;
        }
        let idx = self.cursor.min(self.buffer.len());
        let remain = MAX_INPUT_BYTES.saturating_sub(self.buffer.len());
        if remain == 0 {
            return;
        }
        let mut end = text.len().min(remain);
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        self.buffer.insert_str(idx, &text[..end]);
        self.cursor += end;
    }

    /// Removes the character before the caret.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let idx = self.cursor.saturating_sub(1);
        if self.buffer.is_char_boundary(idx) {
            self.buffer.remove(idx);
            self.cursor = idx;
        }
    }

    /// Removes the character under the caret.
    pub fn delete_forward(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        if self.buffer.is_char_boundary(self.cursor) {
            self.buffer.remove(self.cursor);
        }
    }

    /// Moves the caret one character left.
    pub fn cursor_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let slice = &self.buffer[..self.cursor];
        let prev = slice
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.cursor = prev;
    }

    /// Moves the caret one character right.
    pub fn cursor_right(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        let slice = &self.buffer[self.cursor..];
        let mut it = slice.chars();
        if let Some(c) = it.next() {
            self.cursor += c.len_utf8();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ComposerState;

    #[test]
    fn insert_paste_backspace_delete_and_cursor_movement() {
        let mut s = ComposerState::new();
        assert!(s.insert('a'));
        assert!(s.insert('β'));
        assert_eq!(s.buffer, "aβ");
        assert_eq!(s.cursor, 3);

        s.cursor_left();
        assert_eq!(s.cursor, 1);
        s.paste("XYZ");
        assert_eq!(s.buffer, "aXYZβ");
        assert_eq!(s.cursor, 4);

        s.backspace();
        assert_eq!(s.buffer, "aXYβ");
        assert_eq!(s.cursor, 3);
        s.delete_forward();
        assert_eq!(s.buffer, "aXY");
    }
}
