//! `akmon import` — synthesize `AKMON.md` from other tools’ context files.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use akmon_core::{ContextFile, ContextScan, primary_tool_from_files, scan_context_files};
use akmon_models::{
    CompletionConfig, LlmProvider, Message, MessageRole, StreamEvent, max_tokens_for_model,
};
use anyhow::{Context as _, bail};
use clap::Args;
use futures_util::StreamExt;

/// CLI flags for `akmon import`.
#[derive(Args, Debug, Clone)]
pub struct ImportArgs {
    /// Overwrite an existing `AKMON.md`.
    #[arg(long)]
    pub force: bool,
    /// Print the synthesized markdown without writing.
    #[arg(long)]
    pub dry_run: bool,
    /// Restrict import to one tool ([`ToolOrigin::cli_name`] value), e.g. `claude-code`.
    #[arg(long, value_name = "TOOL")]
    pub from: Option<String>,
}

fn fmt_size_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else {
        format!("{:.1} KB", n as f64 / 1024.0)
    }
}

fn root_marker_missing_lines(root: &Path, scan: &ContextScan) -> Vec<String> {
    let checks: &[&str] = &[
        "AGENTS.md",
        "llms.txt",
        "CLAUDE.md",
        "GEMINI.md",
        ".cursorrules",
        ".windsurfrules",
        ".aider.conf.yml",
        ".clinerules",
        ".github/copilot-instructions.md",
    ];
    let found: std::collections::HashSet<String> =
        scan.files.iter().map(|f| f.path.clone()).collect();
    let mut out = Vec::new();
    for rel in checks {
        if found.contains(*rel) {
            continue;
        }
        if root.join(rel).is_file() {
            continue;
        }
        out.push(format!("  — {rel} not found"));
    }
    out
}

fn synthesis_user_prompt(files: &[ContextFile], project_root: &Path) -> String {
    let project_name = project_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project");
    let mut s = String::new();
    s.push_str(&format!("Project directory: {}\n", project_root.display()));
    s.push_str(&format!(
        "Project name for the `#` title: {project_name}\n\n"
    ));
    for cf in files {
        s.push_str(&format!(
            "=== {}: {} ===\n{}\n=== END ===\n\n",
            cf.tool.display_name(),
            cf.path,
            cf.content
        ));
    }
    s
}

/// Runs `akmon import`: scan, optional filter, one LLM completion, then write or print `AKMON.md`.
pub async fn run_import(
    args: ImportArgs,
    project_root: PathBuf,
    provider: Arc<dyn LlmProvider>,
) -> anyhow::Result<()> {
    println!("Scanning for AI tool context files...");
    let scan = scan_context_files(&project_root);

    let mut files: Vec<ContextFile> = scan.files.clone();
    if let Some(ref from) = args.from {
        let needle = from.to_lowercase();
        files.retain(|cf| cf.tool.cli_name().eq_ignore_ascii_case(&needle));
        if files.is_empty() {
            bail!("No context files match --from {needle}");
        }
    }

    let mut list = files.clone();
    list.sort_by(|a, b| a.path.cmp(&b.path));
    for cf in &list {
        println!(
            "  ✓ {}  ({}, {})",
            cf.path,
            cf.tool.display_name(),
            fmt_size_bytes(cf.size_bytes)
        );
    }
    for line in root_marker_missing_lines(&project_root, &scan) {
        println!("{line}");
    }
    println!();

    let tool_ids: std::collections::HashSet<_> = files.iter().map(|f| f.tool).collect();
    let n_tools = tool_ids.len();
    println!(
        "Found {} context file(s) from {} tool(s).",
        files.len(),
        n_tools
    );
    if let Some(pt) = primary_tool_from_files(&files) {
        println!("Primary tool detected: {}", pt.display_name());
    }
    println!();

    if files.is_empty() {
        println!(
            "No context files found.\n\
             Run 'akmon init' to analyze the project and generate AKMON.md."
        );
        return Ok(());
    }

    if scan.has_akmon_md && !args.force && !args.dry_run {
        println!(
            "AKMON.md already exists.\n\
             Use --force to overwrite.\n\
             Use --dry-run to preview."
        );
        return Ok(());
    }

    let user_content = synthesis_user_prompt(&files, &project_root);
    let project_name = project_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project");
    let system = format!(
        "Generate AKMON.md by synthesizing the provided context files.\n\n\
         Required sections:\n\
         # {project_name}\n\
         ## Product\n\
         ## Architecture\n\
         ## Tech stack\n\
         ## Conventions\n\
         ## Current sprint\n\
         ## Done\n\n\
         Rules: merge duplicates, preserve all conventions, never invent content, \
         under 200 lines, output only markdown."
    );

    let messages = vec![
        Message {
            role: MessageRole::System,
            content: system,
        },
        Message {
            role: MessageRole::User,
            content: user_content,
        },
    ];

    let config = CompletionConfig {
        max_tokens: max_tokens_for_model(provider.completion_model_id()),
        tools: Vec::new(),
        ..CompletionConfig::default()
    };

    let mut stream = provider
        .complete(&messages, &config)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let mut body = String::new();
    while let Some(item) = stream.next().await {
        let ev = item.map_err(|e| anyhow::anyhow!("{e}"))?;
        match ev {
            StreamEvent::TextDelta { text } => body.push_str(&text),
            StreamEvent::Done { .. } => break,
            StreamEvent::UsageReport(_) => {}
            StreamEvent::ProviderReady { .. } => {}
            StreamEvent::StatusHint { .. } => {}
            StreamEvent::Error { error } => bail!("model error: {error}"),
        }
    }

    let dest = project_root.join("AKMON.md");
    if args.dry_run {
        println!("{body}");
        return Ok(());
    }

    std::fs::write(&dest, body.as_str()).with_context(|| dest.display().to_string())?;
    println!("Wrote {}.", dest.display());
    for cf in &files {
        println!("  ✓ included {}", cf.path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_size_small() {
        assert_eq!(fmt_size_bytes(100), "100 B");
    }
}
