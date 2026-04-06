//! Bordered overlay widgets for slash-command UX (help, lists, cost, autocomplete).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::{Overlay, TuiApp};
use crate::slash::{COMMANDS, slash_command_name_prefix};
use crate::slash_exec::{cost_summary_lines, format_session_list_row};
use crate::theme::{ACCENT, ACCENT_DIM, BORDER, FG_MUTED, FG_PRIMARY, OK_GREEN, SELECT_BG};

/// Max command **content** rows in the slash dropdown (one terminal row each; no wrapping).
const SLASH_AC_MAX_VISIBLE: usize = 10;

/// Outer height = content rows + top/bottom [`Borders::ALL`] (ratatui draws borders inside the rect).
#[inline]
fn slash_ac_outer_height(content_rows: u16) -> u16 {
    content_rows.saturating_add(2)
}

/// Vertical rows reserved for [`Overlay::SlashAutocomplete`] (includes border lines).
pub fn slash_autocomplete_row_count(app: &TuiApp) -> u16 {
    match &app.overlay {
        Overlay::SlashAutocomplete { matches, .. } => {
            let content = matches.len().clamp(1, SLASH_AC_MAX_VISIBLE) as u16;
            slash_ac_outer_height(content)
        }
        _ => 0,
    }
}

/// Truncate to at most `max_chars` Unicode scalars (ellipsis if trimmed).
fn clamp_chars(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let n = s.chars().count();
    if n <= max_chars {
        return s.to_string();
    }
    let take = max_chars.saturating_sub(1);
    s.chars().take(take).collect::<String>() + "…"
}

/// Paints the slash command dropdown directly above the input block.
pub fn draw_slash_autocomplete(f: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    let Overlay::SlashAutocomplete { matches, selected } = &app.overlay else {
        return;
    };
    let filter = slash_command_name_prefix(&app.input_buffer).unwrap_or("");
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner_h = (area.height as usize).saturating_sub(2).max(1);
    let target_h = if matches.is_empty() {
        1
    } else {
        matches.len().min(SLASH_AC_MAX_VISIBLE)
    };
    // If the terminal squeezed the rect, still scroll using the rows we can paint.
    let view_h = target_h.min(inner_h);
    let max_start = matches.len().saturating_sub(view_h);
    let view_start = if matches.is_empty() || matches.len() <= view_h {
        0
    } else {
        (*selected).saturating_sub(view_h - 1).min(max_start)
    };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if matches.is_empty() {
        let msg = clamp_chars("(no matching commands)", inner_w.saturating_sub(2).max(1));
        lines.push(Line::from(Span::styled(
            format!("  {msg}"),
            Style::default().fg(FG_MUTED),
        )));
    } else {
        for i in 0..view_h {
            let idx = view_start + i;
            let Some(cmd) = matches.get(idx) else {
                break;
            };
            let (name_st, desc_st) = if idx == *selected {
                (
                    Style::default()
                        .bg(SELECT_BG)
                        .fg(ACCENT)
                        .add_modifier(Modifier::BOLD),
                    Style::default().bg(SELECT_BG).fg(FG_PRIMARY),
                )
            } else {
                (
                    Style::default().fg(ACCENT_DIM).add_modifier(Modifier::BOLD),
                    Style::default().fg(FG_MUTED),
                )
            };
            let prefix = format!("  /{}  ", cmd.name);
            let prefix_len = prefix.chars().count();
            let desc_max = inner_w.saturating_sub(prefix_len).max(1);
            let desc = clamp_chars(cmd.description, desc_max);
            lines.push(Line::from(vec![
                Span::styled(prefix, name_st),
                Span::styled(desc, desc_st),
            ]));
        }
    }
    let mut title_spans: Vec<Span<'static>> = vec![Span::styled(
        " commands ",
        Style::default()
            .fg(FG_MUTED)
            .add_modifier(Modifier::ITALIC | Modifier::BOLD),
    )];
    if !filter.is_empty() {
        title_spans.push(Span::styled(
            format!("/{filter}  "),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    }
    if !matches.is_empty() {
        let above = view_start;
        let below = matches.len().saturating_sub(view_start + view_h);
        let scroll_hint = if above > 0 || below > 0 {
            format!(
                "·  {}/{}  ·  ↑{above} ↓{below}  ",
                *selected + 1,
                matches.len()
            )
        } else {
            format!("·  {}/{}  ", *selected + 1, matches.len())
        };
        title_spans.push(Span::styled(scroll_hint, Style::default().fg(FG_MUTED)));
    }
    if area.width >= 52 {
        title_spans.push(Span::styled(
            "↑↓ · Enter · Esc ",
            Style::default().fg(FG_MUTED),
        ));
    }
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER))
                .title(Line::from(title_spans)),
        ),
        area,
    );
}

#[cfg(test)]
mod slash_ac_tests {
    use super::slash_ac_outer_height;

    #[test]
    fn outer_height_reserves_border_rows() {
        assert_eq!(slash_ac_outer_height(8), 10);
        assert_eq!(slash_ac_outer_height(1), 3);
    }
}

/// Draws transcript-blocking overlays (help, session picker, audit, cost).
pub fn draw_message_overlays(f: &mut Frame<'_>, app: &TuiApp, msg_area: Rect) {
    match &app.overlay {
        Overlay::None | Overlay::SlashAutocomplete { .. } => {}
        Overlay::Help => {
            let h = (COMMANDS.len() as u16)
                .saturating_add(5)
                .min(msg_area.height);
            let w = (msg_area.width.saturating_sub(4)).min(86);
            let r = centered_rect(msg_area, w, h);
            f.render_widget(Clear, r);
            let mut lines: Vec<Line<'static>> = COMMANDS
                .iter()
                .map(|c| {
                    Line::from(vec![
                        Span::styled(
                            format!("/{}", c.name),
                            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("  {}", c.description),
                            Style::default().fg(FG_MUTED),
                        ),
                    ])
                })
                .collect();
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Press any key to close",
                Style::default().fg(FG_MUTED),
            )));
            f.render_widget(
                Paragraph::new(lines).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(BORDER))
                        .title(Span::styled(
                            " help ",
                            Style::default().fg(FG_MUTED).add_modifier(Modifier::ITALIC),
                        )),
                ),
                r,
            );
        }
        Overlay::SessionList {
            sessions,
            selected,
            scroll,
        } => {
            let inner_h = msg_area.height.saturating_sub(2).max(6);
            let w = msg_area.width.saturating_sub(2).max(10);
            let r = centered_rect(msg_area, w, inner_h);
            f.render_widget(Clear, r);
            let body_rows = (r.height.saturating_sub(4)) as usize;
            let max_rows = body_rows.max(1);
            let mut lines: Vec<Line<'static>> = Vec::new();
            if sessions.is_empty() {
                lines.push(Line::from("No previous sessions found."));
            } else {
                let view_start = if sessions.len() <= max_rows {
                    0
                } else {
                    (*scroll).min(sessions.len().saturating_sub(max_rows))
                };
                for row_idx in 0..max_rows {
                    let i = view_start + row_idx;
                    let Some(row) = sessions.get(i) else {
                        break;
                    };
                    let style = if i == *selected {
                        Style::default()
                            .bg(SELECT_BG)
                            .fg(FG_PRIMARY)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(FG_MUTED)
                    };
                    lines.push(Line::from(Span::styled(
                        format_session_list_row(row),
                        style,
                    )));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Enter resume · Esc cancel",
                Style::default().fg(FG_MUTED),
            )));
            f.render_widget(
                Paragraph::new(lines).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(BORDER))
                        .title(Span::styled(
                            " sessions ",
                            Style::default().fg(FG_MUTED).add_modifier(Modifier::ITALIC),
                        )),
                ),
                r,
            );
        }
        Overlay::AuditLog { lines, scroll } => {
            let inner_h = msg_area.height.saturating_sub(4).max(4);
            let w = msg_area.width.saturating_sub(2).max(10);
            let r = centered_rect(msg_area, w, inner_h);
            f.render_widget(Clear, r);
            let body_h = inner_h.saturating_sub(4).max(1) as usize;
            let mut out: Vec<Line<'static>> = Vec::new();
            if lines.is_empty() {
                out.push(Line::from("No audit events recorded yet."));
            } else {
                let start = (*scroll).min(lines.len().saturating_sub(1));
                for line in lines.iter().skip(start).take(body_h) {
                    out.push(Line::from(line.clone()));
                }
            }
            out.push(Line::from(""));
            out.push(Line::from(Span::styled(
                "Esc to close",
                Style::default().fg(FG_MUTED),
            )));
            f.render_widget(
                Paragraph::new(out)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(BORDER))
                            .title(Span::styled(
                                " audit ",
                                Style::default().fg(FG_MUTED).add_modifier(Modifier::ITALIC),
                            )),
                    )
                    .wrap(Wrap { trim: true }),
                r,
            );
        }
        Overlay::ModelPicker {
            rows,
            selectable,
            selected,
            scroll,
        } => {
            let inner_h = msg_area.height.saturating_sub(2).max(8);
            let w = msg_area.width.saturating_sub(2).max(10);
            let r = centered_rect(msg_area, w, inner_h);
            f.render_widget(Clear, r);
            let body_rows = (r.height.saturating_sub(4)) as usize;
            let max_rows = body_rows.max(1);
            let mut lines: Vec<Line<'static>> = Vec::new();
            if rows.is_empty() {
                lines.push(Line::from("No models."));
            } else {
                let view_start = if rows.len() <= max_rows {
                    0
                } else {
                    (*scroll).min(rows.len().saturating_sub(max_rows))
                };
                let sel_row = selectable.get(*selected).copied();
                for row_idx in view_start..(view_start + max_rows).min(rows.len()) {
                    let Some(row) = rows.get(row_idx) else {
                        break;
                    };
                    let style = if Some(row_idx) == sel_row {
                        Style::default()
                            .bg(SELECT_BG)
                            .fg(FG_PRIMARY)
                            .add_modifier(Modifier::BOLD)
                    } else if row.section_header {
                        Style::default().fg(ACCENT_DIM).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(FG_MUTED)
                    };
                    let txt = if row.section_header {
                        format!("── {} ──", row.label)
                    } else {
                        format!("  {}", row.label)
                    };
                    lines.push(Line::from(Span::styled(txt, style)));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Enter select · Esc cancel · ↑↓",
                Style::default().fg(FG_MUTED),
            )));
            f.render_widget(
                Paragraph::new(lines).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(BORDER))
                        .title(Span::styled(
                            " model ",
                            Style::default().fg(FG_MUTED).add_modifier(Modifier::ITALIC),
                        )),
                ),
                r,
            );
        }
        Overlay::CostSummary => {
            let h = 10u16.min(msg_area.height.saturating_sub(2));
            let w = (msg_area.width.saturating_sub(4)).min(48);
            let r = centered_rect(msg_area, w, h);
            f.render_widget(Clear, r);
            let rows = cost_summary_lines(app);
            let mut lines: Vec<Line<'static>> = Vec::new();
            for (i, s) in rows.into_iter().enumerate() {
                if i == 1 {
                    lines.push(Line::from(vec![Span::styled(
                        s,
                        Style::default().fg(OK_GREEN),
                    )]));
                } else {
                    lines.push(Line::from(Span::styled(s, Style::default().fg(FG_PRIMARY))));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Press any key to close",
                Style::default().fg(FG_MUTED),
            )));
            f.render_widget(
                Paragraph::new(lines).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(BORDER))
                        .title(Span::styled(
                            " cost ",
                            Style::default().fg(FG_MUTED).add_modifier(Modifier::ITALIC),
                        )),
                ),
                r,
            );
        }
    }
}

fn centered_rect(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let x = area.x.saturating_add(area.width.saturating_sub(w) / 2);
    let y = area.y.saturating_add(area.height.saturating_sub(h) / 2);
    Rect::new(x, y, w, h)
}
