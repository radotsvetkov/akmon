//! Terminal-safe rendering of untrusted strings for human diff output.
//!
//! Object bytes, provider/tool ids and error messages come from imported bundles and OTEL traces,
//! so they are untrusted. Printed raw, ANSI/OSC escape sequences in them could rewrite the
//! terminal, spoof a `matches: yes` line, or set the clipboard (OSC 52). JSON output does not need
//! this: `serde_json` already escapes control characters.

/// Escapes characters that a terminal may interpret, for printing untrusted text.
///
/// C0 controls (including ESC), DEL, C1 controls (U+0080..=U+009F) and Unicode bidirectional
/// override/isolate characters are written as `\u{..}`; all other text (including backslashes)
/// is left as is, so ordinary previews render unchanged. Tab is kept. Newline is kept only when
/// `keep_newlines` is true (multi-line previews); otherwise it is written as `\n` so a single-line
/// field cannot inject extra report lines. Carriage return is always escaped.
#[must_use]
pub fn sanitize_for_terminal(input: &str, keep_newlines: bool) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\t' => out.push('\t'),
            '\n' if keep_newlines => out.push('\n'),
            '\n' => out.push_str("\\n"),
            c if is_terminal_unsafe(c) => {
                out.push_str(&format!("\\u{{{:x}}}", u32::from(c)));
            }
            c => out.push(c),
        }
    }
    out
}

fn is_terminal_unsafe(c: char) -> bool {
    c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

#[cfg(test)]
mod tests {
    use super::sanitize_for_terminal;

    #[test]
    fn plain_text_is_unchanged() {
        assert_eq!(
            sanitize_for_terminal("hello, wörld\tok", false),
            "hello, wörld\tok"
        );
    }

    #[test]
    fn ansi_and_osc_sequences_are_escaped() {
        let hostile = "\u{1b}[2K\rmatches: yes\u{1b}]52;c;ZXZpbA==\u{7}";
        let s = sanitize_for_terminal(hostile, true);
        assert!(!s.contains('\u{1b}'));
        assert!(!s.contains('\r'));
        assert!(!s.contains('\u{7}'));
        assert_eq!(s, "\\u{1b}[2K\\u{d}matches: yes\\u{1b}]52;c;ZXZpbA==\\u{7}");
    }

    #[test]
    fn c1_controls_del_and_bidi_overrides_are_escaped() {
        let s = sanitize_for_terminal("a\u{9b}b\u{7f}c\u{202e}d\u{2066}e", false);
        assert_eq!(s, "a\\u{9b}b\\u{7f}c\\u{202e}d\\u{2066}e");
    }

    #[test]
    fn newline_handling_depends_on_flag() {
        assert_eq!(sanitize_for_terminal("a\nb", true), "a\nb");
        assert_eq!(sanitize_for_terminal("a\nb", false), "a\\nb");
    }

    #[test]
    fn backslashes_are_left_alone() {
        assert_eq!(sanitize_for_terminal(r"C:\dir\file", false), r"C:\dir\file");
    }
}
