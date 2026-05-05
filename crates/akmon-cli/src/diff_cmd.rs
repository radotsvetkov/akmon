//! `akmon diff` — compare two journal sessions (Item 6.3).

use std::path::PathBuf;
use std::process::ExitCode;

use akmon_diff::{DiffEngine, DiffReportV1, load_source_session_from_journal};
use akmon_query::default_journal_dir;
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

/// Same journal resolution as `run_replay` (`--journal` or `default_journal_dir()`).
fn resolve_diff_journal_dir(args: &DiffArgs) -> Result<PathBuf, String> {
    match &args.journal {
        Some(path) => Ok(path.clone()),
        None => default_journal_dir().map_err(|e| e.to_string()),
    }
}

/// Layer 2: placeholder output (Layer 3 adds real human/JSON formatters).
fn print_diff_placeholder_report(report: &DiffReportV1, format: DiffFormat) -> std::io::Result<()> {
    match format {
        DiffFormat::Human => {
            println!("{report:#?}");
            Ok(())
        }
        DiffFormat::Json => {
            let s = serde_json::to_string_pretty(report)
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            println!("{s}");
            Ok(())
        }
    }
}

/// Load sessions, run comparison, print placeholder report. Exit `1` on mismatch or any error (Layer 3 refines codes).
#[must_use]
pub fn run_diff(args: DiffArgs) -> ExitCode {
    let format = args.format;
    let journal_dir = match resolve_diff_journal_dir(&args) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("akmon: diff: cannot resolve journal directory: {err}");
            return ExitCode::from(1);
        }
    };

    let source_a = match load_source_session_from_journal(&journal_dir, args.session_a) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("akmon: diff: {err:?}");
            return ExitCode::from(1);
        }
    };
    let source_b = match load_source_session_from_journal(&journal_dir, args.session_b) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("akmon: diff: {err:?}");
            return ExitCode::from(1);
        }
    };

    let engine = match DiffEngine::new(source_a, source_b) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("akmon: diff: {err:?}");
            return ExitCode::from(1);
        }
    };

    let report = match if args.resolve {
        engine.run_with_resolve_to_report()
    } else {
        engine.run_to_report()
    } {
        Ok(r) => r,
        Err(err) => {
            eprintln!("akmon: diff: {err:?}");
            return ExitCode::from(1);
        }
    };

    if let Err(err) = print_diff_placeholder_report(&report, format) {
        eprintln!("akmon: diff: failed to print report: {err}");
        return ExitCode::from(1);
    }

    if report.matches {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

#[cfg(test)]
mod tests {
    use super::{DiffArgs, DiffFormat};
    use crate::{Cli, Commands};
    use clap::Parser;
    use std::path::PathBuf;

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
    fn resolve_diff_journal_dir_uses_explicit_path() {
        let args = DiffArgs {
            session_a: uuid::Uuid::nil(),
            session_b: uuid::Uuid::nil(),
            journal: Some(PathBuf::from("/explicit/journal")),
            resolve: false,
            format: DiffFormat::Human,
        };
        assert_eq!(
            super::resolve_diff_journal_dir(&args).expect("ok"),
            PathBuf::from("/explicit/journal")
        );
    }
}
