//! `akmon slo` subcommands for enforceable reliability guardrails.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use akmon_config::AkmonGlobalConfig;
use akmon_core::{
    NormalizedReliabilityMetrics, RegressionGuardConfig, ReliabilitySloThresholds,
    ReliabilityTrendEvaluation, SloCheckStatus, TrendSampleCounts, TrendStatus,
    aggregate_baseline_metrics, evaluate_reliability_slos, evaluate_reliability_trend,
    extract_slo_input_metrics, normalize_reliability_metrics, parse_regression_guard_config_json,
    parse_regression_guard_config_toml, parse_slo_thresholds_json, parse_slo_thresholds_toml,
};
use chrono::{DateTime, Utc};
use clap::Subcommand;
use serde_json::json;

/// Top-level `akmon slo …` options.
#[derive(Debug, Clone, clap::Args)]
pub struct SloArgs {
    /// SLO subcommand.
    #[command(subcommand)]
    pub cmd: SloSubcommand,
}

/// Supported `akmon slo` subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum SloSubcommand {
    /// Verify run/evidence reliability metrics against SLO thresholds.
    Verify {
        /// Path to run-report JSON or evidence JSON.
        path: PathBuf,
        /// Optional thresholds file (`json` or `toml`) with direct keys or nested `[slo]`/`slo`.
        #[arg(long = "thresholds", value_name = "PATH")]
        thresholds: Option<PathBuf>,
        /// Treat skipped checks (missing metrics / insufficient sample) as failures.
        #[arg(long = "strict")]
        strict: bool,
        /// Override: minimum acceptable tool success rate in `[0.0, 1.0]`.
        #[arg(long = "min-tool-success-rate")]
        min_tool_success_rate: Option<f64>,
        /// Override: maximum acceptable timeout rate in `[0.0, 1.0]`.
        #[arg(long = "max-timeout-rate")]
        max_timeout_rate: Option<f64>,
        /// Override: maximum acceptable policy denial rate in `[0.0, 1.0]`.
        #[arg(long = "max-policy-denial-rate")]
        max_policy_denial_rate: Option<f64>,
        /// Override: maximum acceptable tool failure rate in `[0.0, 1.0]`.
        #[arg(long = "max-tool-failure-rate")]
        max_tool_failure_rate: Option<f64>,
        /// Override: maximum retries total.
        #[arg(long = "max-retries-total")]
        max_retries_total: Option<u64>,
        /// Override: maximum timeouts total.
        #[arg(long = "max-timeouts-total")]
        max_timeouts_total: Option<u64>,
        /// Override: minimum tool calls total for rate checks.
        #[arg(long = "min-tool-calls-total")]
        min_tool_calls_total: Option<u64>,
    },
    /// Compare current run/evidence against historical baseline and detect regressions.
    Trend {
        /// Path to current run-report JSON or evidence JSON.
        current_path: PathBuf,
        /// Directory containing prior run/evidence artifacts.
        #[arg(long = "baseline-dir", value_name = "DIR")]
        baseline_dir: Option<PathBuf>,
        /// Explicit baseline artifact path(s); repeatable.
        #[arg(long = "baseline-file", value_name = "PATH", action = clap::ArgAction::Append)]
        baseline_files: Vec<PathBuf>,
        /// Number of most recent valid baseline samples to use.
        #[arg(long = "window", default_value_t = 20)]
        window: usize,
        /// Optional trend config file (`json` or `toml`).
        #[arg(long = "config", value_name = "PATH")]
        config: Option<PathBuf>,
        /// Treat skipped checks as failures.
        #[arg(long = "strict")]
        strict: bool,
    },
}

#[derive(Debug, Clone)]
struct BaselineCandidate {
    path: PathBuf,
    timestamp: Option<DateTime<Utc>>,
    metrics: NormalizedReliabilityMetrics,
}

/// Runs one `akmon slo` invocation.
pub fn run_slo(args: SloArgs, json_output: bool, global: &AkmonGlobalConfig) -> ExitCode {
    match args.cmd {
        SloSubcommand::Verify {
            path,
            thresholds,
            strict,
            min_tool_success_rate,
            max_timeout_rate,
            max_policy_denial_rate,
            max_tool_failure_rate,
            max_retries_total,
            max_timeouts_total,
            min_tool_calls_total,
        } => {
            let cli_overrides = ReliabilitySloThresholds {
                min_tool_success_rate,
                max_timeout_rate,
                max_policy_denial_rate,
                max_tool_failure_rate,
                max_retries_total,
                max_timeouts_total,
                min_tool_calls_total,
            };
            match verify_slo_path(&path, thresholds.as_deref(), strict, global, &cli_overrides) {
                Ok(eval) => {
                    let ok = eval.status == SloCheckStatus::Pass;
                    if json_output {
                        let payload = json!({
                            "ok": ok,
                            "status": if ok { "pass" } else { "fail" },
                            "path": path,
                            "input_kind": eval.input_kind,
                            "session_id": eval.session_id,
                            "strict": eval.strict,
                            "evaluated_metrics": eval.evaluated_metrics,
                            "applied_thresholds": eval.applied_thresholds,
                            "violations": eval.violations,
                            "skipped_checks": eval.skipped_checks,
                        });
                        println!("{payload}");
                    } else if ok {
                        println!(
                            "slo verify: pass ({}) source={:?} violations=0 skipped={}",
                            path.display(),
                            eval.input_kind,
                            eval.skipped_checks.len()
                        );
                    } else {
                        eprintln!(
                            "slo verify: fail ({}) source={:?} violations={} skipped={}",
                            path.display(),
                            eval.input_kind,
                            eval.violations.len(),
                            eval.skipped_checks.len()
                        );
                        for v in &eval.violations {
                            eprintln!(" - [{}] {} ({})", v.check_id, v.message, v.reason_code);
                        }
                        for s in &eval.skipped_checks {
                            eprintln!(
                                " - [skipped:{}] {} ({})",
                                s.check_id, s.message, s.reason_code
                            );
                        }
                    }
                    if ok {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::from(1)
                    }
                }
                Err(message) => {
                    if json_output {
                        let payload = json!({
                            "ok": false,
                            "status": "error",
                            "path": path,
                            "error": message,
                        });
                        println!("{payload}");
                    } else {
                        eprintln!("slo verify: {message}");
                    }
                    ExitCode::from(2)
                }
            }
        }
        SloSubcommand::Trend {
            current_path,
            baseline_dir,
            baseline_files,
            window,
            config,
            strict,
        } => {
            match run_slo_trend(
                &current_path,
                baseline_dir.as_deref(),
                &baseline_files,
                window,
                config.as_deref(),
                strict,
                global,
            ) {
                Ok(eval) => {
                    let ok = eval.status == TrendStatus::Pass;
                    if json_output {
                        let payload = json!({
                            "ok": ok,
                            "status": if ok { "pass" } else { "fail" },
                            "current_path": current_path,
                            "strict": eval.strict,
                            "current_metrics": eval.current_metrics,
                            "baseline_summary": eval.baseline_summary,
                            "applied_regression_config": eval.applied_regression_config,
                            "violations": eval.violations,
                            "skipped": eval.skipped,
                            "sample_counts": eval.sample_counts,
                        });
                        println!("{payload}");
                    } else if ok {
                        println!(
                            "slo trend: pass ({}) baseline_used={}",
                            current_path.display(),
                            eval.sample_counts.used
                        );
                    } else {
                        eprintln!(
                            "slo trend: fail ({}) violations={} skipped={} baseline_used={}",
                            current_path.display(),
                            eval.violations.len(),
                            eval.skipped.len(),
                            eval.sample_counts.used
                        );
                        for v in &eval.violations {
                            eprintln!(
                                " - [{}] current={:.6} baseline={:.6} delta={:.6} threshold={:.6} ({})",
                                v.metric,
                                v.current,
                                v.baseline,
                                v.delta,
                                v.threshold,
                                v.reason_code
                            );
                        }
                        for s in &eval.skipped {
                            eprintln!(
                                " - [skipped:{}] {} ({})",
                                s.metric, s.message, s.reason_code
                            );
                        }
                    }
                    if ok {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::from(1)
                    }
                }
                Err(message) => {
                    if json_output {
                        let payload = json!({
                            "ok": false,
                            "status": "error",
                            "current_path": current_path,
                            "error": message,
                        });
                        println!("{payload}");
                    } else {
                        eprintln!("slo trend: {message}");
                    }
                    ExitCode::from(2)
                }
            }
        }
    }
}

fn verify_slo_path(
    input_path: &Path,
    thresholds_path: Option<&Path>,
    strict: bool,
    global: &AkmonGlobalConfig,
    cli_overrides: &ReliabilitySloThresholds,
) -> Result<akmon_core::ReliabilitySloEvaluation, String> {
    let thresholds = resolve_thresholds(thresholds_path, global, cli_overrides)?;
    let raw = std::fs::read_to_string(input_path)
        .map_err(|e| format!("failed to read input {}: {e}", input_path.display()))?;
    let doc: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("invalid JSON input: {e}"))?;
    let input = extract_slo_input_metrics(&doc)
        .map_err(|e| format!("failed to parse input schema: {e}"))?;
    evaluate_reliability_slos(&input, &thresholds, strict)
        .map_err(|e| format!("failed to evaluate SLOs: {e}"))
}

fn resolve_thresholds(
    thresholds_path: Option<&Path>,
    global: &AkmonGlobalConfig,
    cli_overrides: &ReliabilitySloThresholds,
) -> Result<ReliabilitySloThresholds, String> {
    let mut merged = global.slo.thresholds.clone();
    if let Some(path) = thresholds_path {
        let from_file = load_thresholds_from_file(path)?;
        merged = merged.overlay(&from_file);
    }
    merged = merged.overlay(cli_overrides);
    akmon_core::validate_reliability_slo_thresholds(&merged)
        .map_err(|e| format!("invalid thresholds: {e}"))?;
    Ok(merged)
}

fn load_thresholds_from_file(path: &Path) -> Result<ReliabilitySloThresholds, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read thresholds file {}: {e}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    match ext {
        "json" => {
            let v: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| format!("invalid JSON thresholds file {}: {e}", path.display()))?;
            parse_slo_thresholds_json(&v)
                .map_err(|e| format!("invalid JSON thresholds file {}: {e}", path.display()))
        }
        "toml" => parse_slo_thresholds_toml(&raw)
            .map_err(|e| format!("invalid TOML thresholds file {}: {e}", path.display())),
        _ => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw)
                && let Ok(t) = parse_slo_thresholds_json(&v)
            {
                return Ok(t);
            }
            parse_slo_thresholds_toml(&raw).map_err(|e| {
                format!(
                    "invalid thresholds file {} (tried JSON then TOML): {e}",
                    path.display()
                )
            })
        }
    }
}

fn run_slo_trend(
    current_path: &Path,
    baseline_dir: Option<&Path>,
    baseline_files: &[PathBuf],
    window: usize,
    config_path: Option<&Path>,
    strict: bool,
    global: &AkmonGlobalConfig,
) -> Result<ReliabilityTrendEvaluation, String> {
    if window == 0 {
        return Err("window must be >= 1".to_string());
    }
    if baseline_dir.is_none() && baseline_files.is_empty() {
        return Err("provide --baseline-dir or at least one --baseline-file".to_string());
    }
    let cfg = resolve_trend_config(config_path, global)?;
    let current = load_normalized_metrics_from_path(current_path)?;
    let candidates = load_baseline_candidates(baseline_dir, baseline_files, current_path)?;
    let sample_counts_total = candidates.total;
    let sample_counts_valid = candidates.valid.len();
    let sample_counts_invalid = candidates.invalid;
    let selected = select_last_n_candidates(candidates.valid, window);
    let used_metrics = selected
        .iter()
        .map(|c| c.metrics.clone())
        .collect::<Vec<NormalizedReliabilityMetrics>>();
    let baseline_summary = aggregate_baseline_metrics(&used_metrics);
    let sample_counts = TrendSampleCounts {
        total: sample_counts_total,
        valid: sample_counts_valid,
        invalid: sample_counts_invalid,
        used: used_metrics.len(),
    };
    evaluate_reliability_trend(
        &current,
        baseline_summary.as_ref(),
        &cfg,
        strict,
        sample_counts,
    )
    .map_err(|e| format!("failed to evaluate trend: {e}"))
}

fn resolve_trend_config(
    config_path: Option<&Path>,
    global: &AkmonGlobalConfig,
) -> Result<RegressionGuardConfig, String> {
    let mut merged = global.slo.trend.clone();
    if let Some(path) = config_path {
        let from_file = load_trend_config_from_file(path)?;
        merged = merged.overlay(&from_file);
    }
    akmon_core::validate_regression_guard_config(&merged)
        .map_err(|e| format!("invalid trend config: {e}"))?;
    Ok(merged)
}

fn load_trend_config_from_file(path: &Path) -> Result<RegressionGuardConfig, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read trend config file {}: {e}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    match ext {
        "json" => {
            let v: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| format!("invalid JSON trend config file {}: {e}", path.display()))?;
            parse_regression_guard_config_json(&v)
                .map_err(|e| format!("invalid JSON trend config file {}: {e}", path.display()))
        }
        "toml" => parse_regression_guard_config_toml(&raw)
            .map_err(|e| format!("invalid TOML trend config file {}: {e}", path.display())),
        _ => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw)
                && let Ok(cfg) = parse_regression_guard_config_json(&v)
            {
                return Ok(cfg);
            }
            parse_regression_guard_config_toml(&raw).map_err(|e| {
                format!(
                    "invalid trend config file {} (tried JSON then TOML): {e}",
                    path.display()
                )
            })
        }
    }
}

fn load_normalized_metrics_from_path(path: &Path) -> Result<NormalizedReliabilityMetrics, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read artifact {}: {e}", path.display()))?;
    let doc: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("invalid JSON {}: {e}", path.display()))?;
    let parsed = extract_slo_input_metrics(&doc)
        .map_err(|e| format!("unsupported schema {}: {e}", path.display()))?;
    let Some(metrics) = parsed.reliability_metrics else {
        return Err(format!(
            "artifact {} is missing reliability_metrics",
            path.display()
        ));
    };
    normalize_reliability_metrics(&metrics).ok_or_else(|| {
        format!(
            "artifact {} contains invalid reliability metric values",
            path.display()
        )
    })
}

struct BaselineLoadResult {
    total: usize,
    valid: Vec<BaselineCandidate>,
    invalid: usize,
}

fn load_baseline_candidates(
    baseline_dir: Option<&Path>,
    baseline_files: &[PathBuf],
    current_path: &Path,
) -> Result<BaselineLoadResult, String> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Some(dir) = baseline_dir {
        let rd = std::fs::read_dir(dir)
            .map_err(|e| format!("failed to read baseline dir {}: {e}", dir.display()))?;
        for entry_res in rd {
            let entry = entry_res
                .map_err(|e| format!("failed to read baseline dir entry {}: {e}", dir.display()))?;
            let p = entry.path();
            if p.is_file() {
                paths.push(p);
            }
        }
    }
    paths.extend(baseline_files.iter().cloned());
    paths.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    paths.dedup();
    paths.retain(|p| p != current_path);
    let total = paths.len();
    let mut valid: Vec<BaselineCandidate> = Vec::new();
    let mut invalid = 0usize;
    for p in paths {
        match load_candidate_from_path(&p) {
            Ok(c) => valid.push(c),
            Err(_) => invalid = invalid.saturating_add(1),
        }
    }
    Ok(BaselineLoadResult {
        total,
        valid,
        invalid,
    })
}

fn load_candidate_from_path(path: &Path) -> Result<BaselineCandidate, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read baseline artifact {}: {e}", path.display()))?;
    let doc: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("invalid JSON baseline artifact {}: {e}", path.display()))?;
    let parsed = extract_slo_input_metrics(&doc)
        .map_err(|e| format!("unsupported baseline schema {}: {e}", path.display()))?;
    let timestamp = doc
        .get("generated_at")
        .and_then(serde_json::Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));
    let Some(raw_metrics) = parsed.reliability_metrics else {
        return Err(format!(
            "baseline artifact {} missing reliability_metrics",
            path.display()
        ));
    };
    let Some(metrics) = normalize_reliability_metrics(&raw_metrics) else {
        return Err(format!(
            "baseline artifact {} has invalid metrics",
            path.display()
        ));
    };
    Ok(BaselineCandidate {
        path: path.to_path_buf(),
        timestamp,
        metrics,
    })
}

fn select_last_n_candidates(
    mut valid: Vec<BaselineCandidate>,
    window: usize,
) -> Vec<BaselineCandidate> {
    valid.sort_by(|a, b| {
        let ta = a
            .timestamp
            .map(|t| t.timestamp_millis())
            .unwrap_or(i64::MIN);
        let tb = b
            .timestamp
            .map(|t| t.timestamp_millis())
            .unwrap_or(i64::MIN);
        ta.cmp(&tb)
            .then_with(|| a.path.to_string_lossy().cmp(&b.path.to_string_lossy()))
    });
    if valid.len() <= window {
        return valid;
    }
    valid.split_off(valid.len().saturating_sub(window))
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_config::SloConfig;
    use akmon_core::RunReliabilityMetrics;

    fn write_run_report(path: &Path, metrics: Option<RunReliabilityMetrics>) {
        let mut payload = json!({
            "session_id": "sess-1",
            "status": "success",
            "tool_calls": [],
        });
        if let Some(m) = metrics {
            payload["reliability_metrics"] = serde_json::to_value(m).expect("metrics");
        }
        std::fs::write(path, serde_json::to_string_pretty(&payload).expect("json")).expect("write");
    }

    fn write_evidence(path: &Path, metrics: Option<RunReliabilityMetrics>, generated_at: &str) {
        let mut payload = json!({
            "evidence_schema_version": "evidence.v1",
            "session_id": "sess-1",
            "generated_at": generated_at,
        });
        if let Some(m) = metrics {
            payload["reliability_metrics"] = serde_json::to_value(m).expect("metrics");
        }
        std::fs::write(path, serde_json::to_string_pretty(&payload).expect("json")).expect("write");
    }

    fn healthy_metrics() -> RunReliabilityMetrics {
        RunReliabilityMetrics {
            tool_calls_total: 10,
            tool_calls_success: 10,
            tool_calls_failure: 0,
            tool_latency_ms_total: 100,
            tool_latency_ms_avg: 10,
            tool_latency_ms_p95: Some(15),
            policy_denials_total: 0,
            retries_total: 0,
            timeouts_total: 0,
        }
    }

    fn strict_thresholds() -> ReliabilitySloThresholds {
        ReliabilitySloThresholds {
            min_tool_success_rate: Some(0.95),
            max_tool_failure_rate: Some(0.05),
            max_timeouts_total: Some(0),
            min_tool_calls_total: Some(5),
            ..Default::default()
        }
    }

    fn trend_cfg() -> RegressionGuardConfig {
        RegressionGuardConfig {
            max_success_rate_drop_abs: Some(0.05),
            max_timeout_rate_increase_abs: Some(0.02),
            max_failure_rate_increase_abs: Some(0.03),
            max_retries_increase_ratio: Some(1.0),
            max_latency_avg_increase_ratio: Some(0.5),
            min_baseline_samples: Some(5),
        }
    }

    #[test]
    fn cli_exit_code_pass_fail_and_invalid_input() {
        let dir = tempfile::tempdir().expect("tmp");
        let run_ok = dir.path().join("run-ok.json");
        let run_fail = dir.path().join("run-fail.json");
        let bad = dir.path().join("bad.json");
        write_run_report(&run_ok, Some(healthy_metrics()));
        write_run_report(
            &run_fail,
            Some(RunReliabilityMetrics {
                tool_calls_total: 10,
                tool_calls_success: 1,
                tool_calls_failure: 9,
                timeouts_total: 2,
                ..RunReliabilityMetrics::default()
            }),
        );
        std::fs::write(&bad, "{not-json").expect("write bad");

        let global = AkmonGlobalConfig {
            slo: SloConfig {
                thresholds: strict_thresholds(),
                trend: trend_cfg(),
            },
            ..Default::default()
        };
        let ok_code = run_slo(
            SloArgs {
                cmd: SloSubcommand::Verify {
                    path: run_ok,
                    thresholds: None,
                    strict: false,
                    min_tool_success_rate: None,
                    max_timeout_rate: None,
                    max_policy_denial_rate: None,
                    max_tool_failure_rate: None,
                    max_retries_total: None,
                    max_timeouts_total: None,
                    min_tool_calls_total: None,
                },
            },
            true,
            &global,
        );
        assert_eq!(ok_code, ExitCode::SUCCESS);
        let fail_code = run_slo(
            SloArgs {
                cmd: SloSubcommand::Verify {
                    path: run_fail,
                    thresholds: None,
                    strict: false,
                    min_tool_success_rate: None,
                    max_timeout_rate: None,
                    max_policy_denial_rate: None,
                    max_tool_failure_rate: None,
                    max_retries_total: None,
                    max_timeouts_total: None,
                    min_tool_calls_total: None,
                },
            },
            true,
            &global,
        );
        assert_eq!(fail_code, ExitCode::from(1));
        let bad_code = run_slo(
            SloArgs {
                cmd: SloSubcommand::Verify {
                    path: bad,
                    thresholds: None,
                    strict: false,
                    min_tool_success_rate: None,
                    max_timeout_rate: None,
                    max_policy_denial_rate: None,
                    max_tool_failure_rate: None,
                    max_retries_total: None,
                    max_timeouts_total: None,
                    min_tool_calls_total: None,
                },
            },
            true,
            &global,
        );
        assert_eq!(bad_code, ExitCode::from(2));
    }

    #[test]
    fn non_strict_missing_metrics_skips_but_strict_fails() {
        let dir = tempfile::tempdir().expect("tmp");
        let run = dir.path().join("run-missing-metrics.json");
        write_run_report(&run, None);
        let global = AkmonGlobalConfig {
            slo: SloConfig {
                thresholds: ReliabilitySloThresholds {
                    min_tool_success_rate: Some(0.9),
                    ..Default::default()
                },
                trend: trend_cfg(),
            },
            ..Default::default()
        };
        let non_strict = run_slo(
            SloArgs {
                cmd: SloSubcommand::Verify {
                    path: run.clone(),
                    thresholds: None,
                    strict: false,
                    min_tool_success_rate: None,
                    max_timeout_rate: None,
                    max_policy_denial_rate: None,
                    max_tool_failure_rate: None,
                    max_retries_total: None,
                    max_timeouts_total: None,
                    min_tool_calls_total: None,
                },
            },
            true,
            &global,
        );
        assert_eq!(non_strict, ExitCode::SUCCESS);
        let strict = run_slo(
            SloArgs {
                cmd: SloSubcommand::Verify {
                    path: run,
                    thresholds: None,
                    strict: true,
                    min_tool_success_rate: None,
                    max_timeout_rate: None,
                    max_policy_denial_rate: None,
                    max_tool_failure_rate: None,
                    max_retries_total: None,
                    max_timeouts_total: None,
                    min_tool_calls_total: None,
                },
            },
            true,
            &global,
        );
        assert_eq!(strict, ExitCode::from(1));
    }

    #[test]
    fn supports_run_and_evidence_inputs() {
        let dir = tempfile::tempdir().expect("tmp");
        let run = dir.path().join("run.json");
        let evidence = dir.path().join("evidence.json");
        write_run_report(&run, Some(healthy_metrics()));
        write_evidence(&evidence, Some(healthy_metrics()), "2026-04-20T12:34:56Z");
        let global = AkmonGlobalConfig {
            slo: SloConfig {
                thresholds: strict_thresholds(),
                trend: trend_cfg(),
            },
            ..Default::default()
        };
        let code_run = run_slo(
            SloArgs {
                cmd: SloSubcommand::Verify {
                    path: run,
                    thresholds: None,
                    strict: false,
                    min_tool_success_rate: None,
                    max_timeout_rate: None,
                    max_policy_denial_rate: None,
                    max_tool_failure_rate: None,
                    max_retries_total: None,
                    max_timeouts_total: None,
                    min_tool_calls_total: None,
                },
            },
            true,
            &global,
        );
        let code_evidence = run_slo(
            SloArgs {
                cmd: SloSubcommand::Verify {
                    path: evidence,
                    thresholds: None,
                    strict: false,
                    min_tool_success_rate: None,
                    max_timeout_rate: None,
                    max_policy_denial_rate: None,
                    max_tool_failure_rate: None,
                    max_retries_total: None,
                    max_timeouts_total: None,
                    min_tool_calls_total: None,
                },
            },
            true,
            &global,
        );
        assert_eq!(code_run, ExitCode::SUCCESS);
        assert_eq!(code_evidence, ExitCode::SUCCESS);
    }

    #[test]
    fn thresholds_file_and_cli_overrides_are_applied() {
        let dir = tempfile::tempdir().expect("tmp");
        let run = dir.path().join("run.json");
        write_run_report(
            &run,
            Some(RunReliabilityMetrics {
                tool_calls_total: 10,
                tool_calls_success: 8,
                ..RunReliabilityMetrics::default()
            }),
        );
        let thresholds = dir.path().join("thresholds.toml");
        std::fs::write(
            &thresholds,
            "[slo]\nmin_tool_success_rate = 0.7\nmax_retries_total = 3\n",
        )
        .expect("write");
        let global = AkmonGlobalConfig {
            slo: SloConfig {
                thresholds: ReliabilitySloThresholds {
                    min_tool_success_rate: Some(0.99),
                    ..Default::default()
                },
                trend: trend_cfg(),
            },
            ..Default::default()
        };
        let code = run_slo(
            SloArgs {
                cmd: SloSubcommand::Verify {
                    path: run,
                    thresholds: Some(thresholds),
                    strict: false,
                    min_tool_success_rate: Some(0.75),
                    max_timeout_rate: None,
                    max_policy_denial_rate: None,
                    max_tool_failure_rate: None,
                    max_retries_total: None,
                    max_timeouts_total: None,
                    min_tool_calls_total: None,
                },
            },
            true,
            &global,
        );
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn json_output_shape_has_required_fields() {
        let dir = tempfile::tempdir().expect("tmp");
        let run = dir.path().join("run.json");
        write_run_report(
            &run,
            Some(RunReliabilityMetrics {
                tool_calls_total: 1,
                tool_calls_success: 0,
                tool_calls_failure: 1,
                ..RunReliabilityMetrics::default()
            }),
        );
        let global = AkmonGlobalConfig {
            slo: SloConfig {
                thresholds: ReliabilitySloThresholds {
                    min_tool_success_rate: Some(1.0),
                    ..Default::default()
                },
                trend: trend_cfg(),
            },
            ..Default::default()
        };
        let eval = verify_slo_path(
            &run,
            None,
            false,
            &global,
            &ReliabilitySloThresholds::default(),
        )
        .expect("eval");
        let payload = json!({
            "ok": eval.status == SloCheckStatus::Pass,
            "status": if eval.status == SloCheckStatus::Pass { "pass" } else { "fail" },
            "input_kind": eval.input_kind,
            "session_id": eval.session_id,
            "strict": eval.strict,
            "evaluated_metrics": eval.evaluated_metrics,
            "applied_thresholds": eval.applied_thresholds,
            "violations": eval.violations,
            "skipped_checks": eval.skipped_checks,
        });
        assert!(payload.get("status").is_some());
        assert!(payload.get("evaluated_metrics").is_some());
        assert!(payload.get("applied_thresholds").is_some());
        assert!(payload.get("violations").is_some());
        assert!(payload.get("skipped_checks").is_some());
    }

    #[test]
    fn baseline_parsing_mixed_valid_invalid_artifacts() {
        let dir = tempfile::tempdir().expect("tmp");
        let current = dir.path().join("current.json");
        let baseline_dir = dir.path().join("baseline");
        std::fs::create_dir_all(&baseline_dir).expect("mkdir");
        write_run_report(&current, Some(healthy_metrics()));
        write_run_report(&baseline_dir.join("valid-1.json"), Some(healthy_metrics()));
        std::fs::write(baseline_dir.join("bad.json"), "{broken").expect("write");
        let candidates =
            load_baseline_candidates(Some(&baseline_dir), &[], &current).expect("load");
        assert_eq!(candidates.total, 2);
        assert_eq!(candidates.valid.len(), 1);
        assert_eq!(candidates.invalid, 1);
    }

    #[test]
    fn last_n_window_selection_is_deterministic() {
        let dir = tempfile::tempdir().expect("tmp");
        let c1 = BaselineCandidate {
            path: dir.path().join("b.json"),
            timestamp: None,
            metrics: normalize_reliability_metrics(&healthy_metrics()).expect("norm"),
        };
        let c2 = BaselineCandidate {
            path: dir.path().join("a.json"),
            timestamp: None,
            metrics: normalize_reliability_metrics(&healthy_metrics()).expect("norm"),
        };
        let selected = select_last_n_candidates(vec![c1, c2], 1);
        assert_eq!(selected.len(), 1);
        assert!(selected[0].path.ends_with("b.json"));
    }

    #[test]
    fn trend_command_passes_for_healthy_current_vs_baseline() {
        let dir = tempfile::tempdir().expect("tmp");
        let current = dir.path().join("current.json");
        let baseline_dir = dir.path().join("baseline");
        std::fs::create_dir_all(&baseline_dir).expect("mkdir");
        write_evidence(&current, Some(healthy_metrics()), "2026-04-20T12:34:58Z");
        for i in 0..6 {
            let p = baseline_dir.join(format!("base-{i}.json"));
            write_evidence(
                &p,
                Some(healthy_metrics()),
                &format!("2026-04-20T12:34:{:02}Z", i),
            );
        }
        let global = AkmonGlobalConfig {
            slo: SloConfig {
                thresholds: strict_thresholds(),
                trend: trend_cfg(),
            },
            ..Default::default()
        };
        let code = run_slo(
            SloArgs {
                cmd: SloSubcommand::Trend {
                    current_path: current,
                    baseline_dir: Some(baseline_dir),
                    baseline_files: vec![],
                    window: 5,
                    config: None,
                    strict: false,
                },
            },
            true,
            &global,
        );
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn trend_command_detects_regression_violations() {
        let dir = tempfile::tempdir().expect("tmp");
        let current = dir.path().join("current.json");
        let baseline_dir = dir.path().join("baseline");
        std::fs::create_dir_all(&baseline_dir).expect("mkdir");
        write_run_report(
            &current,
            Some(RunReliabilityMetrics {
                tool_calls_total: 10,
                tool_calls_success: 4,
                tool_calls_failure: 6,
                retries_total: 10,
                timeouts_total: 4,
                tool_latency_ms_avg: 100,
                ..RunReliabilityMetrics::default()
            }),
        );
        for i in 0..6 {
            let p = baseline_dir.join(format!("base-{i}.json"));
            write_evidence(
                &p,
                Some(RunReliabilityMetrics {
                    tool_calls_total: 10,
                    tool_calls_success: 10,
                    tool_calls_failure: 0,
                    retries_total: 1,
                    timeouts_total: 0,
                    tool_latency_ms_avg: 20,
                    ..RunReliabilityMetrics::default()
                }),
                &format!("2026-04-20T12:35:{:02}Z", i),
            );
        }
        let global = AkmonGlobalConfig {
            slo: SloConfig {
                thresholds: strict_thresholds(),
                trend: trend_cfg(),
            },
            ..Default::default()
        };
        let code = run_slo(
            SloArgs {
                cmd: SloSubcommand::Trend {
                    current_path: current,
                    baseline_dir: Some(baseline_dir),
                    baseline_files: vec![],
                    window: 5,
                    config: None,
                    strict: false,
                },
            },
            true,
            &global,
        );
        assert_eq!(code, ExitCode::from(1));
    }

    #[test]
    fn trend_strict_vs_non_strict_on_insufficient_baseline() {
        let dir = tempfile::tempdir().expect("tmp");
        let current = dir.path().join("current.json");
        let baseline_dir = dir.path().join("baseline");
        std::fs::create_dir_all(&baseline_dir).expect("mkdir");
        write_run_report(&current, Some(healthy_metrics()));
        write_run_report(&baseline_dir.join("base-1.json"), Some(healthy_metrics()));
        let global = AkmonGlobalConfig {
            slo: SloConfig {
                thresholds: strict_thresholds(),
                trend: trend_cfg(),
            },
            ..Default::default()
        };
        let non_strict = run_slo(
            SloArgs {
                cmd: SloSubcommand::Trend {
                    current_path: current.clone(),
                    baseline_dir: Some(baseline_dir.clone()),
                    baseline_files: vec![],
                    window: 20,
                    config: None,
                    strict: false,
                },
            },
            true,
            &global,
        );
        assert_eq!(non_strict, ExitCode::SUCCESS);
        let strict = run_slo(
            SloArgs {
                cmd: SloSubcommand::Trend {
                    current_path: current,
                    baseline_dir: Some(baseline_dir),
                    baseline_files: vec![],
                    window: 20,
                    config: None,
                    strict: true,
                },
            },
            true,
            &global,
        );
        assert_eq!(strict, ExitCode::from(1));
    }

    #[test]
    fn trend_json_output_shape_has_required_fields() {
        let dir = tempfile::tempdir().expect("tmp");
        let current = dir.path().join("current.json");
        let baseline_dir = dir.path().join("baseline");
        std::fs::create_dir_all(&baseline_dir).expect("mkdir");
        write_run_report(&current, Some(healthy_metrics()));
        for i in 0..6 {
            write_evidence(
                &baseline_dir.join(format!("base-{i}.json")),
                Some(healthy_metrics()),
                &format!("2026-04-20T12:36:{:02}Z", i),
            );
        }
        let global = AkmonGlobalConfig {
            slo: SloConfig {
                thresholds: strict_thresholds(),
                trend: trend_cfg(),
            },
            ..Default::default()
        };
        let eval = run_slo_trend(&current, Some(&baseline_dir), &[], 5, None, false, &global)
            .expect("trend");
        let payload = json!({
            "status": if eval.status == TrendStatus::Pass { "pass" } else { "fail" },
            "current_metrics": eval.current_metrics,
            "baseline_summary": eval.baseline_summary,
            "applied_regression_config": eval.applied_regression_config,
            "violations": eval.violations,
            "skipped": eval.skipped,
            "sample_counts": eval.sample_counts,
        });
        assert!(payload.get("status").is_some());
        assert!(payload.get("current_metrics").is_some());
        assert!(payload.get("baseline_summary").is_some());
        assert!(payload.get("applied_regression_config").is_some());
        assert!(payload.get("violations").is_some());
        assert!(payload.get("skipped").is_some());
        assert!(payload.get("sample_counts").is_some());
    }
}
