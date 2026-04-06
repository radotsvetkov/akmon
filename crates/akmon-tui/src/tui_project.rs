//! Background `/init` and `/new` jobs (detection, scaffold, single-turn `AKMON.md` synthesis).

use std::sync::{Arc, Mutex};

use akmon_core::project::{
    detect_project, format_project_context_for_init, project_type_label, scaffold_project,
    suggested_akmon_title, ScaffoldKind, ScaffoldLanguage,
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

    lines.push(format!(
        "Detected: {} (markers at project root).",
        project_type_label(&summary)
    ));

    let ctx = format_project_context_for_init(&summary);
    let title = suggested_akmon_title(&summary);
    let provider = match cfg.llm_connect_for_model(cfg.model_name.clone()).resolve() {
        Ok(p) => p,
        Err(msg) => {
            lines.push(format!("Model provider error: {msg}"));
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

    lines.push(format!("AKMON.md written ({} bytes).", body.len()));
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
        lines.push(format!("Directory already exists: {}", dest.display()));
        return (lines, false);
    }

    if let Err(e) = std::fs::create_dir_all(&dest) {
        lines.push(format!("Could not create directory: {e}"));
        return (lines, false);
    }

    let report = match scaffold_project(&dest, name, ScaffoldLanguage::Generic, ScaffoldKind::Generic)
    {
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
        Err(msg) => {
            lines.push(format!("Model provider error: {msg}"));
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

    lines.push(format!("AKMON.md in {name}/ ({} bytes).", body.len()));
    lines.push(format!(
        "cd {name} and run akmon chat to start working in the new project."
    ));
    (lines, false)
}
