//! `akmon diff` — compare two journal sessions (Item 6.3).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use akmon_diff::{DiffEngine, DiffError, DiffReportV1, load_source_session_from_journal};
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

/// Truncation cap for human divergences (matches `REPLAY_HUMAN_DIVERGENCE_CAP` in `main.rs`).
const DIFF_HUMAN_DIVERGENCE_CAP: usize = 10;

/// Same journal resolution as `run_replay` (`--journal` or `default_journal_dir()`).
fn resolve_diff_journal_dir(args: &DiffArgs) -> Result<PathBuf, String> {
    match &args.journal {
        Some(path) => Ok(path.clone()),
        None => default_journal_dir().map_err(|e| e.to_string()),
    }
}

/// Maps [`DiffError`] to CLI exit codes: `2` usage (`InvalidSessionId`), `3` infra (all other variants).
#[must_use]
pub fn diff_error_exit_code(err: &DiffError) -> ExitCode {
    match err {
        DiffError::InvalidSessionId { .. } => ExitCode::from(2),
        DiffError::SourceSessionMissing { .. }
        | DiffError::SourceSessionLoadFailed { .. }
        | DiffError::SourcePreconditionViolated { .. }
        | DiffError::StoreAccessFailed { .. }
        | DiffError::InternalError { .. } => ExitCode::from(3),
    }
}

fn divergence_kind_label(kind: &akmon_diff::DiffDivergenceKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| format!("{kind:?}"))
}

fn render_diff_human_report(report: &DiffReportV1) -> String {
    let mut lines = vec![
        format!(
            "diff: comparing {} vs {}",
            report.session_a_id, report.session_b_id
        ),
        format!("  mode: {}", report.mode),
        format!("  events compared: {}", report.events_compared),
        format!("  session A events: {}", report.session_a_event_count),
        format!("  session B events: {}", report.session_b_event_count),
        format!("  divergence count: {}", report.divergence_count),
        format!("  matches: {}", if report.matches { "yes" } else { "no" }),
    ];

    if !report.matches {
        if let Some(sb) = &report.structural_break {
            lines.push("  structural break:".to_owned());
            lines.push(format!("    position: {}", sb.position));
            lines.push(format!("    expected: {}", sb.expected));
            lines.push(format!("    actual: {}", sb.actual));
        }

        if !report.divergences.is_empty() {
            lines.push("  divergences:".to_owned());
            let shown = report.divergences.len().min(DIFF_HUMAN_DIVERGENCE_CAP);
            for (idx, divergence) in report.divergences.iter().take(shown).enumerate() {
                let pos = divergence
                    .position
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "unknown".to_owned());
                let kind = divergence_kind_label(&divergence.kind);
                let field_suffix = divergence
                    .field
                    .as_deref()
                    .map(|f| format!(" (field: {f})"))
                    .unwrap_or_default();
                lines.push(format!(
                    "    [{}] position {pos}: {kind}{field_suffix}",
                    idx + 1
                ));
                lines.push(format!("          expected: {}", divergence.expected));
                lines.push(format!("          actual:   {}", divergence.actual));
                if let Some(reason) = &divergence.resolved_skip_reason {
                    lines.push(format!("          resolve skipped: {reason}"));
                } else if let Some(r) = &divergence.resolved {
                    lines.push("          resolved:".to_owned());
                    lines.push(format!("            a_size_bytes: {}", r.a_size_bytes));
                    lines.push(format!("            b_size_bytes: {}", r.b_size_bytes));
                    lines.push(format!("            bytes_match: {}", r.bytes_match));
                    if let Some(p) = &r.a_preview {
                        lines.push(format!("            a_preview: {p}"));
                    }
                    if let Some(p) = &r.b_preview {
                        lines.push(format!("            b_preview: {p}"));
                    }
                }
            }
            if report.divergences.len() > DIFF_HUMAN_DIVERGENCE_CAP {
                let remaining = report.divergences.len() - DIFF_HUMAN_DIVERGENCE_CAP;
                lines.push(format!(
                    "    ... (and {remaining} more; use --format json for full list)"
                ));
            }
        }
    }

    lines.join("\n")
}

fn print_diff_report(report: &DiffReportV1, format: DiffFormat) -> std::io::Result<()> {
    match format {
        DiffFormat::Human => {
            println!("{}", render_diff_human_report(report));
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

fn run_diff_engine(journal_dir: &Path, args: &DiffArgs) -> Result<DiffReportV1, DiffError> {
    let source_a = load_source_session_from_journal(journal_dir, args.session_a)?;
    let source_b = load_source_session_from_journal(journal_dir, args.session_b)?;
    let engine = DiffEngine::new(source_a, source_b)?;
    if args.resolve {
        engine.run_with_resolve_to_report()
    } else {
        engine.run_to_report()
    }
}

/// Load sessions, run comparison, print report. Exit `0` on match, `1` on diverge, `2` usage, `3` infra.
#[must_use]
pub fn run_diff(args: DiffArgs) -> ExitCode {
    let format = args.format;
    let journal_dir = match resolve_diff_journal_dir(&args) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("akmon: diff: cannot resolve journal directory: {err}");
            return ExitCode::from(3);
        }
    };

    match run_diff_engine(&journal_dir, &args) {
        Ok(report) => {
            if let Err(err) = print_diff_report(&report, format) {
                eprintln!("akmon: diff: failed to print report: {err}");
                return ExitCode::from(3);
            }
            if report.matches {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(err) => {
            eprintln!("akmon: diff: {err}");
            diff_error_exit_code(&err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DiffArgs, DiffFormat, diff_error_exit_code, render_diff_human_report};
    use crate::{Cli, Commands};
    use akmon_diff::{
        DiffComparison, DiffDivergence, DiffDivergenceKind, DiffError, DiffMode, DiffReportV1,
        ResolvedContent, StructuralBreak,
    };
    use clap::Parser;
    use std::path::PathBuf;
    use std::process::ExitCode;

    const SID_A: &str = "550e8400-e29b-41d4-a716-446655440000";
    const SID_B: &str = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";

    fn report_from_comparison(c: DiffComparison, a_n: usize, b_n: usize) -> DiffReportV1 {
        DiffReportV1::from_comparison(c, a_n, b_n)
    }

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

    #[test]
    fn t_format_diff_human_passing_report() {
        let mut c = DiffComparison::new(SID_A.into(), SID_B.into(), DiffMode::Default);
        c.events_compared = 3;
        let r = report_from_comparison(c, 3, 3);
        let s = render_diff_human_report(&r);
        assert!(r.matches);
        assert!(s.contains("matches: yes"));
        assert!(!s.contains("divergences:"));
        assert!(!s.contains("structural break:"));
    }

    #[test]
    fn t_format_diff_human_failing_with_divergences() {
        let mut c = DiffComparison::new(SID_A.into(), SID_B.into(), DiffMode::Default);
        c.events_compared = 1;
        c.divergences.push(DiffDivergence {
            position: Some(0),
            kind: DiffDivergenceKind::ContentReferenceDifference,
            field: Some("prompt_hash".into()),
            expected: "hash-a".into(),
            actual: "hash-b".into(),
            resolved: None,
            resolved_skip_reason: None,
        });
        let r = report_from_comparison(c, 2, 2);
        let s = render_diff_human_report(&r);
        assert!(!r.matches);
        assert!(s.contains("matches: no"));
        assert!(s.contains("divergences:"));
        assert!(s.contains("prompt_hash"));
        assert!(s.contains("expected: hash-a"));
        assert!(s.contains("actual:   hash-b"));
    }

    #[test]
    fn t_format_diff_human_structural_break() {
        let mut c = DiffComparison::new(SID_A.into(), SID_B.into(), DiffMode::Default);
        c.events_compared = 2;
        c.structural_break = Some(StructuralBreak {
            position: 2,
            expected: "AssistantTurn".into(),
            actual: "ToolCall".into(),
        });
        let r = report_from_comparison(c, 5, 5);
        let s = render_diff_human_report(&r);
        assert!(!r.matches);
        assert!(s.contains("structural break:"));
        assert!(s.contains("position: 2"));
        assert!(s.contains("expected: AssistantTurn"));
        assert!(s.contains("actual: ToolCall"));
    }

    #[test]
    fn t_format_diff_human_truncates_divergences() {
        let mut c = DiffComparison::new(SID_A.into(), SID_B.into(), DiffMode::Default);
        c.events_compared = 1;
        for i in 0..15 {
            c.divergences.push(DiffDivergence {
                position: Some(i),
                kind: DiffDivergenceKind::AssistantContentDifference,
                field: None,
                expected: format!("e{i}"),
                actual: format!("a{i}"),
                resolved: None,
                resolved_skip_reason: None,
            });
        }
        let r = report_from_comparison(c, 2, 2);
        let s = render_diff_human_report(&r);
        assert!(s.contains("[10]"));
        assert!(!s.contains("[11]"));
        assert!(s.contains("... (and 5 more; use --format json for full list)"));
    }

    #[test]
    fn t_format_diff_human_resolved_content() {
        let mut c = DiffComparison::new(SID_A.into(), SID_B.into(), DiffMode::Default);
        c.divergences.push(DiffDivergence {
            position: Some(0),
            kind: DiffDivergenceKind::ToolCallInputDifference,
            field: Some("args_hash".into()),
            expected: "h1".into(),
            actual: "h2".into(),
            resolved: Some(ResolvedContent {
                a_size_bytes: 10,
                b_size_bytes: 12,
                a_preview: Some("alpha".into()),
                b_preview: Some("beta".into()),
                bytes_match: false,
            }),
            resolved_skip_reason: None,
        });
        let r = report_from_comparison(c, 1, 1);
        let s = render_diff_human_report(&r);
        assert!(s.contains("resolved:"));
        assert!(s.contains("a_size_bytes: 10"));
        assert!(s.contains("b_size_bytes: 12"));
        assert!(s.contains("bytes_match: false"));
        assert!(s.contains("a_preview: alpha"));
        assert!(s.contains("b_preview: beta"));
    }

    #[test]
    fn t_format_diff_human_resolved_skip_reason() {
        let mut c = DiffComparison::new(SID_A.into(), SID_B.into(), DiffMode::Default);
        c.divergences.push(DiffDivergence {
            position: Some(0),
            kind: DiffDivergenceKind::ContentReferenceDifference,
            field: Some("x".into()),
            expected: "a".into(),
            actual: "b".into(),
            resolved: None,
            resolved_skip_reason: Some("exceeds 10 MiB cap".into()),
        });
        let r = report_from_comparison(c, 1, 1);
        let s = render_diff_human_report(&r);
        assert!(s.contains("resolve skipped: exceeds 10 MiB cap"));
        assert!(!s.contains("resolved:"));
    }

    #[test]
    fn t_format_diff_json_passing_round_trip() {
        let mut c = DiffComparison::new(SID_A.into(), SID_B.into(), DiffMode::Default);
        c.events_compared = 2;
        let r = report_from_comparison(c, 2, 2);
        let json = serde_json::to_string_pretty(&r).expect("serialize");
        let back: DiffReportV1 = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, r);
        let v: serde_json::Value = serde_json::from_str(&json).expect("value");
        assert_eq!(v["matches"], true);
    }

    #[test]
    fn t_format_diff_json_failing_shape() {
        let mut c = DiffComparison::new(SID_A.into(), SID_B.into(), DiffMode::Default);
        c.divergences.push(DiffDivergence {
            position: None,
            kind: DiffDivergenceKind::EventCountMismatch,
            field: None,
            expected: "3".into(),
            actual: "4".into(),
            resolved: None,
            resolved_skip_reason: None,
        });
        let r = report_from_comparison(c, 3, 4);
        let json = serde_json::to_string_pretty(&r).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&json).expect("value");
        assert_eq!(v["matches"], false);
        let arr = v["divergences"].as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert!(v.get("structural_break").is_some());
    }

    #[test]
    fn t_diff_error_exit_code_source_session_missing() {
        let e = DiffError::SourceSessionMissing {
            session_id: "x".into(),
        };
        assert_eq!(diff_error_exit_code(&e), ExitCode::from(3));
    }

    #[test]
    fn t_diff_error_exit_code_source_session_load_failed() {
        let e = DiffError::SourceSessionLoadFailed {
            session_id: "x".into(),
            source: std::io::Error::other("io").into(),
        };
        assert_eq!(diff_error_exit_code(&e), ExitCode::from(3));
    }

    #[test]
    fn t_diff_error_exit_code_source_precondition_violated() {
        let e = DiffError::SourcePreconditionViolated {
            session_label: "A".into(),
            violation: "empty".into(),
        };
        assert_eq!(diff_error_exit_code(&e), ExitCode::from(3));
    }

    #[test]
    fn t_diff_error_exit_code_store_access_failed() {
        let e = DiffError::StoreAccessFailed {
            source: std::io::Error::other("denied").into(),
        };
        assert_eq!(diff_error_exit_code(&e), ExitCode::from(3));
    }

    #[test]
    fn t_diff_error_exit_code_invalid_session_id() {
        let e = DiffError::InvalidSessionId {
            session_id: "bad".into(),
            reason: "nope".into(),
        };
        assert_eq!(diff_error_exit_code(&e), ExitCode::from(2));
    }

    #[test]
    fn t_diff_error_exit_code_internal_error() {
        let e = DiffError::InternalError {
            context: "ctx".into(),
            source: std::io::Error::other("inner").into(),
        };
        assert_eq!(diff_error_exit_code(&e), ExitCode::from(3));
    }
}
