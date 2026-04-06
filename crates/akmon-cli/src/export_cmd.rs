//! `akmon export` — write `AKMON.md` into other tools’ context file locations.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use clap::Args;

/// CLI flags for `akmon export`.
#[derive(Args, Debug, Clone)]
pub struct ExportArgs {
    /// Write every supported export target.
    #[arg(long)]
    pub all: bool,
    /// Export only one format ([`ExportTarget::cli_name`]).
    #[arg(long, value_name = "TOOL")]
    pub tool: Option<String>,
    /// Print files that would be written without writing.
    #[arg(long)]
    pub dry_run: bool,
}

/// Destination layout for one `akmon export --tool` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportTarget {
    /// Write `CLAUDE.md` at the project root.
    ClaudeCode,
    /// Write `AGENTS.md` at the project root.
    Codex,
    /// Write `.cursor/rules/akmon.mdc` with YAML frontmatter.
    Cursor,
    /// Write `GEMINI.md` at the project root.
    Gemini,
    /// Write `.github/copilot-instructions.md`.
    Copilot,
    /// Write `.windsurfrules` at the project root.
    Windsurf,
    /// Write `.clinerules` at the project root.
    Cline,
    /// Write `.kiro/steering/akmon.md` with YAML frontmatter.
    Kiro,
}

impl ExportTarget {
    fn cli_name(&self) -> &'static str {
        match self {
            ExportTarget::ClaudeCode => "claude-code",
            ExportTarget::Codex => "codex",
            ExportTarget::Cursor => "cursor",
            ExportTarget::Gemini => "gemini",
            ExportTarget::Copilot => "copilot",
            ExportTarget::Windsurf => "windsurf",
            ExportTarget::Cline => "cline",
            ExportTarget::Kiro => "kiro",
        }
    }

    fn all_targets() -> &'static [ExportTarget] {
        &[
            ExportTarget::ClaudeCode,
            ExportTarget::Codex,
            ExportTarget::Cursor,
            ExportTarget::Gemini,
            ExportTarget::Copilot,
            ExportTarget::Windsurf,
            ExportTarget::Cline,
            ExportTarget::Kiro,
        ]
    }
}

fn parse_tool(s: &str) -> Option<ExportTarget> {
    let s = s.to_lowercase();
    for t in ExportTarget::all_targets() {
        if t.cli_name() == s {
            return Some(*t);
        }
    }
    None
}

fn comment_header(tool_cli: &str) -> String {
    format!(
        "<!-- Generated from AKMON.md by Akmon -->\n\
         <!-- Edit AKMON.md then re-run:         -->\n\
         <!-- akmon export --tool {tool_cli}     -->\n\n"
    )
}

fn body_for_target(target: ExportTarget, akmon_body: &str) -> String {
    let hdr = comment_header(target.cli_name());
    match target {
        ExportTarget::Cursor => format!(
            "{hdr}---\n\
             description: Project context from AKMON.md\n\
             alwaysApply: true\n\
             ---\n\n\
             {akmon_body}"
        ),
        ExportTarget::Kiro => format!(
            "{hdr}---\n\
             inclusion: always\n\
             ---\n\n\
             {akmon_body}"
        ),
        _ => format!("{hdr}{akmon_body}"),
    }
}

fn path_for_target(root: &Path, target: ExportTarget) -> PathBuf {
    match target {
        ExportTarget::ClaudeCode => root.join("CLAUDE.md"),
        ExportTarget::Codex => root.join("AGENTS.md"),
        ExportTarget::Cursor => root.join(".cursor/rules/akmon.mdc"),
        ExportTarget::Gemini => root.join("GEMINI.md"),
        ExportTarget::Copilot => root.join(".github/copilot-instructions.md"),
        ExportTarget::Windsurf => root.join(".windsurfrules"),
        ExportTarget::Cline => root.join(".clinerules"),
        ExportTarget::Kiro => root.join(".kiro/steering/akmon.md"),
    }
}

/// Reads `AKMON.md` and writes requested tool-specific copies under `project_root`.
pub fn run_export(args: ExportArgs, project_root: &Path) -> anyhow::Result<()> {
    let ak_path = project_root.join("AKMON.md");
    let akmon = std::fs::read_to_string(&ak_path).with_context(|| {
        format!(
            "AKMON.md not found at {} — run `akmon init` or `akmon import` first.",
            ak_path.display()
        )
    })?;

    let targets: Vec<ExportTarget> = if args.all {
        ExportTarget::all_targets().to_vec()
    } else if let Some(ref t) = args.tool {
        let Some(tg) = parse_tool(t) else {
            bail!("Unknown --tool {t}. Try: claude-code, codex, cursor, gemini, copilot, windsurf, cline, kiro");
        };
        vec![tg]
    } else {
        bail!("Specify --all or --tool <name> (see `akmon export --help`).");
    };

    for tg in targets {
        let path = path_for_target(project_root, tg);
        let text = body_for_target(tg, &akmon);
        if args.dry_run {
            println!("Would write {} ({}):", path.display(), tg.cli_name());
            println!("{text}");
            println!();
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create_dir_all {}", parent.display()))?;
        }
        std::fs::write(&path, text.as_str()).with_context(|| path.display().to_string())?;
        println!("  ✓ {}", path.display());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_cursor_frontmatter() {
        let body = body_for_target(ExportTarget::Cursor, "# Hi");
        assert!(body.contains("alwaysApply: true"));
        assert!(body.contains("description: Project context from AKMON.md"));
        assert!(body.contains("# Hi"));
    }

    #[test]
    fn export_kiro_frontmatter() {
        let body = body_for_target(ExportTarget::Kiro, "x");
        assert!(body.contains("inclusion: always"));
        assert!(body.contains('x'));
    }

    #[test]
    fn export_creates_nested_dirs() {
        let dir = tempfile::tempdir().expect("tmp");
        let root_copy = dir.path().join("sub/root");
        std::fs::create_dir_all(&root_copy).expect("mkdir");
        std::fs::write(root_copy.join("AKMON.md"), "# A").expect("ak");
        run_export(
            ExportArgs {
                all: false,
                tool: Some("cursor".into()),
                dry_run: false,
            },
            &root_copy,
        )
        .expect("export");
        let mdc = root_copy.join(".cursor/rules/akmon.mdc");
        assert!(mdc.is_file());
    }
}
