//! Bottom status strip: session, tokens, cache, cost, hint.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

const BG: ratatui::style::Color = ratatui::style::Color::Rgb(17, 17, 20);

#[must_use]
fn fmt_u32_commas(n: u32) -> String {
    let s = n.to_string();
    let mut out: Vec<char> = Vec::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.into_iter().rev().collect()
}

/// Returns a conservative context window size for UI usage indicators.
#[must_use]
pub fn context_window_for_model(model: &str) -> u64 {
    if model.contains("claude") {
        200_000
    } else if model.starts_with("gpt-4.1") {
        1_047_576
    } else if model.starts_with("gpt-4o") {
        128_000
    } else if model.starts_with("o1") || model.starts_with("o3") {
        200_000
    } else {
        8_192
    }
}

#[must_use]
pub fn context_usage_percent(input_tokens: u32, cache_read_tokens: u32, model: &str) -> u8 {
    let window = context_window_for_model(model);
    // Anthropic cache-read tokens still represent prompt tokens processed by the API.
    // Include them so the context/rate-pressure indicator matches provider-reported usage.
    let used = u64::from(input_tokens).saturating_add(u64::from(cache_read_tokens));
    ((used as f64 / window as f64 * 100.0).min(100.0)) as u8
}

#[must_use]
pub fn render_context_bar(pct: u8) -> (String, Color) {
    let filled = (usize::from(pct) * 20 / 100).min(20);
    let empty = 20usize.saturating_sub(filled);
    let bar = format!("[{}{}] {pct}%", "█".repeat(filled), "░".repeat(empty));
    let color = match pct {
        0..=60 => Color::Green,
        61..=80 => Color::Yellow,
        81..=90 => Color::Rgb(245, 158, 11),
        _ => Color::Red,
    };
    (bar, color)
}

/// Draws the session / usage / contextual hint line.
pub fn render_status_bar(f: &mut ratatui::Frame<'_>, area: Rect, parts: StatusParts) {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let sep = Style::default().fg(ratatui::style::Color::Rgb(55, 62, 74));
    let dg = Style::default().fg(ratatui::style::Color::DarkGray);

    spans.push(Span::styled(parts.session_prefix.clone(), dg));
    spans.push(Span::styled("  │  ", sep));
    spans.push(Span::styled(
        format!("tokens:{}", fmt_u32_commas(parts.input_tokens)),
        dg,
    ));
    spans.push(Span::styled("  ", Style::default()));
    spans.push(Span::styled(parts.context_bar, parts.context_bar_style));
    spans.push(Span::styled("  │  ", sep));
    spans.push(Span::styled(
        format!("out:{}", fmt_u32_commas(parts.output_tokens)),
        dg,
    ));
    if parts.cache > 0 {
        spans.push(Span::styled("  │  ", sep));
        spans.push(Span::styled(
            format!("cache:{}", fmt_u32_commas(parts.cache)),
            parts.cache_style,
        ));
    }
    if parts.cleared > 0 {
        spans.push(Span::styled("  │  ", sep));
        spans.push(Span::styled(
            format!("cleared:~{}", fmt_u32_commas(parts.cleared)),
            dg,
        ));
    }
    if let Some(cost) = parts.cost_line {
        spans.push(Span::styled("  │  ", sep));
        spans.push(Span::styled(cost.text, cost.style));
    }
    let left: String = spans
        .iter()
        .map(|s| s.content.as_ref().to_string())
        .collect::<Vec<_>>()
        .concat();
    let left_w = left.chars().count();
    let hint_w = parts.hint.chars().count();
    let pad = (area.width as usize).saturating_sub(left_w + hint_w);
    spans.push(Span::styled(" ".repeat(pad.min(512)), Style::default()));
    spans.push(Span::styled(
        parts.hint.clone(),
        Style::default()
            .fg(ratatui::style::Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    ));

    let block = Block::default().style(Style::default().bg(BG));
    f.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
}

/// Pre-styled cost fragment when present.
pub struct CostFrag {
    /// Text such as `"~$0.04"` or `"free"`.
    pub text: String,
    /// Ratatui style (red when over threshold).
    pub style: Style,
}

/// Fields required to paint [`render_status_bar`].
pub struct StatusParts {
    /// Short session id (8 chars).
    pub session_prefix: String,
    /// Sum of input (non-cache) tokens for the session.
    pub input_tokens: u32,
    /// Sum of output tokens for the session.
    pub output_tokens: u32,
    /// Context usage bar with percent text (`[██░░...] 42%`).
    pub context_bar: String,
    /// Style for the context usage bar.
    pub context_bar_style: Style,
    /// Cache read tokens.
    pub cache: u32,
    /// Estimated input tokens cleared by micro-compaction (dim when shown).
    pub cleared: u32,
    /// Style for cache field.
    pub cache_style: Style,
    /// Optional cost segment.
    pub cost_line: Option<CostFrag>,
    /// Right-aligned hint text.
    pub hint: String,
}
