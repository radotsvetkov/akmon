//! Layout helpers: convert [`TuiMessage`](crate::message::TuiMessage) rows into ratatui [`Line`]s.

use akmon_core::ContextScan;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::message::TuiMessage;
use crate::theme::{
    ACCENT, ACCENT_DIM, ERR, FG_MUTED, FG_ON_SELECT, FG_PRIMARY, OK_GREEN, SELECT_BG, WARN,
};
use crate::welcome::render_welcome;

/// Paints the scrollable transcript area: branded welcome when `show_welcome`, otherwise `visible` lines.
#[allow(clippy::too_many_arguments)]
pub fn paint_message_viewport(
    f: &mut Frame<'_>,
    msg_area: Rect,
    show_welcome: bool,
    version: &str,
    project_name: &str,
    welcome_spark_phase: bool,
    first_session_ever: bool,
    has_sent_first_message: bool,
    has_akmon_md: bool,
    context_scan: &ContextScan,
    visible: Vec<Line<'static>>,
) {
    if show_welcome {
        f.render_widget(Clear, msg_area);
        render_welcome(
            msg_area,
            f.buffer_mut(),
            version,
            project_name,
            welcome_spark_phase,
            first_session_ever,
            has_sent_first_message,
            has_akmon_md,
            context_scan,
        );
    } else {
        f.render_widget(
            Paragraph::new(visible).block(Block::default().borders(Borders::NONE)),
            msg_area,
        );
    }
}

/// Maps a mouse cell column on visual line `rel_row` (0 = first `"> …"` row) to a UTF-8 byte offset in `buffer`.
///
/// Assumes the same layout as the input widget: first line prefix `"> "` (2 cols), continuation lines `"  "` (2 cols),
/// no hard wrapping (only `\n` breaks lines, at most 6 content lines).
pub(crate) fn map_input_click_to_byte_index(buffer: &str, rel_row: usize, rel_col: usize) -> usize {
    if buffer.is_empty() {
        return 0;
    }
    let lines: Vec<&str> = buffer.split('\n').take(6).collect();
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

/// Clamps `byte` to `[0, buffer.len()]` and snaps backward to a valid UTF-8 boundary.
pub(crate) fn snap_utf8_cursor(buffer: &str, byte: usize) -> usize {
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

/// Returns how many terminal rows [`message_to_lines`] will produce at `width`.
pub fn message_line_count(msg: &TuiMessage, width: u16) -> usize {
    message_to_lines(msg, width, true).len()
}

/// Flattens one logical message into styled lines for the scroll buffer.
pub fn message_to_lines(
    msg: &TuiMessage,
    width: u16,
    stream_cursor_visible: bool,
) -> Vec<Line<'static>> {
    let w = width.max(12) as usize;
    match msg {
        TuiMessage::User { content } => {
            let mut out = vec![Line::from("")];
            for line in wrap_plain(content, w) {
                out.push(Line::from(vec![
                    Span::styled(
                        "You: ",
                        Style::default().fg(ACCENT_DIM).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(line, Style::default().fg(FG_PRIMARY)),
                ]));
            }
            out
        }
        TuiMessage::Assistant { content, complete } => {
            let mut out = Vec::new();
            out.push(Line::from(vec![Span::styled(
                "Akmon: ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )]));
            out.extend(assistant_body_lines(content, w));
            if !complete {
                let cursor = if stream_cursor_visible { "▍" } else { " " };
                out.push(Line::from(vec![Span::styled(
                    cursor.to_string(),
                    Style::default().fg(ACCENT_DIM),
                )]));
            }
            out
        }
        TuiMessage::ToolCall {
            name,
            success,
            result,
            expanded,
            args,
            ..
        } => {
            let mut out = Vec::new();
            let (sym, style) = match success {
                None => ("→", Style::default().fg(WARN)),
                Some(true) => ("✓", Style::default().fg(OK_GREEN)),
                Some(false) => ("✗", Style::default().fg(ERR)),
            };
            out.push(Line::from(vec![
                Span::styled(format!("{sym} {name}"), style),
                Span::raw("  "),
                Span::styled(
                    if *expanded {
                        "[Tab to collapse]"
                    } else {
                        "[Tab to expand]"
                    },
                    Style::default().fg(FG_MUTED),
                ),
            ]));
            if *expanded {
                let args_s =
                    serde_json::to_string_pretty(args).unwrap_or_else(|_| args.to_string());
                out.push(Line::from(vec![Span::styled(
                    "  args:",
                    Style::default().add_modifier(Modifier::BOLD),
                )]));
                for l in wrap_plain(&args_s, w.saturating_sub(2)) {
                    out.push(Line::from(Span::raw(format!("  {l}"))));
                }
                if let Some(r) = result {
                    let short: String = r.chars().take(500).collect();
                    out.push(Line::from(vec![Span::styled(
                        "  result:",
                        Style::default().add_modifier(Modifier::BOLD),
                    )]));
                    for l in wrap_plain(&short, w.saturating_sub(2)) {
                        out.push(Line::from(Span::raw(format!("  {l}"))));
                    }
                }
            } else if let (Some(false), Some(r)) = (success, result) {
                let short: String = r.chars().take(80).collect();
                out.push(Line::from(vec![Span::styled(
                    short,
                    Style::default().fg(ERR),
                )]));
            }
            out
        }
        TuiMessage::Confirmation {
            description,
            diff_preview,
            answered,
            answer,
        } => {
            let mut out = vec![Line::from(vec![Span::styled(
                format!("⚠ {description}"),
                Style::default().fg(WARN),
            )])];
            if let Some(diff) = diff_preview {
                let mono = Style::default().fg(FG_PRIMARY);
                let mut line_iter = diff.lines();
                let mut shown = 0usize;
                while shown < 80 {
                    let Some(line) = line_iter.next() else {
                        break;
                    };
                    let st = if line.starts_with('+') && !line.starts_with("+++") {
                        Style::default().fg(OK_GREEN)
                    } else if line.starts_with('-') && !line.starts_with("---") {
                        Style::default().fg(ERR)
                    } else if line.starts_with("@@") {
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM)
                    } else {
                        mono
                    };
                    let take = line.chars().take(w).collect::<String>();
                    out.push(Line::from(vec![Span::styled(take, st)]));
                    shown += 1;
                }
                if line_iter.next().is_some() {
                    out.push(Line::from(vec![Span::styled(
                        "… (diff truncated)".to_string(),
                        Style::default().fg(FG_MUTED).add_modifier(Modifier::ITALIC),
                    )]));
                }
            }
            if *answered {
                let a = answer
                    .map(|b| if b { "allowed" } else { "denied" })
                    .unwrap_or("?");
                out.push(Line::from(vec![Span::styled(
                    format!("Answered: {a}"),
                    Style::default().fg(FG_MUTED),
                )]));
            } else {
                out.push(Line::from(vec![Span::styled(
                    "[y] Allow  [n] Deny",
                    Style::default().fg(FG_PRIMARY),
                )]));
            }
            out
        }
        TuiMessage::SystemInfo { content } => wrap_plain(content, w)
            .into_iter()
            .map(|l| {
                Line::from(vec![Span::styled(
                    l,
                    Style::default().fg(FG_MUTED).add_modifier(Modifier::ITALIC),
                )])
            })
            .collect(),
        TuiMessage::Error { content } => {
            let mut out = vec![Line::from(vec![
                Span::styled(
                    "Error: ",
                    Style::default().fg(ERR).add_modifier(Modifier::BOLD),
                ),
                Span::styled(content.clone(), Style::default().fg(ERR)),
            ])];
            if content.len() + 8 > w {
                out = wrap_plain(content, w)
                    .into_iter()
                    .enumerate()
                    .map(|(i, l)| {
                        if i == 0 {
                            Line::from(vec![
                                Span::styled(
                                    "Error: ",
                                    Style::default().fg(ERR).add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(l, Style::default().fg(ERR)),
                            ])
                        } else {
                            Line::from(Span::styled(l, Style::default().fg(ERR)))
                        }
                    })
                    .collect();
            }
            out
        }
    }
}

fn wrap_plain(s: &str, width: usize) -> Vec<String> {
    if s.is_empty() {
        return vec![String::new()];
    }
    let mut out = Vec::new();
    for raw_line in s.lines() {
        if raw_line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut cur = String::new();
        for ch in raw_line.chars() {
            if cur.chars().count() >= width {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(ch);
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn assistant_body_lines(content: &str, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for (i, block) in content.split("```").enumerate() {
        if i % 2 == 0 {
            if !block.trim().is_empty() {
                out.extend(parse_markdown_text_block(block, width));
            }
        } else {
            let mut lines = block.lines();
            let lang = lines.next().unwrap_or("").trim();
            let body: String = lines.collect::<Vec<_>>().join("\n");
            out.push(Line::from(vec![Span::styled(
                format!("```{lang}"),
                Style::default().fg(ACCENT_DIM),
            )]));
            for l in wrap_plain(&body, width.saturating_sub(2)) {
                out.push(Line::from(vec![Span::styled(
                    format!(" {l}"),
                    Style::default().bg(SELECT_BG).fg(FG_ON_SELECT),
                )]));
            }
            out.push(Line::from(vec![Span::styled(
                "```",
                Style::default().fg(ACCENT_DIM),
            )]));
        }
    }
    out
}

fn parse_markdown_text_block(block: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for raw in block.lines() {
        lines.extend(parse_bold_line(raw, width));
    }
    if lines.is_empty() && !block.is_empty() {
        lines.extend(parse_bold_line(block, width));
    }
    lines
}

fn parse_bold_line(line: &str, width: usize) -> Vec<Line<'static>> {
    if !line.contains("**") {
        return wrap_plain(line, width)
            .into_iter()
            .map(|l| Line::from(Span::raw(l)))
            .collect();
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find("**") {
        if start > 0 {
            spans.push(Span::raw(rest[..start].to_string()));
        }
        rest = &rest[start + 2..];
        let Some(end) = rest.find("**") else {
            spans.push(Span::raw(format!("**{rest}")));
            return vec![Line::from(spans)];
        };
        spans.push(Span::styled(
            rest[..end].to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        rest = &rest[end + 2..];
    }
    if !rest.is_empty() {
        spans.push(Span::raw(rest.to_string()));
    }
    if spans.is_empty() {
        return vec![];
    }
    vec![Line::from(spans)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn user_message_renders_you_prefix() {
        let m = TuiMessage::User {
            content: "hello".into(),
        };
        let lines = message_to_lines(&m, 80, true);
        let joined = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("|");
        assert!(joined.contains("You:"));
        assert!(joined.contains("hello"));
    }

    #[test]
    fn tool_call_collapsed_arrow() {
        let m = TuiMessage::ToolCall {
            id: "1".into(),
            name: "read_file".into(),
            args: json!({"path": "a.rs"}),
            result: None,
            success: None,
            expanded: false,
        };
        let lines = message_to_lines(&m, 80, true);
        let s: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|sp| sp.content.to_string()))
            .collect();
        assert!(s.contains("read_file"));
        assert!(s.contains('→'));
    }

    #[test]
    fn tool_call_expanded_shows_args() {
        let m = TuiMessage::ToolCall {
            id: "1".into(),
            name: "read_file".into(),
            args: json!({"path": "a.rs"}),
            result: Some("ok".into()),
            success: Some(true),
            expanded: true,
        };
        let lines = message_to_lines(&m, 80, true);
        let flat: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(flat.contains("args"));
        assert!(flat.contains("result"));
    }

    #[test]
    fn input_click_col5_row0_is_byte_three() {
        assert_eq!(super::map_input_click_to_byte_index("abcdef", 0, 5), 3);
    }

    #[test]
    fn input_click_before_prefix_is_zero() {
        assert_eq!(super::map_input_click_to_byte_index("abcdef", 0, 0), 0);
        assert_eq!(super::map_input_click_to_byte_index("abcdef", 0, 1), 0);
    }

    #[test]
    fn input_click_past_content_clamps() {
        assert_eq!(
            super::map_input_click_to_byte_index("ab", 0, 100),
            2,
            "should clamp to end of line"
        );
        assert_eq!(
            super::map_input_click_to_byte_index("a\nbc", 2, 10),
            4,
            "row past last line -> eof"
        );
    }
}
