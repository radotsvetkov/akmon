//! Forwards selected [`Cli`] options to a child `akmon` process (used by `akmon spec`).

use tokio::process::Command;

use crate::Cli;

/// Appends flags to `cmd` so a spawned child behaves like the current CLI for model and tooling.
pub(crate) fn forward_cli_for_child_process(cmd: &mut Command, cli: &Cli, auto_commit: bool) {
    cmd.arg("--model").arg(&cli.model);
    if let Some(ref k) = cli.anthropic_key {
        cmd.arg("--anthropic-key").arg(k);
    }
    if let Some(ref k) = cli.openrouter_key {
        cmd.arg("--openrouter-key").arg(k);
    }
    if let Some(ref k) = cli.openai_key {
        cmd.arg("--openai-key").arg(k);
    }
    if let Some(ref k) = cli.groq_key {
        cmd.arg("--groq-key").arg(k);
    }
    if let Some(ref e) = cli.azure_endpoint {
        cmd.arg("--azure-endpoint").arg(e);
    }
    if let Some(ref k) = cli.azure_key {
        cmd.arg("--azure-key").arg(k);
    }
    cmd.arg("--azure-api-version").arg(&cli.azure_api_version);
    if cli.bedrock {
        cmd.arg("--bedrock");
    }
    cmd.arg("--aws-region").arg(&cli.aws_region);
    if let Some(ref u) = cli.openai_compatible_url {
        cmd.arg("--openai-compatible-url").arg(u);
    }
    if let Some(ref k) = cli.openai_compatible_key {
        cmd.arg("--openai-compatible-key").arg(k);
    }
    cmd.arg("--ollama-url").arg(&cli.ollama_url);
    cmd.arg("--yes");
    if cli.web_fetch {
        cmd.arg("--web-fetch");
    }
    if cli.yes_web {
        cmd.arg("--yes-web");
    }
    for p in &cli.shell_allow {
        cmd.arg("--shell-allow").arg(p);
    }
    for u in &cli.mcp_server {
        cmd.arg("--mcp-server").arg(u);
    }
    if cli.index {
        cmd.arg("--index");
    }
    if auto_commit {
        cmd.arg("--auto-commit");
    }
    // Spec phases always use text child output so the parent can show human-oriented next steps.
    cmd.arg("--output").arg("text");
    if let Some(ref p) = cli.audit_log {
        cmd.arg("--audit-log").arg(p);
    }
    if let Some(ref s) = cli.resume_session {
        cmd.arg("--session").arg(s);
    }
}
