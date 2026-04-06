//! Word-wrapping using display width (CJK-aware via `unicode-width`).

use unicode_width::UnicodeWidthChar;

/// Wraps `text` into lines that fit `width` display columns.
///
/// Splits on whitespace; a single word longer than `width` is truncated with an ellipsis
/// (one display column reserved when `width > 1`).
#[must_use]
pub fn wrap_text(text: &str, width: u16) -> Vec<String> {
    let w = width.max(1) as usize;
    if w == 0 {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut line_width = 0usize;

    let flush = |line: &mut String, line_width: &mut usize, out: &mut Vec<String>| {
        if !line.is_empty() {
            out.push(std::mem::take(line));
            *line_width = 0;
        }
    };

    for word in text.split_whitespace() {
        let word_w = display_width_str(word);
        let gap = if line.is_empty() { 0 } else { 1 };
        if line_width + gap + word_w <= w {
            if gap == 1 {
                line.push(' ');
                line_width += 1;
            }
            line.push_str(word);
            line_width += word_w;
            continue;
        }
        if !line.is_empty() {
            flush(&mut line, &mut line_width, &mut out);
        }
        if word_w <= w {
            line.push_str(word);
            line_width = word_w;
        } else {
            line.push_str(&truncate_to_width(word, w));
            flush(&mut line, &mut line_width, &mut out);
        }
    }
    flush(&mut line, &mut line_width, &mut out);
    if out.is_empty() && !text.is_empty() {
        out.push(String::new());
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

#[must_use]
fn display_width_str(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

#[must_use]
fn truncate_to_width(s: &str, max_w: usize) -> String {
    if max_w <= 1 {
        return "…".to_string();
    }
    let mut acc = 0usize;
    let mut out = String::new();
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if acc + cw > max_w.saturating_sub(1) {
            out.push('…');
            break;
        }
        out.push(ch);
        acc += cw;
    }
    if out.is_empty() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_words() {
        let lines = wrap_text("hello world foo", 10);
        assert!(lines.iter().any(|l| l.contains("hello")));
    }

    #[test]
    fn long_word_truncated() {
        let lines = wrap_text("abcdefghijklmnop", 6);
        assert!(lines[0].contains('…'));
    }
}
