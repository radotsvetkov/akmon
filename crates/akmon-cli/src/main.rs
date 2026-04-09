//! Akmon CLI — project discovery, optional `AKMON.md`, and headless `--task` runs.

mod cli_forward;
mod cli_project;
mod config_cmd;
mod export_cmd;
mod import_cmd;
mod session_index;
mod session_transcript;
mod spec_cmd;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
#[cfg(feature = "semantic-index")]
use std::time::Duration;

use akmon_config::AkmonGlobalConfig;
use akmon_core::{
    AgentConfig, AgentError, AgentEvent, AuditEvent, InteractivePolicyReply, McpServerConfig,
    PolicyEngine, PolicyEngineMode, PolicyVerdict, Sandbox, write_audit_jsonl,
};
use akmon_models::{LlmConnectConfig, LlmProvider, Message, MessageRole, ProviderError};
use akmon_query::{
    AgentSession, SessionRunExit, SpawnSubagentTool, SubagentRuntime, SubagentToolFactory,
    ToolCallSummary, write_handoff_file,
};
#[cfg(feature = "semantic-index")]
use akmon_tools::SemanticSearchTool;
use akmon_tools::{
    ApplyPatchTool, AskFollowupTool, EditTool, GitTool, ListDirectoryTool, MemoryWriteTool,
    PatchTool, ReadFileTool, ReadSpecTool, SearchTool, ShellTool, TodoWriteTool, WebFetchTool,
    WriteFileTool, WriteSpecTool, discover_mcp_tools,
};
use akmon_tui::TuiLaunchConfig;
use clap::{Parser, Subcommand, ValueEnum};
#[cfg(feature = "semantic-index")]
use fastembed::{TextEmbedding, TextInitOptions};
use serde::Serialize;
use serde_json::json;
#[cfg(feature = "semantic-index")]
use tokio::sync::RwLock;
use tokio::sync::mpsc;

/// Builds a semantic index in the background and writes `index_path`, then fills `slot`.
#[cfg(feature = "semantic-index")]
async fn semantic_index_background_build(
    project_root: PathBuf,
    embedder: Arc<std::sync::Mutex<TextEmbedding>>,
    sandbox: Arc<Sandbox>,
    index_path: PathBuf,
    slot: Arc<RwLock<Option<akmon_index::RepoIndex>>>,
) {
    let indexer = akmon_index::Indexer::default();
    match indexer
        .build_index(&project_root, embedder, sandbox.as_ref())
        .await
    {
        Ok(idx) => {
            match index_path.parent() {
                Some(parent) => {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        eprintln!("akmon: failed to create .akmon dir: {e}");
                    }
                }
                None => {
                    eprintln!(
                        "akmon: index save path has no parent (unexpected): {}",
                        index_path.display()
                    );
                }
            }
            match akmon_index::save_index(&idx, &index_path) {
                Ok(()) => {
                    eprintln!(
                        "akmon: index saved to .akmon/index.bin ({} files, {} chunks)",
                        idx.file_count, idx.chunk_count
                    );
                }
                Err(e) => eprintln!("akmon: index save FAILED: {e}"),
            }
            *slot.write().await = Some(idx);
        }
        Err(e) => {
            eprintln!("akmon: warning: semantic index build failed: {e}");
        }
    }
}

/// Runs [`semantic_index_background_build`] on a dedicated OS thread with its own current-thread
/// Tokio runtime so index work is not cancelled when `#[tokio::main]` shuts down.
///
/// The caller should [`std::thread::JoinHandle::join`] the handle before process exit so
/// `index.bin` can be written reliably.
#[cfg(feature = "semantic-index")]
fn spawn_semantic_index_os_thread(
    project_root: PathBuf,
    embedder: Arc<std::sync::Mutex<TextEmbedding>>,
    sandbox: Arc<Sandbox>,
    index_path: PathBuf,
    slot: Arc<RwLock<Option<akmon_index::RepoIndex>>>,
) -> Option<std::thread::JoinHandle<()>> {
    match std::thread::Builder::new()
        .name("akmon-index-build".into())
        .spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                eprintln!("akmon: failed to build index runtime");
                return;
            };
            rt.block_on(async move {
                semantic_index_background_build(project_root, embedder, sandbox, index_path, slot)
                    .await;
            });
        }) {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("akmon: failed to spawn index thread: {e}");
            None
        }
    }
}

/// Gives a short background build time to finish so the first model turn may see an index.
#[cfg(feature = "semantic-index")]
async fn poll_index_ready_up_to_3s(slot: &Arc<RwLock<Option<akmon_index::RepoIndex>>>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if slot.read().await.is_some() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Returns true if `path` is an existing `.git` directory or gitdir file (worktrees).
fn git_working_tree_marker_present(git_path: &Path) -> bool {
    match std::fs::symlink_metadata(git_path) {
        Ok(m) => m.is_dir() || m.is_file(),
        Err(_) => false,
    }
}

/// At most this many directories are checked when walking upward for `.git` (current dir first).
const GIT_ROOT_MAX_DIR_CHECKS: usize = 5;

/// Walks upward from `root` for at most [`GIT_ROOT_MAX_DIR_CHECKS`] directories.
///
/// If the environment variable `AKMON_DEBUG_GIT` is set (any value), prints each directory checked
/// and whether a `.git` marker was found to stderr (for troubleshooting discovery).
fn walk_up_for_git_limited(mut root: PathBuf, max_dir_checks: usize) -> Option<PathBuf> {
    let debug_git = std::env::var_os("AKMON_DEBUG_GIT").is_some();
    for _ in 0..max_dir_checks {
        let git_path = root.join(".git");
        let found = git_working_tree_marker_present(&git_path);
        if debug_git {
            eprintln!(
                "akmon: debug git: dir={} git_marker_present={} .git_path={}",
                root.display(),
                found,
                git_path.display()
            );
        }
        if found {
            return Some(root);
        }
        root = root.parent()?.to_path_buf();
    }
    None
}

fn push_git_walk_start(candidates: &mut Vec<PathBuf>, p: PathBuf) {
    if !candidates.iter().any(|c| c == &p) {
        candidates.push(p);
    }
}

/// Walks upward from `start` looking for a `.git` file or directory, at most
/// [`GIT_ROOT_MAX_DIR_CHECKS`] levels per candidate start path. Returns the directory that
/// contains `.git`, or [`None`] if none is found within that depth.
///
/// Tries several starting paths (`dunce::canonicalize`, [`Path::canonicalize`], and the logical
/// absolute path from [`std::env::current_dir`]) so a repo is found when only one representation
/// resolves correctly.
pub(crate) fn find_git_project_root(start: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(c) = dunce::canonicalize(start) {
        push_git_walk_start(&mut candidates, c);
    }

    if let Ok(c) = start.canonicalize() {
        push_git_walk_start(&mut candidates, c);
    }

    let logical = if start.is_absolute() {
        start.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(start),
            Err(_) => start.to_path_buf(),
        }
    };
    push_git_walk_start(&mut candidates, logical);

    for root in candidates {
        if let Some(found) = walk_up_for_git_limited(root, GIT_ROOT_MAX_DIR_CHECKS) {
            return Some(found);
        }
    }
    None
}

/// Reads `AKMON.md` from `project_root` when the file exists.
///
/// Warns when the file is large: it is reinjected on every model call, so oversized files can
/// dominate input cost despite prompt caching.
const AKMON_MD_MAX_CHARS: usize = 2000;

fn load_akmon_md(project_root: &Path) -> std::io::Result<Option<String>> {
    let path = project_root.join("AKMON.md");
    if path.is_file() {
        let content = std::fs::read_to_string(path)?;
        if content.len() > AKMON_MD_MAX_CHARS {
            tracing::warn!(
                akmon_md_chars = content.len(),
                akmon_md_tokens_estimate = content.len() / 4,
                "AKMON.md is large; files over {} chars (~500+ tokens) add more cost than they save — consider trimming",
                AKMON_MD_MAX_CHARS
            );
        }
        Ok(Some(content))
    } else {
        Ok(None)
    }
}

/// How human-readable versus machine-readable the process should be on stdout.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum OutputFormat {
    /// Stream assistant text and tool progress to the terminal (default).
    #[default]
    Text,
    /// Suppress streaming output; emit a single JSON object on stdout when the run finishes.
    Json,
}

/// Aggregated token counters for `--output json` (Anthropic cache fields are zero when unused).
#[derive(Debug, Serialize)]
struct RunUsageSummary {
    /// Sum of provider `input_tokens` across completions in this run.
    total_input_tokens: u32,
    /// Sum of `cache_read_input_tokens` (prompt-cache hits) when the backend reports them.
    total_cache_read_tokens: u32,
    /// Sum of `output_tokens` across completions in this run.
    total_output_tokens: u32,
}

/// Why a headless run stopped (JSON `--output json`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)] // Interrupted reserved for future signal handling
enum ExitReason {
    Completed,
    MaxTurns,
    BudgetLimit,
    Error,
    Interrupted,
}

/// Resolved run summary for `--output json` (stable field names for automation).
#[derive(Debug, Serialize)]
struct RunReport {
    /// Session identifier ([`AgentConfig::session_id`] as hyphenated `Uuid`).
    session_id: String,
    /// `"success"` or `"error"`.
    status: &'static str,
    /// Machine-readable stop reason (headless integrations).
    exit_reason: ExitReason,
    /// Full assistant reply text accumulated from stream deltas.
    result: String,
    /// Tool completions in chronological order.
    tool_calls: Vec<ToolCallSummary>,
    /// Present only when `status` is `"error"`; JSON `null` on success.
    error: Option<String>,
    /// Path passed to [`write_audit_jsonl`] for this run (default or `--audit-log`).
    audit_log_path: String,
    /// Token totals for this run (Ollama typically leaves cache fields at zero).
    usage: RunUsageSummary,
    /// Estimated cumulative USD (heuristic; zero when unknown or local).
    cost_usd: f64,
    /// Prompt-cache read tokens (duplicate of `usage` for CI consumers).
    cache_read_tokens: u64,
    /// Sandbox-relative paths touched by successful writes/edits this run.
    files_written: Vec<String>,
}

/// Single JSON object on stdout for `--output json` when exiting before any agent session exists.
fn print_json_early_error_and_exit(error: String) -> ! {
    let error_report = json!({
        "session_id": "",
        "status": "error",
        "result": "",
        "tool_calls": [],
        "error": error,
        "exit_reason": "error",
        "cost_usd": 0.0,
        "files_written": [],
        "cache_read_tokens": 0,
    });
    println!("{}", error_report);
    std::process::exit(2);
}

/// Configuration / setup failure: stderr in text mode, JSON on stdout when `--output json`.
fn exit_early_config_error(
    cli: &Cli,
    error: String,
    index_thread: Option<&mut Option<std::thread::JoinHandle<()>>>,
    text_exit_code: i32,
) -> ! {
    if let Some(slot) = index_thread
        && let Some(h) = slot.take()
    {
        eprintln!("akmon: waiting for index to finish building...");
        let _ = h.join();
    }
    if cli.output == OutputFormat::Json {
        print_json_early_error_and_exit(error);
    }
    eprintln!("akmon: {error}");
    std::process::exit(text_exit_code);
}

/// [`resolve_resume_session_id`] failure (text mode prints two lines).
fn exit_resume_session_error(cli: &Cli, e: String) -> ! {
    if cli.output == OutputFormat::Json {
        print_json_early_error_and_exit(format!("{e}\nStart a new session: akmon"));
    }
    eprintln!("akmon: {e}");
    eprintln!("Start a new session: akmon");
    std::process::exit(1);
}

/// Command-line interface for the Akmon agent binary.
#[derive(Parser, Debug)]
#[command(
    name = "akmon",
    version,
    about = "Local-first AI coding agent. Runs with Ollama (local) or Anthropic API. All actions are audited."
)]
pub(crate) struct Cli {
    /// Optional subcommand (`chat` is equivalent to omitting `--task`).
    #[command(subcommand)]
    command: Option<Commands>,
    /// Task string for a headless agent run. When omitted, Akmon opens the interactive TUI.
    #[arg(short = 't', long = "task", global = true)]
    task: Option<String>,
    /// Model name: Ollama tag (e.g. llama3.2), or a Claude id if `ANTHROPIC_API_KEY` / `--anthropic-key` is set.
    #[arg(long = "model", default_value = "llama3.2", global = true)]
    model: String,
    /// Anthropic API key; defaults to the `ANTHROPIC_API_KEY` environment variable when unset.
    #[arg(
        long = "anthropic-key",
        env = "ANTHROPIC_API_KEY",
        hide_env_values = true,
        global = true,
        help = "Anthropic API key (falls back to ANTHROPIC_API_KEY env var)"
    )]
    anthropic_key: Option<String>,
    /// OpenRouter API key (`OPENROUTER_API_KEY`).
    #[arg(
        long = "openrouter-key",
        env = "OPENROUTER_API_KEY",
        hide_env_values = true,
        global = true
    )]
    openrouter_key: Option<String>,
    /// OpenAI API key (`OPENAI_API_KEY`).
    #[arg(
        long = "openai-key",
        env = "OPENAI_API_KEY",
        hide_env_values = true,
        global = true
    )]
    openai_key: Option<String>,
    /// Groq API key (`GROQ_API_KEY`).
    #[arg(
        long = "groq-key",
        env = "GROQ_API_KEY",
        hide_env_values = true,
        global = true
    )]
    groq_key: Option<String>,
    /// Azure OpenAI deployment URL (…/deployments/NAME/chat/completions).
    #[arg(long = "azure-endpoint", env = "AZURE_OPENAI_ENDPOINT", global = true)]
    azure_endpoint: Option<String>,
    #[arg(
        long = "azure-key",
        env = "AZURE_OPENAI_API_KEY",
        hide_env_values = true,
        global = true
    )]
    azure_key: Option<String>,
    /// Azure `api-version` query parameter (default `2024-02-01`).
    #[arg(
        long = "azure-api-version",
        default_value = "2024-02-01",
        global = true
    )]
    azure_api_version: String,
    /// Use Amazon Bedrock (reads `AWS_*` credentials from the environment).
    #[arg(long = "bedrock", global = true)]
    bedrock: bool,
    /// AWS region for Bedrock.
    #[arg(
        long = "aws-region",
        env = "AWS_DEFAULT_REGION",
        default_value = "us-east-1",
        global = true
    )]
    aws_region: String,
    /// Custom OpenAI-compatible API base (no `/chat/completions` suffix).
    #[arg(long = "openai-compatible-url", global = true)]
    openai_compatible_url: Option<String>,
    #[arg(long = "openai-compatible-key", hide_env_values = true, global = true)]
    openai_compatible_key: Option<String>,
    /// Base URL for the Ollama HTTP API (ignored when using Anthropic).
    #[arg(
        long = "ollama-url",
        default_value = "http://localhost:11434",
        global = true
    )]
    ollama_url: String,
    /// Auto-approve read-only tools only; writes and `shell` still require confirmation.
    #[arg(short = 'y', long = "yes", global = true)]
    yes: bool,
    /// `text`: stream tokens to the terminal; `json`: print one session summary object at the end.
    #[arg(
        long = "output",
        value_name = "FORMAT",
        default_value = "text",
        value_enum
    )]
    output: OutputFormat,
    /// JSON Lines audit file path (default: `<project>/.akmon/audit/<session_id>.jsonl`).
    #[arg(long = "audit-log", value_name = "PATH")]
    audit_log: Option<PathBuf>,
    /// Glob pattern allowed for the `shell` tool (argv-style commands only). Repeat for multiple patterns.
    #[arg(long = "shell-allow", value_name = "PATTERN")]
    shell_allow: Vec<String>,
    /// Enable the `web_fetch` tool. Disabled by default. When enabled, the agent can fetch public URLs. Internal and private network addresses are always blocked.
    #[arg(long = "web-fetch")]
    web_fetch: bool,
    /// Auto-approve `web_fetch` requests to public URLs (use with `--web-fetch` and `--yes`). SSRF protection still applies. `WriteFile` and `shell` still require confirmation.
    #[arg(long = "yes-web")]
    yes_web: bool,
    /// Connect to an MCP server at this URL and register all tools it lists (repeatable for multiple servers).
    #[arg(long = "mcp-server", value_name = "URL")]
    mcp_server: Vec<String>,
    /// Build or load a semantic index of the project; registers the `semantic_search` tool. Cache: `.akmon/index.bin`.
    #[arg(long = "index", global = true)]
    index: bool,
    /// After each successful `edit` or `write_file`, run `git add` and `git commit` with an auto-generated message (requires a git repo).
    #[arg(long = "auto-commit", global = true)]
    auto_commit: bool,
    /// Analyze the codebase and produce a written plan. The agent uses read-only tools only; the plan is saved under `.akmon/plans/`. Run again without `--plan` to implement.
    #[arg(long = "plan", global = true)]
    plan: bool,
    /// Use architect mode: plan with `--planner-model` (or config), then implement with `--model`.
    #[arg(long = "architect", global = true)]
    architect: bool,
    /// Model id for the planning phase when using `--architect`. Overrides `[architect] planner_model` in `~/.akmon/config.toml`; default `llama3.2`.
    #[arg(long = "planner-model", value_name = "MODEL", global = true)]
    planner_model: Option<String>,
    /// Resume the last session for this project directory (uses `~/.akmon/last_session.json`).
    #[arg(short = 'c', long = "continue", global = true, conflicts_with = "resume_session")]
    continue_last: bool,
    /// Resume a specific session id (full UUID or unique `*.json` prefix under `~/.akmon/sessions/`).
    #[arg(short = 's', long = "session", global = true, conflicts_with = "continue_last")]
    resume_session: Option<String>,
    /// Display name for this session (status line / JSON tooling).
    #[arg(short = 'n', long = "name", global = true)]
    session_name: Option<String>,
    /// Stop after estimated spend reaches this USD amount (headless only; ignored for Ollama).
    #[arg(long = "max-budget-usd", global = true, value_name = "USD")]
    max_budget_usd: Option<f64>,
    /// Extra directories merged into the sandbox (in addition to the project root). Repeatable.
    #[arg(long = "add-dir", global = true, value_name = "DIR", action = clap::ArgAction::Append)]
    add_dirs: Vec<PathBuf>,
    /// Model to try on repeated HTTP 429/529 before giving up (headless only).
    #[arg(long = "fallback-model", global = true, value_name = "MODEL")]
    fallback_model: Option<String>,
}

/// Top-level `akmon` subcommands.
#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// Interactive full-screen terminal UI (same as `akmon` without `--task`).
    Chat,
    /// Analyze the current project and generate `AKMON.md` using the configured model.
    Init,
    /// Create a new project directory with starter files and `AKMON.md`.
    New(cli_project::NewCmd),
    /// Manage `~/.akmon/config.toml` (models, keys, MCP).
    Config(config_cmd::ConfigArgs),
    /// Structured spec workflow under `.akmon/specs/<feature>/`.
    Spec(spec_cmd::SpecCmd),
    /// Synthesize `AKMON.md` from other tools' context files (Claude, Cursor, …).
    Import(import_cmd::ImportArgs),
    /// Write `AKMON.md` into another tool's expected paths (`--all` or `--tool`).
    Export(export_cmd::ExportArgs),
}

#[cfg(feature = "semantic-index")]
type SemanticIndexParts = (
    Arc<RwLock<Option<akmon_index::RepoIndex>>>,
    Arc<std::sync::Mutex<TextEmbedding>>,
);

/// Builds the tool registry; [`ShellTool`] is registered only when at least one `--shell-allow` pattern is set
/// and `plan_mode` is `false`.
///
/// [`WebFetchTool`] is registered only when `web_fetch` is true (`--web-fetch`).
///
/// When `plan_mode` is `true`, only read-oriented tools are registered (no write, shell, git, MCP added here).
///
/// [`SemanticSearchTool`] is included when built with `semantic-index` and `semantic` is [`Some`].
fn build_tool_registry(
    shell_allow: &[String],
    web_fetch: bool,
    has_git_root: bool,
    plan_mode: bool,
    #[cfg(feature = "semantic-index")] semantic: Option<SemanticIndexParts>,
) -> Vec<Box<dyn akmon_tools::Tool>> {
    if plan_mode {
        let mut tools: Vec<Box<dyn akmon_tools::Tool>> = vec![
            Box::new(ReadFileTool::new()),
            Box::new(ListDirectoryTool::new()),
            Box::new(SearchTool::new()),
            Box::new(AskFollowupTool),
            Box::new(TodoWriteTool),
            Box::new(MemoryWriteTool),
        ];
        if web_fetch {
            tools.push(Box::new(WebFetchTool::new()));
        }
        #[cfg(feature = "semantic-index")]
        if let Some((slot, emb)) = semantic {
            tools.push(Box::new(SemanticSearchTool::new(slot, Some(emb))));
        }
        return tools;
    }
    let mut tools: Vec<Box<dyn akmon_tools::Tool>> = vec![
        Box::new(ReadFileTool::new()),
        Box::new(WriteFileTool::new()),
        Box::new(ListDirectoryTool::new()),
        Box::new(SearchTool::new()),
        Box::new(EditTool::new()),
        Box::new(PatchTool::new()),
        Box::new(ApplyPatchTool::new()),
        Box::new(AskFollowupTool),
        Box::new(TodoWriteTool),
        Box::new(MemoryWriteTool),
    ];
    if web_fetch {
        tools.push(Box::new(WebFetchTool::new()));
    }
    if !shell_allow.is_empty() {
        tools.push(Box::new(ShellTool::new(shell_allow.to_vec())));
    }
    #[cfg(feature = "semantic-index")]
    if let Some((slot, emb)) = semantic {
        tools.push(Box::new(SemanticSearchTool::new(slot, Some(emb))));
    }
    if has_git_root {
        tools.push(Box::new(GitTool::new()));
    }
    tools
}

#[cfg(feature = "semantic-index")]
#[allow(clippy::too_many_arguments)]
fn cli_attach_specs_subagent(
    tools: &mut Vec<Box<dyn akmon_tools::Tool>>,
    cli: &Cli,
    has_git_root: bool,
    plan_mode: bool,
    provider: &Arc<dyn LlmProvider>,
    sandbox: &Arc<Sandbox>,
    akmon_md: &Option<String>,
    semantic_parts: Option<SemanticIndexParts>,
) {
    tools.push(Box::new(ReadSpecTool::new()));
    if !plan_mode {
        tools.push(Box::new(WriteSpecTool::new()));
    }
    let shell_allow = cli.shell_allow.clone();
    let web_fetch = cli.web_fetch;
    let semantic = semantic_parts.clone();
    let plan_for_sub = plan_mode;
    let factory: SubagentToolFactory = Arc::new(move || {
        build_tool_registry(
            &shell_allow,
            web_fetch,
            has_git_root,
            plan_for_sub,
            semantic.clone(),
        )
    });
    let rt = Arc::new(SubagentRuntime {
        provider: Arc::clone(provider),
        policy: Arc::new(PolicyEngine::new(PolicyEngineMode::Interactive)),
        sandbox: Arc::clone(sandbox),
        akmon_md: akmon_md.clone(),
        plan_mode,
        confirmation_timeout_secs: 30,
        tool_factory: factory,
    });
    tools.push(Box::new(SpawnSubagentTool::new(rt)));
}

#[cfg(not(feature = "semantic-index"))]
fn cli_attach_specs_subagent(
    tools: &mut Vec<Box<dyn akmon_tools::Tool>>,
    cli: &Cli,
    has_git_root: bool,
    plan_mode: bool,
    provider: &Arc<dyn LlmProvider>,
    sandbox: &Arc<Sandbox>,
    akmon_md: &Option<String>,
) {
    tools.push(Box::new(ReadSpecTool::new()));
    if !plan_mode {
        tools.push(Box::new(WriteSpecTool::new()));
    }
    let shell_allow = cli.shell_allow.clone();
    let web_fetch = cli.web_fetch;
    let plan_for_sub = plan_mode;
    let factory: SubagentToolFactory = Arc::new(move || {
        build_tool_registry(
            &shell_allow,
            web_fetch,
            has_git_root,
            plan_for_sub,
        )
    });
    let rt = Arc::new(SubagentRuntime {
        provider: Arc::clone(provider),
        policy: Arc::new(PolicyEngine::new(PolicyEngineMode::Interactive)),
        sandbox: Arc::clone(sandbox),
        akmon_md: akmon_md.clone(),
        plan_mode,
        confirmation_timeout_secs: 30,
        tool_factory: factory,
    });
    tools.push(Box::new(SpawnSubagentTool::new(rt)));
}

fn load_user_global_config() -> AkmonGlobalConfig {
    akmon_config::akmon_config_path()
        .as_ref()
        .and_then(|p| akmon_config::load_config_from(p).ok())
        .unwrap_or_default()
}

fn coalesce_opt(a: Option<String>, b: Option<String>) -> Option<String> {
    a.filter(|s| !s.trim().is_empty())
        .or_else(|| b.filter(|s| !s.trim().is_empty()))
}

/// Builds [`LlmConnectConfig`] from CLI flags merged with `~/.akmon/config.toml`.
pub(crate) fn llm_connect_from_cli(
    cli: &Cli,
    global: &AkmonGlobalConfig,
    model: String,
) -> LlmConnectConfig {
    let azure_ver = if cli.azure_api_version.is_empty() {
        global
            .azure_api_version
            .clone()
            .unwrap_or_else(|| "2024-02-01".into())
    } else {
        cli.azure_api_version.clone()
    };
    LlmConnectConfig {
        model,
        ollama_url: cli.ollama_url.clone(),
        anthropic_api_key: coalesce_opt(
            cli.anthropic_key.clone(),
            global.anthropic_api_key.clone(),
        ),
        openrouter_api_key: coalesce_opt(
            cli.openrouter_key.clone(),
            global.openrouter_api_key.clone(),
        ),
        openai_api_key: coalesce_opt(cli.openai_key.clone(), global.openai_api_key.clone()),
        groq_api_key: coalesce_opt(cli.groq_key.clone(), global.groq_api_key.clone()),
        azure_openai_endpoint: coalesce_opt(
            cli.azure_endpoint.clone(),
            global.azure_openai_endpoint.clone(),
        ),
        azure_openai_api_key: coalesce_opt(
            cli.azure_key.clone(),
            global.azure_openai_api_key.clone(),
        ),
        azure_api_version: azure_ver,
        bedrock_explicit: cli.bedrock,
        aws_region: cli.aws_region.clone(),
        openai_compatible_url: coalesce_opt(
            cli.openai_compatible_url.clone(),
            global.openai_compatible_url.clone(),
        ),
        openai_compatible_api_key: coalesce_opt(
            cli.openai_compatible_key.clone(),
            global.openai_compatible_api_key.clone(),
        ),
    }
}

fn resolve_llm(
    cli: &Cli,
    global: &AkmonGlobalConfig,
    model: String,
) -> Result<Arc<dyn LlmProvider>, ProviderError> {
    llm_connect_from_cli(cli, global, model).resolve()
}

/// Resolves the planner model for architect mode: `--planner-model`, then `~/.akmon/config.toml` `[architect]`, then `llama3.2`.
pub(crate) fn planner_model_for_tui(cli: &Cli) -> String {
    let global = load_user_global_config();
    cli.planner_model
        .clone()
        .or(global.architect.planner_model.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "llama3.2".into())
}

/// Default JSONL audit path under `project_root`: `.akmon/audit/{session_id}.jsonl`.
fn default_audit_log_path(project_root: &Path, session_id: uuid::Uuid) -> PathBuf {
    project_root
        .join(".akmon")
        .join("audit")
        .join(format!("{session_id}.jsonl"))
}

/// Resolves the audit file path: explicit `--audit-log` or the default under `.akmon/audit/`.
fn resolve_audit_log_path(
    project_root: &Path,
    session_id: uuid::Uuid,
    custom: Option<PathBuf>,
) -> PathBuf {
    match custom {
        Some(p) => p,
        None => default_audit_log_path(project_root, session_id),
    }
}

fn resolve_resume_session_id(cli: &Cli, project_root: &Path) -> Result<Option<uuid::Uuid>, String> {
    if cli.continue_last {
        let index = session_index::SessionIndex::load();
        let Some(entry) = index.get_for_project(project_root) else {
            return Err("No previous session found for this directory.".into());
        };
        let u = uuid::Uuid::parse_str(&entry.session_id)
            .map_err(|e| format!("invalid session id in index: {e}"))?;
        return Ok(Some(u));
    }
    if let Some(ref s) = cli.resume_session {
        return session_transcript::resolve_session_id_from_cli_arg(s).map(Some);
    }
    Ok(None)
}

fn sandbox_for_cli(project_root: PathBuf, has_git_root: bool, add_dirs: &[PathBuf]) -> Arc<Sandbox> {
    let extra: Vec<PathBuf> = add_dirs
        .iter()
        .filter_map(|p| dunce::canonicalize(p).ok())
        .collect();
    if extra.is_empty() {
        Arc::new(Sandbox::with_git_root(project_root, has_git_root))
    } else {
        Arc::new(Sandbox::with_additional_roots_git(project_root, extra, has_git_root))
    }
}

fn model_messages_to_tui(msgs: Vec<Message>) -> Vec<akmon_tui::TuiMessage> {
    use akmon_tui::TuiMessage;
    msgs.into_iter()
        .filter_map(|m| match m.role {
            MessageRole::User => Some(TuiMessage::User {
                content: m.content,
            }),
            MessageRole::Assistant => Some(TuiMessage::Assistant {
                content: m.content,
                complete: true,
            }),
            _ => None,
        })
        .collect()
}

fn exit_reason_ok(session: &AgentSession) -> ExitReason {
    match session.last_run_exit() {
        SessionRunExit::BudgetLimit => ExitReason::BudgetLimit,
        SessionRunExit::Completed => ExitReason::Completed,
    }
}

fn exit_reason_err(e: &AgentError) -> ExitReason {
    match e {
        AgentError::IterationLimitReached { .. } => ExitReason::MaxTurns,
        _ => ExitReason::Error,
    }
}

fn headless_persist(
    project_root: &Path,
    session: &AgentSession,
    model: &str,
    started_at: chrono::DateTime<chrono::Utc>,
) {
    let msgs: Vec<Message> = session.context_messages().to_vec();
    let started_str = started_at.to_rfc3339();
    if let Err(e) = session_transcript::save_headless_session_file(session_transcript::HeadlessSessionSnapshot {
        session_id: session.session_id(),
        project_root,
        model,
        messages: &msgs,
        started_at_rfc3339: &started_str,
        total_input_tokens: session.total_input_tokens(),
        total_cache_read_tokens: session.total_cache_read_tokens(),
        total_output_tokens: session.total_output_tokens(),
    }) {
        eprintln!("akmon: warning: could not save session snapshot: {e}");
    }
    let mut index = session_index::SessionIndex::load();
    index.record(
        project_root,
        session_index::SessionEntry {
            session_id: session.session_id().to_string(),
            model: model.into(),
            started_at: started_str,
            turn_count: session.user_turns_finished,
        },
    );
}

/// Prints [`AgentEvent`]s for the TTY and forwards interactive policy replies.
async fn run_event_printer(
    mut ev_rx: mpsc::Receiver<AgentEvent>,
    policy_tx: mpsc::Sender<InteractivePolicyReply>,
    output: OutputFormat,
) {
    while let Some(ev) = ev_rx.recv().await {
        match ev {
            AgentEvent::TextDelta { text } => {
                if output == OutputFormat::Text {
                    print!("{text}");
                    let _ = std::io::stdout().flush();
                }
            }
            AgentEvent::ToolCallDispatched { name, .. } => {
                if output == OutputFormat::Text {
                    println!("\n→ {name}");
                }
            }
            AgentEvent::ToolCallCompleted {
                name,
                success,
                message,
                ..
            } => {
                if output == OutputFormat::Text {
                    if success {
                        println!("✓ {name}");
                    } else {
                        eprintln!("✗ {name}: {message}");
                    }
                }
            }
            AgentEvent::StatusInfo { message } => {
                if output == OutputFormat::Text {
                    eprintln!("{message}");
                }
            }
            AgentEvent::SummarizationStarted => {
                if output == OutputFormat::Text {
                    eprintln!("akmon: context summarization started…");
                }
            }
            AgentEvent::ContextSummarized {
                messages_replaced,
                tokens_freed,
            } => {
                if output == OutputFormat::Text {
                    eprintln!(
                        "akmon: context summarized (messages_replaced={messages_replaced}, tokens_freed≈{tokens_freed})"
                    );
                }
            }
            AgentEvent::ConfirmationRequired {
                description,
                diff_preview,
            } => {
                eprintln!("{description}");
                if let Some(diff) = diff_preview {
                    eprint!("{}", akmon_tools::colorize_unified_diff(&diff));
                }
                let line: String = tokio::task::spawn_blocking(|| {
                    print!("Allow? [y=once / Y=remember session / n=N]: ");
                    let _ = std::io::stdout().flush();
                    let mut buf = String::new();
                    let _ = std::io::stdin().read_line(&mut buf);
                    buf
                })
                .await
                .unwrap_or_default();
                let t = line.trim();
                let reply = if t == "Y" {
                    InteractivePolicyReply {
                        verdict: PolicyVerdict::Allow,
                        remember_for_session: true,
                        allow_all_writes_session: false,
                        shell_allow_prefix: None,
                    }
                } else if t.eq_ignore_ascii_case("y") {
                    InteractivePolicyReply::allow_once()
                } else {
                    InteractivePolicyReply::deny()
                };
                let _ = policy_tx.send(reply).await;
            }
            AgentEvent::UsageReport {
                input_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                ..
            } => {
                if output == OutputFormat::Text
                    && (cache_read_tokens > 0 || cache_creation_tokens > 0)
                {
                    eprintln!(
                        "akmon: tokens — input:{input_tokens} cache_hit:{cache_read_tokens} cache_write:{cache_creation_tokens}"
                    );
                }
            }
            AgentEvent::Done => {
                if output == OutputFormat::Text {
                    println!("\nDone.");
                }
            }
            AgentEvent::Error { error, .. } => {
                if output == OutputFormat::Text {
                    eprintln!("{error}");
                }
            }
            _ => {}
        }
    }
}

fn seed_project_dot_akmon_if_applicable(project_root: &Path, has_git_root: bool) {
    if !cli_project::should_ensure_project_dot_akmon(project_root, has_git_root) {
        return;
    }
    if let Err(e) = akmon_core::ensure_dot_akmon_layout(project_root) {
        eprintln!("akmon: warning: could not create .akmon directories: {e}");
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            exit_early_config_error(
                &cli,
                format!("cannot read current directory: {e}"),
                None,
                2,
            );
        }
    };

    let (project_root, has_git_root) = cli_project::resolve_sandbox_root(&cwd);
    if !has_git_root {
        eprintln!("akmon: no git repository found.");
        eprintln!(
            "Using cwd as sandbox: {}",
            dunce::canonicalize(&project_root)
                .unwrap_or_else(|_| project_root.clone())
                .display()
        );
        eprintln!("Run git init to enable git features.");
    }

    match &cli.command {
        Some(Commands::Init) => {
            return cli_project::run_init(&cli, &project_root).await;
        }
        Some(Commands::Import(args)) => {
            let provider = match cli_project::resolve_provider(&cli) {
                Ok(p) => p,
                Err(e) => {
                    exit_early_config_error(&cli, e.to_string(), None, 2);
                }
            };
            return match import_cmd::run_import(args.clone(), project_root.clone(), provider).await
            {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("akmon: import: {e:#}");
                    ExitCode::from(1)
                }
            };
        }
        Some(Commands::Export(args)) => {
            return match export_cmd::run_export(args.clone(), &project_root) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("akmon: export: {e:#}");
                    ExitCode::from(1)
                }
            };
        }
        Some(Commands::New(args)) => {
            return cli_project::run_new(&cli, args, &cwd).await;
        }
        Some(Commands::Config(c)) => {
            return config_cmd::run_config(c.clone()).await;
        }
        Some(Commands::Spec(sc)) => {
            return spec_cmd::run_spec(&cli, &project_root, sc.clone()).await;
        }
        Some(Commands::Chat) | None => {}
    }

    let Some(task) = cli.task.clone() else {
        seed_project_dot_akmon_if_applicable(&project_root, has_git_root);
        let global = load_user_global_config();
        let azure_ver = if cli.azure_api_version.is_empty() {
            global
                .azure_api_version
                .clone()
                .unwrap_or_else(|| "2024-02-01".into())
        } else {
            cli.azure_api_version.clone()
        };
        let akmon_content = match load_akmon_md(&project_root) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("akmon: warning: failed to read AKMON.md: {e}");
                None
            }
        };
        let has_akmon_md = project_root.join("AKMON.md").is_file();

        let mode_label = if cli.yes { "AUTO" } else { "INTERACTIVE" };
        let resolved_resume = match resolve_resume_session_id(&cli, &project_root) {
            Ok(x) => x,
            Err(e) => {
                exit_resume_session_error(&cli, e);
            }
        };
        let session_id = resolved_resume.unwrap_or_else(uuid::Uuid::new_v4);
        let resume_messages = if resolved_resume.is_some() {
            match session_transcript::load_resume_messages(session_id, &project_root) {
                Ok(m) => Some(model_messages_to_tui(m)),
                Err(e) => {
                    eprintln!("akmon: warning: could not load session transcript: {e}");
                    None
                }
            }
        } else {
            None
        };
        let audit_log_path =
            resolve_audit_log_path(&project_root, session_id, cli.audit_log.clone());

        #[cfg(feature = "semantic-index")]
        let mut index_thread: Option<std::thread::JoinHandle<()>> = None;
        #[cfg(not(feature = "semantic-index"))]
        let index_thread: Option<std::thread::JoinHandle<()>> = None;

        #[cfg(feature = "semantic-index")]
        let semantic_index: Option<akmon_tui::SemanticIndexSlot> = if cli.index {
            let sandbox = sandbox_for_cli(project_root.clone(), has_git_root, &cli.add_dirs);
            let index_path = project_root.join(".akmon").join("index.bin");
            if !index_path.is_file() {
                eprintln!("akmon: downloading embedding model (~22MB) on first use...");
            }
            match TextEmbedding::try_new(
                TextInitOptions::default().with_show_download_progress(true),
            ) {
                Ok(m) => {
                    let emb = Arc::new(std::sync::Mutex::new(m));
                    let slot = Arc::new(RwLock::new(None));

                    if index_path.is_file() {
                        match akmon_index::load_index(&index_path) {
                            Ok(idx) => {
                                eprintln!(
                                    "akmon: semantic index loaded ({} files, {} chunks)",
                                    idx.file_count, idx.chunk_count
                                );
                                *slot.write().await = Some(idx);
                            }
                            Err(e) => {
                                eprintln!(
                                    "akmon: warning: could not load semantic index, rebuilding: {e}"
                                );
                                let slot_bg = Arc::clone(&slot);
                                let root_bg = project_root.clone();
                                let sandbox_bg = Arc::clone(&sandbox);
                                let emb_bg = Arc::clone(&emb);
                                let path_bg = index_path.clone();
                                index_thread = spawn_semantic_index_os_thread(
                                    root_bg, emb_bg, sandbox_bg, path_bg, slot_bg,
                                );
                                poll_index_ready_up_to_3s(&slot).await;
                            }
                        }
                    } else {
                        let slot_bg = Arc::clone(&slot);
                        let root_bg = project_root.clone();
                        let sandbox_bg = Arc::clone(&sandbox);
                        let emb_bg = Arc::clone(&emb);
                        let path_bg = index_path.clone();
                        index_thread = spawn_semantic_index_os_thread(
                            root_bg, emb_bg, sandbox_bg, path_bg, slot_bg,
                        );
                        poll_index_ready_up_to_3s(&slot).await;
                    }

                    Some((slot, emb))
                }
                Err(e) => {
                    eprintln!("akmon: warning: semantic index disabled (embedding model): {e}");
                    None
                }
            }
        } else {
            None
        };
        #[cfg(not(feature = "semantic-index"))]
        let semantic_index = {
            if cli.index {
                eprintln!(
                    "akmon: --index is ignored (this binary was built without the `semantic-index` feature)."
                );
            }
            None
        };

        let tui_config = TuiLaunchConfig {
            version: env!("CARGO_PKG_VERSION").to_string(),
            project_root: project_root.clone(),
            model_name: cli.model.clone(),
            mode_label: mode_label.to_string(),
            session_id,
            max_iterations: AgentConfig::default().max_iterations,
            index_enabled: cli.index,
            anthropic_key: coalesce_opt(
                cli.anthropic_key.clone(),
                global.anthropic_api_key.clone(),
            ),
            openrouter_key: coalesce_opt(
                cli.openrouter_key.clone(),
                global.openrouter_api_key.clone(),
            ),
            openai_key: coalesce_opt(cli.openai_key.clone(), global.openai_api_key.clone()),
            groq_key: coalesce_opt(cli.groq_key.clone(), global.groq_api_key.clone()),
            azure_endpoint: coalesce_opt(
                cli.azure_endpoint.clone(),
                global.azure_openai_endpoint.clone(),
            ),
            azure_key: coalesce_opt(cli.azure_key.clone(), global.azure_openai_api_key.clone()),
            azure_api_version: azure_ver,
            bedrock: cli.bedrock,
            aws_region: cli.aws_region.clone(),
            openai_compatible_url: coalesce_opt(
                cli.openai_compatible_url.clone(),
                global.openai_compatible_url.clone(),
            ),
            openai_compatible_key: coalesce_opt(
                cli.openai_compatible_key.clone(),
                global.openai_compatible_api_key.clone(),
            ),
            ollama_url: cli.ollama_url.clone(),
            shell_allow: cli.shell_allow.clone(),
            web_fetch: cli.web_fetch,
            yes_web: cli.yes_web,
            auto_yes: cli.yes,
            mcp_servers: cli.mcp_server.clone(),
            audit_log_path,
            akmon_md: akmon_content,
            has_akmon_md,
            sandbox_has_git_root: has_git_root,
            semantic_index,
            auto_commit: cli.auto_commit,
            planner_model: planner_model_for_tui(&cli),
            display_theme: global.display.theme,
            session_display_name: cli.session_name.clone(),
            resume_messages,
        };
        let tui_outcome = akmon_tui::run_interactive(tui_config).await;
        if let Some(handle) = index_thread {
            eprintln!("akmon: waiting for index to finish building...");
            let _ = handle.join();
        }
        return match tui_outcome {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("akmon: TUI error: {e}");
                ExitCode::from(1)
            }
        };
    };

    seed_project_dot_akmon_if_applicable(&project_root, has_git_root);

    let akmon_content = match load_akmon_md(&project_root) {
        Ok(c) => c,
        Err(e) => {
            exit_early_config_error(
                &cli,
                format!("failed to read AKMON.md: {e}"),
                None,
                2,
            );
        }
    };

    if cli.plan && cli.architect {
        exit_early_config_error(
            &cli,
            "--plan cannot be combined with --architect".into(),
            None,
            2,
        );
    }

    if cli.output == OutputFormat::Text {
        eprintln!("akmon: project root: {}", project_root.display());
        match &akmon_content {
            Some(text) => eprintln!("akmon: AKMON.md loaded ({} bytes)", text.len()),
            None => eprintln!("akmon: AKMON.md not found (optional)"),
        }
    }

    let resolved_resume = match resolve_resume_session_id(&cli, &project_root) {
        Ok(x) => x,
        Err(e) => {
            exit_resume_session_error(&cli, e);
        }
    };
    let session_id = resolved_resume.unwrap_or_else(uuid::Uuid::new_v4);
    let headless_started_at = chrono::Utc::now();
    let resume_ctx: Vec<Message> = if resolved_resume.is_some() {
        session_transcript::load_resume_messages(session_id, &project_root).unwrap_or_else(|e| {
            eprintln!("akmon: warning: could not load session transcript: {e}");
            Vec::new()
        })
    } else {
        Vec::new()
    };

    let agent_config = AgentConfig {
        session_id,
        auto_commit: cli.auto_commit,
        max_budget_usd: cli.max_budget_usd,
        fallback_model: cli.fallback_model.clone(),
        ..Default::default()
    };
    let audit_log_path = resolve_audit_log_path(&project_root, session_id, cli.audit_log.clone());
    let global = load_user_global_config();

    let policy_mode = if cli.yes {
        if cli.web_fetch && cli.yes_web {
            PolicyEngineMode::AutoApproveReadsAndFetch {
                confirm_writes: true,
            }
        } else {
            PolicyEngineMode::AutoApproveReads {
                confirm_writes: true,
            }
        }
    } else {
        PolicyEngineMode::Interactive
    };
    let policy = Arc::new(PolicyEngine::new(policy_mode));
    let sandbox = sandbox_for_cli(project_root.clone(), has_git_root, &cli.add_dirs);

    #[cfg(feature = "semantic-index")]
    let mut index_thread: Option<std::thread::JoinHandle<()>> = None;
    #[cfg(not(feature = "semantic-index"))]
    let mut index_thread: Option<std::thread::JoinHandle<()>> = None;

    #[cfg(feature = "semantic-index")]
    let semantic_parts: Option<SemanticIndexParts> = if cli.index {
        let index_path = project_root.join(".akmon").join("index.bin");
        if !index_path.is_file() {
            eprintln!("akmon: downloading embedding model (~22MB) on first use...");
        }
        match TextEmbedding::try_new(TextInitOptions::default().with_show_download_progress(true)) {
            Ok(m) => {
                let emb = Arc::new(std::sync::Mutex::new(m));
                let slot = Arc::new(RwLock::new(None));

                if index_path.is_file() {
                    match akmon_index::load_index(&index_path) {
                        Ok(idx) => {
                            eprintln!(
                                "akmon: semantic index loaded ({} files, {} chunks)",
                                idx.file_count, idx.chunk_count
                            );
                            *slot.write().await = Some(idx);
                        }
                        Err(e) => {
                            eprintln!(
                                "akmon: warning: could not load semantic index, rebuilding: {e}"
                            );
                            let slot_bg = Arc::clone(&slot);
                            let root_bg = project_root.clone();
                            let sandbox_bg = Arc::clone(&sandbox);
                            let emb_bg = Arc::clone(&emb);
                            let path_bg = index_path.clone();
                            index_thread = spawn_semantic_index_os_thread(
                                root_bg, emb_bg, sandbox_bg, path_bg, slot_bg,
                            );
                            poll_index_ready_up_to_3s(&slot).await;
                        }
                    }
                } else {
                    let slot_bg = Arc::clone(&slot);
                    let root_bg = project_root.clone();
                    let sandbox_bg = Arc::clone(&sandbox);
                    let emb_bg = Arc::clone(&emb);
                    let path_bg = index_path.clone();
                    index_thread = spawn_semantic_index_os_thread(
                        root_bg, emb_bg, sandbox_bg, path_bg, slot_bg,
                    );
                    poll_index_ready_up_to_3s(&slot).await;
                }

                Some((slot, emb))
            }
            Err(e) => {
                eprintln!("akmon: warning: semantic index disabled (embedding model): {e}");
                None
            }
        }
    } else {
        None
    };

    #[cfg(not(feature = "semantic-index"))]
    if cli.index {
        eprintln!(
            "akmon: --index is ignored (this binary was built without the `semantic-index` feature)."
        );
    }

    if cli.plan {
        let provider = match resolve_llm(&cli, &global, cli.model.clone()) {
            Ok(p) => p,
            Err(e) => {
                exit_early_config_error(&cli, e.to_string(), Some(&mut index_thread), 2);
            }
        };
        let mut tools = build_tool_registry(
            &cli.shell_allow,
            cli.web_fetch,
            has_git_root,
            true,
            #[cfg(feature = "semantic-index")]
            semantic_parts.clone(),
        );
        #[cfg(feature = "semantic-index")]
        cli_attach_specs_subagent(
            &mut tools,
            &cli,
            has_git_root,
            true,
            &provider,
            &sandbox,
            &akmon_content,
            semantic_parts.clone(),
        );
        #[cfg(not(feature = "semantic-index"))]
        cli_attach_specs_subagent(
            &mut tools,
            &cli,
            has_git_root,
            true,
            &provider,
            &sandbox,
            &akmon_content,
        );
        let plan_agent_config = AgentConfig {
            auto_commit: false,
            ..agent_config.clone()
        };
        let mut session = AgentSession::new(
            plan_agent_config,
            Arc::clone(&policy),
            provider,
            tools,
            Arc::clone(&sandbox),
            akmon_content.clone(),
            true,
        );
        if !resume_ctx.is_empty() {
            session.restore_context_from_messages(resume_ctx.clone());
        }
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(256);
        let (policy_tx, policy_rx) = mpsc::channel::<InteractivePolicyReply>(32);
        let printer = tokio::spawn(run_event_printer(ev_rx, policy_tx, cli.output));
        let mut policy_opt = Some(policy_rx);
        let run_outcome = session
            .run(task.clone(), ev_tx, &mut policy_opt, &mut None, None)
            .await;
        drop(policy_opt);
        let _ = printer.await;
        let plan_body = session.result_text().to_string();
        let saved_path = match akmon_core::save_plan_markdown(&project_root, &task, &plan_body) {
            Ok(p) => p,
            Err(e) => {
                exit_early_config_error(
                    &cli,
                    format!("failed to save plan: {e}"),
                    Some(&mut index_thread),
                    2,
                );
            }
        };
        if cli.output == OutputFormat::Text {
            println!("{plan_body}");
            println!();
            println!("Plan saved to {}", saved_path.display());
            println!();
            println!("Review:  cat {}", saved_path.display());
            println!("Edit:    $EDITOR {}", saved_path.display());
            println!(
                "Implement: akmon --task 'implement the plan in {}'",
                saved_path.display()
            );
        }
        if let Err(e) = write_audit_jsonl(&audit_log_path, session.audit_events()) {
            eprintln!(
                "akmon: failed to write audit log {}: {e}",
                audit_log_path.display()
            );
        }
        let _ = write_handoff_file(&session, &project_root, &cli.model);
        headless_persist(&project_root, &session, &cli.model, headless_started_at);
        if let Some(handle) = index_thread {
            eprintln!("akmon: waiting for index to finish building...");
            eprintln!(
                "akmon: (CPU-bound embedding — more `akmon:` lines may appear until the index is saved)"
            );
            let _ = handle.join();
        }
        if cli.output == OutputFormat::Json {
            let (status, error_opt): (&'static str, Option<String>) = match &run_outcome {
                Ok(()) => ("success", None),
                Err(e) => ("error", Some(e.to_string())),
            };
            let exit_reason = match &run_outcome {
                Ok(()) => exit_reason_ok(&session),
                Err(e) => exit_reason_err(e),
            };
            let report = RunReport {
                session_id: session.session_id().to_string(),
                status,
                exit_reason,
                result: plan_body,
                tool_calls: session.tool_call_summaries().to_vec(),
                error: error_opt,
                audit_log_path: audit_log_path.to_string_lossy().into_owned(),
                usage: RunUsageSummary {
                    total_input_tokens: session.total_input_tokens(),
                    total_cache_read_tokens: session.total_cache_read_tokens(),
                    total_output_tokens: session.total_output_tokens(),
                },
                cost_usd: session.total_cost_usd(),
                cache_read_tokens: u64::from(session.total_cache_read_tokens()),
                files_written: session
                    .modified_paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect(),
            };
            let json_line = match serde_json::to_string(&report) {
                Ok(s) => s,
                Err(e) => {
                    print_json_early_error_and_exit(format!("failed to serialize run report: {e}"));
                }
            };
            println!("{json_line}");
        }
        return match run_outcome {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                if cli.output == OutputFormat::Text {
                    eprintln!("akmon: {e}");
                }
                exit_code_for_agent_error(&e)
            }
        };
    }

    if cli.architect {
        let planner_model = planner_model_for_tui(&cli);
        let provider_planner = match resolve_llm(&cli, &global, planner_model.clone()) {
            Ok(p) => p,
            Err(e) => {
                exit_early_config_error(&cli, e.to_string(), Some(&mut index_thread), 2);
            }
        };
        let mut tools_planner = build_tool_registry(
            &cli.shell_allow,
            cli.web_fetch,
            has_git_root,
            true,
            #[cfg(feature = "semantic-index")]
            semantic_parts.clone(),
        );
        #[cfg(feature = "semantic-index")]
        cli_attach_specs_subagent(
            &mut tools_planner,
            &cli,
            has_git_root,
            true,
            &provider_planner,
            &sandbox,
            &akmon_content,
            semantic_parts.clone(),
        );
        #[cfg(not(feature = "semantic-index"))]
        cli_attach_specs_subagent(
            &mut tools_planner,
            &cli,
            has_git_root,
            true,
            &provider_planner,
            &sandbox,
            &akmon_content,
        );
        let planner_agent_config = AgentConfig {
            auto_commit: false,
            ..agent_config.clone()
        };
        let mut planner_session = AgentSession::new(
            planner_agent_config,
            Arc::clone(&policy),
            provider_planner,
            tools_planner,
            Arc::clone(&sandbox),
            akmon_content.clone(),
            true,
        );
        if !resume_ctx.is_empty() {
            planner_session.restore_context_from_messages(resume_ctx.clone());
        }
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(256);
        let (policy_tx, policy_rx) = mpsc::channel::<InteractivePolicyReply>(32);
        let printer = tokio::spawn(run_event_printer(ev_rx, policy_tx, cli.output));
        let mut policy_opt = Some(policy_rx);
        let plan_run = planner_session
            .run(task.clone(), ev_tx, &mut policy_opt, &mut None, None)
            .await;
        drop(policy_opt);
        let _ = printer.await;
        if let Err(e) = plan_run {
            if let Err(audit_err) =
                write_audit_jsonl(&audit_log_path, planner_session.audit_events())
            {
                eprintln!(
                    "akmon: failed to write audit log {}: {audit_err}",
                    audit_log_path.display()
                );
            }
            if let Some(handle) = index_thread {
                eprintln!("akmon: waiting for index to finish building...");
                let _ = handle.join();
            }
            if cli.output == OutputFormat::Text {
                eprintln!("akmon: {e}");
            }
            return exit_code_for_agent_error(&e);
        }
        let plan_text = planner_session.result_text().to_string();
        eprintln!("akmon: architect — plan complete (planner: {planner_model})");
        if let Err(e) = akmon_core::save_plan_markdown(&project_root, &task, &plan_text) {
            eprintln!("akmon: warning: failed to save plan file: {e}");
        }
        let provider_main = match resolve_llm(&cli, &global, cli.model.clone()) {
            Ok(p) => p,
            Err(e) => {
                exit_early_config_error(&cli, e.to_string(), Some(&mut index_thread), 2);
            }
        };
        let mut tools = build_tool_registry(
            &cli.shell_allow,
            cli.web_fetch,
            has_git_root,
            false,
            #[cfg(feature = "semantic-index")]
            semantic_parts.clone(),
        );
        for url in &cli.mcp_server {
            let server = McpServerConfig {
                name: url.clone(),
                url: url.clone(),
                description: String::new(),
            };
            match discover_mcp_tools(&server).await {
                Ok(mcp_tools) => {
                    eprintln!(
                        "akmon: MCP server {} — {} tools registered",
                        url,
                        mcp_tools.len()
                    );
                    for t in mcp_tools {
                        tools.push(Box::new(t));
                    }
                }
                Err(e) => {
                    eprintln!("akmon: MCP server {url} unavailable: {e}");
                }
            }
        }
        #[cfg(feature = "semantic-index")]
        cli_attach_specs_subagent(
            &mut tools,
            &cli,
            has_git_root,
            false,
            &provider_main,
            &sandbox,
            &akmon_content,
            semantic_parts.clone(),
        );
        #[cfg(not(feature = "semantic-index"))]
        cli_attach_specs_subagent(
            &mut tools,
            &cli,
            has_git_root,
            false,
            &provider_main,
            &sandbox,
            &akmon_content,
        );
        let mut session = AgentSession::new(
            agent_config,
            Arc::clone(&policy),
            provider_main,
            tools,
            Arc::clone(&sandbox),
            akmon_content,
            false,
        );
        let impl_task = format!(
            "Implement this plan exactly:\n\n{plan_text}\n\nOriginal task: {task}\n\nFollow the plan step by step.\nDo not deviate from the plan without explaining why."
        );
        let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(256);
        let (policy_tx, policy_rx) = mpsc::channel::<InteractivePolicyReply>(32);
        let printer = tokio::spawn(run_event_printer(ev_rx, policy_tx, cli.output));
        let mut policy_opt = Some(policy_rx);
        let run_outcome = session.run(impl_task, ev_tx, &mut policy_opt, &mut None, None).await;
        drop(policy_opt);
        let _ = printer.await;
        let mut combined_audit: Vec<AuditEvent> = Vec::new();
        combined_audit.extend(planner_session.audit_events().iter().cloned());
        combined_audit.extend(session.audit_events().iter().cloned());
        if let Err(e) = write_audit_jsonl(&audit_log_path, &combined_audit) {
            eprintln!(
                "akmon: failed to write audit log {}: {e}",
                audit_log_path.display()
            );
        }
        let _ = write_handoff_file(&session, &project_root, &cli.model);
        headless_persist(&project_root, &session, &cli.model, headless_started_at);
        if let Some(handle) = index_thread {
            eprintln!("akmon: waiting for index to finish building...");
            eprintln!(
                "akmon: (CPU-bound embedding — more `akmon:` lines may appear until the index is saved)"
            );
            let _ = handle.join();
        }
        if cli.output == OutputFormat::Json {
            let (status, error_opt): (&'static str, Option<String>) = match &run_outcome {
                Ok(()) => ("success", None),
                Err(e) => ("error", Some(e.to_string())),
            };
            let exit_reason = match &run_outcome {
                Ok(()) => exit_reason_ok(&session),
                Err(e) => exit_reason_err(e),
            };
            let report = RunReport {
                session_id: session.session_id().to_string(),
                status,
                exit_reason,
                result: session.result_text().to_string(),
                tool_calls: session.tool_call_summaries().to_vec(),
                error: error_opt,
                audit_log_path: audit_log_path.to_string_lossy().into_owned(),
                usage: RunUsageSummary {
                    total_input_tokens: session.total_input_tokens(),
                    total_cache_read_tokens: session.total_cache_read_tokens(),
                    total_output_tokens: session.total_output_tokens(),
                },
                cost_usd: session.total_cost_usd(),
                cache_read_tokens: u64::from(session.total_cache_read_tokens()),
                files_written: session
                    .modified_paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect(),
            };
            let json_line = match serde_json::to_string(&report) {
                Ok(s) => s,
                Err(e) => {
                    print_json_early_error_and_exit(format!("failed to serialize run report: {e}"));
                }
            };
            println!("{json_line}");
        }
        return match run_outcome {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                if cli.output == OutputFormat::Text {
                    eprintln!("akmon: {e}");
                }
                exit_code_for_agent_error(&e)
            }
        };
    }

    let provider = match resolve_llm(&cli, &global, cli.model.clone()) {
        Ok(p) => p,
        Err(e) => {
            exit_early_config_error(&cli, e.to_string(), Some(&mut index_thread), 2);
        }
    };

    let mut tools = build_tool_registry(
        &cli.shell_allow,
        cli.web_fetch,
        has_git_root,
        false,
        #[cfg(feature = "semantic-index")]
        semantic_parts.clone(),
    );
    for url in &cli.mcp_server {
        let server = McpServerConfig {
            name: url.clone(),
            url: url.clone(),
            description: String::new(),
        };
        match discover_mcp_tools(&server).await {
            Ok(mcp_tools) => {
                eprintln!(
                    "akmon: MCP server {} — {} tools registered",
                    url,
                    mcp_tools.len()
                );
                for t in mcp_tools {
                    tools.push(Box::new(t));
                }
            }
            Err(e) => {
                eprintln!("akmon: MCP server {url} unavailable: {e}");
            }
        }
    }
    #[cfg(feature = "semantic-index")]
    cli_attach_specs_subagent(
        &mut tools,
        &cli,
        has_git_root,
        false,
        &provider,
        &sandbox,
        &akmon_content,
        semantic_parts,
    );
    #[cfg(not(feature = "semantic-index"))]
    cli_attach_specs_subagent(
        &mut tools,
        &cli,
        has_git_root,
        false,
        &provider,
        &sandbox,
        &akmon_content,
    );

    let mut session = AgentSession::new(
        agent_config,
        Arc::clone(&policy),
        provider,
        tools,
        Arc::clone(&sandbox),
        akmon_content,
        false,
    );

    if !resume_ctx.is_empty() {
        session.restore_context_from_messages(resume_ctx);
    }

    let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(256);
    let (policy_tx, policy_rx) = mpsc::channel::<InteractivePolicyReply>(32);
    let printer = tokio::spawn(run_event_printer(ev_rx, policy_tx, cli.output));

    let mut policy_opt = Some(policy_rx);
    let run_outcome = session.run(task, ev_tx, &mut policy_opt, &mut None, None).await;

    drop(policy_opt);

    let _ = printer.await;

    if let Err(e) = write_audit_jsonl(&audit_log_path, session.audit_events()) {
        eprintln!(
            "akmon: failed to write audit log {}: {e}",
            audit_log_path.display()
        );
    }

    let _ = write_handoff_file(&session, &project_root, &cli.model);

    headless_persist(&project_root, &session, &cli.model, headless_started_at);

    if let Some(handle) = index_thread {
        eprintln!("akmon: waiting for index to finish building...");
        eprintln!(
            "akmon: (CPU-bound embedding — more `akmon:` lines may appear until the index is saved)"
        );
        let _ = handle.join();
    }

    let audit_log_path_str = audit_log_path.to_string_lossy().into_owned();

    if cli.output == OutputFormat::Json {
        let (status, error_opt): (&'static str, Option<String>) = match &run_outcome {
            Ok(()) => ("success", None),
            Err(e) => ("error", Some(e.to_string())),
        };
        let exit_reason = match &run_outcome {
            Ok(()) => exit_reason_ok(&session),
            Err(e) => exit_reason_err(e),
        };
        let report = RunReport {
            session_id: session.session_id().to_string(),
            status,
            exit_reason,
            result: session.result_text().to_string(),
            tool_calls: session.tool_call_summaries().to_vec(),
            error: error_opt,
            audit_log_path: audit_log_path_str,
            usage: RunUsageSummary {
                total_input_tokens: session.total_input_tokens(),
                total_cache_read_tokens: session.total_cache_read_tokens(),
                total_output_tokens: session.total_output_tokens(),
            },
            cost_usd: session.total_cost_usd(),
            cache_read_tokens: u64::from(session.total_cache_read_tokens()),
            files_written: session
                .modified_paths
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
        };
        let json_line = match serde_json::to_string(&report) {
            Ok(s) => s,
            Err(e) => {
                print_json_early_error_and_exit(format!("failed to serialize run report: {e}"));
            }
        };
        println!("{json_line}");
        return match run_outcome {
            Ok(()) => ExitCode::SUCCESS,
            Err(ref e) => exit_code_for_agent_error(e),
        };
    }

    match run_outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("akmon: {e}");
            exit_code_for_agent_error(&e)
        }
    }
}

fn exit_code_for_agent_error(e: &AgentError) -> ExitCode {
    match e {
        AgentError::PolicyDenied { .. } => ExitCode::from(3),
        AgentError::IterationLimitReached { .. }
        | AgentError::ModelError { .. }
        | AgentError::ToolError { .. }
        | AgentError::ResponseTruncated
        | AgentError::SessionFailed { .. }
        | AgentError::InvalidTransition { .. } => ExitCode::from(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(
        shell_allow: &[String],
        web_fetch: bool,
        has_git_root: bool,
        plan_mode: bool,
    ) -> Vec<Box<dyn akmon_tools::Tool>> {
        build_tool_registry(
            shell_allow,
            web_fetch,
            has_git_root,
            plan_mode,
            #[cfg(feature = "semantic-index")]
            None,
        )
    }

    #[test]
    fn shell_tool_omitted_without_allow_flags() {
        let t = reg(&[], false, false, false);
        assert!(!t.iter().any(|x| x.name() == "shell"));
    }

    #[test]
    fn shell_tool_registered_when_allow_patterns_present() {
        let t = reg(&["echo *".into()], false, false, false);
        assert!(t.iter().any(|x| x.name() == "shell"));
    }

    #[test]
    fn web_fetch_tool_omitted_without_flag() {
        let t = reg(&[], false, false, false);
        assert!(!t.iter().any(|x| x.name() == "web_fetch"));
    }

    #[test]
    fn web_fetch_tool_registered_when_flag_set() {
        let t = reg(&[], true, false, false);
        assert!(t.iter().any(|x| x.name() == "web_fetch"));
    }

    #[test]
    fn git_tool_registered_when_has_git_root() {
        let t = reg(&[], false, true, false);
        assert!(t.iter().any(|x| x.name() == "git"));
        let t2 = reg(&[], false, false, false);
        assert!(!t2.iter().any(|x| x.name() == "git"));
    }

    #[test]
    fn plan_mode_registry_has_reads_only() {
        let t = reg(&["echo *".into()], true, true, true);
        assert!(t.iter().any(|x| x.name() == "read_file"));
        assert!(t.iter().any(|x| x.name() == "list_directory"));
        assert!(t.iter().any(|x| x.name() == "search"));
        assert!(t.iter().any(|x| x.name() == "web_fetch"));
        assert!(!t.iter().any(|x| x.name() == "write_file"));
        assert!(!t.iter().any(|x| x.name() == "shell"));
        assert!(!t.iter().any(|x| x.name() == "git"));
    }

    #[test]
    fn run_report_json_has_expected_shape() {
        let report = RunReport {
            session_id: "550e8400-e29b-41d4-a716-446655440000".into(),
            status: "success",
            exit_reason: ExitReason::Completed,
            result: "hello".into(),
            tool_calls: vec![ToolCallSummary {
                name: "read_file".into(),
                success: true,
                message: "ok".into(),
            }],
            error: None,
            audit_log_path: "/tmp/x.jsonl".into(),
            usage: RunUsageSummary {
                total_input_tokens: 10,
                total_cache_read_tokens: 3,
                total_output_tokens: 7,
            },
            cost_usd: 0.01,
            cache_read_tokens: 3,
            files_written: vec!["src/main.rs".into()],
        };
        let v = serde_json::to_value(&report).expect("serialize");
        assert_eq!(v["session_id"], "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(v["status"], "success");
        assert_eq!(v["exit_reason"], "completed");
        assert_eq!(v["result"], "hello");
        assert!(v["error"].is_null());
        assert_eq!(v["audit_log_path"], "/tmp/x.jsonl");
        assert_eq!(v["usage"]["total_input_tokens"], 10);
        assert_eq!(v["usage"]["total_cache_read_tokens"], 3);
        assert_eq!(v["usage"]["total_output_tokens"], 7);
        assert_eq!(v["cost_usd"], 0.01);
        assert_eq!(v["cache_read_tokens"], 3);
        let files = v["files_written"].as_array().expect("files");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], "src/main.rs");
        let tools = v["tool_calls"].as_array().expect("array");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "read_file");
        assert_eq!(tools[0]["success"], true);
        assert_eq!(tools[0]["message"], "ok");
    }
}
