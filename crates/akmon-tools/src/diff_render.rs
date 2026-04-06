//! Unified diffs for confirmation previews (`write_file`, `edit`).

use diffy::create_patch;

/// Plain unified diff of `original` vs `new_content` for `path` (no ANSI).
pub fn unified_diff_text(original: &str, new_content: &str, path: &str) -> String {
    let patch = create_patch(original, new_content);
    let mut base = format!("--- a/{path}\n+++ b/{path}\n");
    base.push_str(&patch.to_string());
    base
}

/// Adds ANSI foreground colors to a plain unified diff string.
///
/// Lines starting with `+` (except `+++`) are green; `-` (except `---`) red; `@@` dim cyan.
pub fn colorize_unified_diff(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().saturating_mul(2));
    for line in raw.lines() {
        let colored = if line.starts_with('+') && !line.starts_with("+++") {
            format!("\x1b[32m{line}\x1b[0m")
        } else if line.starts_with('-') && !line.starts_with("---") {
            format!("\x1b[31m{line}\x1b[0m")
        } else if line.starts_with("@@") {
            format!("\x1b[2;36m{line}\x1b[0m")
        } else {
            line.to_string()
        };
        out.push_str(&colored);
        out.push('\n');
    }
    out
}

/// Full ANSI-colored unified diff (for stdout/stderr in headless mode).
pub fn render_diff(original: &str, new_content: &str, path: &str) -> String {
    colorize_unified_diff(&unified_diff_text(original, new_content, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_diff_shows_plus_and_minus() {
        let d = unified_diff_text("a\n", "b\n", "f.txt");
        assert!(d.contains("+b"));
        assert!(d.contains("-a"));
    }

    #[test]
    fn render_diff_includes_ansi_for_additions() {
        let d = render_diff("x\n", "x\ny\n", "f.txt");
        assert!(d.contains("\x1b[32m"));
        assert!(d.contains("+y"));
    }
}
