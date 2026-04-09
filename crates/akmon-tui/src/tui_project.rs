//! Background `/init` and `/new` jobs (detection, scaffold, single-turn `AKMON.md` synthesis).

use std::sync::{Arc, Mutex};

use tokio::process::Command;

use akmon_core::project::{
    ScaffoldKind, ScaffoldLanguage, detect_project, format_project_context_for_init,
    project_type_label, scaffold_project, suggested_akmon_title,
};
use akmon_query::generate_akmon_md_markdown;

use crate::config::TuiLaunchConfig;

/// Work requested from the TUI slash handler and executed on the Tokio runtime.
#[derive(Debug, Clone)]
pub enum ProjectUiJob {
    /// Generate `AKMON.md` for the active [`TuiLaunchConfig::project_root`].
    Init,
    /// Create `project_root/<name>/` with a generic scaffold plus `AKMON.md`.
    New {
        /// Single path segment directory name.
        name: String,
    },
    /// Run `akmon import` in a subprocess with forwarded CLI flags.
    Import,
    /// Run `akmon export --all` in a subprocess with forwarded CLI flags.
    Export,
}

/// Runs one job; returns UI lines and whether to reload `AKMON.md` into session config.
pub async fn run_project_job(
    job: ProjectUiJob,
    shared: &Arc<Mutex<TuiLaunchConfig>>,
) -> (Vec<String>, bool) {
    let cfg = match shared.lock() {
        Ok(g) => g.clone(),
        Err(e) => e.into_inner().clone(),
    };

    match job {
        ProjectUiJob::Init => run_init_job(&cfg).await,
        ProjectUiJob::New { name } => run_new_job(&cfg, &name).await,
        ProjectUiJob::Import => run_import_export_subprocess(&cfg, &["import"]).await,
        ProjectUiJob::Export => run_import_export_subprocess(&cfg, &["export", "--all"]).await,
    }
}

fn forward_tui_launch_to_child(cmd: &mut Command, cfg: &TuiLaunchConfig) {
    cmd.arg("--model").arg(&cfg.model_name);
    if let Some(ref k) = cfg.anthropic_key {
        cmd.arg("--anthropic-key").arg(k);
    }
    if let Some(ref k) = cfg.openrouter_key {
        cmd.arg("--openrouter-key").arg(k);
    }
    if let Some(ref k) = cfg.openai_key {
        cmd.arg("--openai-key").arg(k);
    }
    if let Some(ref k) = cfg.groq_key {
        cmd.arg("--groq-key").arg(k);
    }
    if let Some(ref e) = cfg.azure_endpoint {
        cmd.arg("--azure-endpoint").arg(e);
    }
    if let Some(ref k) = cfg.azure_key {
        cmd.arg("--azure-key").arg(k);
    }
    cmd.arg("--azure-api-version").arg(&cfg.azure_api_version);
    if cfg.bedrock {
        cmd.arg("--bedrock");
    }
    cmd.arg("--aws-region").arg(&cfg.aws_region);
    if let Some(ref u) = cfg.openai_compatible_url {
        cmd.arg("--openai-compatible-url").arg(u);
    }
    if let Some(ref k) = cfg.openai_compatible_key {
        cmd.arg("--openai-compatible-key").arg(k);
    }
    cmd.arg("--ollama-url").arg(&cfg.ollama_url);
    cmd.arg("--yes");
    if cfg.web_fetch {
        cmd.arg("--web-fetch");
    }
    if cfg.yes_web {
        cmd.arg("--yes-web");
    }
    for p in &cfg.shell_allow {
        cmd.arg("--shell-allow").arg(p);
    }
    for u in &cfg.mcp_servers {
        cmd.arg("--mcp-server").arg(u);
    }
    if cfg.index_enabled {
        cmd.arg("--index");
    }
    if cfg.auto_commit {
        cmd.arg("--auto-commit");
    }
    cmd.arg("--output").arg("text");
    cmd.arg("--audit-log").arg(&cfg.audit_log_path);
    cmd.arg("--session").arg(cfg.session_id.to_string());
}

async fn run_import_export_subprocess(
    cfg: &TuiLaunchConfig,
    subargs: &[&str],
) -> (Vec<String>, bool) {
    let mut lines = Vec::new();
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            lines.push(format!("Cannot resolve current executable: {e}"));
            return (lines, false);
        }
    };
    let mut cmd = Command::new(exe);
    cmd.current_dir(&cfg.project_root);
    forward_tui_launch_to_child(&mut cmd, cfg);
    for a in subargs {
        cmd.arg(a);
    }
    match cmd.output().await {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            for line in stdout.lines() {
                lines.push(line.to_string());
            }
            for line in stderr.lines() {
                if !line.is_empty() {
                    lines.push(line.to_string());
                }
            }
            if !out.status.success() {
                lines.push(format!(
                    "Process exited with status {:?}",
                    out.status.code()
                ));
            }
            let reload_akmon = out.status.success() && subargs.first().copied() == Some("import");
            (lines, reload_akmon)
        }
        Err(e) => {
            lines.push(format!("Failed to spawn akmon subprocess: {e}"));
            (lines, false)
        }
    }
}

async fn run_init_job(cfg: &TuiLaunchConfig) -> (Vec<String>, bool) {
    let mut lines = Vec::new();
    let root = &cfg.project_root;
    let summary = match detect_project(root) {
        Ok(s) => s,
        Err(e) => {
            lines.push(format!("Project detection failed: {e}"));
            return (lines, false);
        }
    };

    let detected = project_type_label(&summary);
    lines.push(format!(
        "Detected: {detected} (markers at project root).",
    ));

    let ctx = format_project_context_for_init(&summary);
    let title = suggested_akmon_title(&summary);
    let provider = match cfg.llm_connect_for_model(cfg.model_name.clone()).resolve() {
        Ok(p) => p,
        Err(e) => {
            lines.push(format!("Model provider error: {e}"));
            return (lines, false);
        }
    };
    let body = match generate_akmon_md_markdown(&*provider, &ctx, None, &title).await {
        Ok(s) => s,
        Err(e) => {
            lines.push(format!("Model error while generating AKMON.md: {e}"));
            return (lines, false);
        }
    };

    let dest = root.join("AKMON.md");
    if let Err(e) = std::fs::write(&dest, body.as_bytes()) {
        lines.push(format!("Failed to write AKMON.md: {e}"));
        return (lines, false);
    }

    let n = body.len();
    lines.push(format!("AKMON.md written ({n} bytes)."));
    lines.push("AKMON.md created and loaded.".to_string());
    (lines, true)
}

async fn run_new_job(cfg: &TuiLaunchConfig, name: &str) -> (Vec<String>, bool) {
    let mut lines = Vec::new();
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        lines.push("Project name must be a single path segment.".to_string());
        return (lines, false);
    }

    let dest = cfg.project_root.join(name);
    if dest.exists() {
        let path = dest.display();
        lines.push(format!("Directory already exists: {path}"));
        return (lines, false);
    }

    if let Err(e) = std::fs::create_dir_all(&dest) {
        lines.push(format!("Could not create directory: {e}"));
        return (lines, false);
    }

    let report = match scaffold_project(
        &dest,
        name,
        ScaffoldLanguage::Generic,
        ScaffoldKind::Generic,
    ) {
        Ok(r) => r,
        Err(e) => {
            lines.push(format!("Scaffold failed: {e}"));
            return (lines, false);
        }
    };

    for f in &report.files_created {
        lines.push(format!("Created {f}"));
    }

    match std::process::Command::new("git")
        .args(["init"])
        .current_dir(&dest)
        .status()
    {
        Ok(s) if s.success() => lines.push("git init completed.".to_string()),
        Ok(_) => lines.push("git init returned non-zero (skipped or failed).".to_string()),
        Err(e) => lines.push(format!("git init not run: {e}")),
    }

    let summary = match detect_project(&dest) {
        Ok(s) => s,
        Err(e) => {
            lines.push(format!("Re-detect failed: {e}"));
            return (lines, false);
        }
    };

    let ctx = format_project_context_for_init(&summary);
    let title = suggested_akmon_title(&summary);
    let provider = match cfg.llm_connect_for_model(cfg.model_name.clone()).resolve() {
        Ok(p) => p,
        Err(e) => {
            lines.push(format!("Model provider error: {e}"));
            return (lines, false);
        }
    };
    let body = match generate_akmon_md_markdown(&*provider, &ctx, None, &title).await {
        Ok(s) => s,
        Err(e) => {
            lines.push(format!("Model error while generating AKMON.md: {e}"));
            return (lines, false);
        }
    };

    let akmon_path = dest.join("AKMON.md");
    if let Err(e) = std::fs::write(&akmon_path, body.as_bytes()) {
        lines.push(format!("Failed to write AKMON.md: {e}"));
        return (lines, false);
    }

    let n = body.len();
    lines.push(format!("AKMON.md in {name}/ ({n} bytes)."));
    lines.push(format!(
        "cd {name} and run akmon chat to start working in the new project."
    ));
    (lines, false)
}
