//! Akmon CLI — project discovery, optional `AKMON.md`, and headless `--task` runs.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use akmon_core::{
    write_audit_jsonl, AgentConfig, AgentError, AgentEvent, McpServerConfig, PolicyEngine,
    PolicyEngineMode, PolicyVerdict, Sandbox, Secret,
};
use akmon_models::{AnthropicBackend, LlmProvider, OllamaBackend};
use akmon_query::{AgentSession, ToolCallSummary};
use akmon_tools::{
    discover_mcp_tools, EditTool, ListDirectoryTool, PatchTool, ReadFileTool, SearchTool, ShellTool,
    WebFetchTool, WriteFileTool,
};
use clap::{Parser, ValueEnum};
use serde::Serialize;
use tokio::sync::mpsc;

/// Returns true if `path` is an existing `.git` directory or gitdir file (worktrees).
fn git_working_tree_marker_present(git_path: &Path) -> bool {
    match std::fs::symlink_metadata(git_path) {
        Ok(m) => m.is_dir() || m.is_file(),
        Err(_) => false,
    }
}

/// Walks upward from `root` until a `.git` file or directory is found.
///
/// If the environment variable `AKMON_DEBUG_GIT` is set (any value), prints each directory checked
/// and whether a `.git` marker was found to stderr (for troubleshooting discovery).
fn walk_up_for_git(mut root: PathBuf) -> Option<PathBuf> {
    let debug_git = std::env::var_os("AKMON_DEBUG_GIT").is_some();
    loop {
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
}

fn push_git_walk_start(candidates: &mut Vec<PathBuf>, p: PathBuf) {
    if !candidates.iter().any(|c| c == &p) {
        candidates.push(p);
    }
}

/// Walks upward from `start` looking for a `.git` file or directory. Returns
/// the directory that contains `.git`, or [`None`] if none is found.
///
/// Tries several starting paths (`dunce::canonicalize`, [`Path::canonicalize`], and the logical
/// absolute path from [`std::env::current_dir`]) so a repo is found when only one representation
/// resolves correctly.
fn find_git_project_root(start: &Path) -> Option<PathBuf> {
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
        if let Some(found) = walk_up_for_git(root) {
            return Some(found);
        }
    }
    None
}

/// Reads `AKMON.md` from `project_root` when the file exists.
fn load_akmon_md(project_root: &Path) -> std::io::Result<Option<String>> {
    let path = project_root.join("AKMON.md");
    if path.is_file() {
        std::fs::read_to_string(path).map(Some)
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

/// Resolved run summary for `--output json` (stable field names for automation).
#[derive(Debug, Serialize)]
struct RunReport {
    /// Session identifier ([`AgentConfig::session_id`] as hyphenated `Uuid`).
    session_id: String,
    /// `"success"` or `"error"`.
    status: &'static str,
    /// Full assistant reply text accumulated from stream deltas.
    result: String,
    /// Tool completions in chronological order.
    tool_calls: Vec<ToolCallSummary>,
    /// Present only when `status` is `"error"`; JSON `null` on success.
    error: Option<String>,
    /// Path passed to [`write_audit_jsonl`] for this run (default or `--audit-log`).
    audit_log_path: String,
}

/// Command-line interface for the Akmon agent binary.
#[derive(Parser, Debug)]
#[command(
    name = "akmon",
    version,
    about = "Local-first AI coding agent. Runs with Ollama (local) or Anthropic API. All actions are audited."
)]
struct Cli {
    /// Task string for one headless agent run (interactive TUI is not implemented yet).
    #[arg(short = 't', long = "task")]
    task: Option<String>,
    /// Model name: Ollama tag (e.g. llama3.2), or a Claude id if `ANTHROPIC_API_KEY` / `--anthropic-key` is set.
    #[arg(long = "model", default_value = "llama3.2")]
    model: String,
    /// Anthropic API key; defaults to the `ANTHROPIC_API_KEY` environment variable when unset.
    #[arg(
        long = "anthropic-key",
        env = "ANTHROPIC_API_KEY",
        hide_env_values = true,
        help = "Anthropic API key (falls back to ANTHROPIC_API_KEY env var)"
    )]
    anthropic_key: Option<String>,
    /// Base URL for the Ollama HTTP API (ignored when using Anthropic).
    #[arg(long = "ollama-url", default_value = "http://localhost:11434")]
    ollama_url: String,
    /// Auto-approve read-only tools only; writes and `shell` still require confirmation.
    #[arg(short = 'y', long = "yes")]
    yes: bool,
    /// `text`: stream tokens to the terminal; `json`: print one session summary object at the end.
    #[arg(long = "output", value_name = "FORMAT", default_value = "text", value_enum)]
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
}

/// Builds the tool registry; [`ShellTool`] is registered only when at least one `--shell-allow` pattern is set.
///
/// [`WebFetchTool`] is registered only when `web_fetch` is true (`--web-fetch`).
fn build_tool_registry(shell_allow: &[String], web_fetch: bool) -> Vec<Box<dyn akmon_tools::Tool>> {
    let mut tools: Vec<Box<dyn akmon_tools::Tool>> = vec![
        Box::new(ReadFileTool::new()),
        Box::new(WriteFileTool::new()),
        Box::new(ListDirectoryTool::new()),
        Box::new(SearchTool::new()),
        Box::new(EditTool::new()),
        Box::new(PatchTool::new()),
    ];
    if web_fetch {
        tools.push(Box::new(WebFetchTool::new()));
    }
    if !shell_allow.is_empty() {
        tools.push(Box::new(ShellTool::new(shell_allow.to_vec())));
    }
    tools
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

/// Prints [`AgentEvent`]s for the TTY and forwards interactive policy replies.
async fn run_event_printer(
    mut ev_rx: mpsc::Receiver<AgentEvent>,
    policy_tx: mpsc::Sender<PolicyVerdict>,
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
            AgentEvent::ConfirmationRequired { description } => {
                eprintln!("{description}");
                let line: String = tokio::task::spawn_blocking(|| {
                    print!("Allow? [y/N]: ");
                    let _ = std::io::stdout().flush();
                    let mut buf = String::new();
                    let _ = std::io::stdin().read_line(&mut buf);
                    buf
                })
                .await
                .unwrap_or_default();
                let verdict = if line.trim().eq_ignore_ascii_case("y") {
                    PolicyVerdict::Allow
                } else {
                    PolicyVerdict::Deny
                };
                let _ = policy_tx.send(verdict).await;
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

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("akmon: cannot read current directory: {e}");
            return ExitCode::from(2);
        }
    };

    let (project_root, git_warning) = match find_git_project_root(&cwd) {
        Some(root) => (root, None),
        None => (
            cwd.clone(),
            Some(
                "no git repository detected — using current directory as project root; \
                 sandbox boundary is weaker without a repo root",
            ),
        ),
    };

    if let Some(msg) = git_warning {
        eprintln!("akmon: warning: {msg}");
    }

    let akmon_content = match load_akmon_md(&project_root) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("akmon: failed to read AKMON.md: {e}");
            return ExitCode::from(2);
        }
    };

    let Some(task) = cli.task else {
        println!("Interactive mode not yet implemented. Use --task to run a task.");
        return ExitCode::SUCCESS;
    };

    if cli.output == OutputFormat::Text {
        eprintln!("akmon: project root: {}", project_root.display());
        match &akmon_content {
            Some(text) => eprintln!("akmon: AKMON.md loaded ({} bytes)", text.len()),
            None => eprintln!("akmon: AKMON.md not found (optional)"),
        }
    }

    let agent_config = AgentConfig::default();
    let audit_log_path = resolve_audit_log_path(
        &project_root,
        agent_config.session_id,
        cli.audit_log.clone(),
    );

    let provider: Arc<dyn LlmProvider> = match &cli.anthropic_key {
        Some(key) if cli.model.to_lowercase().starts_with("claude") => Arc::new(
            AnthropicBackend::new(Secret::new(key.clone()), cli.model.clone()),
        ),
        _ => Arc::new(OllamaBackend::new(
            cli.ollama_url.clone(),
            cli.model.clone(),
        )),
    };

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
    let sandbox = Arc::new(Sandbox::new(project_root.clone()));
    let mut tools = build_tool_registry(&cli.shell_allow, cli.web_fetch);
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
                eprintln!("akmon: MCP server {} unavailable: {e}", url);
            }
        }
    }

    let mut session = AgentSession::new(
        agent_config,
        Arc::clone(&policy),
        provider,
        tools,
        Arc::clone(&sandbox),
        akmon_content,
    );

    let (ev_tx, ev_rx) = mpsc::channel::<AgentEvent>(256);
    let (policy_tx, policy_rx) = mpsc::channel::<PolicyVerdict>(32);
    let printer = tokio::spawn(run_event_printer(ev_rx, policy_tx, cli.output));

    let mut policy_opt = Some(policy_rx);
    let run_outcome = session.run(task, ev_tx, &mut policy_opt).await;

    drop(policy_opt);

    let _ = printer.await;

    if let Err(e) = write_audit_jsonl(&audit_log_path, session.audit_events()) {
        eprintln!(
            "akmon: failed to write audit log {}: {e}",
            audit_log_path.display()
        );
    }

    let audit_log_path_str = audit_log_path.to_string_lossy().into_owned();

    if cli.output == OutputFormat::Json {
        let (status, error_opt): (&'static str, Option<String>) = match &run_outcome {
            Ok(()) => ("success", None),
            Err(e) => ("error", Some(e.to_string())),
        };
        let report = RunReport {
            session_id: session.session_id().to_string(),
            status,
            result: session.result_text().to_string(),
            tool_calls: session.tool_call_summaries().to_vec(),
            error: error_opt,
            audit_log_path: audit_log_path_str,
        };
        let json_line = match serde_json::to_string(&report) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("akmon: failed to serialize run report: {e}");
                return ExitCode::from(2);
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

    #[test]
    fn shell_tool_omitted_without_allow_flags() {
        let t = build_tool_registry(&[], false);
        assert!(!t.iter().any(|x| x.name() == "shell"));
    }

    #[test]
    fn shell_tool_registered_when_allow_patterns_present() {
        let t = build_tool_registry(&["echo *".into()], false);
        assert!(t.iter().any(|x| x.name() == "shell"));
    }

    #[test]
    fn web_fetch_tool_omitted_without_flag() {
        let t = build_tool_registry(&[], false);
        assert!(!t.iter().any(|x| x.name() == "web_fetch"));
    }

    #[test]
    fn web_fetch_tool_registered_when_flag_set() {
        let t = build_tool_registry(&[], true);
        assert!(t.iter().any(|x| x.name() == "web_fetch"));
    }

    #[test]
    fn run_report_json_has_expected_shape() {
        let report = RunReport {
            session_id: "550e8400-e29b-41d4-a716-446655440000".into(),
            status: "success",
            result: "hello".into(),
            tool_calls: vec![ToolCallSummary {
                name: "read_file".into(),
                success: true,
                message: "ok".into(),
            }],
            error: None,
            audit_log_path: "/tmp/x.jsonl".into(),
        };
        let v = serde_json::to_value(&report).expect("serialize");
        assert_eq!(v["session_id"], "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(v["status"], "success");
        assert_eq!(v["result"], "hello");
        assert!(v["error"].is_null());
        assert_eq!(v["audit_log_path"], "/tmp/x.jsonl");
        let tools = v["tool_calls"].as_array().expect("array");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "read_file");
        assert_eq!(tools[0]["success"], true);
        assert_eq!(tools[0]["message"], "ok");
    }
}
