//! Small text helpers shared across provider backends.

/// Returns the longest prefix of `s` that fits within `max` bytes and ends on a UTF-8
/// character boundary.
///
/// Slicing a `&str` at an arbitrary byte offset panics when the offset lands inside a
/// multi-byte codepoint. Provider HTTP error bodies are proxy- or server-controlled and
/// routinely carry non-ASCII text (localized messages, emoji), so error-path truncation
/// must use this instead of a raw `&s[..max]`, which would crash the live request.
pub(crate) fn truncate_at_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_truncate_short_string_unchanged() {
        assert_eq!(truncate_at_char_boundary("hello", 512), "hello");
    }

    #[test]
    fn t_truncate_on_multibyte_boundary_does_not_panic() {
        // 511 ASCII bytes then a 3-byte '€' occupying bytes 511..514: byte offset 512 lands
        // inside the euro sign, so a naive `&s[..512]` would panic.
        let s = format!("{}{}", "a".repeat(511), "€");
        assert_eq!(s.len(), 514);
        let out = truncate_at_char_boundary(&s, 512);
        assert_eq!(out.len(), 511);
        assert!(out.chars().all(|c| c == 'a'));
    }

    #[test]
    fn t_truncate_exact_boundary_kept() {
        let s = "a".repeat(512);
        assert_eq!(truncate_at_char_boundary(&s, 512).len(), 512);
    }
}
