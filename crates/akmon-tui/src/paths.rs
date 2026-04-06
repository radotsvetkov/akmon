//! Path presentation helpers for chrome.

use std::path::Path;

/// `$HOME`-aware, tail-preserved truncation for status lines.
pub(crate) fn shorten_from_left_chars(s: &str, max_chars: usize) -> String {
    let n = s.chars().count();
    if n <= max_chars {
        return s.to_string();
    }
    let take = max_chars.saturating_sub(3);
    let skip = n.saturating_sub(take);
    let tail: String = s.chars().skip(skip).collect();
    format!("...{tail}")
}

/// Project root shown in chrome: `$HOME` → `~`, tail-preserved truncation at `max_chars`.
pub(crate) fn cwd_shortened(project_root: &Path) -> String {
    let path = std::fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf());
    let mut s = path.to_string_lossy().into_owned();
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
        && s.starts_with(&home)
    {
        s = format!("~{}", &s[home.len()..]);
    }
    shorten_from_left_chars(&s, 40)
}
