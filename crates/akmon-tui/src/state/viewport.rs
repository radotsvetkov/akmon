#![allow(dead_code)]
//! Scrollable transcript viewport state.

/// Tracks scroll position, pin-to-bottom behavior, and “new below” hints.
#[derive(Debug, Clone)]
pub struct ViewportState {
    /// Index of the first visible flattened line (0 = top of history).
    pub scroll_offset: usize,
    /// Total flattened lines last rendered (updated each draw).
    pub total_lines: usize,
    /// When `true`, new content keeps the view at the bottom.
    pub pinned_to_bottom: bool,
    /// `true` after the user scrolls up away from the latest line.
    pub user_scrolled: bool,
    /// `true` when new messages arrived while scrolled up.
    pub has_new_below: usize,
}

impl Default for ViewportState {
    fn default() -> Self {
        Self {
            scroll_offset: 0,
            total_lines: 0,
            pinned_to_bottom: true,
            user_scrolled: false,
            has_new_below: 0,
        }
    }
}

impl ViewportState {
    /// Creates a viewport pinned to the bottom.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Max scroll offset for a viewport of `viewport_height` lines.
    #[must_use]
    pub fn max_offset(&self, viewport_height: usize) -> usize {
        self.total_lines.saturating_sub(viewport_height)
    }

    /// Clamps `scroll_offset` to valid range.
    pub fn clamp_offset(&mut self, viewport_height: usize) {
        let m = self.max_offset(viewport_height);
        if self.scroll_offset > m {
            self.scroll_offset = m;
        }
    }

    /// Scroll up by `delta`; unpins from bottom.
    pub fn scroll_up(&mut self, delta: usize, viewport_height: usize) {
        let base = if self.pinned_to_bottom {
            self.max_offset(viewport_height)
        } else {
            self.scroll_offset
        };
        self.pinned_to_bottom = false;
        self.user_scrolled = true;
        self.scroll_offset = base.saturating_sub(delta);
    }

    /// Scroll down by `delta`; re-pins when reaching bottom.
    pub fn scroll_down(&mut self, delta: usize, viewport_height: usize) {
        let base = if self.pinned_to_bottom {
            self.max_offset(viewport_height)
        } else {
            self.scroll_offset
        };
        let m = self.max_offset(viewport_height);
        self.scroll_offset = (base + delta).min(m);
        if self.scroll_offset >= m {
            self.pinned_to_bottom = true;
            self.user_scrolled = false;
            self.has_new_below = 0;
        }
    }

    /// Jumps to the top of history.
    pub fn scroll_top(&mut self) {
        self.scroll_offset = 0;
        self.pinned_to_bottom = false;
        self.user_scrolled = true;
    }

    /// Jumps to bottom and pins.
    pub fn scroll_bottom(&mut self, viewport_height: usize) {
        self.scroll_offset = self.max_offset(viewport_height);
        self.pinned_to_bottom = true;
        self.user_scrolled = false;
        self.has_new_below = 0;
    }

    /// Called when transcript length changes after messages update.
    pub fn on_total_lines_changed(&mut self, new_total: usize, viewport_height: usize) {
        let prev_total = self.total_lines;
        self.total_lines = new_total;
        if self.pinned_to_bottom {
            self.scroll_offset = self.max_offset(viewport_height);
        } else if new_total > prev_total && self.user_scrolled {
            self.has_new_below = self
                .has_new_below
                .saturating_add(new_total.saturating_sub(prev_total));
        }
        self.clamp_offset(viewport_height);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_offset_clamped() {
        let mut v = ViewportState {
            total_lines: 10,
            ..ViewportState::default()
        };
        v.scroll_up(100, 5);
        assert!(v.scroll_offset <= v.max_offset(5));
    }

    #[test]
    fn pin_restored_at_bottom() {
        let mut v = ViewportState {
            total_lines: 20,
            ..ViewportState::default()
        };
        v.scroll_up(5, 5);
        assert!(!v.pinned_to_bottom);
        v.scroll_down(100, 5);
        assert!(v.pinned_to_bottom);
    }
}
