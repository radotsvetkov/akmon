//! Branded empty-state welcome screen (pixel anvil + quick-start hints).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

/// Renders the Akmon welcome art and copy into `buf` inside `area` when the transcript is empty.
///
/// Safe for very small or narrow terminals: returns immediately below 10×5; lines longer than
/// `area.width` are truncated by character count so layout never panics.
/// `spark_use_alt` swaps spark glyphs (✦ ↔ ✧) on the UI’s ~500 ms tick.
/// When `show_missing_akmon_hint` is true, shows dim amber `/init` and `/new` hints before the
/// quick-start lines.
pub fn render_welcome(
    area: Rect,
    buf: &mut Buffer,
    version: &str,
    project_name: &str,
    spark_use_alt: bool,
    show_missing_akmon_hint: bool,
) {
    if area.width < 10 || area.height < 5 {
        return;
    }

    let sparks = if spark_use_alt {
        "  ✧    ✧  ✧  "
    } else {
        "  ✦    ✦  ✦  "
    };

    let anvil: &[(&str, Color)] = &[
        (sparks, Color::Rgb(246, 173, 85)),
        ("    ████████████    ", Color::Rgb(200, 228, 242)),
        ("  ████████████████  ", Color::Rgb(160, 200, 220)),
        ("████████████████████", Color::Rgb(122, 170, 191)),
        ("██████████████████  ", Color::Rgb(106, 152, 172)),
        ("      ████████      ", Color::Rgb(74, 112, 128)),
        ("      ████████      ", Color::Rgb(74, 112, 128)),
        ("    ████████████    ", Color::Rgb(54, 86, 106)),
        ("  ████████████████  ", Color::Rgb(44, 72, 88)),
        ("████████████████████", Color::Rgb(30, 52, 64)),
        ("██████████████████████", Color::Rgb(22, 40, 56)),
    ];

    let title = "A K M O N";
    let subtitle = "local-first ai coding agent";
    let ver = format!("v{version}");
    let divider: String = "─".repeat(area.width as usize);
    let hint1 = " ollama   akmon chat --model llama3.2";
    let hint2 = " claude   --model claude-haiku-4-5-20251001";
    let hint3 = " index    add --index for code search";
    let bottom = "type a message or / for commands";
    let proj = format!("· {project_name} ·");

    // 11 anvil + title + subtitle + version + project + [optional 3 nudge lines] + divider + 3 hints + divider + bottom.
    let total_h = if show_missing_akmon_hint {
        24u16
    } else {
        21u16
    };
    let mut y = area
        .y
        .saturating_add(area.height.saturating_sub(total_h) / 2);

    for (text, color) in anvil {
        let style = Style::default().fg(*color);
        y = draw_centered_line_truncated(buf, area, y, text, style);
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
        y = draw_centered_line_truncated(buf, area, y, "/init  analyze this project", nudge_style);
        y = draw_centered_line_truncated(
            buf,
            area,
            y,
            "/new   scaffold a new project",
            nudge_style,
        );
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
        render_welcome(Rect::new(0, 0, 10, 5), &mut buf, "1.3.0", "p", false, false);
    }

    #[test]
    fn render_welcome_narrow_area_no_panic() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 8, 40));
        render_welcome(
            Rect::new(0, 0, 8, 40),
            &mut buf,
            "1.3.0",
            "proj",
            true,
            true,
        );
    }

    #[test]
    fn render_welcome_via_terminal_smoke() {
        let backend = TestBackend::new(80, 40);
        let mut term = Terminal::new(backend).expect("terminal");
        let _ = term.draw(|f| {
            let area = f.size();
            render_welcome(area, f.buffer_mut(), "1.3.0", "Akmon", false, false);
        });
    }
}
