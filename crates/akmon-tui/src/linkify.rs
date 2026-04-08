//! OSC 8 terminal hyperlinks for URLs in transcript text.

use regex::Regex;
use std::sync::LazyLock;

static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(https?://[^\s)]+|http://localhost:\d+[^\s)]*|http://127\.0\.0\.1:\d+[^\s)]*)",
    )
    .expect("url regex")
});

/// Wraps `url` as a clickable OSC 8 link with visible `label` (often the same as the URL).
#[must_use]
pub fn make_terminal_link(url: &str, label: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{label}\x1b]8;;\x1b\\")
}

/// Inserts OSC 8 hyperlinks for URLs in plain text (best-effort).
#[must_use]
pub fn linkify_text(s: &str) -> String {
    URL_RE
        .replace_all(s, |caps: &regex::Captures<'_>| {
            let url = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            if url.is_empty() {
                return String::new();
            }
            make_terminal_link(url, url)
        })
        .into_owned()
}
