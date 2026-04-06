//! Mouse hit-testing and wrap-aware row counts for the compose area.

/// Logical input buffer may grow large; UI caps visible newline splits for hit testing.
pub const INPUT_UI_LINE_CAP: usize = 12;

/// Clamps `byte` to `[0, buffer.len()]` and snaps backward to a valid UTF-8 boundary.
#[must_use]
pub fn snap_utf8_cursor(buffer: &str, byte: usize) -> usize {
    let byte = byte.min(buffer.len());
    if buffer.is_empty() {
        return 0;
    }
    if buffer.is_char_boundary(byte) {
        return byte;
    }
    let mut p = byte;
    while p > 0 && !buffer.is_char_boundary(p) {
        p -= 1;
    }
    p
}

/// Rows one logical line occupies when rendered with a 2-column prefix and wrap.
#[must_use]
pub fn visual_row_count_for_wrapped_line(line: &str, inner_width: u16) -> usize {
    let iw = inner_width.max(3) as usize;
    let prefix = 2usize;
    let first_avail = iw.saturating_sub(prefix).max(1);
    let c = line.chars().count();
    if c <= first_avail {
        return 1;
    }
    let rest = c - first_avail;
    1 + rest.saturating_add(iw - 1) / iw
}

/// Total visible rows for the compose buffer (wrap-aware).
#[must_use]
pub fn input_body_row_count(buffer: &str, inner_width: u16) -> usize {
    let iw = inner_width.max(8);
    let mut total = 0usize;
    for line in buffer.split('\n').take(INPUT_UI_LINE_CAP) {
        total += visual_row_count_for_wrapped_line(line, iw);
    }
    total.max(1)
}

fn line_start_byte(buffer: &str, line_idx: usize) -> usize {
    let mut n = 0usize;
    for (i, seg) in buffer.split('\n').enumerate() {
        if i == line_idx {
            return n;
        }
        n = n.saturating_add(seg.len()).saturating_add(1);
    }
    buffer.len()
}

fn byte_at_char_index_in_line(line: &str, char_idx: usize) -> usize {
    if char_idx == 0 {
        return 0;
    }
    line.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(line.len())
}

/// Maps a mouse cell (relative to inner input rect) to a UTF-8 byte offset in `buffer`.
#[must_use]
pub fn map_input_click_wrapped(
    buffer: &str,
    inner_width: u16,
    rel_row: usize,
    rel_col: usize,
) -> usize {
    if buffer.is_empty() {
        return 0;
    }
    let iw = inner_width.max(3) as usize;
    let prefix = 2usize;
    let first_avail = iw.saturating_sub(prefix).max(1);
    let mut row_acc = 0usize;
    let lines: Vec<&str> = buffer.split('\n').take(INPUT_UI_LINE_CAP).collect();
    for (line_idx, line) in lines.iter().enumerate() {
        let vrows = visual_row_count_for_wrapped_line(line, inner_width);
        if rel_row < row_acc + vrows {
            let local = rel_row - row_acc;
            let n = line.chars().count();
            let char_idx = if local == 0 {
                let col = rel_col.saturating_sub(prefix);
                col.min(first_avail.min(n))
            } else {
                let skip = first_avail.saturating_add((local - 1).saturating_mul(iw));
                let col = rel_col.min(iw.saturating_sub(1));
                (skip + col).min(n)
            };
            let start = line_start_byte(buffer, line_idx);
            let b = start + byte_at_char_index_in_line(line, char_idx);
            return snap_utf8_cursor(buffer, b);
        }
        row_acc += vrows;
    }
    buffer.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn map_input_click_to_byte_index(
        buffer: &str,
        rel_row: usize,
        rel_col: usize,
    ) -> usize {
        if buffer.is_empty() {
            return 0;
        }
        let lines: Vec<&str> = buffer.split('\n').take(INPUT_UI_LINE_CAP).collect();
        if rel_row >= lines.len() {
            return buffer.len();
        }
        let mut line_start = 0usize;
        for (i, line) in lines.iter().enumerate() {
            if i == rel_row {
                let col_in_content = rel_col.saturating_sub(2);
                let char_idx = col_in_content.min(line.chars().count());
                let byte_in_line = line
                    .char_indices()
                    .nth(char_idx)
                    .map(|(b, _)| b)
                    .unwrap_or(line.len());
                return snap_utf8_cursor(buffer, line_start + byte_in_line);
            }
            line_start = line_start.saturating_add(line.len()).saturating_add(1);
        }
        buffer.len()
    }

    #[test]
    fn click_column_maps() {
        assert_eq!(map_input_click_to_byte_index("abcdef", 0, 5), 3);
    }
}
