//! Centered permission dialog (Tab / Enter / Esc).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::layout::centered_rect;
use crate::state::{ConfirmChoice, ConfirmationDialog, OperationType};

const GREEN: ratatui::style::Color = ratatui::style::Color::Rgb(78, 201, 176);
const RED: ratatui::style::Color = ratatui::style::Color::Rgb(244, 71, 71);
const GREY: ratatui::style::Color = ratatui::style::Color::Rgb(204, 204, 204);
const AMBER: ratatui::style::Color = ratatui::style::Color::Rgb(245, 158, 11);

/// First whitespace-delimited token for shell prefix rules (`python a.py` → `python`, `npm run dev` → `npm`).
#[must_use]
pub fn shell_prefix_hint(cmd: &str) -> String {
    cmd.split_whitespace().next().unwrap_or("").to_string()
}

/// Centered modal for `ask_followup`: question text + optional suggestions + live draft preview.
pub fn render_question_overlay(
    f: &mut ratatui::Frame<'_>,
    viewport: Rect,
    question: &str,
    suggestions: &[String],
    draft: &str,
) {
    let modal_w = viewport.width.saturating_sub(4).max(44);
    let modal_h = viewport.height.saturating_sub(4).max(10);
    let r = centered_rect(viewport, modal_w, modal_h);
    f.render_widget(Clear, r);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(AMBER))
        .title(Span::styled(
            " Question ",
            Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let mut lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            "The assistant asks:",
            Style::default().fg(GREY),
        )),
        Line::from(""),
        Line::from(Span::styled(
            question.to_string(),
            Style::default().fg(GREY),
        )),
    ];
    if !suggestions.is_empty() {
        lines.push(Line::from(""));
        for (i, s) in suggestions.iter().take(6).enumerate() {
            lines.push(Line::from(Span::styled(
                format!("  {}. {s}", i + 1),
                Style::default().fg(GREY),
            )));
        }
    }
    lines.push(Line::from(""));
    let draft_line = if draft.is_empty() {
        "(type your answer in the bar below — Enter to send, Esc to send empty)"
    } else {
        draft
    };
    lines.push(Line::from(Span::styled(
        format!("Your reply: {draft_line}"),
        Style::default().fg(AMBER),
    )));
    lines.push(Line::from(Span::styled(
        "  Enter — submit · Esc — empty reply · typing goes to the compose bar",
        Style::default().fg(ratatui::style::Color::DarkGray),
    )));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Draws the centered permission window inside `viewport` bounds (choices + Enter to confirm).
pub fn render_confirmation_overlay(
    f: &mut ratatui::Frame<'_>,
    viewport: Rect,
    dlg: &ConfirmationDialog,
) {
    let modal_w = viewport.width.saturating_sub(4).max(44);
    let modal_h = viewport.height.saturating_sub(4).max(12);
    let r = centered_rect(viewport, modal_w, modal_h);
    f.render_widget(Clear, r);
    let title = dlg.title.as_str();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(AMBER))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let mut lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            "The assistant needs your approval for:",
            Style::default().fg(GREY),
        )),
        Line::from(""),
    ];
    lines.extend(operation_lines(&dlg.operation));
    if let Some(diff) = &dlg.diff_or_preview {
        lines.push(Line::from(""));
        let skip = dlg.scroll_offset as usize;
        for (_i, dl) in diff.lines().enumerate().skip(skip).take(8) {
            let st = if dl.starts_with('+') && !dl.starts_with("+++") {
                Style::default().fg(GREEN)
            } else if dl.starts_with('-') && !dl.starts_with("---") {
                Style::default().fg(RED)
            } else if dl.starts_with("@@") {
                Style::default().fg(ratatui::style::Color::DarkGray)
            } else {
                Style::default().fg(GREY)
            };
            lines.push(Line::from(Span::styled(dl.to_string(), st)));
        }
    }
    lines.push(Line::from(""));
    match &dlg.operation {
        OperationType::WriteFile { .. } | OperationType::EditFile { .. } => {
            lines.push(choice_line(
                " [y] Allow once ",
                ConfirmChoice::Allow,
                dlg.selected_option,
            ));
            lines.push(choice_line(
                " [s] Allow this session (this path only) ",
                ConfirmChoice::AllowAlways,
                dlg.selected_option,
            ));
            if dlg.broad_choice_enabled {
                lines.push(choice_line(
                    " [p] Allow all writes this session ",
                    ConfirmChoice::AllowBroad,
                    dlg.selected_option,
                ));
            }
            lines.push(choice_line(
                " [n] Deny ",
                ConfirmChoice::Deny,
                dlg.selected_option,
            ));
        }
        OperationType::RunShell { command } => {
            let pfx = shell_prefix_hint(command);
            lines.push(choice_line(
                " [y] Allow once ",
                ConfirmChoice::Allow,
                dlg.selected_option,
            ));
            lines.push(choice_line(
                " [s] Allow this session (this command only) ",
                ConfirmChoice::AllowAlways,
                dlg.selected_option,
            ));
            if dlg.broad_choice_enabled {
                let lbl = format!(" [r] Allow all: `{pfx}`* this session ");
                lines.push(choice_line(
                    lbl.as_str(),
                    ConfirmChoice::AllowBroad,
                    dlg.selected_option,
                ));
            }
            lines.push(choice_line(
                " [n] Deny ",
                ConfirmChoice::Deny,
                dlg.selected_option,
            ));
        }
        _ => {
            lines.push(choice_line(
                " [y] Allow once ",
                ConfirmChoice::Allow,
                dlg.selected_option,
            ));
            lines.push(choice_line(
                " [s] Allow this session ",
                ConfirmChoice::AllowAlways,
                dlg.selected_option,
            ));
            if dlg.broad_choice_enabled {
                lines.push(choice_line(
                    dlg.broad_choice_label.as_str(),
                    ConfirmChoice::AllowBroad,
                    dlg.selected_option,
                ));
            }
            lines.push(choice_line(
                " [n] Deny ",
                ConfirmChoice::Deny,
                dlg.selected_option,
            ));
        }
    }
    lines.push(Line::from(Span::styled(
        "  Tab or Shift+Tab · ← → — pick option    PgUp/PgDn — scroll diff",
        Style::default().fg(ratatui::style::Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "  Enter confirms · 1/y · 2/s session · p/r broad · Esc/n deny",
        Style::default().fg(ratatui::style::Color::DarkGray),
    )));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn operation_lines(op: &OperationType) -> Vec<Line<'static>> {
    match op {
        OperationType::WriteFile { path, .. } | OperationType::EditFile { path, .. } => vec![
            Line::from(Span::styled(
                "File change",
                Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(path.clone(), Style::default().fg(GREY))),
        ],
        OperationType::RunShell { command } => vec![
            Line::from(Span::styled(
                "Shell command",
                Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                command.clone(),
                Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
            )),
        ],
        OperationType::WebFetch { url } => vec![
            Line::from(Span::styled(
                "Network fetch",
                Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(url.clone(), Style::default().fg(GREY))),
        ],
        OperationType::GitCommit { message, .. } => {
            vec![Line::from("GitCommit"), Line::from(message.clone())]
        }
        OperationType::Generic { description } => {
            vec![Line::from(Span::raw(description.clone()))]
        }
    }
}

fn choice_line(label: &str, me: ConfirmChoice, sel: ConfirmChoice) -> Line<'static> {
    let sym = if sel == me { "●" } else { "○" };
    let st = if sel == me {
        Style::default().fg(AMBER).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(GREY)
    };
    Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(format!("{sym} "), st),
        Span::styled(label.to_string(), st),
    ])
}

fn path_from_file_change_description(description: &str) -> Option<String> {
    description.lines().find_map(|l| {
        let t = l.trim();
        t.strip_prefix("Path:")
            .map(str::trim)
            .map(|s| s.to_string())
    })
}

/// Parses [`crate::message::TuiMessage::Confirmation`] into dialog state.
pub fn dialog_from_confirmation(description: &str, diff: Option<&str>) -> ConfirmationDialog {
    let diff_owned = diff.map(ToString::to_string);
    let op = if description.contains("Shell command requires confirmation") {
        let cmd = description
            .lines()
            .find(|l| l.contains("Proposed command:"))
            .and_then(|l| l.split("Proposed command:").nth(1))
            .map(str::trim)
            .unwrap_or("command")
            .to_string();
        OperationType::RunShell { command: cmd }
    } else if description.contains("Network fetch requires confirmation") {
        let url = description
            .lines()
            .find(|l| l.contains("URL:"))
            .and_then(|l| l.split("URL:").nth(1))
            .map(str::trim)
            .unwrap_or("(unknown URL)")
            .to_string();
        OperationType::WebFetch { url }
    } else if description.contains("File change requires confirmation") {
        let path = path_from_file_change_description(description).unwrap_or_else(|| "file".into());
        OperationType::WriteFile {
            path,
            diff: diff_owned.clone().unwrap_or_default(),
        }
    } else if let Some(rest) = description.strip_prefix("Path:") {
        OperationType::WriteFile {
            path: rest.trim().to_string(),
            diff: diff_owned.clone().unwrap_or_default(),
        }
    } else {
        OperationType::Generic {
            description: description.to_string(),
        }
    };
    let (broad_choice_enabled, broad_choice_label) = match &op {
        OperationType::WriteFile { .. } | OperationType::EditFile { .. } => {
            (true, " Allow all writes this session ".to_string())
        }
        OperationType::RunShell { command } => {
            let p = shell_prefix_hint(command);
            (true, format!(" Allow shell prefix `{p}` this session "))
        }
        _ => (false, String::new()),
    };
    ConfirmationDialog {
        title: "Approve this action?".into(),
        operation: op,
        diff_or_preview: diff_owned,
        selected_option: ConfirmChoice::Allow,
        scroll_offset: 0,
        broad_choice_enabled,
        broad_choice_label,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::OperationType;

    #[test]
    fn dialog_parses_file_change_policy_description() {
        let desc = "File change requires confirmation.\n  Path: src/main.rs";
        let dlg = dialog_from_confirmation(desc, None);
        match dlg.operation {
            OperationType::WriteFile { path, .. } => assert_eq!(path, "src/main.rs"),
            o => panic!("expected WriteFile, got {o:?}"),
        }
    }

    #[test]
    fn dialog_parses_network_fetch_description() {
        let desc = "Network fetch requires confirmation.\n  URL: https://example.com";
        let dlg = dialog_from_confirmation(desc, None);
        match dlg.operation {
            OperationType::WebFetch { url } => assert_eq!(url, "https://example.com"),
            o => panic!("expected WebFetch, got {o:?}"),
        }
    }
}
