//! Allowlisted subprocess execution without invoking a shell interpreter.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use akmon_core::Permission;
use async_trait::async_trait;
use glob::Pattern;
use serde_json::Value as JsonValue;
use tokio::task::JoinHandle;

use crate::Tool;
use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};

/// Default wall-clock limit for a single subprocess (seconds).
const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// Default cap on combined stdout+stderr bytes kept in the tool result.
const DEFAULT_MAX_OUTPUT_BYTES: usize = 524_288;

/// Bytes treated as shell metacharacters — forbidden in every argv token.
const SHELL_METACHAR_BYTES: &[u8] = b";|&`$()><\\'\"";

fn shell_placeholder_permissions() -> &'static [Permission] {
    use std::sync::OnceLock;
    static CELL: OnceLock<[Permission; 1]> = OnceLock::new();
    CELL.get_or_init(|| {
        [Permission::ExecuteCommand {
            command: String::new(),
            cwd: PathBuf::new(),
        }]
    })
    .as_slice()
}

/// Runs argv-only commands (whitespace split, no `sh -c`) that match configured glob patterns.
pub struct ShellTool {
    patterns: Vec<Pattern>,
    timeout_secs: u64,
    max_output_bytes: usize,
}

impl ShellTool {
    /// Compiles `allowlist` entries as [`glob::Pattern`] values; malformed patterns are skipped.
    pub fn new(allowlist: Vec<String>) -> Self {
        let patterns = allowlist
            .into_iter()
            .filter_map(|s| Pattern::new(&s).ok())
            .collect();
        Self {
            patterns,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }

    #[cfg(test)]
    fn with_limits(allowlist: Vec<String>, timeout_secs: u64, max_output_bytes: usize) -> Self {
        let patterns = allowlist
            .into_iter()
            .filter_map(|s| Pattern::new(&s).ok())
            .collect();
        Self {
            patterns,
            timeout_secs,
            max_output_bytes,
        }
    }

    fn allowlisted(&self, binary: &str, full_command: &str) -> bool {
        self.patterns
            .iter()
            .any(|p| p.matches(full_command) || p.matches(binary))
    }
}

fn token_metacharacter(token: &str) -> Option<char> {
    for &b in token.as_bytes() {
        if SHELL_METACHAR_BYTES.contains(&b) {
            return Some(char::from(b));
        }
    }
    None
}

fn parse_whitespace_command(cmd: &str) -> Option<(String, Vec<String>)> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    let binary = parts.first()?.to_string();
    let args: Vec<String> = parts[1..].iter().map(|s| (*s).to_string()).collect();
    Some((binary, args))
}

fn truncate_outputs(
    stdout: &[u8],
    stderr: &[u8],
    max: usize,
    notice: &str,
) -> (String, String, bool) {
    let total = stdout.len().saturating_add(stderr.len());
    if total <= max {
        return (
            String::from_utf8_lossy(stdout).into_owned(),
            String::from_utf8_lossy(stderr).into_owned(),
            false,
        );
    }
    let so_take = std::cmp::min(stdout.len(), max);
    let rest = max.saturating_sub(so_take);
    let se_take = std::cmp::min(stderr.len(), rest);
    let mut so = String::from_utf8_lossy(&stdout[..so_take]).into_owned();
    let mut se = String::from_utf8_lossy(&stderr[..se_take]).into_owned();
    if se_take < stderr.len() {
        se.push_str(notice);
    } else if so_take < stdout.len() {
        so.push_str(notice);
    } else {
        se.push_str(notice);
    }
    (so, se, true)
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Execute a shell command from the configured allowlist. Use for git, cargo, and other permitted commands."
    }

    fn required_permissions(&self) -> &[Permission] {
        shell_placeholder_permissions()
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute. Must match an allowlist pattern."
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: JsonValue, ctx: &ToolContext) -> ToolOutput {
        let cmd_line = match args.get("command").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "missing \"command\" field".into(),
                };
            }
        };

        let full_command = cmd_line.trim();
        let (binary, argv) = match parse_whitespace_command(cmd_line) {
            Some(p) => p,
            None => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "empty command".into(),
                };
            }
        };

        for t in std::iter::once(&binary).chain(argv.iter()) {
            if let Some(ch) = token_metacharacter(t) {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: format!("shell metacharacter detected in argument: {ch}"),
                };
            }
        }

        if !self.allowlisted(&binary, full_command) {
            return ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                message: format!("command not in allowlist: {full_command}"),
            };
        }

        let workdir = ctx.primary_root();
        let mut std_cmd = std::process::Command::new(&binary);
        std_cmd
            .args(&argv)
            .current_dir(&workdir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = match std_cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::SubprocessFailed,
                    message: format!("failed to spawn process: {e}"),
                };
            }
        };

        let pid = child.id();
        let join: JoinHandle<std::io::Result<std::process::Output>> =
            tokio::task::spawn_blocking(move || child.wait_with_output());

        let output = match tokio::time::timeout(Duration::from_secs(self.timeout_secs), join).await
        {
            Ok(Ok(Ok(out))) => out,
            Ok(Ok(Err(e))) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::SubprocessFailed,
                    message: format!("wait failed: {e}"),
                };
            }
            Ok(Err(e)) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::SubprocessFailed,
                    message: format!("join failed: {e}"),
                };
            }
            Err(_) => {
                if pid != 0 {
                    #[cfg(unix)]
                    {
                        let _ = std::process::Command::new("kill")
                            .args(["-9", &pid.to_string()])
                            .status();
                    }
                    #[cfg(windows)]
                    {
                        let _ = std::process::Command::new("taskkill")
                            .args(["/PID", &pid.to_string(), "/F"])
                            .status();
                    }
                }
                return ToolOutput::Error {
                    code: ToolErrorCode::SubprocessFailed,
                    message: format!("command timed out after {}s", self.timeout_secs),
                };
            }
        };

        let code = output.status.code().unwrap_or(-1);
        let notice = format!("[output truncated at {}KB]", self.max_output_bytes / 1024);
        let (stdout_s, stderr_s, truncated) = truncate_outputs(
            &output.stdout,
            &output.stderr,
            self.max_output_bytes,
            &notice,
        );

        let payload = serde_json::json!({
            "exit_code": code,
            "stdout": stdout_s,
            "stderr": stderr_s,
            "truncated": truncated,
        });

        match serde_json::to_string(&payload) {
            Ok(content) => ToolOutput::Success { content },
            Err(e) => ToolOutput::Error {
                code: ToolErrorCode::SubprocessFailed,
                message: format!("serialize result: {e}"),
            },
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx(tmp: &std::path::Path) -> ToolContext {
        let sandbox = akmon_core::Sandbox::new(tmp);
        let policy = std::sync::Arc::new(akmon_core::PolicyEngine::new(
            akmon_core::PolicyEngineMode::DenyAll,
        ));
        ToolContext::new(sandbox, policy)
    }

    #[tokio::test]
    async fn allowed_echo_runs() {
        let dir = tempfile::tempdir().expect("tmp");
        let tool = ShellTool::new(vec!["echo *".into()]);
        let out = tool
            .execute(json!({"command": "echo hello"}), &ctx(dir.path()))
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success: {out:?}");
        };
        let v: serde_json::Value = serde_json::from_str(&content).expect("json");
        assert_eq!(v["exit_code"], 0);
        let s = v["stdout"].as_str().expect("stdout");
        assert!(s.contains("hello"), "stdout={s:?}");
    }

    #[tokio::test]
    async fn not_allowlisted_rejected() {
        let dir = tempfile::tempdir().expect("tmp");
        let tool = ShellTool::new(vec!["git *".into()]);
        let out = tool
            .execute(json!({"command": "echo x"}), &ctx(dir.path()))
            .await;
        let ToolOutput::Error { message, code } = out else {
            panic!("expected error");
        };
        assert_eq!(code, ToolErrorCode::InvalidArgs);
        assert!(message.contains("not in allowlist"), "message={message}");
    }

    #[tokio::test]
    async fn metacharacter_rejected() {
        let dir = tempfile::tempdir().expect("tmp");
        let tool = ShellTool::new(vec!["echo *".into()]);
        let out = tool
            .execute(json!({"command": "echo a;b"}), &ctx(dir.path()))
            .await;
        let ToolOutput::Error { message, code } = out else {
            panic!("expected error");
        };
        assert_eq!(code, ToolErrorCode::InvalidArgs);
        assert!(
            message.contains("metacharacter detected"),
            "message={message}"
        );
    }

    #[tokio::test]
    async fn timeout_kills_and_errors() {
        let dir = tempfile::tempdir().expect("tmp");
        let tool = ShellTool::with_limits(vec!["sleep *".into()], 1, DEFAULT_MAX_OUTPUT_BYTES);
        let out = tool
            .execute(json!({"command": "sleep 5"}), &ctx(dir.path()))
            .await;
        let ToolOutput::Error { message, code } = out else {
            panic!("expected error: {out:?}");
        };
        assert_eq!(code, ToolErrorCode::SubprocessFailed);
        assert!(message.contains("timed out"), "message={message}");
    }

    #[tokio::test]
    async fn output_truncation_notice() {
        let dir = tempfile::tempdir().expect("tmp");
        let big = dir.path().join("big.bin");
        let body = vec![b'x'; 600_000];
        std::fs::write(&big, &body).expect("write");
        let tool = ShellTool::with_limits(vec!["cat *".into()], 30, 10_000);
        let name = big
            .file_name()
            .expect("big.bin should have a file name")
            .to_string_lossy();
        let cmd = format!("cat {name}");
        let out = tool
            .execute(json!({"command": cmd}), &ctx(dir.path()))
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success: {out:?}");
        };
        let v: serde_json::Value = serde_json::from_str(&content).expect("json");
        assert_eq!(v["truncated"], true);
        let combined = format!(
            "{}{}",
            v["stdout"].as_str().unwrap_or(""),
            v["stderr"].as_str().unwrap_or("")
        );
        assert!(
            combined.contains("[output truncated at 9KB]"),
            "combined={combined:?}"
        );
    }
}
