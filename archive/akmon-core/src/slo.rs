//! Reliability SLO thresholds and guardrail evaluation helpers.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::RunReliabilityMetrics;

/// Configurable thresholds for reliability guardrail checks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReliabilitySloThresholds {
    /// Minimum acceptable tool success rate (`tool_calls_success / tool_calls_total`).
    pub min_tool_success_rate: Option<f64>,
    /// Maximum acceptable timeout rate (`timeouts_total / tool_calls_total`).
    pub max_timeout_rate: Option<f64>,
    /// Maximum acceptable policy denial rate (`policy_denials_total / tool_calls_total`).
    pub max_policy_denial_rate: Option<f64>,
    /// Maximum acceptable tool failure rate (`tool_calls_failure / tool_calls_total`).
    pub max_tool_failure_rate: Option<f64>,
    /// Maximum acceptable number of retries.
    pub max_retries_total: Option<u64>,
    /// Maximum acceptable number of timeouts.
    pub max_timeouts_total: Option<u64>,
    /// Minimum sample size for rate-based checks.
    pub min_tool_calls_total: Option<u64>,
}

impl ReliabilitySloThresholds {
    /// Overlays `other` on top of `self` (non-`None` values in `other` win).
    pub fn overlay(&self, other: &Self) -> Self {
        Self {
            min_tool_success_rate: other.min_tool_success_rate.or(self.min_tool_success_rate),
            max_timeout_rate: other.max_timeout_rate.or(self.max_timeout_rate),
            max_policy_denial_rate: other.max_policy_denial_rate.or(self.max_policy_denial_rate),
            max_tool_failure_rate: other.max_tool_failure_rate.or(self.max_tool_failure_rate),
            max_retries_total: other.max_retries_total.or(self.max_retries_total),
            max_timeouts_total: other.max_timeouts_total.or(self.max_timeouts_total),
            min_tool_calls_total: other.min_tool_calls_total.or(self.min_tool_calls_total),
        }
    }
}

/// Source schema kind used for SLO verification input parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SloInputKind {
    /// Headless run report (`akmon --output json --task ...`).
    RunReport,
    /// Evidence artifact (`.akmon/evidence/<session-id>.json`).
    Evidence,
}

/// Extracted reliability metrics from one supported JSON input file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SloInputMetrics {
    /// Parsed source schema kind.
    pub input_kind: SloInputKind,
    /// Optional session id extracted from input.
    pub session_id: Option<String>,
    /// Reliability metrics block when present.
    pub reliability_metrics: Option<RunReliabilityMetrics>,
}

/// Outcome status of one threshold check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SloCheckStatus {
    /// Threshold passed.
    Pass,
    /// Threshold violated.
    Violation,
    /// Check skipped (for missing metrics or insufficient samples).
    Skipped,
}

/// One check result row in SLO evaluation output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SloCheckResult {
    /// Stable check id (`min_tool_success_rate`, etc.).
    pub check_id: String,
    /// Pass/violation/skipped status.
    pub status: SloCheckStatus,
    /// Stable reason code for automation.
    pub reason_code: String,
    /// Human-readable explanation.
    pub message: String,
    /// Actual metric value when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<Value>,
    /// Applied threshold value when relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<Value>,
}

/// Full evaluation result for one input + threshold set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReliabilitySloEvaluation {
    /// Overall pass/fail status.
    pub status: SloCheckStatus,
    /// Source schema kind.
    pub input_kind: SloInputKind,
    /// Optional session identifier.
    pub session_id: Option<String>,
    /// Whether strict mode was enabled.
    pub strict: bool,
    /// Reliability metrics used during evaluation.
    pub evaluated_metrics: Option<RunReliabilityMetrics>,
    /// Effective threshold set after merges/overrides.
    pub applied_thresholds: ReliabilitySloThresholds,
    /// Violating checks.
    pub violations: Vec<SloCheckResult>,
    /// Skipped checks and reasons.
    pub skipped_checks: Vec<SloCheckResult>,
}

/// SLO parsing/validation/evaluation failures.
#[derive(Debug, Error)]
pub enum SloError {
    /// Invalid threshold range or shape.
    #[error("invalid threshold `{field}`: {message}")]
    InvalidThreshold {
        /// Threshold field name.
        field: &'static str,
        /// Human-readable detail.
        message: String,
    },
    /// Input JSON does not match a supported run/evidence schema.
    #[error("unsupported SLO input schema: expected run report or evidence artifact")]
    UnsupportedInputSchema,
    /// Input JSON parse failed.
    #[error("invalid JSON input: {0}")]
    InvalidJson(#[from] serde_json::Error),
    /// Input object missing required metrics block.
    #[error("invalid reliability_metrics block: {0}")]
    InvalidMetrics(String),
    /// Threshold file contents were invalid.
    #[error("invalid SLO thresholds file: {0}")]
    InvalidThresholdFile(String),
}

/// Validates one threshold set.
pub fn validate_reliability_slo_thresholds(
    thresholds: &ReliabilitySloThresholds,
) -> Result<(), SloError> {
    validate_rate("min_tool_success_rate", thresholds.min_tool_success_rate)?;
    validate_rate("max_timeout_rate", thresholds.max_timeout_rate)?;
    validate_rate("max_policy_denial_rate", thresholds.max_policy_denial_rate)?;
    validate_rate("max_tool_failure_rate", thresholds.max_tool_failure_rate)?;
    Ok(())
}

fn validate_rate(field: &'static str, v: Option<f64>) -> Result<(), SloError> {
    if let Some(rate) = v
        && (!(0.0..=1.0).contains(&rate) || rate.is_nan())
    {
        return Err(SloError::InvalidThreshold {
            field,
            message: "must be in [0.0, 1.0]".to_string(),
        });
    }
    Ok(())
}

/// Parses metrics from one run-report or evidence JSON document.
pub fn extract_slo_input_metrics(doc: &Value) -> Result<SloInputMetrics, SloError> {
    let input_kind = if doc.get("evidence_schema_version").is_some() {
        SloInputKind::Evidence
    } else if doc.get("status").is_some() && doc.get("tool_calls").is_some() {
        SloInputKind::RunReport
    } else {
        return Err(SloError::UnsupportedInputSchema);
    };
    let session_id = doc
        .get("session_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let reliability_metrics = match doc.get("reliability_metrics") {
        Some(raw) => {
            let parsed: RunReliabilityMetrics = serde_json::from_value(raw.clone())
                .map_err(|e| SloError::InvalidMetrics(format!("deserialize error: {e}")))?;
            Some(parsed)
        }
        None => None,
    };
    Ok(SloInputMetrics {
        input_kind,
        session_id,
        reliability_metrics,
    })
}

/// Parses threshold configuration from JSON (direct keys or `{ "slo": { ... } }`).
pub fn parse_slo_thresholds_json(doc: &Value) -> Result<ReliabilitySloThresholds, SloError> {
    #[derive(Debug, Deserialize)]
    struct Wrapper {
        #[serde(default)]
        slo: Option<ReliabilitySloThresholds>,
    }
    let direct: ReliabilitySloThresholds = serde_json::from_value(doc.clone())
        .map_err(|e| SloError::InvalidThresholdFile(e.to_string()))?;
    let wrapped: Wrapper = serde_json::from_value(doc.clone())
        .map_err(|e| SloError::InvalidThresholdFile(e.to_string()))?;
    let merged = if let Some(slo) = wrapped.slo {
        slo.overlay(&direct)
    } else {
        direct
    };
    validate_reliability_slo_thresholds(&merged)?;
    Ok(merged)
}

/// Parses threshold configuration from TOML (direct keys or `[slo]` table).
pub fn parse_slo_thresholds_toml(raw: &str) -> Result<ReliabilitySloThresholds, SloError> {
    #[derive(Debug, Deserialize, Default)]
    struct Wrapper {
        #[serde(default)]
        slo: Option<ReliabilitySloThresholds>,
        #[serde(default)]
        min_tool_success_rate: Option<f64>,
        #[serde(default)]
        max_timeout_rate: Option<f64>,
        #[serde(default)]
        max_policy_denial_rate: Option<f64>,
        #[serde(default)]
        max_tool_failure_rate: Option<f64>,
        #[serde(default)]
        max_retries_total: Option<u64>,
        #[serde(default)]
        max_timeouts_total: Option<u64>,
        #[serde(default)]
        min_tool_calls_total: Option<u64>,
    }
    let parsed: Wrapper =
        toml::from_str(raw).map_err(|e| SloError::InvalidThresholdFile(e.to_string()))?;
    let direct = ReliabilitySloThresholds {
        min_tool_success_rate: parsed.min_tool_success_rate,
        max_timeout_rate: parsed.max_timeout_rate,
        max_policy_denial_rate: parsed.max_policy_denial_rate,
        max_tool_failure_rate: parsed.max_tool_failure_rate,
        max_retries_total: parsed.max_retries_total,
        max_timeouts_total: parsed.max_timeouts_total,
        min_tool_calls_total: parsed.min_tool_calls_total,
    };
    let merged = if let Some(slo) = parsed.slo {
        slo.overlay(&direct)
    } else {
        direct
    };
    validate_reliability_slo_thresholds(&merged)?;
    Ok(merged)
}

/// Evaluates thresholds against one parsed run/evidence metrics input.
pub fn evaluate_reliability_slos(
    input: &SloInputMetrics,
    thresholds: &ReliabilitySloThresholds,
    strict: bool,
) -> Result<ReliabilitySloEvaluation, SloError> {
    validate_reliability_slo_thresholds(thresholds)?;
    let mut violations: Vec<SloCheckResult> = Vec::new();
    let mut skipped_checks: Vec<SloCheckResult> = Vec::new();

    let metrics = input.reliability_metrics.as_ref();
    let min_calls = thresholds.min_tool_calls_total.unwrap_or(0);
    let calls = metrics.map(|m| m.tool_calls_total);
    let insufficient_sample = calls.is_some_and(|v| v < min_calls);

    let mut register_check = |result: SloCheckResult| match result.status {
        SloCheckStatus::Violation => violations.push(result),
        SloCheckStatus::Skipped => {
            if strict {
                let mut strict_result = result.clone();
                strict_result.status = SloCheckStatus::Violation;
                strict_result.reason_code = format!("strict_{}", strict_result.reason_code);
                strict_result.message = format!("strict mode: {}", strict_result.message);
                violations.push(strict_result);
            } else {
                skipped_checks.push(result);
            }
        }
        SloCheckStatus::Pass => {}
    };

    if thresholds.min_tool_success_rate.is_some() {
        register_check(evaluate_rate_threshold(
            "min_tool_success_rate",
            metrics,
            thresholds.min_tool_success_rate,
            min_calls,
            insufficient_sample,
            |m| safe_rate(m.tool_calls_success, m.tool_calls_total),
            true,
        ));
    }
    if thresholds.max_timeout_rate.is_some() {
        register_check(evaluate_rate_threshold(
            "max_timeout_rate",
            metrics,
            thresholds.max_timeout_rate,
            min_calls,
            insufficient_sample,
            |m| safe_rate(m.timeouts_total, m.tool_calls_total),
            false,
        ));
    }
    if thresholds.max_policy_denial_rate.is_some() {
        register_check(evaluate_rate_threshold(
            "max_policy_denial_rate",
            metrics,
            thresholds.max_policy_denial_rate,
            min_calls,
            insufficient_sample,
            |m| safe_rate(m.policy_denials_total, m.tool_calls_total),
            false,
        ));
    }
    if thresholds.max_tool_failure_rate.is_some() {
        register_check(evaluate_rate_threshold(
            "max_tool_failure_rate",
            metrics,
            thresholds.max_tool_failure_rate,
            min_calls,
            insufficient_sample,
            |m| safe_rate(m.tool_calls_failure, m.tool_calls_total),
            false,
        ));
    }

    if let Some(max) = thresholds.max_retries_total {
        register_check(evaluate_total_max("max_retries_total", metrics, max, |m| {
            m.retries_total
        }));
    }
    if let Some(max) = thresholds.max_timeouts_total {
        register_check(evaluate_total_max(
            "max_timeouts_total",
            metrics,
            max,
            |m| m.timeouts_total,
        ));
    }
    let status = if violations.is_empty() {
        SloCheckStatus::Pass
    } else {
        SloCheckStatus::Violation
    };
    Ok(ReliabilitySloEvaluation {
        status,
        input_kind: input.input_kind,
        session_id: input.session_id.clone(),
        strict,
        evaluated_metrics: input.reliability_metrics.clone(),
        applied_thresholds: thresholds.clone(),
        violations,
        skipped_checks,
    })
}

fn evaluate_rate_threshold(
    check_id: &str,
    metrics: Option<&RunReliabilityMetrics>,
    threshold: Option<f64>,
    min_calls: u64,
    insufficient_sample: bool,
    actual_fn: impl Fn(&RunReliabilityMetrics) -> Option<f64>,
    is_min: bool,
) -> SloCheckResult {
    let Some(threshold_value) = threshold else {
        return SloCheckResult {
            check_id: check_id.to_string(),
            status: SloCheckStatus::Skipped,
            reason_code: "threshold_not_enabled".to_string(),
            message: "threshold not enabled".to_string(),
            actual: None,
            threshold: None,
        };
    };
    let Some(m) = metrics else {
        return skipped_result(
            check_id,
            "missing_metrics",
            "reliability_metrics block is missing",
            None,
            Some(Value::from(threshold_value)),
        );
    };
    if insufficient_sample {
        return skipped_result(
            check_id,
            "insufficient_sample",
            format!(
                "tool_calls_total={} is below min_tool_calls_total={}",
                m.tool_calls_total, min_calls
            ),
            Some(Value::from(m.tool_calls_total)),
            Some(Value::from(min_calls)),
        );
    }
    let Some(actual_value) = actual_fn(m) else {
        return skipped_result(
            check_id,
            "rate_unavailable",
            "rate denominator is zero".to_string(),
            None,
            Some(Value::from(threshold_value)),
        );
    };
    let pass = if is_min {
        actual_value >= threshold_value
    } else {
        actual_value <= threshold_value
    };
    if pass {
        SloCheckResult {
            check_id: check_id.to_string(),
            status: SloCheckStatus::Pass,
            reason_code: "pass".to_string(),
            message: "threshold satisfied".to_string(),
            actual: Some(Value::from(actual_value)),
            threshold: Some(Value::from(threshold_value)),
        }
    } else {
        SloCheckResult {
            check_id: check_id.to_string(),
            status: SloCheckStatus::Violation,
            reason_code: "threshold_violated".to_string(),
            message: format!("actual={actual_value:.6} threshold={threshold_value:.6}"),
            actual: Some(Value::from(actual_value)),
            threshold: Some(Value::from(threshold_value)),
        }
    }
}

fn evaluate_total_max(
    check_id: &str,
    metrics: Option<&RunReliabilityMetrics>,
    max: u64,
    actual_fn: impl Fn(&RunReliabilityMetrics) -> u64,
) -> SloCheckResult {
    let Some(m) = metrics else {
        return skipped_result(
            check_id,
            "missing_metrics",
            "reliability_metrics block is missing",
            None,
            Some(Value::from(max)),
        );
    };
    let actual = actual_fn(m);
    if actual <= max {
        SloCheckResult {
            check_id: check_id.to_string(),
            status: SloCheckStatus::Pass,
            reason_code: "pass".to_string(),
            message: "threshold satisfied".to_string(),
            actual: Some(Value::from(actual)),
            threshold: Some(Value::from(max)),
        }
    } else {
        SloCheckResult {
            check_id: check_id.to_string(),
            status: SloCheckStatus::Violation,
            reason_code: "threshold_violated".to_string(),
            message: format!("actual={actual} threshold={max}"),
            actual: Some(Value::from(actual)),
            threshold: Some(Value::from(max)),
        }
    }
}

fn skipped_result(
    check_id: &str,
    reason_code: &str,
    message: impl Into<String>,
    actual: Option<Value>,
    threshold: Option<Value>,
) -> SloCheckResult {
    SloCheckResult {
        check_id: check_id.to_string(),
        status: SloCheckStatus::Skipped,
        reason_code: reason_code.to_string(),
        message: message.into(),
        actual,
        threshold,
    }
}

fn safe_rate(numerator: u64, denominator: u64) -> Option<f64> {
    if denominator == 0 {
        return None;
    }
    Some(numerator as f64 / denominator as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_metrics() -> RunReliabilityMetrics {
        RunReliabilityMetrics {
            tool_calls_total: 10,
            tool_calls_success: 9,
            tool_calls_failure: 1,
            tool_latency_ms_total: 500,
            tool_latency_ms_avg: 50,
            tool_latency_ms_p95: Some(90),
            policy_denials_total: 1,
            retries_total: 1,
            timeouts_total: 0,
        }
    }

    #[test]
    fn threshold_validation_rejects_out_of_range_values() {
        let bad = ReliabilitySloThresholds {
            min_tool_success_rate: Some(1.2),
            ..Default::default()
        };
        let err = validate_reliability_slo_thresholds(&bad).expect_err("invalid");
        assert!(err.to_string().contains("min_tool_success_rate"));
    }

    #[test]
    fn healthy_metrics_pass_all_checks() {
        let input = SloInputMetrics {
            input_kind: SloInputKind::RunReport,
            session_id: Some("sess-1".into()),
            reliability_metrics: Some(sample_metrics()),
        };
        let thresholds = ReliabilitySloThresholds {
            min_tool_success_rate: Some(0.8),
            max_timeout_rate: Some(0.2),
            max_policy_denial_rate: Some(0.3),
            max_tool_failure_rate: Some(0.3),
            max_retries_total: Some(5),
            max_timeouts_total: Some(2),
            min_tool_calls_total: Some(5),
        };
        let eval = evaluate_reliability_slos(&input, &thresholds, false).expect("eval");
        assert_eq!(eval.status, SloCheckStatus::Pass);
        assert!(eval.violations.is_empty());
    }

    #[test]
    fn each_threshold_type_can_violate() {
        let input = SloInputMetrics {
            input_kind: SloInputKind::RunReport,
            session_id: Some("sess-1".into()),
            reliability_metrics: Some(RunReliabilityMetrics {
                tool_calls_total: 10,
                tool_calls_success: 3,
                tool_calls_failure: 7,
                tool_latency_ms_total: 0,
                tool_latency_ms_avg: 0,
                tool_latency_ms_p95: None,
                policy_denials_total: 6,
                retries_total: 9,
                timeouts_total: 8,
            }),
        };
        let thresholds = ReliabilitySloThresholds {
            min_tool_success_rate: Some(0.9),
            max_timeout_rate: Some(0.1),
            max_policy_denial_rate: Some(0.2),
            max_tool_failure_rate: Some(0.1),
            max_retries_total: Some(2),
            max_timeouts_total: Some(1),
            min_tool_calls_total: Some(1),
        };
        let eval = evaluate_reliability_slos(&input, &thresholds, false).expect("eval");
        assert_eq!(eval.status, SloCheckStatus::Violation);
        assert_eq!(eval.violations.len(), 6);
    }

    #[test]
    fn missing_metrics_non_strict_skips_and_strict_fails() {
        let input = SloInputMetrics {
            input_kind: SloInputKind::Evidence,
            session_id: Some("sess-1".into()),
            reliability_metrics: None,
        };
        let thresholds = ReliabilitySloThresholds {
            min_tool_success_rate: Some(0.95),
            max_retries_total: Some(1),
            ..Default::default()
        };
        let non_strict = evaluate_reliability_slos(&input, &thresholds, false).expect("eval");
        assert_eq!(non_strict.status, SloCheckStatus::Pass);
        assert_eq!(non_strict.skipped_checks.len(), 2);
        let strict = evaluate_reliability_slos(&input, &thresholds, true).expect("eval");
        assert_eq!(strict.status, SloCheckStatus::Violation);
        assert_eq!(strict.violations.len(), 2);
    }

    #[test]
    fn insufficient_sample_is_skipped_and_fails_in_strict() {
        let input = SloInputMetrics {
            input_kind: SloInputKind::RunReport,
            session_id: Some("sess-1".into()),
            reliability_metrics: Some(RunReliabilityMetrics {
                tool_calls_total: 2,
                tool_calls_success: 2,
                ..RunReliabilityMetrics::default()
            }),
        };
        let thresholds = ReliabilitySloThresholds {
            min_tool_success_rate: Some(0.9),
            min_tool_calls_total: Some(5),
            ..Default::default()
        };
        let non_strict = evaluate_reliability_slos(&input, &thresholds, false).expect("eval");
        assert_eq!(non_strict.status, SloCheckStatus::Pass);
        assert_eq!(non_strict.skipped_checks.len(), 1);
        let strict = evaluate_reliability_slos(&input, &thresholds, true).expect("eval");
        assert_eq!(strict.status, SloCheckStatus::Violation);
        assert_eq!(strict.violations.len(), 1);
    }

    #[test]
    fn parses_run_report_input() {
        let v = json!({
            "session_id": "sess-1",
            "status": "success",
            "tool_calls": [],
            "reliability_metrics": sample_metrics()
        });
        let parsed = extract_slo_input_metrics(&v).expect("parse");
        assert_eq!(parsed.input_kind, SloInputKind::RunReport);
        assert!(parsed.reliability_metrics.is_some());
    }

    #[test]
    fn parses_evidence_input() {
        let v = json!({
            "evidence_schema_version": "evidence.v1",
            "session_id": "sess-1",
            "reliability_metrics": sample_metrics()
        });
        let parsed = extract_slo_input_metrics(&v).expect("parse");
        assert_eq!(parsed.input_kind, SloInputKind::Evidence);
        assert!(parsed.reliability_metrics.is_some());
    }

    #[test]
    fn parses_thresholds_from_json_direct_and_wrapped() {
        let direct = json!({"min_tool_success_rate": 0.9, "max_retries_total": 2});
        let wrapped = json!({"slo": {"min_tool_success_rate": 0.9, "max_retries_total": 2}});
        let a = parse_slo_thresholds_json(&direct).expect("direct");
        let b = parse_slo_thresholds_json(&wrapped).expect("wrapped");
        assert_eq!(a.min_tool_success_rate, Some(0.9));
        assert_eq!(b.max_retries_total, Some(2));
    }

    #[test]
    fn parses_thresholds_from_toml_direct_and_wrapped() {
        let direct = "min_tool_success_rate = 0.95\nmax_timeouts_total = 2\n";
        let wrapped = "[slo]\nmin_tool_success_rate = 0.95\nmax_timeouts_total = 2\n";
        let a = parse_slo_thresholds_toml(direct).expect("direct");
        let b = parse_slo_thresholds_toml(wrapped).expect("wrapped");
        assert_eq!(a.min_tool_success_rate, Some(0.95));
        assert_eq!(b.max_timeouts_total, Some(2));
    }
}
