//! Line-oriented syntax highlighting for fenced code blocks (no external highlighters).

use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Span;

/// Foreground palette loosely matching VS Code dark+.
mod palette {
    use ratatui::style::Color;
    pub(super) const KEYWORD: Color = Color::Rgb(86, 156, 214);
    pub(super) const STRING: Color = Color::Rgb(206, 145, 120);
    pub(super) const COMMENT: Color = Color::Rgb(106, 153, 85);
    pub(super) const NUMBER: Color = Color::Rgb(181, 206, 168);
    pub(super) const DEFAULT: Color = Color::Rgb(212, 212, 212);
    pub(super) const SHELL_CMD: Color = Color::Rgb(86, 156, 214);
    pub(super) const SHELL_FLAG: Color = Color::Rgb(242, 204, 96);
}

const RUST_KW: &[&str] = &[
    "fn", "pub", "let", "mut", "use", "struct", "enum", "impl", "return", "if", "else", "match",
    "for", "while", "loop", "async", "await", "move", "where", "type", "trait", "const", "static",
    "unsafe", "Self", "self", "super", "crate", "mod", "as", "ref", "dyn", "break", "continue",
    "in", "where",
];

const PY_KW: &[&str] = &[
    "def", "class", "import", "from", "return", "if", "elif", "else", "for", "while", "with", "as",
    "pass", "break", "continue", "lambda", "yield", "async", "await", "try", "except", "finally",
    "raise", "global", "nonlocal", "True", "False", "None",
];

const JS_KW: &[&str] = &[
    "function", "const", "let", "var", "return", "if", "else", "for", "while", "async", "await",
    "try", "catch", "finally", "throw", "class", "extends", "import", "export", "default", "from",
    "new", "typeof", "instanceof", "true", "false", "null", "undefined",
];

/// Applies a very small lexer to one line of code for the given `lang` hint.
#[must_use]
pub fn highlight_line(line: &str, lang: &str) -> Vec<Span<'static>> {
    let lang = lang.to_ascii_lowercase();
    match lang.as_str() {
        "rust" | "rs" => highlight_rust_line(line),
        "python" | "py" => highlight_kw_line(line, PY_KW),
        "javascript" | "js" | "typescript" | "ts" | "tsx" | "jsx" => highlight_kw_line(line, JS_KW),
        "bash" | "sh" | "shell" | "zsh" => highlight_shell_line(line),
        _ => vec![Span::styled(line.to_string(), Style::default().fg(palette::DEFAULT))],
    }
}

/// Background fill for code blocks.
#[must_use]
pub fn code_block_bg() -> Color {
    Color::Rgb(30, 30, 30)
}

fn highlight_rust_line(line: &str) -> Vec<Span<'static>> {
    let t = line.trim_start();
    if t.starts_with("//") {
        return vec![Span::styled(
            line.to_string(),
            Style::default().fg(palette::COMMENT),
        )];
    }
    highlight_kw_line(line, RUST_KW)
}

fn highlight_kw_line(line: &str, kws: &[&str]) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix("//") {
            spans.push(Span::styled(
                format!("//{stripped}"),
                Style::default().fg(palette::COMMENT),
            ));
            break;
        }
        if let Some(stripped) = rest.strip_prefix("/*")
            && let Some(end) = stripped.find("*/")
        {
            let (_comment, after) = stripped.split_at(end + 2);
            spans.push(Span::styled(
                format!("/*{}*/", &stripped[..end]),
                Style::default().fg(palette::COMMENT),
            ));
            rest = after;
            continue;
        }
        // string literals
        if let Some(q) = rest.chars().next()
            && (q == '"' || q == '\'')
            && let Some(end) = find_closing_quote(rest, q)
        {
            spans.push(Span::styled(
                rest[..=end].to_string(),
                Style::default().fg(palette::STRING),
            ));
            rest = &rest[end + 1..];
            continue;
        }
        let token = next_token(rest);
        if token.is_empty() {
            break;
        }
        let tok = &rest[..token.len()];
        rest = &rest[token.len()..];
        if is_number(tok) {
            spans.push(Span::styled(
                tok.to_string(),
                Style::default().fg(palette::NUMBER),
            ));
        } else if kws.contains(&tok) {
            spans.push(Span::styled(
                tok.to_string(),
                Style::default().fg(palette::KEYWORD),
            ));
        } else {
            spans.push(Span::styled(
                tok.to_string(),
                Style::default().fg(palette::DEFAULT),
            ));
        }
    }
    if spans.is_empty() {
        spans.push(Span::styled(
            line.to_string(),
            Style::default().fg(palette::DEFAULT),
        ));
    }
    spans
}

fn highlight_shell_line(line: &str) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut parts = line.split_whitespace();
    let Some(first) = parts.next() else {
        return vec![Span::styled(
            line.to_string(),
            Style::default().fg(palette::DEFAULT),
        )];
    };
    let offset = line.find(first).unwrap_or(0);
    if offset > 0 {
        spans.push(Span::styled(
            line[..offset].to_string(),
            Style::default().fg(palette::DEFAULT),
        ));
    }
    spans.push(Span::styled(
        first.to_string(),
        Style::default().fg(palette::SHELL_CMD),
    ));
    let mut pos = offset + first.len();
    for w in parts {
        let idx = line[pos..].find(w).map(|i| pos + i).unwrap_or(pos);
        if idx > pos {
            spans.push(Span::styled(
                line[pos..idx].to_string(),
                Style::default().fg(palette::DEFAULT),
            ));
        }
        let is_flag = w.starts_with('-');
        spans.push(Span::styled(
            w.to_string(),
            Style::default().fg(if is_flag {
                palette::SHELL_FLAG
            } else {
                palette::DEFAULT
            }),
        ));
        pos = idx + w.len();
    }
    if pos < line.len() {
        spans.push(Span::styled(
            line[pos..].to_string(),
            Style::default().fg(palette::DEFAULT),
        ));
    }
    spans
}

fn find_closing_quote(s: &str, q: char) -> Option<usize> {
    let mut esc = false;
    for (i, c) in s.char_indices().skip(1) {
        if esc {
            esc = false;
            continue;
        }
        if c == '\\' {
            esc = true;
            continue;
        }
        if c == q {
            return Some(i);
        }
    }
    None
}

fn next_token(s: &str) -> &str {
    if s.is_empty() {
        return "";
    }
    let mut end = 0usize;
    for (i, c) in s.char_indices() {
        if c.is_alphanumeric() || c == '_' {
            end = i + c.len_utf8();
        } else {
            if end == 0 {
                end = i + c.len_utf8();
            }
            break;
        }
    }
    if end == 0 {
        s.chars().next().map(|c| &s[..c.len_utf8()]).unwrap_or("")
    } else {
        &s[..end]
    }
}

fn is_number(tok: &str) -> bool {
    !tok.is_empty() && tok.chars().all(|c| c.is_ascii_digit() || c == '_' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_keyword_blue() {
        let s = highlight_line("fn main() {", "rust");
        let joined: String = s.iter().map(|x| x.content.as_ref()).collect();
        assert!(joined.contains("fn"));
    }

    #[test]
    fn comment_green() {
        let s = highlight_line("// x", "rust");
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn unknown_lang_plain() {
        let s = highlight_line("hello", "foobar");
        assert!(!s.is_empty());
    }
}
