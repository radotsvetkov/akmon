//! Char-boundary-safe text helpers.
//!
//! Slicing a `&str` (or calling [`String::truncate`]) at an arbitrary byte offset panics
//! when the offset lands inside a multi-byte codepoint. Model output, tool arguments, file
//! contents and subprocess output routinely carry non-ASCII text, and the release profile
//! uses `panic = "abort"`, so every byte-budget truncation of such text must go through
//! these helpers instead of a raw `&s[..n]`.

/// Returns the longest prefix of `s` that is at most `max` bytes long and ends on a UTF-8
/// character boundary.
pub fn truncate_at_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or_default()
}

/// In-place variant of [`truncate_at_char_boundary`]: shortens `s` to at most `max` bytes
/// without splitting a codepoint.
pub fn truncate_string_at_char_boundary(s: &mut String, max: usize) {
    let keep = truncate_at_char_boundary(s, max).len();
    s.truncate(keep);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_string_unchanged() {
        assert_eq!(truncate_at_char_boundary("hello", 10), "hello");
        assert_eq!(truncate_at_char_boundary("", 0), "");
    }

    #[test]
    fn cut_inside_multibyte_walks_back() {
        // 1999 ASCII bytes then a 3-byte '€' at bytes 1999..2002.
        let s = format!("{}€tail", "a".repeat(1999));
        let out = truncate_at_char_boundary(&s, 2000);
        assert_eq!(out.len(), 1999);
        assert!(out.chars().all(|c| c == 'a'));
    }

    #[test]
    fn exact_boundary_kept() {
        let s = "ab€cd";
        assert_eq!(truncate_at_char_boundary(s, 5), "ab€");
        assert_eq!(truncate_at_char_boundary(s, 4), "ab");
        assert_eq!(truncate_at_char_boundary(s, 0), "");
    }

    #[test]
    fn emoji_and_cjk_never_split() {
        let s = "😀漢字";
        for max in 0..=s.len() {
            let out = truncate_at_char_boundary(s, max);
            assert!(out.len() <= max);
            assert!(s.starts_with(out));
        }
    }

    #[test]
    fn in_place_variant_does_not_panic() {
        let mut s = format!("{}é", "x".repeat(9));
        truncate_string_at_char_boundary(&mut s, 10);
        assert_eq!(s, "x".repeat(9));
        let mut short = String::from("ok");
        truncate_string_at_char_boundary(&mut short, 10);
        assert_eq!(short, "ok");
    }
}
