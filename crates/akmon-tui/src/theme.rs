//! Terminal RGB palette for transcript, chrome, and overlays (Gemini-style dark accents).

use ratatui::style::Color;

/// Near-white body text.
pub const FG_PRIMARY: Color = Color::Rgb(230, 237, 243);
/// Secondary labels and borders.
pub const FG_MUTED: Color = Color::Rgb(139, 148, 158);
/// Subtle border lines.
pub const BORDER: Color = Color::Rgb(55, 62, 74);
/// Cyan accent (brand / prompts).
pub const ACCENT: Color = Color::Rgb(125, 211, 252);
/// Softer accent for hints.
pub const ACCENT_DIM: Color = Color::Rgb(88, 166, 205);
/// Success / cache highlights.
pub const OK_GREEN: Color = Color::Rgb(126, 231, 135);
/// Selected row in pickers.
pub const SELECT_BG: Color = Color::Rgb(45, 55, 72);
/// Warning / confirmation.
pub const WARN: Color = Color::Rgb(242, 204, 96);
/// Errors and failed tool runs.
pub const ERR: Color = Color::Rgb(248, 113, 113);
