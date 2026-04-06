//! Terminal RGB palette for transcript, chrome, and overlays (Gemini-style dark accents).
//!
//! Primary body text uses [`Color::Reset`] so it follows the host terminal’s configured
//! foreground—readable on light backgrounds as well as dark. Use [`FG_ON_SELECT`] anywhere we
//! paint a dark panel ([`SELECT_BG`]) so contrast stays correct.

use ratatui::style::Color;

/// Body and chrome text on the default terminal background (inherits light/dark from the profile).
pub const FG_PRIMARY: Color = Color::Reset;
/// Light foreground for text drawn on [`SELECT_BG`] (selection rows, code blocks on panel fill).
pub const FG_ON_SELECT: Color = Color::Rgb(230, 237, 243);
/// Secondary labels and metadata (slightly deepened for legibility on light themes).
pub const FG_MUTED: Color = Color::Rgb(100, 108, 118);
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
