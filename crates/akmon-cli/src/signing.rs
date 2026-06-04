//! Post-session signing hook runner (Decision D-05).
//!
//! Akmon never embeds a signer. This module invokes a user-configured command
//! (from the trusted `~/.akmon/config.toml` `[signing]` section) with a
//! completed session's head hash, so operators can wire cosign, GPG, or any
//! tool to produce an independent, detached attestation over the tamper-evident
//! head. The command is executed via argv (no shell). The head hash is supplied
//! two ways: every `{head}` / `{session_id}` token in the configured arguments
//! is substituted, and `AKMON_SESSION_HEAD` / `AKMON_SESSION_ID` are exported to
//! the command's environment. The command is terminated if it exceeds its
//! timeout.

use std::process::Stdio;
use std::time::{Duration, Instant};

use akmon_config::SigningConfig;

/// Result of attempting to run the configured signing command.
#[derive(Debug)]
pub enum SigningOutcome {
    /// No signing command is configured; nothing was run.
    Disabled,
    /// The command was spawned and ran to completion (success or non-zero exit).
    Completed {
        /// Process exit status code, when available.
        exit_code: Option<i32>,
        /// Whether the process exited with a success status.
        success: bool,
        /// Captured stdout (UTF-8 lossy).
        stdout: String,
        /// Captured stderr (UTF-8 lossy).
        stderr: String,
        /// Wall-clock duration of the command.
        elapsed: Duration,
    },
    /// The command exceeded its timeout and was terminated.
    TimedOut {
        /// The timeout that was exceeded.
        timeout: Duration,
    },
    /// The command could not be spawned (for example, executable not found).
    SpawnError {
        /// Human-readable failure description.
        message: String,
    },
}

/// Runs the configured signing command for a completed session head.
///
/// Returns [`SigningOutcome::Disabled`] when no command is configured. Every
/// `{head}` / `{session_id}` token in the configured arguments is substituted,
/// and `AKMON_SESSION_HEAD` / `AKMON_SESSION_ID` are exported to the command's
/// environment. The command runs via argv (no shell) and is killed if it runs
/// longer than the effective timeout.
pub async fn run_signing_hook(
    config: &SigningConfig,
    session_head_hex: &str,
    session_id: &str,
) -> SigningOutcome {
    let Some((program, args)) = config.command.split_first() else {
        return SigningOutcome::Disabled;
    };

    let substituted_args: Vec<String> = args
        .iter()
        .map(|arg| {
            arg.replace("{head}", session_head_hex)
                .replace("{session_id}", session_id)
        })
        .collect();

    let timeout = Duration::from_secs(config.effective_timeout_secs());
    let started = Instant::now();

    let mut command = tokio::process::Command::new(program);
    command
        .args(&substituted_args)
        .env("AKMON_SESSION_HEAD", session_head_hex)
        .env("AKMON_SESSION_ID", session_id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // If we drop the child on timeout, ensure the OS process is reaped.
        .kill_on_drop(true);

    let child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return SigningOutcome::SpawnError {
                message: format!("failed to spawn signing command '{program}': {err}"),
            };
        }
    };

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => SigningOutcome::Completed {
            exit_code: output.status.code(),
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            elapsed: started.elapsed(),
        },
        Ok(Err(err)) => SigningOutcome::SpawnError {
            message: format!("signing command I/O error: {err}"),
        },
        // The pending future (which owns the child) is dropped here; kill_on_drop
        // terminates the process.
        Err(_elapsed) => SigningOutcome::TimedOut { timeout },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(command: &[&str], timeout_secs: Option<u64>) -> SigningConfig {
        SigningConfig {
            command: command.iter().map(|s| (*s).to_owned()).collect(),
            timeout_secs,
        }
    }

    #[tokio::test]
    async fn disabled_when_no_command() {
        let outcome = run_signing_hook(&cfg(&[], None), "deadbeef", "sid").await;
        assert!(matches!(outcome, SigningOutcome::Disabled));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn completed_success_for_true() {
        let outcome = run_signing_hook(&cfg(&["true"], Some(10)), "abc123", "sid").await;
        match outcome {
            SigningOutcome::Completed {
                success, exit_code, ..
            } => {
                assert!(success);
                assert_eq!(exit_code, Some(0));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn completed_failure_for_false() {
        let outcome = run_signing_hook(&cfg(&["false"], Some(10)), "abc123", "sid").await;
        match outcome {
            SigningOutcome::Completed { success, .. } => assert!(!success),
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn head_is_available_via_env() {
        // The script reads the head from the environment we export.
        let outcome = run_signing_hook(
            &cfg(
                &["sh", "-c", "printf '%s' \"$AKMON_SESSION_HEAD\""],
                Some(10),
            ),
            "cafef00d",
            "sid",
        )
        .await;
        match outcome {
            SigningOutcome::Completed { stdout, .. } => assert!(stdout.contains("cafef00d")),
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn head_placeholder_is_substituted_in_args() {
        // `sh -c 'printf %s "$0"' {head}` echoes the substituted positional arg.
        let outcome = run_signing_hook(
            &cfg(&["sh", "-c", "printf '%s' \"$0\"", "{head}"], Some(10)),
            "cafef00d",
            "sid",
        )
        .await;
        match outcome {
            SigningOutcome::Completed { stdout, .. } => assert_eq!(stdout, "cafef00d"),
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn times_out_for_long_running_command() {
        let outcome = run_signing_hook(&cfg(&["sleep", "30"], Some(1)), "abc", "sid").await;
        assert!(matches!(outcome, SigningOutcome::TimedOut { .. }));
    }

    #[tokio::test]
    async fn spawn_error_for_missing_program() {
        let outcome = run_signing_hook(
            &cfg(&["akmon-definitely-not-a-real-binary-xyz"], Some(5)),
            "abc",
            "sid",
        )
        .await;
        assert!(matches!(outcome, SigningOutcome::SpawnError { .. }));
    }
}
