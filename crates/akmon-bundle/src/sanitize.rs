//! Terminal-safe rendering of attacker-controlled strings.
//!
//! Every string that comes out of a bundle manifest (signature `scheme`/`key_id`, operator identity
//! fields, `created_at` values, producer name/version, ...) is attacker-controlled: anyone holding
//! the file can edit `manifest.json` without breaking integrity, because those fields are metadata.
//! Printing such a value raw lets a crafted bundle inject a newline (forging an extra
//! "verified" report line), an ANSI/CSI escape sequence (rewriting what the terminal shows), or a
//! bidirectional override (visually reordering text). [`sanitize_display`] is the single escaper
//! both verifier binaries and `akmon bundle prove-openssl` route such values through before display.

/// Escapes characters that can alter how a terminal (or a Markdown viewer) renders surrounding
/// text, keeping the value on one visible line.
///
/// C0 controls and DEL render as `\xNN`; everything else in this list renders as `\u{NNNN}`:
/// - C0 controls (U+0000–U+001F) and DEL (U+007F),
/// - C1 controls (U+0080–U+009F; U+009B is interpreted as CSI by some terminals),
/// - Unicode format/bidi characters: U+00AD, U+061C, U+180E, U+200B–U+200F, U+2028–U+202E,
///   U+2060–U+206F, U+FEFF, and U+FFF9–U+FFFB,
/// - other invisible Unicode format (Cf) characters: U+0600–U+0605, U+06DD, U+070F, U+0890–U+0891,
///   U+08E2, U+110BD, U+110CD, U+13430–U+1343F, U+1BCA0–U+1BCA3, U+1D173–U+1D17A, and the TAG
///   characters U+E0001 and U+E0020–U+E007F (which can smuggle hidden text).
///
/// A literal backslash is doubled so an escaped rendering can never be confused with a value that
/// already contained the text `\x1b`.
#[must_use]
pub fn sanitize_display(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        let cp = ch as u32;
        if ch == '\\' {
            out.push_str("\\\\");
        } else if cp < 0x20 || cp == 0x7f {
            out.push_str(&format!("\\x{cp:02x}"));
        } else if (0x80..=0x9f).contains(&cp) || is_format_or_bidi(cp) {
            out.push_str(&format!("\\u{{{cp:04x}}}"));
        } else {
            out.push(ch);
        }
    }
    out
}

/// Unicode format (Cf) and bidi-control code points that can visually reorder or hide text.
fn is_format_or_bidi(cp: u32) -> bool {
    matches!(
        cp,
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x2028..=0x202E
            | 0x2060..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_plain_text_is_unchanged() {
        assert_eq!(sanitize_display("alice@example.com"), "alice@example.com");
        assert_eq!(sanitize_display("Zoë Ångström"), "Zoë Ångström");
    }

    #[test]
    fn t_c0_and_del_are_escaped() {
        assert_eq!(sanitize_display("a\nb"), "a\\x0ab");
        assert_eq!(sanitize_display("\x1b[31mred"), "\\x1b[31mred");
        assert_eq!(sanitize_display("x\u{7f}"), "x\\x7f");
    }

    #[test]
    fn t_c1_controls_are_escaped() {
        assert_eq!(sanitize_display("a\u{9b}31m"), "a\\u{009b}31m");
        assert_eq!(sanitize_display("\u{85}"), "\\u{0085}");
    }

    #[test]
    fn t_bidi_and_format_chars_are_escaped() {
        assert_eq!(sanitize_display("abc\u{202e}fed"), "abc\\u{202e}fed");
        assert_eq!(sanitize_display("\u{2066}x\u{2069}"), "\\u{2066}x\\u{2069}");
        assert_eq!(sanitize_display("a\u{200b}b"), "a\\u{200b}b");
        assert_eq!(sanitize_display("\u{feff}"), "\\u{feff}");
    }

    #[test]
    fn t_tag_and_other_invisible_format_chars_are_escaped() {
        assert_eq!(sanitize_display("a\u{e0001}b"), "a\\u{e0001}b");
        assert_eq!(
            sanitize_display("\u{e0041}\u{e007f}"),
            "\\u{e0041}\\u{e007f}"
        );
        assert_eq!(sanitize_display("x\u{0600}"), "x\\u{0600}");
        assert_eq!(sanitize_display("\u{1d173}"), "\\u{1d173}");
        assert_eq!(sanitize_display("\u{110bd}"), "\\u{110bd}");
        // Ordinary non-ASCII text right next to the ranges stays intact.
        assert_eq!(sanitize_display("\u{0606}\u{e0080}"), "\u{0606}\u{e0080}");
    }

    #[test]
    fn t_backslash_is_doubled() {
        assert_eq!(sanitize_display("a\\x0ab"), "a\\\\x0ab");
    }

    #[test]
    fn t_output_never_contains_raw_line_breaks() {
        let hostile = "x] key_id=a\n    - verified [ed25519\r\u{2028}\u{2029}";
        let out = sanitize_display(hostile);
        assert!(!out.contains('\n'));
        assert!(!out.contains('\r'));
        assert!(!out.contains('\u{2028}'));
        assert!(!out.contains('\u{2029}'));
    }
}
