//! `akmon diff` — compare two journal sessions (Item 6.3).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, ValueEnum};
use uuid::Uuid;

/// Human vs JSON output for `akmon diff` (aligned with `replay` and `inspect` format flags).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum DiffFormat {
    /// Human-readable summary (default).
    #[default]
    Human,
    /// Emit `DiffReportV1` as JSON.
    Json,
}

/// Arguments for `akmon diff <SESSION_A> <SESSION_B>`.
#[derive(Args, Debug, Clone)]
pub struct DiffArgs {
    /// First session UUID.
    #[arg(value_name = "SESSION_A")]
    pub session_a: Uuid,
    /// Second session UUID.
    #[arg(value_name = "SESSION_B")]
    pub session_b: Uuid,
    /// Journal directory. Defaults to per-user journal (`$XDG_STATE_HOME/akmon/journal`).
    #[arg(long, value_name = "PATH")]
    pub journal: Option<PathBuf>,
    /// Dereference object hashes for byte-level field comparison (mirrors `inspect --resolve`).
    #[arg(long)]
    pub resolve: bool,
    #[arg(long, value_enum, default_value_t = DiffFormat::Human)]
    pub format: DiffFormat,
}

/// Layer 1 placeholder until engine wiring (Item 6.3 layer 2).
#[must_use]
pub fn run_diff(_args: DiffArgs) -> ExitCode {
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::{DiffArgs, DiffFormat};
    use crate::{Cli, Commands};
    use clap::Parser;
    use std::path::PathBuf;
    use std::process::ExitCode;
    use uuid::Uuid;

    const SID_A: &str = "550e8400-e29b-41d4-a716-446655440000";
    const SID_B: &str = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";

    #[test]
    fn t_diff_subcommand_parses_session_ids() {
        let cli = Cli::try_parse_from(["akmon", "diff", SID_A, SID_B]).expect("parse diff");
        match cli.command {
            Some(Commands::Diff(args)) => {
                assert_eq!(args.session_a.to_string(), SID_A);
                assert_eq!(args.session_b.to_string(), SID_B);
                assert!(args.journal.is_none());
                assert!(!args.resolve);
                assert_eq!(args.format, DiffFormat::Human);
            }
            other => panic!("expected diff command, got {other:?}"),
        }
    }

    #[test]
    fn t_diff_subcommand_parses_resolve_flag() {
        let cli = Cli::try_parse_from(["akmon", "diff", SID_A, SID_B, "--resolve"])
            .expect("parse resolve");
        match cli.command {
            Some(Commands::Diff(args)) => assert!(args.resolve),
            other => panic!("expected diff command, got {other:?}"),
        }
    }

    #[test]
    fn t_diff_subcommand_parses_journal_flag() {
        let cli = Cli::try_parse_from([
            "akmon",
            "diff",
            SID_A,
            SID_B,
            "--journal",
            "/tmp/journal-dir",
        ])
        .expect("parse journal");
        match cli.command {
            Some(Commands::Diff(args)) => {
                assert_eq!(args.journal, Some(PathBuf::from("/tmp/journal-dir")));
            }
            other => panic!("expected diff command, got {other:?}"),
        }
    }

    #[test]
    fn t_diff_subcommand_parses_format_json() {
        let cli = Cli::try_parse_from(["akmon", "diff", SID_A, SID_B, "--format", "json"])
            .expect("parse");
        match cli.command {
            Some(Commands::Diff(args)) => assert_eq!(args.format, DiffFormat::Json),
            other => panic!("expected diff command, got {other:?}"),
        }
    }

    #[test]
    fn t_diff_subcommand_rejects_invalid_uuid_a() {
        let err =
            Cli::try_parse_from(["akmon", "diff", "not-a-uuid", SID_B]).expect_err("must fail");
        let rendered = err.to_string();
        assert!(
            rendered.contains("invalid value")
                || rendered.contains("invalid character")
                || rendered.contains("UUID"),
            "unexpected clap error: {rendered}"
        );
    }

    #[test]
    fn t_diff_subcommand_rejects_invalid_uuid_b() {
        let err =
            Cli::try_parse_from(["akmon", "diff", SID_A, "not-a-uuid"]).expect_err("must fail");
        let rendered = err.to_string();
        assert!(
            rendered.contains("invalid value")
                || rendered.contains("invalid character")
                || rendered.contains("UUID"),
            "unexpected clap error: {rendered}"
        );
    }

    #[test]
    fn t_diff_subcommand_rejects_invalid_format() {
        let err = Cli::try_parse_from(["akmon", "diff", SID_A, SID_B, "--format", "yaml"])
            .expect_err("must fail");
        let rendered = err.to_string();
        assert!(
            rendered.contains("invalid value") || rendered.contains("possible values"),
            "unexpected clap error: {rendered}"
        );
    }

    #[test]
    fn t_diff_subcommand_rejects_missing_session_b() {
        let err = Cli::try_parse_from(["akmon", "diff", SID_A]).expect_err("must fail");
        let rendered = err.to_string();
        assert!(
            rendered.contains("SESSION_B") || rendered.contains("required"),
            "unexpected clap error: {rendered}"
        );
    }

    #[test]
    fn run_diff_stub_succeeds() {
        let args = DiffArgs {
            session_a: Uuid::parse_str(SID_A).expect("uuid"),
            session_b: Uuid::parse_str(SID_B).expect("uuid"),
            journal: None,
            resolve: false,
            format: DiffFormat::Human,
        };
        assert_eq!(super::run_diff(args), ExitCode::SUCCESS);
    }
}
