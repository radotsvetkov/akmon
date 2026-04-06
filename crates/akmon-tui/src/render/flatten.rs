//! Flatten [`TuiMessage`] rows into wrapped [`ratatui::text::Line`]s for the viewport.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value as JsonValue;

use super::code::{code_block_bg, highlight_line};
use super::wrap::wrap_text;
use crate::message::TuiMessage;

const AMBER: ratatui::style::Color = ratatui::style::Color::Rgb(245, 158, 11);
const BORDER: ratatui::style::Color = ratatui::style::Color::Rgb(42, 42, 46);
const FG: ratatui::style::Color = ratatui::style::Color::Rgb(226, 224, 216);
const DIM: ratatui::style::Color = ratatui::style::Color::Rgb(100, 108, 118);
const GREEN: ratatui::style::Color = ratatui::style::Color::Rgb(78, 201, 176);
const RED: ratatui::style::Color = ratatui::style::Color::Rgb(239, 68, 68);
const DIFF_HEADER: ratatui::style::Color = ratatui::style::Color::Rgb(100, 100, 120);
const ERR: ratatui::style::Color = ratatui::style::Color::Rgb(248, 113, 113);

/// Total lines one message occupies at `width`.
#[must_use]
pub fn message_line_count(msg: &TuiMessage, width: u16) -> usize {
    flatten_message(msg, width, true).len()
}

/// Back-compat: synonym for [`flatten_message`] with streaming cursor support.
#[must_use]
pub fn message_to_lines(
    msg: &TuiMessage,
    width: u16,
    stream_cursor_visible: bool,
) -> Vec<Line<'static>> {
    flatten_message(msg, width, stream_cursor_visible)
}

#[must_use]
pub fn flatten_transcript(
    messages: &[TuiMessage],
    width: u16,
    stream_cursor_visible: bool,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for m in messages {
        out.extend(flatten_message(m, width, stream_cursor_visible));
    }
    out
}

#[must_use]
fn flatten_message(
    msg: &TuiMessage,
    width: u16,
    stream_cursor_visible: bool,
) -> Vec<Line<'static>> {
    let w = width.saturating_sub(4).max(8);
    match msg {
        TuiMessage::User { content } => user_block(content, w),
        TuiMessage::Assistant { content, complete } => {
            assistant_with_code(content, w, *complete, stream_cursor_visible)
        }
        TuiMessage::SystemInfo { content } => system_line(content, w),
        TuiMessage::Error { content } => vec![Line::from(Span::styled(
            format!("⚠ {content}"),
            Style::default().fg(ERR),
        ))],
        TuiMessage::ToolCall {
            name,
            args,
            result,
            success,
            expanded,
            ..
        } => tool_card(name, args, result.as_deref(), *success, *expanded, w),
        TuiMessage::Confirmation {
            description,
            answered,
            answer,
            ..
        } => {
            // Pending prompts use the centered modal only; avoid duplicate lines above the input.
            if !*answered {
                Vec::new()
            } else {
                confirmation_short(description, *answered, *answer, w)
            }
        }
    }
}

fn user_block(content: &str, w: u16) -> Vec<Line<'static>> {
    let mut v = vec![Line::from(vec![
        Span::styled("┌─ ", Style::default().fg(BORDER)),
        Span::styled(
            "You ",
            Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
        ),
        Span::styled(
            "─".repeat(w.saturating_sub(8) as usize),
            Style::default().fg(BORDER),
        ),
    ])];
    for line in wrap_text(content, w) {
        v.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(BORDER)),
            Span::styled(line, Style::default().fg(FG)),
        ]));
    }
    v.push(Line::from(Span::styled(
        format!("└{}", "─".repeat(w.saturating_sub(1) as usize)),
        Style::default().fg(BORDER),
    )));
    v
}

fn assistant_with_code(
    content: &str,
    w: u16,
    complete: bool,
    stream_blink: bool,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    for seg in split_fences(content) {
        match seg {
            Segment::Plain(text) => {
                if text.trim().is_empty() {
                    continue;
                }
                for line in wrap_text(&text, w) {
                    let label = if out.is_empty() {
                        vec![
                            Span::styled(
                                "Akmon ",
                                Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(line, Style::default().fg(FG)),
                        ]
                    } else {
                        vec![
                            Span::styled("     ", Style::default()),
                            Span::styled(line, Style::default().fg(FG)),
                        ]
                    };
                    out.push(Line::from(label));
                }
            }
            Segment::Code { lang, body } => {
                out.push(Line::from(vec![
                    Span::styled("┌─ ", Style::default().fg(BORDER)),
                    Span::styled(
                        format!("{lang} "),
                        Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        "─".repeat((w as usize).saturating_sub(4 + lang.len().min(w as usize))),
                        Style::default().fg(BORDER),
                    ),
                ]));
                for raw in body.lines() {
                    let hl = highlight_line(raw, &lang);
                    let mut spans = vec![
                        Span::styled("│ ", Style::default().fg(BORDER)),
                        Span::styled("", Style::default().bg(code_block_bg())),
                    ];
                    for mut s in hl {
                        s.style = s.style.bg(code_block_bg());
                        spans.push(s);
                    }
                    out.push(Line::from(spans));
                }
                out.push(Line::from(Span::styled(
                    format!("└{}", "─".repeat(w.saturating_sub(1) as usize)),
                    Style::default().fg(BORDER),
                )));
            }
        }
    }
    if out.is_empty() {
        out.push(Line::from(Span::styled(
            "Akmon ",
            Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
        )));
    }
    if !complete {
        let cur = if stream_blink { "▍" } else { " " };
        out.push(Line::from(vec![
            Span::styled("     ", Style::default()),
            Span::styled(cur, Style::default().fg(AMBER)),
        ]));
    }
    out
}

enum Segment {
    Plain(String),
    Code { lang: String, body: String },
}

fn split_fences(content: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut rest = content;
    loop {
        if let Some(i) = rest.find("```") {
            if i > 0 {
                out.push(Segment::Plain(rest[..i].to_string()));
            }
            let after = &rest[i + 3..];
            let (lang, body_start) = match after.find('\n') {
                Some(nl) => (after[..nl].trim().to_string(), nl + 1),
                None => (String::new(), 0),
            };
            let inner = after.get(body_start..).unwrap_or("");
            if let Some(j) = inner.find("```") {
                out.push(Segment::Code {
                    lang,
                    body: inner[..j].to_string(),
                });
                rest = inner.get(j + 3..).unwrap_or_default();
            } else {
                out.push(Segment::Code {
                    lang,
                    body: inner.to_string(),
                });
                break;
            }
        } else {
            if !rest.is_empty() {
                out.push(Segment::Plain(rest.to_string()));
            }
            break;
        }
    }
    if out.is_empty() {
        out.push(Segment::Plain(content.to_string()));
    }
    out
}

fn system_line(content: &str, w: u16) -> Vec<Line<'static>> {
    let text = format!("─ {content} ");
    let line = if text.chars().count() > w as usize {
        text.chars().take(w as usize).collect::<String>()
    } else {
        let pad = w as usize - text.chars().count();
        format!("{text}{}", "─".repeat(pad))
    };
    vec![Line::from(Span::styled(
        line,
        Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
    ))]
}

fn tool_card(
    name: &str,
    args: &serde_json::Value,
    result: Option<&str>,
    success: Option<bool>,
    expanded: bool,
    w: u16,
) -> Vec<Line<'static>> {
    let summary = args_summary(name, args);
    let status = match (success, result.as_ref()) {
        (Some(true), _) => ("✓", GREEN),
        (Some(false), _) => ("✗", RED),
        (None, _) if result.is_none() => ("⠿", AMBER),
        _ => ("○", DIM),
    };
    let mut v = vec![Line::from(vec![
        Span::styled("→ ", Style::default().fg(AMBER)),
        Span::styled(format!("{name}  "), Style::default().fg(FG)),
        Span::styled(
            status.0,
            Style::default().fg(status.1).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {summary}  "), Style::default().fg(DIM)),
        Span::styled("[Tab]", Style::default().fg(DIM)),
    ])];
    if expanded {
        v.push(Line::from(vec![
            Span::styled("┌─ ", Style::default().fg(BORDER)),
            Span::styled(name.to_string(), Style::default().fg(AMBER)),
            Span::styled(
                format!(
                    " {}─",
                    "─".repeat((w as usize).saturating_sub(name.len() + 6))
                ),
                Style::default().fg(BORDER),
            ),
        ]));
        let st = match success {
            Some(true) => ("✓ success", GREEN),
            Some(false) => ("✗ failed", RED),
            None => ("… running", AMBER),
        };
        v.push(Line::from(vec![
            Span::styled("│ Status:   ", Style::default().fg(DIM)),
            Span::styled(st.0, Style::default().fg(st.1)),
        ]));
        v.push(Line::from(vec![
            Span::styled("│ Input:    ", Style::default().fg(DIM)),
            Span::styled(
                serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string()),
                Style::default().fg(FG),
            ),
        ]));
        if let Some(r) = result {
            if let Ok(val) = serde_json::from_str::<JsonValue>(r) {
                if val.get("type").and_then(|x| x.as_str()) == Some("file_edit_diff") {
                    if let Some(diff) = val.get("diff").and_then(|x| x.as_str()) {
                        append_colored_unified_diff_lines(&mut v, diff, w);
                    } else {
                        append_wrapped_output_lines(&mut v, r, w);
                    }
                } else {
                    append_wrapped_output_lines(&mut v, r, w);
                }
            } else {
                append_wrapped_output_lines(&mut v, r, w);
            }
        }
        v.push(Line::from(Span::styled(
            format!("└{}", "─".repeat(w.saturating_sub(1) as usize)),
            Style::default().fg(BORDER),
        )));
    }
    v
}

fn append_wrapped_output_lines(v: &mut Vec<Line<'static>>, text: &str, w: u16) {
    for line in wrap_text(text, w.saturating_sub(4)) {
        v.push(Line::from(vec![
            Span::styled("│ Output:   ", Style::default().fg(DIM)),
            Span::styled(line, Style::default().fg(FG)),
        ]));
    }
}

fn append_colored_unified_diff_lines(v: &mut Vec<Line<'static>>, diff: &str, w: u16) {
    let col_w = w.saturating_sub(4);
    for raw in diff.lines() {
        let style = if raw.starts_with('+') && !raw.starts_with("+++") {
            Style::default().fg(GREEN)
        } else if raw.starts_with('-') && !raw.starts_with("---") {
            Style::default().fg(RED)
        } else if raw.starts_with('@') {
            Style::default().fg(DIFF_HEADER)
        } else {
            Style::default().fg(FG)
        };
        for line in wrap_text(raw, col_w) {
            v.push(Line::from(vec![
                Span::styled("│ Output:   ", Style::default().fg(DIM)),
                Span::styled(line, style),
            ]));
        }
    }
}

fn args_summary(name: &str, args: &serde_json::Value) -> String {
    match name {
        "read_file" | "write_file" | "edit" => args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "apply_patch" => args
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "shell" => args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .chars()
            .take(40)
            .collect(),
        _ => String::new(),
    }
}

fn confirmation_short(
    description: &str,
    answered: bool,
    answer: Option<bool>,
    w: u16,
) -> Vec<Line<'static>> {
    let line: String = description
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(description)
        .chars()
        .take(w as usize)
        .collect();
    if answered {
        let (sym, ok) = match answer {
            Some(true) => ("✓", GREEN),
            Some(false) => ("✗", RED),
            None => ("?", DIM),
        };
        vec![Line::from(vec![
            Span::styled(format!("{sym} "), Style::default().fg(ok)),
            Span::styled(line, Style::default().fg(DIM)),
        ])]
    } else {
        vec![Line::from(vec![
            Span::styled("⚠ ", Style::default().fg(AMBER)),
            Span::styled(line, Style::default().fg(FG)),
        ])]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fence_emits_code_block() {
        let m = TuiMessage::Assistant {
            content: "Hi\n```rust\nfn a(){}\n```\nDone".into(),
            complete: true,
        };
        let lines = flatten_message(&m, 60, true);
        let s: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|sp| sp.content.to_string()))
            .collect();
        assert!(s.contains("fn"));
    }

    #[test]
    fn wrap_text_line_count() {
        let m = TuiMessage::User {
            content: "a b c d e f g h i j".into(),
        };
        assert!(message_line_count(&m, 12) >= 2);
    }

    #[test]
    fn rust_keyword_in_flatten() {
        let m = TuiMessage::Assistant {
            content: "```rust\nlet x = 1;\n```".into(),
            complete: true,
        };
        let lines = flatten_message(&m, 50, true);
        let flat: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(flat.contains('l'));
    }
}
