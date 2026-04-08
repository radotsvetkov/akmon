//! Slash-command definitions, parsing, and autocomplete matching.

/// Metadata for one `/command` supported by the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommand {
    /// Primary name without the leading `/`.
    pub name: &'static str,
    /// Alternate names (e.g. `/quit` for `/exit`).
    pub aliases: &'static [&'static str],
    /// One-line description for help and autocomplete.
    pub description: &'static str,
    /// Whether an argument may follow the command word.
    pub takes_arg: bool,
}

/// Static registry of all slash commands (order defines default listing).
pub const COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "help",
        aliases: &[],
        description: "Show all commands",
        takes_arg: false,
    },
    SlashCommand {
        name: "clear",
        aliases: &[],
        description: "Clear on-screen history (agent context unchanged)",
        takes_arg: false,
    },
    SlashCommand {
        name: "reset",
        aliases: &[],
        description: "Start a new session (saves current first)",
        takes_arg: false,
    },
    SlashCommand {
        name: "init",
        aliases: &[],
        description: "Analyze this project and generate AKMON.md",
        takes_arg: false,
    },
    SlashCommand {
        name: "import",
        aliases: &[],
        description: "Run akmon import (other tools' context → AKMON.md)",
        takes_arg: false,
    },
    SlashCommand {
        name: "export",
        aliases: &[],
        description: "Run akmon export --all (AKMON.md → tool formats)",
        takes_arg: false,
    },
    SlashCommand {
        name: "new",
        aliases: &[],
        description: "Scaffold a new project in this directory (requires a name)",
        takes_arg: true,
    },
    SlashCommand {
        name: "sessions",
        aliases: &[],
        description: "List past sessions to resume",
        takes_arg: false,
    },
    SlashCommand {
        name: "resume",
        aliases: &[],
        description: "Resume session by id (or open list with no arg)",
        takes_arg: true,
    },
    SlashCommand {
        name: "model",
        aliases: &["models"],
        description: "Show or set the model for next turns",
        takes_arg: true,
    },
    SlashCommand {
        name: "index",
        aliases: &[],
        description: "Show semantic index status",
        takes_arg: false,
    },
    SlashCommand {
        name: "audit",
        aliases: &[],
        description: "View this session's audit log",
        takes_arg: false,
    },
    SlashCommand {
        name: "cost",
        aliases: &[],
        description: "Token usage and cost estimate",
        takes_arg: false,
    },
    SlashCommand {
        name: "plan",
        aliases: &[],
        description: "Next message: read-only plan (no writes)",
        takes_arg: false,
    },
    SlashCommand {
        name: "implement",
        aliases: &[],
        description: "Run implementation for the last /plan output",
        takes_arg: false,
    },
    SlashCommand {
        name: "edit-plan",
        aliases: &[],
        description: "Open the latest plan in $EDITOR",
        takes_arg: false,
    },
    SlashCommand {
        name: "view-plan",
        aliases: &[],
        description: "Show the latest plan in the chat view",
        takes_arg: false,
    },
    SlashCommand {
        name: "architect",
        aliases: &[],
        description: "Next message: planner model then main model",
        takes_arg: false,
    },
    SlashCommand {
        name: "spec",
        aliases: &[],
        description: "List feature specs under .akmon/specs",
        takes_arg: false,
    },
    SlashCommand {
        name: "update-context",
        aliases: &[],
        description: "Edit AKMON.md in $EDITOR and reload",
        takes_arg: false,
    },
    SlashCommand {
        name: "doctor",
        aliases: &[],
        description: "Show provider keys and Ollama status",
        takes_arg: false,
    },
    SlashCommand {
        name: "exit",
        aliases: &["quit", "q"],
        description: "Save and exit",
        takes_arg: false,
    },
];

fn command_by_name(name: &str) -> Option<&'static SlashCommand> {
    let n = name.to_lowercase();
    for c in COMMANDS {
        if c.name == n {
            return Some(c);
        }
        for a in c.aliases {
            if a == &n {
                return Some(c);
            }
        }
    }
    None
}

/// Parses a full input line like `/model claude` into a command and optional argument tail.
///
/// Returns [`None`] when the line does not start with `/`, the command is unknown, or there is
/// trailing garbage after an argument where none is expected.
pub fn parse_slash_input(input: &str) -> Option<(&'static SlashCommand, Option<&str>)> {
    let s = input.trim();
    if !s.starts_with('/') {
        return None;
    }
    let rest = s[1..].trim_start();
    if rest.is_empty() {
        return None;
    }
    let mut parts = rest.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or("");
    if head.is_empty() {
        return None;
    }
    let arg = parts.next().map(str::trim).filter(|a| !a.is_empty());
    let cmd = command_by_name(head)?;
    if !cmd.takes_arg && arg.is_some() {
        return None;
    }
    Some((cmd, arg))
}

/// Commands whose primary name or an alias starts with `prefix` (case-insensitive, no `/`).
///
/// An empty `prefix` returns every command in [`COMMANDS`] order.
///
/// If nothing starts with `prefix` but `prefix` has at least two characters, also matches names
/// or aliases that **contain** `prefix` (helps typos like `/dit` → `audit`).
pub fn matching_commands(prefix: &str) -> Vec<&'static SlashCommand> {
    let p = prefix.to_lowercase();
    if p.is_empty() {
        return COMMANDS.iter().collect();
    }
    let mut v: Vec<&'static SlashCommand> = COMMANDS
        .iter()
        .filter(|c| {
            let n = c.name.to_lowercase();
            n.starts_with(&p) || c.aliases.iter().any(|a| a.to_lowercase().starts_with(&p))
        })
        .collect();
    if v.is_empty() && p.len() >= 2 {
        v = COMMANDS
            .iter()
            .filter(|c| {
                let n = c.name.to_lowercase();
                n.contains(&p) || c.aliases.iter().any(|a| a.to_lowercase().contains(&p))
            })
            .collect();
    }
    v
}

/// Returns the in-progress command token after a leading `/` (no slash in the result).
///
/// [`None`] means autocomplete should hide: the buffer does not start a slash command, the first
/// token is followed by non-space argument text, or the slash line is only whitespace after `/`.
///
/// Only the **first line** after `/` is considered, so multiline drafts like `/mo` + newline still
/// filter commands. Trailing spaces after a partial token (e.g. `/mo `) still yield a prefix for
/// completion.
pub fn slash_command_name_prefix(input_buffer: &str) -> Option<&str> {
    let s = input_buffer.trim_start().trim_start_matches(|c: char| {
        ['\u{feff}', '\u{200b}', '\u{200c}', '\u{200d}'].contains(&c)
    });
    if !s.starts_with('/') {
        return None;
    }
    let rest = &s[1..];
    if rest.is_empty() {
        return Some("");
    }
    if rest.chars().next().is_some_and(|c| c.is_whitespace()) {
        return None;
    }
    let first_line_end = rest.find('\n').unwrap_or(rest.len());
    if first_line_end < rest.len() {
        let after_first = rest[first_line_end + 1..].trim();
        if !after_first.is_empty() {
            return None;
        }
    }
    let line1 = &rest[..first_line_end];
    let end = line1.find(char::is_whitespace).unwrap_or(line1.len());
    let word = &line1[..end];
    let after_word = line1[end..].trim_start();
    if !after_word.is_empty() {
        return None;
    }
    Some(word)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_persist::load_session_summaries;

    #[test]
    fn parse_help() {
        let (c, a) = parse_slash_input("/help").expect("parse");
        assert_eq!(c.name, "help");
        assert!(a.is_none());
    }

    #[test]
    fn parse_model_with_arg() {
        let (c, a) = parse_slash_input("/model haiku").expect("parse");
        assert_eq!(c.name, "model");
        assert_eq!(a, Some("haiku"));
    }

    #[test]
    fn parse_unknown() {
        assert!(parse_slash_input("/unknown").is_none());
    }

    #[test]
    fn parse_not_slash() {
        assert!(parse_slash_input("not a slash").is_none());
    }

    #[test]
    fn match_h_prefix() {
        let m = matching_commands("h");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].name, "help");
    }

    #[test]
    fn match_empty_all() {
        assert_eq!(matching_commands("").len(), COMMANDS.len());
        assert!(COMMANDS.iter().any(|c| c.name == "import"));
        assert!(COMMANDS.iter().any(|c| c.name == "export"));
    }

    #[test]
    fn match_res_resume_and_reset() {
        let m = matching_commands("res");
        let names: Vec<_> = m.iter().map(|c| c.name).collect();
        assert!(names.contains(&"resume"));
        assert!(names.contains(&"reset"));
    }

    #[test]
    fn parse_reset_session() {
        let (c, a) = parse_slash_input("/reset").expect("parse");
        assert_eq!(c.name, "reset");
        assert!(a.is_none());
    }

    #[test]
    fn parse_new_scaffold_with_name() {
        let (c, a) = parse_slash_input("/new my-app").expect("parse");
        assert_eq!(c.name, "new");
        assert_eq!(a, Some("my-app"));
    }

    #[test]
    fn parse_models_alias_maps_to_model() {
        let (c, a) = parse_slash_input("/models").expect("parse");
        assert_eq!(c.name, "model");
        assert!(a.is_none());
    }

    #[test]
    fn slash_prefix_partial_then_newline_only_still_completes() {
        assert_eq!(slash_command_name_prefix("/mo\n"), Some("mo"));
    }

    #[test]
    fn slash_prefix_hides_when_second_line_has_text() {
        assert_eq!(slash_command_name_prefix("/mo\nmore"), None);
    }

    #[test]
    fn slash_prefix_trailing_spaces_after_token() {
        assert_eq!(slash_command_name_prefix("/mo  "), Some("mo"));
    }

    #[test]
    fn match_mo_includes_model() {
        let m = matching_commands("mo");
        let names: Vec<_> = m.iter().map(|c| c.name).collect();
        assert!(names.contains(&"model"));
    }

    #[test]
    fn match_two_char_substring_finds_audit() {
        let m = matching_commands("dit");
        let names: Vec<_> = m.iter().map(|c| c.name).collect();
        assert!(names.contains(&"audit"));
    }

    #[test]
    fn match_single_char_still_prefix_only() {
        let m = matching_commands("e");
        let names: Vec<_> = m.iter().map(|c| c.name).collect();
        assert!(names.contains(&"exit"));
        assert!(!names.contains(&"reset"));
    }

    #[test]
    fn load_session_summaries_two_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let older = r#"{"session_id":"11111111-1111-1111-1111-111111111111","project_root":"/p","model":"m","started_at":"2020-01-01T00:00:00+00:00","messages":[{"role":"user","content":"hi"}],"total_input_tokens":0,"total_cache_read_tokens":0,"total_output_tokens":0}"#;
        let newer = r#"{"session_id":"22222222-2222-2222-2222-222222222222","project_root":"/p","model":"m","started_at":"2024-01-01T00:00:00+00:00","messages":[{"role":"user","content":"yo"}],"total_input_tokens":1,"total_cache_read_tokens":0,"total_output_tokens":0}"#;
        std::fs::write(dir.path().join("a.json"), older).expect("write");
        std::fs::write(dir.path().join("b.json"), newer).expect("write");
        let s = load_session_summaries(dir.path());
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].session_id, "22222222-2222-2222-2222-222222222222");
        assert_eq!(s[1].session_id, "11111111-1111-1111-1111-111111111111");
    }

    #[test]
    fn load_session_summaries_empty_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(load_session_summaries(dir.path()).is_empty());
    }
}
