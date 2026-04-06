//! `akmon init` / `akmon new` — project detection, scaffolding, and `AKMON.md` synthesis.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use akmon_core::project::{
    ProjectSummary, ProjectType, ScaffoldKind, ScaffoldLanguage, count_source_files_for_summary,
    detect_project, format_project_context_for_init, scaffold_project, suggested_akmon_title,
};
use akmon_core::scan_context_files;
use akmon_models::LlmProvider;
use akmon_query::generate_akmon_md_markdown;
use clap::Args;
use clap::ValueEnum;

use crate::Cli;
use crate::import_cmd::{ImportArgs, run_import};

/// Arguments for `akmon new`.
#[derive(Args, Debug, Clone)]
pub struct NewCmd {
    /// Directory name for the new project (created under the current working directory).
    pub name: String,
    /// Optional free-text description included in the `AKMON.md` prompt.
    pub description: Option<String>,
    /// Primary language scaffold.
    #[arg(long, value_enum)]
    pub lang: Option<NewLang>,
    /// Project shape (affects templates when paired with `--lang`).
    #[arg(long = "type", value_enum)]
    pub project_type: Option<NewKind>,
    /// Skip `git init` in the new directory.
    #[arg(long = "no-git", action = clap::ArgAction::SetTrue)]
    pub no_git: bool,
}

/// `--lang` values for `akmon new`.
#[derive(Clone, Copy, Debug, ValueEnum, Eq, PartialEq)]
pub enum NewLang {
    /// Rust (`cargo` layout).
    Rust,
    /// Node.js (`package.json`).
    Node,
    /// Python (`pyproject.toml` / `src`).
    Python,
    /// Go (`go.mod`).
    Go,
}

/// `--type` values for `akmon new`.
#[derive(Clone, Copy, Debug, ValueEnum, Eq, PartialEq)]
pub enum NewKind {
    /// Command-line binary.
    Cli,
    /// Library package.
    Lib,
    /// Web-style Node scaffold.
    Web,
    /// Python API (FastAPI) scaffold.
    Api,
}

impl From<NewLang> for ScaffoldLanguage {
    fn from(v: NewLang) -> Self {
        match v {
            NewLang::Rust => ScaffoldLanguage::Rust,
            NewLang::Node => ScaffoldLanguage::Node,
            NewLang::Python => ScaffoldLanguage::Python,
            NewLang::Go => ScaffoldLanguage::Go,
        }
    }
}

impl From<NewKind> for ScaffoldKind {
    fn from(v: NewKind) -> Self {
        match v {
            NewKind::Cli => ScaffoldKind::Cli,
            NewKind::Lib => ScaffoldKind::Lib,
            NewKind::Web => ScaffoldKind::Web,
            NewKind::Api => ScaffoldKind::Api,
        }
    }
}

fn load_global_config() -> akmon_config::AkmonGlobalConfig {
    akmon_config::load_user_config()
        .map(|(_, c)| c)
        .unwrap_or_default()
}

pub(crate) fn resolve_provider(cli: &Cli) -> Result<Arc<dyn LlmProvider>, String> {
    let global = load_global_config();
    crate::llm_connect_from_cli(cli, &global, cli.model.clone()).resolve()
}

fn init_headline(summary: &ProjectSummary) -> String {
    match &summary.project_type {
        ProjectType::Rust { name, .. } => format!("Rust project: {name}"),
        ProjectType::Node { name, .. } => format!("Node project: {name}"),
        ProjectType::Python { name, .. } => format!("Python project: {name}"),
        ProjectType::Go { name, .. } => format!("Go project: {name}"),
        ProjectType::Generic { .. } => "Unclassified project".to_string(),
    }
}

fn detection_cue(summary: &ProjectSummary) -> &'static str {
    match summary.project_type {
        ProjectType::Rust { .. } => "Cargo.toml",
        ProjectType::Node { .. } => "package.json",
        ProjectType::Python { .. } => "Python packaging",
        ProjectType::Go { .. } => "go.mod",
        ProjectType::Generic { .. } => "(no standard markers)",
    }
}

fn prompt_yes_no(question: &str) -> bool {
    use std::io::Write;
    let _ = std::io::stderr().flush();
    eprint!("{question}");
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(_) => line.trim().eq_ignore_ascii_case("y"),
        Err(_) => false,
    }
}

fn prompt_import_default_yes(question: &str) -> bool {
    use std::io::Write;
    let _ = std::io::stderr().flush();
    eprint!("{question}");
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(_) => {
            let t = line.trim();
            !t.eq_ignore_ascii_case("n")
        }
        Err(_) => true,
    }
}

/// Runs `akmon init` in `project_root` (typically the Git root or cwd).
pub async fn run_init(cli: &Cli, project_root: &Path) -> ExitCode {
    println!("Detecting project...");
    let summary = match detect_project(project_root) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("akmon: failed to inspect project: {e}");
            return ExitCode::from(2);
        }
    };

    let headline = init_headline(&summary);
    let cue = detection_cue(&summary);
    println!("✓ {headline} ({cue})");

    let n = count_source_files_for_summary(&summary);
    if n > 0 {
        println!("✓ Found {n} source files");
    }

    if project_root.join("README.md").is_file() {
        println!("✓ Found README.md");
    }

    let dest = project_root.join("AKMON.md");
    let scan = scan_context_files(project_root);
    if !scan.files.is_empty() {
        let mut labels: Vec<String> = scan
            .files
            .iter()
            .map(|f| f.tool.display_name().to_string())
            .collect();
        labels.sort();
        labels.dedup();
        let list = labels.join(", ");
        println!("Found context from: {list}");
        if prompt_import_default_yes("Import these? [Y/n] ") {
            if dest.is_file() && !cli.yes {
                let ok = prompt_yes_no("AKMON.md already exists. Overwrite? [y/N] ");
                if !ok {
                    eprintln!("akmon: cancelled.");
                    return ExitCode::from(3);
                }
            }
            let provider = match resolve_provider(cli) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("akmon: {e}");
                    return ExitCode::from(2);
                }
            };
            return match run_import(
                ImportArgs {
                    force: true,
                    dry_run: false,
                    from: None,
                },
                project_root.to_path_buf(),
                provider,
            )
            .await
            {
                Ok(()) => {
                    println!("\nRun akmon chat to start coding.");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("akmon: import: {e:#}");
                    ExitCode::from(1)
                }
            };
        }
    }

    if dest.is_file() && !cli.yes {
        let ok = prompt_yes_no("AKMON.md already exists. Overwrite? [y/N] ");
        if !ok {
            eprintln!("akmon: cancelled.");
            return ExitCode::from(3);
        }
    }

    let ctx = format_project_context_for_init(&summary);
    let title = suggested_akmon_title(&summary);
    let provider = match resolve_provider(cli) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("akmon: {e}");
            return ExitCode::from(2);
        }
    };

    println!(
        "Generating AKMON.md with {}...",
        short_model_label(&cli.model)
    );

    let body = match generate_akmon_md_markdown(&*provider, &ctx, None, &title).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("akmon: model error: {e}");
            return ExitCode::from(1);
        }
    };

    if let Err(e) = std::fs::write(&dest, body.as_bytes()) {
        eprintln!("akmon: failed to write {}: {e}", dest.display());
        return ExitCode::from(2);
    }

    println!("✓ AKMON.md created ({} bytes)", body.len());
    println!("\nRun akmon chat to start coding.");
    ExitCode::SUCCESS
}

fn short_model_label(model: &str) -> String {
    if model.to_lowercase().starts_with("claude")
        && let Some(rest) = model.strip_prefix("claude-")
    {
        return format!("{rest}…");
    }
    model.to_string()
}

fn map_scaffold_choice(
    lang: Option<NewLang>,
    kind: Option<NewKind>,
) -> (ScaffoldLanguage, ScaffoldKind) {
    match (lang, kind) {
        (Some(NewLang::Rust), Some(NewKind::Cli) | None) => {
            (ScaffoldLanguage::Rust, ScaffoldKind::Cli)
        }
        (Some(NewLang::Rust), Some(NewKind::Lib)) => (ScaffoldLanguage::Rust, ScaffoldKind::Lib),
        (Some(NewLang::Rust), Some(NewKind::Web | NewKind::Api)) => {
            (ScaffoldLanguage::Generic, ScaffoldKind::Generic)
        }

        (Some(NewLang::Node), Some(NewKind::Web) | None) => {
            (ScaffoldLanguage::Node, ScaffoldKind::Web)
        }
        (Some(NewLang::Node), _) => (ScaffoldLanguage::Generic, ScaffoldKind::Generic),

        (Some(NewLang::Python), Some(NewKind::Api) | None) => {
            (ScaffoldLanguage::Python, ScaffoldKind::Api)
        }
        (Some(NewLang::Python), _) => (ScaffoldLanguage::Generic, ScaffoldKind::Generic),

        (Some(NewLang::Go), Some(NewKind::Cli) | None) => (ScaffoldLanguage::Go, ScaffoldKind::Cli),
        (Some(NewLang::Go), _) => (ScaffoldLanguage::Generic, ScaffoldKind::Generic),

        (None, Some(NewKind::Lib)) => (ScaffoldLanguage::Rust, ScaffoldKind::Lib),
        (None, Some(NewKind::Web)) => (ScaffoldLanguage::Node, ScaffoldKind::Web),
        (None, Some(NewKind::Api)) => (ScaffoldLanguage::Python, ScaffoldKind::Api),
        (None, Some(NewKind::Cli)) | (None, None) => {
            (ScaffoldLanguage::Generic, ScaffoldKind::Generic)
        }
    }
}

/// Runs `akmon new` — creates `cwd/name`, scaffolds, optional `git init`, then writes `AKMON.md`.
pub async fn run_new(cli: &Cli, args: &NewCmd, cwd: &Path) -> ExitCode {
    if args.name.contains('/') || args.name.contains('\\') || args.name.is_empty() {
        eprintln!("akmon: project name must be a single path segment.");
        return ExitCode::from(2);
    }

    let dest_dir = cwd.join(&args.name);
    if dest_dir.exists() {
        eprintln!("akmon: destination already exists: {}", dest_dir.display());
        return ExitCode::from(2);
    }

    println!("Creating {}/", args.name);

    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        eprintln!("akmon: failed to create directory: {e}");
        return ExitCode::from(2);
    }

    let (slang, skind) = map_scaffold_choice(args.lang, args.project_type);
    let report = match scaffold_project(&dest_dir, &args.name, slang, skind) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("akmon: scaffold failed: {e}");
            return ExitCode::from(2);
        }
    };

    for f in &report.files_created {
        println!("✓ {f}");
    }

    if !args.no_git {
        match std::process::Command::new("git")
            .args(["init"])
            .current_dir(&dest_dir)
            .status()
        {
            Ok(s) if s.success() => println!("✓ git init"),
            Ok(_) => eprintln!("akmon: warning: git init returned non-zero status"),
            Err(e) => eprintln!("akmon: warning: could not run git init: {e}"),
        }
    }

    let summary = match detect_project(&dest_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("akmon: failed to re-detect project: {e}");
            return ExitCode::from(2);
        }
    };

    let ctx = format_project_context_for_init(&summary);
    let title = suggested_akmon_title(&summary);
    let provider = match resolve_provider(cli) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("akmon: {e}");
            return ExitCode::from(2);
        }
    };

    println!("\nGenerating AKMON.md...");
    let body =
        match generate_akmon_md_markdown(&*provider, &ctx, args.description.as_deref(), &title)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                eprintln!("akmon: model error: {e}");
                return ExitCode::from(1);
            }
        };

    let akmon_path = dest_dir.join("AKMON.md");
    if let Err(e) = std::fs::write(&akmon_path, body.as_bytes()) {
        eprintln!("akmon: failed to write AKMON.md: {e}");
        return ExitCode::from(2);
    }

    println!("✓ AKMON.md ({} bytes)", body.len());
    println!("\nReady. Run:");
    println!("  cd {}", args.name);
    println!("  akmon chat");
    ExitCode::SUCCESS
}

/// Resolves sandbox root: Git within five parent hops, else `cwd`. Returns `(root, has_git_root)`.
pub fn resolve_sandbox_root(cwd: &Path) -> (PathBuf, bool) {
    match crate::find_git_project_root(cwd) {
        Some(root) => (root, true),
        None => (cwd.to_path_buf(), false),
    }
}

/// When `false`, skip seeding `<root>/.akmon/*` so we do not treat `$HOME` as a project workspace.
pub(crate) fn should_ensure_project_dot_akmon(project_root: &Path, has_git_root: bool) -> bool {
    if has_git_root {
        return true;
    }
    let Ok(root) = dunce::canonicalize(project_root) else {
        return true;
    };
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return true;
    };
    let Ok(home) = dunce::canonicalize(home) else {
        return true;
    };
    root != home
}
