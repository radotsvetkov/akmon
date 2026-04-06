//! Native `git` tool: structured status, diff, log, and safe mutating commands inside the sandbox.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use akmon_core::AuditEvent;
use async_trait::async_trait;
use chrono::Utc;
use regex::Regex;
use serde_json::{Value as JsonValue, json};
use tokio::task::JoinHandle;

use crate::Tool;
use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};

/// Wall-clock limit for each git subprocess.
const GIT_TIMEOUT_SECS: u64 = 30;
/// Max diff/show text returned before truncation notice.
const MAX_DIFF_CHARS: usize = 8000;
/// Max `git log` lines returned.
const MAX_LOG_LINES: usize = 50;

/// Subcommands that must never be forwarded to git.
const DISALLOWED: &[&str] = &[
    "push",
    "pull",
    "fetch",
    "clone",
    "remote",
    "config",
    "credential",
    "filter-branch",
];

/// Runs git in `root` with non-interactive env; returns stdout as UTF-8 lossy string.
fn run_git_output(root: &Path, args: &[&str]) -> Result<std::process::Output, std::io::Error> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "echo")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.output()
}

async fn run_git_timeout(root: PathBuf, args: Vec<String>) -> Result<std::process::Output, String> {
    let join: JoinHandle<std::io::Result<std::process::Output>> =
        tokio::task::spawn_blocking(move || {
            let argv_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            run_git_output(&root, &argv_refs)
        });
    match tokio::time::timeout(Duration::from_secs(GIT_TIMEOUT_SECS), join).await {
        Ok(Ok(Ok(o))) => Ok(o),
        Ok(Ok(Err(e))) => Err(format!("git I/O error: {e}")),
        Ok(Err(e)) => Err(format!("git task join: {e}")),
        Err(_) => Err(format!("git timed out after {GIT_TIMEOUT_SECS}s")),
    }
}

fn output_json_or_err(out: std::process::Output, ctx: &str) -> Result<String, String> {
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let hint = err.trim();
        if hint.is_empty() {
            return Err(format!("git {ctx} failed (exit {})", out.status));
        }
        return Err(format!("git {ctx}: {hint}"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Structured git operations in the project repository.
pub struct GitTool;

impl GitTool {
    /// Creates a [`GitTool`] instance (stateless).
    pub fn new() -> Self {
        Self
    }
}

impl Default for GitTool {
    fn default() -> Self {
        Self::new()
    }
}

fn git_placeholder_permissions() -> &'static [akmon_core::Permission] {
    use std::sync::OnceLock;
    static CELL: OnceLock<[akmon_core::Permission; 1]> = OnceLock::new();
    CELL.get_or_init(|| {
        [akmon_core::Permission::ExecuteCommand {
            command: "git".into(),
            cwd: PathBuf::new(),
        }]
    })
    .as_slice()
}

fn json_string_array(args: &JsonValue) -> Vec<String> {
    args.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn validate_subcommand(sub: &str) -> Result<(), String> {
    if DISALLOWED.contains(&sub) {
        return Err(format!(
            "git {sub} is not allowed. Allowed: status, diff, log, add, commit, branch, stash, show, restore"
        ));
    }
    Ok(())
}

fn validate_paths_in_sandbox(ctx: &ToolContext, paths: &[String]) -> Result<(), ToolOutput> {
    for p in paths {
        if p.starts_with('-') {
            continue;
        }
        if ctx.resolve_path(p).is_err() {
            return Err(ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                message: format!("path escapes sandbox or is invalid: {p}"),
            });
        }
    }
    Ok(())
}

/// After a successful `edit` or `write_file`, stage and commit the touched file (best-effort).
///
/// Returns an [`AuditEvent::AgentStep`] when a commit was created or when a failure should be recorded.
pub fn try_auto_commit_after_file_tool(
    root: &Path,
    session_id: &str,
    tool_name: &str,
    args: &JsonValue,
) -> Option<AuditEvent> {
    if !matches!(tool_name, "edit" | "write_file") {
        return None;
    }
    let path = args.get("path")?.as_str()?;
    let old_str = args.get("old_str").and_then(|v| v.as_str());
    let snippet = old_str.map(collapse_for_message).unwrap_or_default();
    let file_part = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path);
    let msg_tail = if snippet.is_empty() {
        file_part.to_string()
    } else {
        format!("{file_part} — {snippet}")
    };
    let message = format!("akmon: {tool_name} {msg_tail}");

    let add_out = match run_git_output(root, &["add", "--", path]) {
        Ok(o) => o,
        Err(e) => {
            return Some(AuditEvent::AgentStep {
                session_id: session_id.to_string(),
                timestamp: Utc::now(),
                description: format!("akmon auto-commit: git add failed: {e}"),
            });
        }
    };
    if !add_out.status.success() {
        let e = String::from_utf8_lossy(&add_out.stderr);
        return Some(AuditEvent::AgentStep {
            session_id: session_id.to_string(),
            timestamp: Utc::now(),
            description: format!("akmon auto-commit: git add failed: {}", e.trim()),
        });
    }

    let commit_out = match run_git_output(root, &["commit", "-m", &message]) {
        Ok(o) => o,
        Err(e) => {
            return Some(AuditEvent::AgentStep {
                session_id: session_id.to_string(),
                timestamp: Utc::now(),
                description: format!("akmon auto-commit: git commit failed: {e}"),
            });
        }
    };
    if !commit_out.status.success() {
        let e = String::from_utf8_lossy(&commit_out.stderr);
        return Some(AuditEvent::AgentStep {
            session_id: session_id.to_string(),
            timestamp: Utc::now(),
            description: format!("akmon auto-commit: {}", e.trim()),
        });
    }

    let hash_out = run_git_output(root, &["rev-parse", "--short", "HEAD"]).ok()?;
    let hash = String::from_utf8_lossy(&hash_out.stdout).trim().to_string();
    Some(AuditEvent::AgentStep {
        session_id: session_id.to_string(),
        timestamp: Utc::now(),
        description: format!("akmon auto-commit: committed {hash}"),
    })
}

fn collapse_for_message(s: &str) -> String {
    let t: String = s.chars().filter(|c| !c.is_control()).collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    t.chars().take(60).collect()
}

fn parse_diff_stat(root: &Path, diff_args: &[String]) -> (u32, u32, u32) {
    let mut cmd_args = vec!["diff".to_string(), "--stat".to_string()];
    cmd_args.extend(diff_args.iter().cloned());
    let argv_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
    let Ok(out) = run_git_output(root, &argv_refs) else {
        return (0, 0, 0);
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let files_re = Regex::new(r"(\d+)\s+files?\s+changed").ok();
    let ins_re = Regex::new(r"(\d+)\s+insertions?\(\+\)").ok();
    let del_re = Regex::new(r"(\d+)\s+deletions?\(-\)").ok();
    let fc = files_re
        .as_ref()
        .and_then(|r| r.captures(&text))
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0);
    let ins = ins_re
        .as_ref()
        .and_then(|r| r.captures(&text))
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0);
    let del = del_re
        .as_ref()
        .and_then(|r| r.captures(&text))
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0);
    (fc, ins, del)
}

#[async_trait]
impl Tool for GitTool {
    fn name(&self) -> &str {
        "git"
    }

    fn description(&self) -> &str {
        "Run git operations in the project repository. Supports status, diff, log, add, commit, branch, and stash. Use git status before editing to understand the current state. Use git diff after editing to verify changes before committing."
    }

    fn required_permissions(&self) -> &[akmon_core::Permission] {
        git_placeholder_permissions()
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subcommand": {
                    "type": "string",
                    "description": "Git subcommand",
                    "enum": ["status", "diff", "log", "add", "commit", "branch", "stash", "show", "restore"]
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Additional arguments for the subcommand."
                },
                "message": {
                    "type": "string",
                    "description": "Commit message (required for commit)."
                }
            },
            "required": ["subcommand"]
        })
    }

    async fn execute(&self, args: JsonValue, ctx: &ToolContext) -> ToolOutput {
        let root = ctx.primary_root();
        let sub = match args.get("subcommand").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "missing \"subcommand\" field".into(),
                };
            }
        };

        if let Err(msg) = validate_subcommand(sub) {
            return ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                message: msg,
            };
        }

        let argv = json_string_array(args.get("args").unwrap_or(&JsonValue::Null));

        match sub {
            "status" => match run_git_output(&root, &["status", "--porcelain=v1"]) {
                Ok(out) => {
                    let porcelain = String::from_utf8_lossy(&out.stdout);
                    let branch_out = run_git_output(&root, &["branch", "--show-current"]).ok();
                    let branch = branch_out
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "(detached)".into());
                    let mut staged = Vec::new();
                    let mut unstaged = Vec::new();
                    let mut untracked = Vec::new();
                    for line in porcelain.lines() {
                        if line.is_empty() {
                            continue;
                        }
                        if let Some(rest) = line.strip_prefix("?? ") {
                            untracked.push(json!({"path": rest}));
                            continue;
                        }
                        if line.len() < 4 {
                            continue;
                        }
                        let xs = line.chars().take(2).collect::<String>();
                        let path = line[3..].trim();
                        let idx = xs.chars().next().unwrap_or(' ');
                        let wt = xs.chars().nth(1).unwrap_or(' ');
                        if idx != ' ' && idx != '?' {
                            staged.push(json!({"status": idx.to_string(), "path": path}));
                        }
                        if wt != ' ' && wt != '?' {
                            unstaged.push(json!({"status": wt.to_string(), "path": path}));
                        }
                    }
                    let clean = staged.is_empty() && unstaged.is_empty() && untracked.is_empty();
                    ToolOutput::Success {
                        content: serde_json::to_string_pretty(&json!({
                            "branch": branch,
                            "clean": clean,
                            "staged": staged,
                            "unstaged": unstaged,
                            "untracked": untracked,
                        }))
                        .unwrap_or_else(|_| "{}".into()),
                    }
                }
                Err(e) => ToolOutput::Error {
                    code: ToolErrorCode::SubprocessFailed,
                    message: format!("git status: {e}"),
                },
            },
            "diff" => {
                let mut gargs = vec!["diff".to_string()];
                gargs.extend(argv.clone());
                let stat = parse_diff_stat(&root, &argv);
                let out = match run_git_timeout(root.clone(), gargs.clone()).await {
                    Ok(o) => o,
                    Err(e) => {
                        return ToolOutput::Error {
                            code: ToolErrorCode::SubprocessFailed,
                            message: e,
                        };
                    }
                };
                let mut diff = match output_json_or_err(out, "diff") {
                    Ok(s) => s,
                    Err(e) => {
                        return ToolOutput::Error {
                            code: ToolErrorCode::SubprocessFailed,
                            message: e,
                        };
                    }
                };
                let truncated = diff.len() > MAX_DIFF_CHARS;
                if truncated {
                    diff.truncate(MAX_DIFF_CHARS);
                    diff.push_str("\n[diff truncated — use args to narrow scope]");
                }
                let (fc, ins, del) = stat;
                ToolOutput::Success {
                    content: serde_json::to_string_pretty(&json!({
                        "args": argv,
                        "diff": diff,
                        "truncated": truncated,
                        "files_changed": fc,
                        "insertions": ins,
                        "deletions": del,
                    }))
                    .unwrap_or_else(|_| "{}".into()),
                }
            }
            "log" => {
                let mut gargs = vec!["log".to_string()];
                if argv.is_empty() {
                    gargs.push("--oneline".into());
                    gargs.push("-10".into());
                } else {
                    gargs.extend(argv.clone());
                }
                let out = match run_git_timeout(root.clone(), gargs.clone()).await {
                    Ok(o) => o,
                    Err(e) => {
                        return ToolOutput::Error {
                            code: ToolErrorCode::SubprocessFailed,
                            message: e,
                        };
                    }
                };
                let mut log = match output_json_or_err(out, "log") {
                    Ok(s) => s,
                    Err(e) => {
                        return ToolOutput::Error {
                            code: ToolErrorCode::SubprocessFailed,
                            message: e,
                        };
                    }
                };
                let lines: Vec<&str> = log.lines().take(MAX_LOG_LINES).collect();
                let truncated = log.lines().count() > MAX_LOG_LINES;
                log = lines.join("\n");
                ToolOutput::Success {
                    content: serde_json::to_string_pretty(&json!({
                        "args": if argv.is_empty() {
                            json!(["--oneline", "-10"])
                        } else {
                            json!(argv)
                        },
                        "log": log,
                        "truncated": truncated,
                    }))
                    .unwrap_or_else(|_| "{}".into()),
                }
            }
            "add" => {
                if argv.iter().any(|a| a == "-p" || a == "--patch") {
                    return ToolOutput::Error {
                        code: ToolErrorCode::InvalidArgs,
                        message: "Interactive git add is not supported. Pass specific file paths instead."
                            .into(),
                    };
                }
                let paths: Vec<String> = argv
                    .iter()
                    .filter(|a| !a.starts_with('-'))
                    .cloned()
                    .collect();
                if let Err(e) = validate_paths_in_sandbox(ctx, &paths) {
                    return e;
                }
                let mut gargs = vec!["add".to_string()];
                gargs.extend(argv.clone());
                let out = match run_git_timeout(root.clone(), gargs).await {
                    Ok(o) => o,
                    Err(e) => {
                        return ToolOutput::Error {
                            code: ToolErrorCode::SubprocessFailed,
                            message: e,
                        };
                    }
                };
                if let Err(e) = output_json_or_err(out, "add") {
                    return ToolOutput::Error {
                        code: ToolErrorCode::SubprocessFailed,
                        message: e,
                    };
                }
                ToolOutput::Success {
                    content: serde_json::to_string_pretty(&json!({
                        "staged": paths,
                        "success": true,
                    }))
                    .unwrap_or_else(|_| "{}".into()),
                }
            }
            "commit" => {
                let msg = match args.get("message").and_then(|v| v.as_str()) {
                    Some(m) if !m.trim().is_empty() => m.to_string(),
                    _ => {
                        return ToolOutput::Error {
                            code: ToolErrorCode::InvalidArgs,
                            message: "commit requires a message field".into(),
                        };
                    }
                };
                let st = match run_git_output(&root, &["status", "--porcelain"]) {
                    Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
                    Err(e) => {
                        return ToolOutput::Error {
                            code: ToolErrorCode::SubprocessFailed,
                            message: format!("git status: {e}"),
                        };
                    }
                };
                let staged_lines = st
                    .lines()
                    .filter(|l| !l.starts_with("??") && !l.is_empty())
                    .count();
                if staged_lines == 0 {
                    return ToolOutput::Error {
                        code: ToolErrorCode::InvalidArgs,
                        message: format!(
                            "nothing staged. Use git add to stage files first. Current status: {}",
                            if st.trim().is_empty() {
                                "clean"
                            } else {
                                "has unstaged/untracked"
                            }
                        ),
                    };
                }
                let out = match run_git_timeout(
                    root.clone(),
                    vec!["commit".into(), "-m".into(), msg.clone()],
                )
                .await
                {
                    Ok(o) => o,
                    Err(e) => {
                        return ToolOutput::Error {
                            code: ToolErrorCode::SubprocessFailed,
                            message: e,
                        };
                    }
                };
                if let Err(e) = output_json_or_err(out, "commit") {
                    return ToolOutput::Error {
                        code: ToolErrorCode::SubprocessFailed,
                        message: e,
                    };
                }
                let hash_out = run_git_output(&root, &["rev-parse", "--short", "HEAD"]).ok();
                let hash = hash_out
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();
                let br = run_git_output(&root, &["branch", "--show-current"]).ok();
                let branch = br
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();
                let stat = parse_diff_stat(&root, &["HEAD~1".into(), "HEAD".into()]);
                let (fc, ins, del) = stat;
                ToolOutput::Success {
                    content: serde_json::to_string_pretty(&json!({
                        "hash": hash,
                        "branch": branch,
                        "message": msg,
                        "files_changed": fc,
                        "insertions": ins,
                        "deletions": del,
                    }))
                    .unwrap_or_else(|_| "{}".into()),
                }
            }
            "branch" => {
                let mut gargs = vec!["branch".to_string()];
                gargs.extend(argv.clone());
                let out = match run_git_timeout(root.clone(), gargs).await {
                    Ok(o) => o,
                    Err(e) => {
                        return ToolOutput::Error {
                            code: ToolErrorCode::SubprocessFailed,
                            message: e,
                        };
                    }
                };
                let stdout = match output_json_or_err(out, "branch") {
                    Ok(s) => s,
                    Err(e) => {
                        return ToolOutput::Error {
                            code: ToolErrorCode::SubprocessFailed,
                            message: e,
                        };
                    }
                };
                if argv.is_empty() {
                    let cur_out = run_git_output(&root, &["branch", "--show-current"]).ok();
                    let current = cur_out
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .unwrap_or_default();
                    let branches: Vec<String> = stdout
                        .lines()
                        .filter_map(|l| {
                            let t = l.trim();
                            if t.starts_with('*') {
                                Some(t.trim_start_matches('*').trim().to_string())
                            } else if !t.is_empty() {
                                Some(t.to_string())
                            } else {
                                None
                            }
                        })
                        .collect();
                    ToolOutput::Success {
                        content: serde_json::to_string_pretty(&json!({
                            "current": current,
                            "branches": branches,
                        }))
                        .unwrap_or_else(|_| "{}".into()),
                    }
                } else {
                    let action = if argv.iter().any(|a| a == "-d" || a == "-D") {
                        "deleted"
                    } else {
                        "created"
                    };
                    let bname = argv
                        .iter()
                        .find(|a| !a.starts_with('-'))
                        .cloned()
                        .unwrap_or_default();
                    ToolOutput::Success {
                        content: serde_json::to_string_pretty(&json!({
                            "success": true,
                            "action": action,
                            "branch": bname,
                        }))
                        .unwrap_or_else(|_| "{}".into()),
                    }
                }
            }
            "stash" => {
                let mut gargs = vec!["stash".to_string()];
                gargs.extend(argv.clone());
                let out = match run_git_timeout(root.clone(), gargs).await {
                    Ok(o) => o,
                    Err(e) => {
                        return ToolOutput::Error {
                            code: ToolErrorCode::SubprocessFailed,
                            message: e,
                        };
                    }
                };
                let text = match output_json_or_err(out, "stash") {
                    Ok(s) => s,
                    Err(e) => {
                        return ToolOutput::Error {
                            code: ToolErrorCode::SubprocessFailed,
                            message: e,
                        };
                    }
                };
                let action = argv.first().map(|s| s.as_str()).unwrap_or("push");
                ToolOutput::Success {
                    content: serde_json::to_string_pretty(&json!({
                        "success": true,
                        "action": action,
                        "output": text.trim(),
                    }))
                    .unwrap_or_else(|_| "{}".into()),
                }
            }
            "show" => {
                let mut gargs = vec!["show".to_string(), "--stat".to_string()];
                if argv.is_empty() {
                    gargs.push("HEAD".into());
                } else {
                    gargs.extend(argv.clone());
                }
                let out = match run_git_timeout(root.clone(), gargs.clone()).await {
                    Ok(o) => o,
                    Err(e) => {
                        return ToolOutput::Error {
                            code: ToolErrorCode::SubprocessFailed,
                            message: e,
                        };
                    }
                };
                let mut stat_text = match output_json_or_err(out, "show") {
                    Ok(s) => s,
                    Err(e) => {
                        return ToolOutput::Error {
                            code: ToolErrorCode::SubprocessFailed,
                            message: e,
                        };
                    }
                };
                let truncated = stat_text.len() > MAX_DIFF_CHARS;
                if truncated {
                    stat_text.truncate(MAX_DIFF_CHARS);
                    stat_text.push_str("\n[diff truncated — use args to narrow scope]");
                }
                let commit = argv.first().cloned().unwrap_or_else(|| "HEAD".into());
                ToolOutput::Success {
                    content: serde_json::to_string_pretty(&json!({
                        "commit": commit,
                        "author": "",
                        "date": "",
                        "message": "",
                        "diff": stat_text,
                        "truncated": truncated,
                    }))
                    .unwrap_or_else(|_| "{}".into()),
                }
            }
            "restore" => {
                let paths: Vec<String> = argv
                    .iter()
                    .filter(|a| !a.starts_with('-'))
                    .cloned()
                    .collect();
                if paths.is_empty() {
                    return ToolOutput::Error {
                        code: ToolErrorCode::InvalidArgs,
                        message: "restore requires at least one file path in args".into(),
                    };
                }
                if let Err(e) = validate_paths_in_sandbox(ctx, &paths) {
                    return e;
                }
                let mut gargs = vec!["restore".to_string()];
                gargs.extend(argv.clone());
                let out = match run_git_timeout(root.clone(), gargs).await {
                    Ok(o) => o,
                    Err(e) => {
                        return ToolOutput::Error {
                            code: ToolErrorCode::SubprocessFailed,
                            message: e,
                        };
                    }
                };
                if let Err(e) = output_json_or_err(out, "restore") {
                    return ToolOutput::Error {
                        code: ToolErrorCode::SubprocessFailed,
                        message: e,
                    };
                }
                ToolOutput::Success {
                    content: serde_json::to_string_pretty(&json!({
                        "restored": paths,
                        "success": true,
                    }))
                    .unwrap_or_else(|_| "{}".into()),
                }
            }
            _ => ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                message: format!("unsupported subcommand: {sub}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_core::PolicyEngine;
    use serde_json::json;

    fn repo_with_commit() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        let _ = run_git_output(root, &["init"]).expect("init");
        let _ = run_git_output(root, &["config", "user.email", "t@t"]).expect("cfg");
        let _ = run_git_output(root, &["config", "user.name", "t"]).expect("cfg");
        std::fs::write(root.join("f.txt"), "a\n").expect("w");
        let _ = run_git_output(root, &["add", "f.txt"]).expect("add");
        let _ = run_git_output(root, &["commit", "-m", "init"]).expect("commit");
        dir
    }

    fn ctx(root: &Path) -> ToolContext {
        ToolContext::new(
            akmon_core::Sandbox::new(root),
            std::sync::Arc::new(PolicyEngine::new(akmon_core::PolicyEngineMode::DenyAll)),
        )
    }

    #[tokio::test]
    async fn git_status_structured_json() {
        let dir = repo_with_commit();
        let t = GitTool::new();
        let out = t
            .execute(json!({"subcommand": "status"}), &ctx(dir.path()))
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success");
        };
        assert!(content.contains("\"branch\""));
        assert!(content.contains("\"clean\""));
    }

    #[tokio::test]
    async fn git_diff_returns_text() {
        let dir = repo_with_commit();
        std::fs::write(dir.path().join("f.txt"), "b\n").expect("w");
        let t = GitTool::new();
        let out = t
            .execute(json!({"subcommand": "diff"}), &ctx(dir.path()))
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success");
        };
        assert!(content.contains("\"diff\""));
    }

    #[tokio::test]
    async fn git_log_defaults_to_ten_lines() {
        let dir = repo_with_commit();
        let t = GitTool::new();
        let out = t
            .execute(json!({"subcommand": "log"}), &ctx(dir.path()))
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success");
        };
        assert!(content.contains("--oneline"));
    }

    #[tokio::test]
    async fn git_add_rejects_patch_mode() {
        let dir = repo_with_commit();
        let t = GitTool::new();
        let out = t
            .execute(
                json!({"subcommand": "add", "args": ["-p"]}),
                &ctx(dir.path()),
            )
            .await;
        let ToolOutput::Error { message, .. } = out else {
            panic!("expected err");
        };
        assert!(message.contains("Interactive git add"));
    }

    #[tokio::test]
    async fn git_commit_requires_message() {
        let dir = repo_with_commit();
        let t = GitTool::new();
        let out = t
            .execute(json!({"subcommand": "commit"}), &ctx(dir.path()))
            .await;
        let ToolOutput::Error { message, .. } = out else {
            panic!("expected err");
        };
        assert!(message.contains("message"));
    }

    #[tokio::test]
    async fn git_commit_nothing_staged_errors() {
        let dir = repo_with_commit();
        let t = GitTool::new();
        let out = t
            .execute(
                json!({"subcommand": "commit", "message": "x"}),
                &ctx(dir.path()),
            )
            .await;
        let ToolOutput::Error { message, .. } = out else {
            panic!("expected err");
        };
        assert!(message.contains("nothing staged"));
    }

    #[tokio::test]
    async fn git_push_disallowed() {
        let dir = repo_with_commit();
        let t = GitTool::new();
        let out = t
            .execute(json!({"subcommand": "push"}), &ctx(dir.path()))
            .await;
        let ToolOutput::Error { message, .. } = out else {
            panic!("expected err");
        };
        assert!(message.contains("not allowed"));
    }

    #[tokio::test]
    async fn git_add_validates_sandbox_path() {
        let dir = repo_with_commit();
        let t = GitTool::new();
        let out = t
            .execute(
                json!({"subcommand": "add", "args": ["../escape.rs"]}),
                &ctx(dir.path()),
            )
            .await;
        assert!(matches!(out, ToolOutput::Error { .. }));
    }

    #[test]
    fn auto_commit_creates_commit_after_write() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        let _ = run_git_output(root, &["init"]).expect("init");
        let _ = run_git_output(root, &["config", "user.email", "t@t"]).expect("cfg");
        let _ = run_git_output(root, &["config", "user.name", "t"]).expect("cfg");
        std::fs::write(root.join("a.rs"), "//x\n").expect("w");
        let ev =
            try_auto_commit_after_file_tool(root, "sid", "write_file", &json!({"path": "a.rs"}));
        assert!(ev.is_some());
        let log = run_git_output(root, &["log", "--oneline", "-1"]).expect("log");
        let s = String::from_utf8_lossy(&log.stdout);
        assert!(s.contains("akmon:"));
    }
}
