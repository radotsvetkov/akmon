//! Root layout: header, viewport, optional context bar, input, status.

use ratatui::layout::Rect;

/// Cached layout split for the current terminal size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutRects {
    /// Top header line (`AKMON`, cwd, model).
    pub header: Rect,
    /// Scrollable transcript area.
    pub viewport: Rect,
    /// One line above input when session context is shown.
    pub context_bar: Option<Rect>,
    /// Slash-command autocomplete (`None` when height is 0).
    pub slash_autocomplete: Option<Rect>,
    /// Multiline compose area (bordered block).
    pub input: Rect,
    /// Bottom status line.
    pub status: Rect,
}

/// Minimum terminal dimensions before showing a resize warning.
pub const MIN_TERM_WIDTH: u16 = 60;
/// Minimum terminal height (rows).
pub const MIN_TERM_HEIGHT: u16 = 16;

/// Returns `true` when the terminal is too small for the full layout.
#[must_use]
pub fn terminal_too_small(area: Rect) -> bool {
    area.width < MIN_TERM_WIDTH || area.height < MIN_TERM_HEIGHT
}

/// Inner height from newline count only (clamped 3–8). The live TUI uses wrap-aware sizing in `runner`.
#[must_use]
#[allow(dead_code)]
pub fn input_inner_height(buffer: &str) -> u16 {
    let logical_lines = buffer.matches('\n').count().saturating_add(1);
    let with_pad = logical_lines.saturating_add(2);
    (with_pad.clamp(3, 8)) as u16
}

/// Total outer height of the input block including top and bottom borders.
#[must_use]
pub fn input_block_outer_height(inner_lines: u16) -> u16 {
    inner_lines.saturating_add(2)
}

/// Recomputes [`LayoutRects`] for the full terminal `area`.
///
/// `input_inner_h` is the number of content rows inside the bordered input (3–8).
/// `show_context` reserves one line between viewport and slash/input stack.
/// `slash_ac_outer_h` is the outer height of the slash autocomplete block (0 when hidden).
#[must_use]
pub fn compute_layout(
    area: Rect,
    show_context: bool,
    input_inner_h: u16,
    slash_ac_outer_h: u16,
) -> LayoutRects {
    let header_h = 1u16;
    let status_h = 1u16;
    let ctx_h = if show_context { 1u16 } else { 0u16 };
    let input_outer = input_block_outer_height(input_inner_h);
    let slash_h = slash_ac_outer_h;

    let stack_below_viewport = ctx_h
        .saturating_add(slash_h)
        .saturating_add(input_outer)
        .saturating_add(status_h);
    let viewport_h = area
        .height
        .saturating_sub(header_h)
        .saturating_sub(stack_below_viewport)
        .max(5);

    let y_after_header = area.y.saturating_add(header_h);
    let header = Rect::new(area.x, area.y, area.width, header_h);
    let viewport = Rect::new(area.x, y_after_header, area.width, viewport_h);

    let mut y = y_after_header.saturating_add(viewport_h);
    let context_bar = if show_context {
        let r = Rect::new(area.x, y, area.width, 1);
        y = y.saturating_add(1);
        Some(r)
    } else {
        None
    };

    let slash_autocomplete = if slash_h > 0 {
        let r = Rect::new(area.x, y, area.width, slash_h);
        y = y.saturating_add(slash_h);
        Some(r)
    } else {
        None
    };

    let input = Rect::new(area.x, y, area.width, input_outer);
    y = y.saturating_add(input_outer);
    let status = Rect::new(area.x, y, area.width, status_h);

    LayoutRects {
        header,
        viewport,
        context_bar,
        slash_autocomplete,
        input,
        status,
    }
}

/// Clips `inner` to `outer` (axis-aligned bounding boxes).
#[must_use]
pub fn intersect_rect(outer: Rect, inner: Rect) -> Rect {
    let x1 = inner.x.max(outer.x);
    let y1 = inner.y.max(outer.y);
    let x2 = (inner.x + inner.width).min(outer.x + outer.width);
    let y2 = (inner.y + inner.height).min(outer.y + outer.height);
    let w = x2.saturating_sub(x1);
    let h = y2.saturating_sub(y1);
    Rect::new(x1, y1, w, h)
}

/// Centers `w`×`h` inside `area`.
#[must_use]
pub fn centered_rect(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let x = area.x.saturating_add(area.width.saturating_sub(w) / 2);
    let y = area.y.saturating_add(area.height.saturating_sub(h) / 2);
    Rect::new(x, y, w, h)
}

/// Point-in-rectangle test for mouse and hit testing.
#[inline]
#[must_use]
pub fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x
        && col < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_fills_remaining() {
        let full = Rect::new(0, 0, 80, 24);
        let lr = compute_layout(full, false, 3, 0);
        let sum = 1 + lr.viewport.height + lr.input.height + lr.status.height;
        assert_eq!(sum, full.height);
    }

    #[test]
    fn viewport_subtracts_context_bar() {
        let full = Rect::new(0, 0, 80, 24);
        let a = compute_layout(full, false, 3, 0);
        let b = compute_layout(full, true, 3, 0);
        assert_eq!(b.viewport.height, a.viewport.height.saturating_sub(1));
    }

    #[test]
    fn slash_autocomplete_shrinks_viewport() {
        let full = Rect::new(0, 0, 80, 24);
        let no_ac = compute_layout(full, false, 3, 0);
        let with_ac = compute_layout(full, false, 3, 6);
        assert!(
            with_ac.viewport.height < no_ac.viewport.height,
            "expected autocomplete stack to reduce viewport height"
        );
        assert!(with_ac.slash_autocomplete.is_some());
    }

    #[test]
    fn input_inner_clamped() {
        assert_eq!(input_inner_height(""), 3);
        let many = "\n".repeat(20);
        assert_eq!(input_inner_height(&many), 8);
    }

    #[test]
    fn rect_contains_inside() {
        let r = Rect::new(2, 3, 10, 5);
        assert!(rect_contains(r, 2, 3));
        assert!(rect_contains(r, 11, 7));
        assert!(!rect_contains(r, 1, 3));
        assert!(!rect_contains(r, 2, 8));
    }
}
