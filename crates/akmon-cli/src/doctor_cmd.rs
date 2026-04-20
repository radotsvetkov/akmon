//! `akmon doctor` diagnostics for provider operability.

use std::process::ExitCode;
use std::time::Duration;

use akmon_models::LlmConnectConfig;
use clap::Subcommand;
use serde::Serialize;
use serde_json::json;

/// Top-level `akmon doctor …` options.
#[derive(Debug, Clone, clap::Args)]
pub struct DoctorArgs {
    /// Doctor subcommand.
    #[command(subcommand)]
    pub cmd: DoctorSubcommand,
}

/// Supported `akmon doctor` subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum DoctorSubcommand {
    /// Diagnose configured/active provider health and remediation hints.
    Providers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CheckStatus {
    Pass,
    Warn,
    Fail,
    Skip,
}

#[derive(Debug, Clone, Serialize)]
struct CheckResult {
    name: String,
    status: CheckStatus,
    detail: String,
}

#[derive(Debug, Clone, Serialize)]
struct ModelHints {
    available: bool,
    detail: String,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderDiagnosis {
    provider: String,
    configured: bool,
    active: bool,
    healthy: bool,
    critical: bool,
    checks: Vec<CheckResult>,
    remediation: Vec<String>,
    model_hints: Option<ModelHints>,
}

#[derive(Debug, Clone, Serialize)]
struct DoctorProvidersReport {
    ok: bool,
    active_provider: Option<String>,
    providers: Vec<ProviderDiagnosis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderKind {
    Ollama,
    OpenAiCompatible,
    OpenAi,
    OpenRouter,
    AzureOpenAi,
    Bedrock,
}

impl ProviderKind {
    fn name(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::OpenAiCompatible => "openai_compatible",
            Self::OpenAi => "openai",
            Self::OpenRouter => "openrouter",
            Self::AzureOpenAi => "azure_openai",
            Self::Bedrock => "bedrock",
        }
    }
}

#[derive(Debug, Clone)]
struct ReachabilityProbe {
    name: String,
    url: String,
    auth_header: Option<String>,
    extra_headers: Vec<(String, String)>,
}

/// Runs one `akmon doctor` invocation.
pub async fn run_doctor(
    args: DoctorArgs,
    json_output: bool,
    connect: &LlmConnectConfig,
) -> ExitCode {
    match args.cmd {
        DoctorSubcommand::Providers => {
            let report = diagnose_providers(connect).await;
            if json_output {
                println!("{}", render_doctor_json(&report));
            } else {
                print_doctor_text(&report);
            }
            if report.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
    }
}

fn render_doctor_json(report: &DoctorProvidersReport) -> serde_json::Value {
    json!(report)
}

fn print_doctor_text(report: &DoctorProvidersReport) {
    let overall = if report.ok { "healthy" } else { "unhealthy" };
    match &report.active_provider {
        Some(active) => println!("doctor providers: {overall} (active={active})"),
        None => println!("doctor providers: {overall}"),
    }
    for p in &report.providers {
        let state = if p.healthy { "ok" } else { "issues" };
        let active = if p.active { ", active" } else { "" };
        println!("\n- {}: {}{}", p.provider, state, active);
        for c in &p.checks {
            let marker = match c.status {
                CheckStatus::Pass => "✓",
                CheckStatus::Warn => "!",
                CheckStatus::Fail => "✗",
                CheckStatus::Skip => "~",
            };
            println!("  {marker} {}: {}", c.name, c.detail);
        }
        if !p.remediation.is_empty() {
            println!("  remediation:");
            for hint in &p.remediation {
                println!("   - {hint}");
            }
        }
        if let Some(h) = &p.model_hints {
            println!("  model_hints: {}", h.detail);
        }
    }
}

async fn diagnose_providers(connect: &LlmConnectConfig) -> DoctorProvidersReport {
    let active = map_active_provider(&connect.inferred_backend_name().to_lowercase());
    let mut providers = Vec::new();
    for kind in [
        ProviderKind::Ollama,
        ProviderKind::OpenAiCompatible,
        ProviderKind::OpenAi,
        ProviderKind::OpenRouter,
        ProviderKind::AzureOpenAi,
        ProviderKind::Bedrock,
    ] {
        let mut diagnosis = diagnose_provider_static(connect, kind, active == Some(kind));
        if let Some(probe) = diagnosis_probe(connect, kind) {
            apply_probe_result(&mut diagnosis, &probe, probe_http(&probe).await);
        }
        diagnosis.healthy = diagnosis
            .checks
            .iter()
            .all(|c| !matches!(c.status, CheckStatus::Fail));
        providers.push(diagnosis);
    }
    let mut ok = true;
    for p in &providers {
        if p.active && p.critical && !p.healthy {
            ok = false;
        }
    }
    DoctorProvidersReport {
        ok,
        active_provider: active.map(|k| k.name().to_string()),
        providers,
    }
}

fn map_active_provider(name: &str) -> Option<ProviderKind> {
    match name {
        "ollama" => Some(ProviderKind::Ollama),
        "openai-compatible" => Some(ProviderKind::OpenAiCompatible),
        "openai" => Some(ProviderKind::OpenAi),
        "openrouter" => Some(ProviderKind::OpenRouter),
        "azure openai" => Some(ProviderKind::AzureOpenAi),
        "aws bedrock" => Some(ProviderKind::Bedrock),
        _ => None,
    }
}

fn diagnosis_probe(
    connect: &LlmConnectConfig,
    provider: ProviderKind,
) -> Option<ReachabilityProbe> {
    match provider {
        ProviderKind::Ollama => Some(ReachabilityProbe {
            name: "endpoint_reachability".into(),
            url: format!("{}/api/tags", trim_slash(&connect.ollama_url)),
            auth_header: None,
            extra_headers: Vec::new(),
        }),
        ProviderKind::OpenAiCompatible => {
            let base = connect.openai_compatible_url.as_ref()?.trim();
            Some(ReachabilityProbe {
                name: "endpoint_reachability".into(),
                url: format!("{}/models", trim_slash(base)),
                auth_header: connect
                    .openai_compatible_api_key
                    .as_ref()
                    .map(|k| format!("Bearer {k}")),
                extra_headers: Vec::new(),
            })
        }
        ProviderKind::OpenAi => Some(ReachabilityProbe {
            name: "endpoint_reachability".into(),
            url: "https://api.openai.com/v1/models".into(),
            auth_header: connect
                .openai_api_key
                .as_ref()
                .map(|k| format!("Bearer {k}")),
            extra_headers: Vec::new(),
        }),
        ProviderKind::OpenRouter => Some(ReachabilityProbe {
            name: "endpoint_reachability".into(),
            url: "https://openrouter.ai/api/v1/models".into(),
            auth_header: connect
                .openrouter_api_key
                .as_ref()
                .map(|k| format!("Bearer {k}")),
            extra_headers: Vec::new(),
        }),
        ProviderKind::AzureOpenAi => {
            let ep = connect.azure_openai_endpoint.as_ref()?.trim();
            let sep = if ep.contains('?') { "&" } else { "?" };
            Some(ReachabilityProbe {
                name: "endpoint_reachability".into(),
                url: format!("{ep}{sep}api-version={}", connect.azure_api_version),
                auth_header: None,
                extra_headers: connect
                    .azure_openai_api_key
                    .as_ref()
                    .map(|k| vec![("api-key".to_string(), k.clone())])
                    .unwrap_or_default(),
            })
        }
        ProviderKind::Bedrock => None,
    }
}

async fn probe_http(probe: &ReachabilityProbe) -> Result<(u16, String), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.get(&probe.url);
    if let Some(auth) = &probe.auth_header {
        req = req.header("Authorization", auth);
    }
    for (k, v) in &probe.extra_headers {
        req = req.header(k, v);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let code = resp.status().as_u16();
    let body_hint = if code >= 400 {
        "received HTTP error response".to_string()
    } else {
        "reachable".to_string()
    };
    Ok((code, body_hint))
}

fn apply_probe_result(
    diagnosis: &mut ProviderDiagnosis,
    probe: &ReachabilityProbe,
    result: Result<(u16, String), String>,
) {
    let safe_url = sanitize_url_for_display(&probe.url);
    let (status, detail) = match result {
        Ok((code, hint)) => {
            if code < 500 {
                (CheckStatus::Pass, format!("{safe_url} ({code}, {hint})"))
            } else {
                (
                    CheckStatus::Fail,
                    format!("{safe_url} (HTTP {code}, server-side failure)"),
                )
            }
        }
        Err(_) => (
            CheckStatus::Fail,
            format!("{safe_url} (unreachable: network request failed)"),
        ),
    };
    diagnosis.checks.push(CheckResult {
        name: probe.name.clone(),
        status,
        detail,
    });
}

fn diagnose_provider_static(
    connect: &LlmConnectConfig,
    provider: ProviderKind,
    active: bool,
) -> ProviderDiagnosis {
    let mut checks = Vec::new();
    let mut remediation = Vec::new();
    let mut model_hints = None;
    let configured;
    let critical;

    match provider {
        ProviderKind::Ollama => {
            configured = true;
            critical = active;
            checks.push(CheckResult {
                name: "base_url".into(),
                status: endpoint_sanity(&connect.ollama_url)
                    .map(|_| CheckStatus::Pass)
                    .unwrap_or(CheckStatus::Fail),
                detail: match endpoint_sanity(&connect.ollama_url) {
                    Ok(_) => format!(
                        "{} (valid URL)",
                        sanitize_url_for_display(&connect.ollama_url)
                    ),
                    Err(e) => format!("{} ({e})", sanitize_url_for_display(&connect.ollama_url)),
                },
            });
            model_hints = Some(ModelHints {
                available: true,
                detail: "available via /api/tags reachability check".into(),
            });
            remediation.push("If unreachable, start Ollama with `ollama serve`.".into());
        }
        ProviderKind::OpenAiCompatible => {
            let has_url = connect
                .openai_compatible_url
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty());
            let has_key = connect
                .openai_compatible_api_key
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty());
            configured = has_url || has_key;
            critical = active;
            checks.push(CheckResult {
                name: "api_key".into(),
                status: if has_key {
                    CheckStatus::Pass
                } else if configured {
                    CheckStatus::Fail
                } else {
                    CheckStatus::Skip
                },
                detail: if has_key {
                    "set (masked)".into()
                } else if configured {
                    "missing key for configured endpoint".into()
                } else {
                    "not configured".into()
                },
            });
            checks.push(CheckResult {
                name: "base_url".into(),
                status: if !has_url {
                    CheckStatus::Skip
                } else if let Some(url) = &connect.openai_compatible_url {
                    if endpoint_sanity(url).is_ok() {
                        CheckStatus::Pass
                    } else {
                        CheckStatus::Fail
                    }
                } else {
                    CheckStatus::Skip
                },
                detail: connect
                    .openai_compatible_url
                    .as_ref()
                    .map(|u| match endpoint_sanity(u) {
                        Ok(_) => format!("{} (valid URL)", sanitize_url_for_display(u)),
                        Err(e) => format!("{} ({e})", sanitize_url_for_display(u)),
                    })
                    .unwrap_or_else(|| "not configured".into()),
            });
            remediation.push("Set both `--openai-compatible-url` and `--openai-compatible-key` (or config equivalents).".into());
        }
        ProviderKind::OpenAi => {
            let has_key = connect
                .openai_api_key
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty());
            configured = has_key;
            critical = active;
            checks.push(CheckResult {
                name: "api_key".into(),
                status: if has_key {
                    CheckStatus::Pass
                } else if active {
                    CheckStatus::Fail
                } else {
                    CheckStatus::Skip
                },
                detail: if has_key {
                    "set (masked)".into()
                } else {
                    "missing OPENAI_API_KEY".into()
                },
            });
            model_hints = Some(ModelHints {
                available: true,
                detail: "available via /v1/models check".into(),
            });
            remediation.push("Set `OPENAI_API_KEY` or `--openai-key`.".into());
        }
        ProviderKind::OpenRouter => {
            let has_key = connect
                .openrouter_api_key
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty());
            configured = has_key;
            critical = active;
            checks.push(CheckResult {
                name: "api_key".into(),
                status: if has_key {
                    CheckStatus::Pass
                } else if active {
                    CheckStatus::Fail
                } else {
                    CheckStatus::Skip
                },
                detail: if has_key {
                    "set (masked)".into()
                } else {
                    "missing OPENROUTER_API_KEY".into()
                },
            });
            model_hints = Some(ModelHints {
                available: true,
                detail: "available via /api/v1/models check".into(),
            });
            remediation.push(
                "Set `OPENROUTER_API_KEY` and use slash model ids (e.g. `anthropic/claude-haiku-4-5`)."
                    .into(),
            );
        }
        ProviderKind::AzureOpenAi => {
            let has_endpoint = connect
                .azure_openai_endpoint
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty());
            let has_key = connect
                .azure_openai_api_key
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty());
            configured = has_endpoint || has_key;
            critical = active;
            checks.push(CheckResult {
                name: "endpoint".into(),
                status: if !has_endpoint {
                    if configured {
                        CheckStatus::Fail
                    } else {
                        CheckStatus::Skip
                    }
                } else if let Some(endpoint) = &connect.azure_openai_endpoint {
                    if azure_endpoint_sanity(endpoint).is_ok() {
                        CheckStatus::Pass
                    } else {
                        CheckStatus::Fail
                    }
                } else {
                    CheckStatus::Skip
                },
                detail: connect
                    .azure_openai_endpoint
                    .as_ref()
                    .map(|ep| match azure_endpoint_sanity(ep) {
                        Ok(_) => format!(
                            "{} (valid Azure deployment URL)",
                            sanitize_url_for_display(ep)
                        ),
                        Err(e) => format!("{} ({e})", sanitize_url_for_display(ep)),
                    })
                    .unwrap_or_else(|| "not configured".into()),
            });
            checks.push(CheckResult {
                name: "api_key".into(),
                status: if has_key {
                    CheckStatus::Pass
                } else if configured || active {
                    CheckStatus::Fail
                } else {
                    CheckStatus::Skip
                },
                detail: if has_key {
                    "set (masked)".into()
                } else {
                    "missing AZURE_OPENAI_API_KEY / --azure-key".into()
                },
            });
            checks.push(CheckResult {
                name: "api_version".into(),
                status: if connect.azure_api_version.trim().is_empty() {
                    CheckStatus::Warn
                } else {
                    CheckStatus::Pass
                },
                detail: if connect.azure_api_version.trim().is_empty() {
                    "empty api-version may fail requests".into()
                } else {
                    format!("{} (configured)", connect.azure_api_version)
                },
            });
            remediation.push(
                "Use deployment endpoint ending in `/openai/deployments/<name>/chat/completions`."
                    .into(),
            );
            remediation.push("Set `AZURE_OPENAI_API_KEY` and confirm `--azure-api-version` matches your deployment.".into());
        }
        ProviderKind::Bedrock => {
            let aws_id = std::env::var("AWS_ACCESS_KEY_ID").ok();
            let aws_secret = std::env::var("AWS_SECRET_ACCESS_KEY").ok();
            let configured = aws_id.as_ref().is_some_and(|v| !v.trim().is_empty())
                && aws_secret.as_ref().is_some_and(|v| !v.trim().is_empty());
            critical = active || connect.bedrock_explicit;
            checks.push(CheckResult {
                name: "aws_credentials".into(),
                status: if configured {
                    CheckStatus::Pass
                } else if critical {
                    CheckStatus::Fail
                } else {
                    CheckStatus::Skip
                },
                detail: if configured {
                    "AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY detected (masked)".into()
                } else {
                    "missing AWS credentials for Bedrock".into()
                },
            });
            checks.push(CheckResult {
                name: "region".into(),
                status: if connect.aws_region.trim().is_empty() {
                    CheckStatus::Fail
                } else {
                    CheckStatus::Pass
                },
                detail: if connect.aws_region.trim().is_empty() {
                    "missing AWS region".into()
                } else {
                    format!("{} (configured)", connect.aws_region)
                },
            });
            remediation.push(
                "Set AWS credentials and region (e.g. `AWS_DEFAULT_REGION=us-east-1`).".into(),
            );
            remediation.push(
                "If you use temporary credentials, ensure session token env vars are exported."
                    .into(),
            );
            return ProviderDiagnosis {
                provider: provider.name().into(),
                configured,
                active,
                healthy: false,
                critical,
                checks,
                remediation,
                model_hints: Some(ModelHints {
                    available: false,
                    detail: "model list probe skipped (Bedrock uses AWS auth + SDK pathways)"
                        .into(),
                }),
            };
        }
    }

    ProviderDiagnosis {
        provider: provider.name().into(),
        configured,
        active,
        healthy: false,
        critical,
        checks,
        remediation,
        model_hints,
    }
}

fn endpoint_sanity(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("unsupported scheme `{other}`")),
    }
    if parsed.host_str().is_none() {
        return Err("missing host".into());
    }
    Ok(())
}

fn azure_endpoint_sanity(url: &str) -> Result<(), String> {
    endpoint_sanity(url)?;
    let lower = url.to_ascii_lowercase();
    if !lower.contains("/openai/deployments/") || !lower.ends_with("/chat/completions") {
        return Err("expected /openai/deployments/<name>/chat/completions path".into());
    }
    Ok(())
}

fn trim_slash(input: &str) -> String {
    input.trim_end_matches('/').to_string()
}

fn sanitize_url_for_display(input: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(input) else {
        return input.to_string();
    };
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    if parsed.query().is_some() {
        parsed.set_query(Some("redacted"));
    }
    parsed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_connect() -> LlmConnectConfig {
        LlmConnectConfig {
            model: "llama3.2".into(),
            ollama_url: "http://localhost:11434".into(),
            anthropic_api_key: None,
            openrouter_api_key: Some("sk-or-secret".into()),
            openai_api_key: Some("sk-openai-secret".into()),
            groq_api_key: None,
            azure_openai_endpoint: Some(
                "https://example.openai.azure.com/openai/deployments/demo/chat/completions".into(),
            ),
            azure_openai_api_key: Some("azure-secret".into()),
            azure_api_version: "2024-02-01".into(),
            bedrock_explicit: false,
            aws_region: "us-east-1".into(),
            openai_compatible_url: Some("https://lm.example/v1".into()),
            openai_compatible_api_key: Some("compat-secret".into()),
        }
    }

    #[test]
    fn json_output_shape_contains_expected_blocks() {
        let mut report = DoctorProvidersReport {
            ok: true,
            active_provider: Some("ollama".into()),
            providers: vec![diagnose_provider_static(
                &sample_connect(),
                ProviderKind::OpenAi,
                false,
            )],
        };
        report.providers[0].healthy = true;
        let payload = render_doctor_json(&report);
        assert!(payload["ok"].is_boolean());
        assert!(payload["providers"].is_array());
        assert!(payload["providers"][0]["checks"].is_array());
    }

    #[test]
    fn malformed_endpoint_is_reported_as_failure() {
        let mut cfg = sample_connect();
        cfg.openai_compatible_url = Some("not-a-url".into());
        let d = diagnose_provider_static(&cfg, ProviderKind::OpenAiCompatible, true);
        assert!(
            d.checks
                .iter()
                .any(|c| c.name == "base_url" && matches!(c.status, CheckStatus::Fail))
        );
    }

    #[test]
    fn missing_key_is_failure_when_provider_active() {
        let mut cfg = sample_connect();
        cfg.openai_api_key = None;
        let d = diagnose_provider_static(&cfg, ProviderKind::OpenAi, true);
        assert!(
            d.checks
                .iter()
                .any(|c| c.name == "api_key" && matches!(c.status, CheckStatus::Fail))
        );
    }

    #[test]
    fn no_secret_leakage_in_rendered_json() {
        let d = diagnose_provider_static(&sample_connect(), ProviderKind::OpenRouter, true);
        let report = DoctorProvidersReport {
            ok: false,
            active_provider: Some("openrouter".into()),
            providers: vec![d],
        };
        let raw = render_doctor_json(&report).to_string();
        assert!(!raw.contains("sk-or-secret"));
        assert!(!raw.contains("sk-openai-secret"));
        assert!(!raw.contains("compat-secret"));
        assert!(!raw.contains("azure-secret"));
    }
}
