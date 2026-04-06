//! Branded empty-state welcome screen (pixel anvil + quick-start hints).

use akmon_core::ContextScan;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

/// Same silhouette as `README.md` (U+2593 / U+2592 + sparks), for a consistent brand mark.
const ANVIL_BODY: &[&str] = &[
    "           ▓▓▓",
    "           ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓",
    "         ▒▒▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▒▒",
    "         ▒▒▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▒▒",
    "           ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓",
    "             ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓",
    "               ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓",
    "                   ▓▓▓▓▓▓▓▓▓▓▓▓",
    "                    ▓▓      ▓▓",
    "                    ▓▓      ▓▓",
    "                 ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓",
    "               ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓",
    "             ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓",
    "           ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓",
];

const SPARK_COLOR: Color = Color::Rgb(246, 173, 85);

fn anvil_body_color(row: usize, row_count: usize) -> Color {
    let row_count = row_count.max(1);
    let denom = row_count.saturating_sub(1).max(1);
    let t = row as f32 / denom as f32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lerp = |a: f32, b: f32| (a + t * (b - a)).round() as u8;
    Color::Rgb(lerp(200.0, 22.0), lerp(228.0, 40.0), lerp(242.0, 56.0))
}

fn steel_highlight(base: Color) -> Color {
    match base {
        Color::Rgb(r, g, b) => Color::Rgb(
            r.saturating_add(28),
            g.saturating_add(24),
            b.saturating_add(20),
        ),
        _ => Color::Rgb(220, 238, 250),
    }
}

fn spark_line(spark_use_alt: bool) -> String {
    let c = if spark_use_alt { '✧' } else { '✦' };
    format!("            {c}        {c}        {c}")
}

fn styled_anvil_row(row_text: &str, body_row: usize, body_rows: usize) -> Line<'static> {
    let base = anvil_body_color(body_row, body_rows);
    let hi = steel_highlight(base);
    let mut spans: Vec<Span> = Vec::new();
    for ch in row_text.chars() {
        let st = match ch {
            '▓' => Style::default().fg(base),
            '▒' => Style::default().fg(hi),
            ' ' => Style::default(),
            _ => Style::default().fg(base),
        };
        spans.push(Span::styled(ch.to_string(), st));
    }
    Line::from(spans)
}

fn draw_centered_line_styled(buf: &mut Buffer, area: Rect, y: u16, line: Line<'static>) -> u16 {
    if y >= area.y + area.height {
        return y;
    }
    let max_w = area.width as usize;
    let line = if line.width() > max_w {
        truncate_line_to_chars(line, max_w)
    } else {
        line
    };
    let w = line.width().min(max_w) as u16;
    let x = area.x + area.width.saturating_sub(w) / 2;
    let rw = w
        .max(1)
        .min(area.width.saturating_sub(x.saturating_sub(area.x)));
    Paragraph::new(line).render(Rect::new(x, y, rw, 1), buf);
    y.saturating_add(1)
}

fn truncate_line_to_chars(line: Line<'static>, max_chars: usize) -> Line<'static> {
    let mut taken = 0usize;
    let mut out: Vec<Span> = Vec::new();
    for span in line.spans {
        if taken >= max_chars {
            break;
        }
        let mut chunk = String::new();
        for ch in span.content.chars() {
            if taken >= max_chars {
                break;
            }
            chunk.push(ch);
            taken += 1;
        }
        if !chunk.is_empty() {
            out.push(Span::styled(chunk, span.style));
        }
    }
    Line::from(out)
}

/// Renders the Akmon welcome art and copy into `buf` inside `area` when the transcript is empty.
///
/// Safe for very small or narrow terminals: returns immediately below 10×5; lines longer than
/// `area.width` are truncated by character count so layout never panics.
/// `spark_use_alt` swaps spark glyphs (✦ ↔ ✧) on the UI’s ~500 ms tick.
/// When `show_missing_akmon_hint` is true, shows dim amber quick-start hints before the divider.
///
/// If other-tool context files exist ([`ContextScan::files`]) and `AKMON.md` is absent, suggests
/// `/import`; otherwise `/init` and `/new`.
pub fn render_welcome(
    area: Rect,
    buf: &mut Buffer,
    version: &str,
    project_name: &str,
    spark_use_alt: bool,
    show_missing_akmon_hint: bool,
    context_scan: &ContextScan,
) {
    if area.width < 10 || area.height < 5 {
        return;
    }

    let spark = Line::from(Span::styled(
        spark_line(spark_use_alt),
        Style::default().fg(SPARK_COLOR),
    ));

    let title = "A K M O N";
    let subtitle = "local-first ai coding agent";
    let ver = format!("v{version}");
    let divider: String = "─".repeat(area.width as usize);
    let hint1 = " ollama   akmon chat --model llama3.2";
    let hint2 = " claude   --model claude-haiku-4-5-20251001";
    let hint3 = " index    add --index for code search";
    let bottom = "type a message or / for commands";
    let proj = format!("· {project_name} ·");

    // Spark + blank + README anvil rows + title + subtitle + version + project + [optional nudge block] + divider + 3 hints + divider + bottom.
    // spark row + blank + anvil rows
    let art_h = 2u16.saturating_add(ANVIL_BODY.len() as u16);
    let mut unique_tools: Vec<&'static str> = context_scan
        .files
        .iter()
        .map(|f| f.tool.display_name())
        .collect();
    unique_tools.sort_unstable();
    unique_tools.dedup();
    let nudge_lines: u16 = if show_missing_akmon_hint {
        if context_scan.files.is_empty() {
            3
        } else {
            2u16.saturating_add(unique_tools.len() as u16)
                .saturating_add(3)
        }
    } else {
        0
    };
    // title, subtitle, version, project + divider + 3 hints + divider + bottom
    let tail_h = 4u16 + 6u16 + nudge_lines;
    let total_h = art_h.saturating_add(tail_h);
    let mut y = area
        .y
        .saturating_add(area.height.saturating_sub(total_h) / 2);

    let body_rows = ANVIL_BODY.len();
    y = draw_centered_line_styled(buf, area, y, spark);
    y = y.saturating_add(1);
    for (i, row) in ANVIL_BODY.iter().enumerate() {
        let line = styled_anvil_row(row, i, body_rows);
        y = draw_centered_line_styled(buf, area, y, line);
    }

    y = draw_centered_line_truncated(
        buf,
        area,
        y,
        title,
        Style::default()
            .fg(Color::Rgb(184, 212, 232))
            .add_modifier(Modifier::BOLD),
    );
    y = draw_centered_line_truncated(
        buf,
        area,
        y,
        subtitle,
        Style::default().fg(Color::Rgb(74, 112, 128)),
    );
    y = draw_centered_line_truncated(
        buf,
        area,
        y,
        ver.as_str(),
        Style::default().fg(Color::Rgb(42, 72, 88)),
    );
    y = draw_centered_line_truncated(
        buf,
        area,
        y,
        proj.as_str(),
        Style::default().fg(Color::Rgb(60, 90, 105)),
    );

    let nudge_style = Style::default()
        .fg(Color::Rgb(181, 131, 55))
        .add_modifier(Modifier::DIM);
    if show_missing_akmon_hint {
        y = draw_centered_line_truncated(buf, area, y, "no AKMON.md found", nudge_style);
        if context_scan.files.is_empty() {
            y = draw_centered_line_truncated(
                buf,
                area,
                y,
                "/init  analyze this project",
                nudge_style,
            );
            y = draw_centered_line_truncated(
                buf,
                area,
                y,
                "/new   scaffold a new project",
                nudge_style,
            );
        } else {
            y = draw_centered_line_truncated(
                buf,
                area,
                y,
                "context detected from other tools:",
                nudge_style,
            );
            for name in unique_tools {
                let line = format!("  ✓ {name}");
                y = draw_centered_line_truncated(buf, area, y, line.as_str(), nudge_style);
            }
            y = draw_centered_line_truncated(buf, area, y, "", nudge_style);
            y = draw_centered_line_truncated(
                buf,
                area,
                y,
                "/import  convert to AKMON.md",
                nudge_style,
            );
            y = draw_centered_line_truncated(
                buf,
                area,
                y,
                "/init    analyze project fresh",
                nudge_style,
            );
        }
    }

    let divider_style = Style::default().fg(Color::DarkGray);
    let hint_style = Style::default().fg(Color::Rgb(42, 72, 88));
    y = draw_centered_line_truncated(buf, area, y, &divider, divider_style);
    y = draw_centered_line_truncated(buf, area, y, hint1, hint_style);
    y = draw_centered_line_truncated(buf, area, y, hint2, hint_style);
    y = draw_centered_line_truncated(buf, area, y, hint3, hint_style);
    y = draw_centered_line_truncated(buf, area, y, &divider, divider_style);
    let _ = draw_centered_line_truncated(
        buf,
        area,
        y,
        bottom,
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    );
}

fn draw_centered_line_truncated(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    text: &str,
    style: Style,
) -> u16 {
    if y >= area.y + area.height {
        return y;
    }
    let max_w = area.width as usize;
    let truncated: String = text.chars().take(max_w).collect();
    let line = Line::from(Span::styled(truncated.as_str(), style));
    let w = line.width().min(max_w) as u16;
    let x = area.x + area.width.saturating_sub(w) / 2;
    let rw = w
        .max(1)
        .min(area.width.saturating_sub(x.saturating_sub(area.x)));
    Paragraph::new(line).render(Rect::new(x, y, rw, 1), buf);
    y.saturating_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn render_welcome_tiny_area_no_panic() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 5));
        let scan = ContextScan {
            files: vec![],
            has_akmon_md: false,
            primary_tool: None,
        };
        render_welcome(
            Rect::new(0, 0, 10, 5),
            &mut buf,
            "1.3.0",
            "p",
            false,
            false,
            &scan,
        );
    }

    #[test]
    fn render_welcome_narrow_area_no_panic() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 8, 40));
        let scan = ContextScan {
            files: vec![],
            has_akmon_md: false,
            primary_tool: None,
        };
        render_welcome(
            Rect::new(0, 0, 8, 40),
            &mut buf,
            "1.3.0",
            "proj",
            true,
            true,
            &scan,
        );
    }

    #[test]
    fn render_welcome_via_terminal_smoke() {
        let backend = TestBackend::new(80, 40);
        let mut term = Terminal::new(backend).expect("terminal");
        let scan = ContextScan {
            files: vec![],
            has_akmon_md: false,
            primary_tool: None,
        };
        let _ = term.draw(|f| {
            let area = f.size();
            render_welcome(area, f.buffer_mut(), "1.3.0", "Akmon", false, false, &scan);
        });
    }
}
