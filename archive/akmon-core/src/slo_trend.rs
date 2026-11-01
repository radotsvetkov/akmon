//! Reliability trend baseline aggregation and regression detection.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::RunReliabilityMetrics;

/// Normalized metrics used for baseline aggregation and trend comparisons.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedReliabilityMetrics {
    /// Total tool calls in the run.
    pub tool_calls_total: u64,
    /// Tool success rate (`tool_calls_success / tool_calls_total`).
    pub tool_success_rate: Option<f64>,
    /// Tool failure rate (`tool_calls_failure / tool_calls_total`).
    pub tool_failure_rate: Option<f64>,
    /// Timeout rate (`timeouts_total / tool_calls_total`).
    pub timeout_rate: Option<f64>,
    /// Total retries for this run.
    pub retries_total: u64,
    /// Average tool latency in milliseconds.
    pub tool_latency_ms_avg: u64,
}

/// Aggregated baseline statistics for one normalized metric.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineMetricStats {
    /// Arithmetic mean across baseline samples.
    pub mean: f64,
    /// Median across baseline samples.
    pub median: f64,
}

/// Baseline summary used for trend comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineReliabilitySummary {
    /// Number of baseline samples included in aggregation.
    pub sample_count: usize,
    /// Tool success rate stats.
    pub tool_success_rate: Option<BaselineMetricStats>,
    /// Tool failure rate stats.
    pub tool_failure_rate: Option<BaselineMetricStats>,
    /// Timeout rate stats.
    pub timeout_rate: Option<BaselineMetricStats>,
    /// Retries total stats.
    pub retries_total: Option<BaselineMetricStats>,
    /// Tool latency average stats.
    pub tool_latency_ms_avg: Option<BaselineMetricStats>,
}

/// Regression guardrail tolerances for trend evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RegressionGuardConfig {
    /// Maximum allowed absolute drop in tool success rate.
    pub max_success_rate_drop_abs: Option<f64>,
    /// Maximum allowed absolute increase in timeout rate.
    pub max_timeout_rate_increase_abs: Option<f64>,
    /// Maximum allowed absolute increase in tool failure rate.
    pub max_failure_rate_increase_abs: Option<f64>,
    /// Maximum allowed relative increase ratio for retries total.
    pub max_retries_increase_ratio: Option<f64>,
    /// Maximum allowed relative increase ratio for average tool latency.
    pub max_latency_avg_increase_ratio: Option<f64>,
    /// Minimum required valid baseline samples.
    pub min_baseline_samples: Option<usize>,
}

impl RegressionGuardConfig {
    /// Overlays `other` on top of `self` (non-`None` values in `other` win).
    pub fn overlay(&self, other: &Self) -> Self {
        Self {
            max_success_rate_drop_abs: other
                .max_success_rate_drop_abs
                .or(self.max_success_rate_drop_abs),
            max_timeout_rate_increase_abs: other
                .max_timeout_rate_increase_abs
                .or(self.max_timeout_rate_increase_abs),
            max_failure_rate_increase_abs: other
                .max_failure_rate_increase_abs
                .or(self.max_failure_rate_increase_abs),
            max_retries_increase_ratio: other
                .max_retries_increase_ratio
                .or(self.max_retries_increase_ratio),
            max_latency_avg_increase_ratio: other
                .max_latency_avg_increase_ratio
                .or(self.max_latency_avg_increase_ratio),
            min_baseline_samples: other.min_baseline_samples.or(self.min_baseline_samples),
        }
    }
}

/// Baseline parsing counts surfaced in trend output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrendSampleCounts {
    /// Total baseline artifacts discovered.
    pub total: usize,
    /// Baseline artifacts parsed as valid reliability samples.
    pub valid: usize,
    /// Baseline artifacts rejected as invalid/unparseable.
    pub invalid: usize,
    /// Valid samples used after applying `last N` window.
    pub used: usize,
}

/// One regression violation record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendViolation {
    /// Stable metric key.
    pub metric: String,
    /// Stable reason code.
    pub reason_code: String,
    /// Human-readable explanation.
    pub message: String,
    /// Current metric value.
    pub current: f64,
    /// Baseline comparison value.
    pub baseline: f64,
    /// Applied tolerance threshold.
    pub threshold: f64,
    /// Computed delta value compared with threshold semantics.
    pub delta: f64,
}

/// One skipped trend check record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendSkippedCheck {
    /// Stable metric/check key.
    pub metric: String,
    /// Stable reason code.
    pub reason_code: String,
    /// Human-readable explanation.
    pub message: String,
}

/// Overall trend check status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrendStatus {
    /// All enabled checks passed.
    Pass,
    /// One or more regression checks violated thresholds.
    RegressionViolation,
}

/// Regression evaluation result for current vs baseline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReliabilityTrendEvaluation {
    /// Overall evaluation status.
    pub status: TrendStatus,
    /// Strict mode flag.
    pub strict: bool,
    /// Current normalized metrics.
    pub current_metrics: NormalizedReliabilityMetrics,
    /// Baseline aggregated summary.
    pub baseline_summary: Option<BaselineReliabilitySummary>,
    /// Effective guardrail config.
    pub applied_regression_config: RegressionGuardConfig,
    /// Regression violations.
    pub violations: Vec<TrendViolation>,
    /// Skipped checks and reasons.
    pub skipped: Vec<TrendSkippedCheck>,
    /// Baseline sample parsing/selection counts.
    pub sample_counts: TrendSampleCounts,
}

/// Trend parsing and evaluation errors.
#[derive(Debug, Error)]
pub enum TrendError {
    /// Invalid guardrail config values.
    #[error("invalid trend guard config `{field}`: {message}")]
    InvalidConfig {
        /// Invalid config field.
        field: &'static str,
        /// Validation detail.
        message: String,
    },
    /// Invalid trend config file input.
    #[error("invalid trend config file: {0}")]
    InvalidConfigFile(String),
}

/// Normalizes one raw reliability metrics block for trend comparison.
pub fn normalize_reliability_metrics(
    raw: &RunReliabilityMetrics,
) -> Option<NormalizedReliabilityMetrics> {
    if !is_finite_nonnegative(raw.tool_latency_ms_avg as f64) {
        return None;
    }
    Some(NormalizedReliabilityMetrics {
        tool_calls_total: raw.tool_calls_total,
        tool_success_rate: safe_rate(raw.tool_calls_success, raw.tool_calls_total),
        tool_failure_rate: safe_rate(raw.tool_calls_failure, raw.tool_calls_total),
        timeout_rate: safe_rate(raw.timeouts_total, raw.tool_calls_total),
        retries_total: raw.retries_total,
        tool_latency_ms_avg: raw.tool_latency_ms_avg,
    })
}

/// Aggregates baseline samples using mean + median per metric.
pub fn aggregate_baseline_metrics(
    samples: &[NormalizedReliabilityMetrics],
) -> Option<BaselineReliabilitySummary> {
    if samples.is_empty() {
        return None;
    }
    let success = collect_rates(samples.iter().map(|s| s.tool_success_rate));
    let failure = collect_rates(samples.iter().map(|s| s.tool_failure_rate));
    let timeout = collect_rates(samples.iter().map(|s| s.timeout_rate));
    let retries = stats_from_values(
        samples
            .iter()
            .map(|s| s.retries_total as f64)
            .collect::<Vec<_>>()
            .as_slice(),
    );
    let latency = stats_from_values(
        samples
            .iter()
            .map(|s| s.tool_latency_ms_avg as f64)
            .collect::<Vec<_>>()
            .as_slice(),
    );
    Some(BaselineReliabilitySummary {
        sample_count: samples.len(),
        tool_success_rate: stats_from_values(&success),
        tool_failure_rate: stats_from_values(&failure),
        timeout_rate: stats_from_values(&timeout),
        retries_total: retries,
        tool_latency_ms_avg: latency,
    })
}

fn collect_rates<I: Iterator<Item = Option<f64>>>(iter: I) -> Vec<f64> {
    iter.filter_map(|v| v.filter(|x| x.is_finite() && *x >= 0.0))
        .collect::<Vec<_>>()
}

fn stats_from_values(values: &[f64]) -> Option<BaselineMetricStats> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values
        .iter()
        .copied()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .collect::<Vec<_>>();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(|a, b| a.total_cmp(b));
    let sum = sorted.iter().copied().sum::<f64>();
    let mean = sum / sorted.len() as f64;
    let mid = sorted.len() / 2;
    let median = if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    };
    Some(BaselineMetricStats { mean, median })
}

/// Validates trend guard config values.
pub fn validate_regression_guard_config(config: &RegressionGuardConfig) -> Result<(), TrendError> {
    validate_nonnegative(
        "max_success_rate_drop_abs",
        config.max_success_rate_drop_abs,
    )?;
    validate_nonnegative(
        "max_timeout_rate_increase_abs",
        config.max_timeout_rate_increase_abs,
    )?;
    validate_nonnegative(
        "max_failure_rate_increase_abs",
        config.max_failure_rate_increase_abs,
    )?;
    validate_nonnegative(
        "max_retries_increase_ratio",
        config.max_retries_increase_ratio,
    )?;
    validate_nonnegative(
        "max_latency_avg_increase_ratio",
        config.max_latency_avg_increase_ratio,
    )?;
    Ok(())
}

fn validate_nonnegative(field: &'static str, value: Option<f64>) -> Result<(), TrendError> {
    if let Some(v) = value
        && (!v.is_finite() || v < 0.0)
    {
        return Err(TrendError::InvalidConfig {
            field,
            message: "must be finite and >= 0".to_string(),
        });
    }
    Ok(())
}

/// Parses trend guard config from JSON.
pub fn parse_regression_guard_config_json(
    doc: &Value,
) -> Result<RegressionGuardConfig, TrendError> {
    #[derive(Debug, Deserialize, Default)]
    struct Wrapper {
        #[serde(default)]
        trend: Option<RegressionGuardConfig>,
        #[serde(default)]
        slo: Option<SloWrapper>,
    }
    #[derive(Debug, Deserialize, Default)]
    struct SloWrapper {
        #[serde(default)]
        trend: Option<RegressionGuardConfig>,
    }
    let direct: RegressionGuardConfig = serde_json::from_value(doc.clone())
        .map_err(|e| TrendError::InvalidConfigFile(e.to_string()))?;
    let wrapped: Wrapper = serde_json::from_value(doc.clone())
        .map_err(|e| TrendError::InvalidConfigFile(e.to_string()))?;

    let from_nested = wrapped
        .slo
        .and_then(|s| s.trend)
        .or(wrapped.trend)
        .unwrap_or_default();
    let merged = from_nested.overlay(&direct);
    validate_regression_guard_config(&merged)?;
    Ok(merged)
}

/// Parses trend guard config from TOML.
pub fn parse_regression_guard_config_toml(raw: &str) -> Result<RegressionGuardConfig, TrendError> {
    #[derive(Debug, Deserialize, Default)]
    struct Root {
        #[serde(default)]
        slo: Option<SloSection>,
        #[serde(default)]
        trend: Option<RegressionGuardConfig>,
        #[serde(default)]
        max_success_rate_drop_abs: Option<f64>,
        #[serde(default)]
        max_timeout_rate_increase_abs: Option<f64>,
        #[serde(default)]
        max_failure_rate_increase_abs: Option<f64>,
        #[serde(default)]
        max_retries_increase_ratio: Option<f64>,
        #[serde(default)]
        max_latency_avg_increase_ratio: Option<f64>,
        #[serde(default)]
        min_baseline_samples: Option<usize>,
    }
    #[derive(Debug, Deserialize, Default)]
    struct SloSection {
        #[serde(default)]
        trend: Option<RegressionGuardConfig>,
    }
    let parsed: Root =
        toml::from_str(raw).map_err(|e| TrendError::InvalidConfigFile(e.to_string()))?;
    let direct = RegressionGuardConfig {
        max_success_rate_drop_abs: parsed.max_success_rate_drop_abs,
        max_timeout_rate_increase_abs: parsed.max_timeout_rate_increase_abs,
        max_failure_rate_increase_abs: parsed.max_failure_rate_increase_abs,
        max_retries_increase_ratio: parsed.max_retries_increase_ratio,
        max_latency_avg_increase_ratio: parsed.max_latency_avg_increase_ratio,
        min_baseline_samples: parsed.min_baseline_samples,
    };
    let nested = parsed
        .slo
        .and_then(|s| s.trend)
        .or(parsed.trend)
        .unwrap_or_default();
    let merged = nested.overlay(&direct);
    validate_regression_guard_config(&merged)?;
    Ok(merged)
}

/// Evaluates current run metrics against baseline summary + regression guardrails.
pub fn evaluate_reliability_trend(
    current: &NormalizedReliabilityMetrics,
    baseline: Option<&BaselineReliabilitySummary>,
    config: &RegressionGuardConfig,
    strict: bool,
    sample_counts: TrendSampleCounts,
) -> Result<ReliabilityTrendEvaluation, TrendError> {
    validate_regression_guard_config(config)?;
    let mut violations: Vec<TrendViolation> = Vec::new();
    let mut skipped: Vec<TrendSkippedCheck> = Vec::new();

    let min_samples = config.min_baseline_samples.unwrap_or(5);
    if baseline.is_none() {
        handle_skipped(
            strict,
            &mut violations,
            &mut skipped,
            "baseline",
            "missing_baseline",
            "no valid baseline summary available",
        );
    }
    if let Some(b) = baseline
        && b.sample_count < min_samples
    {
        handle_skipped(
            strict,
            &mut violations,
            &mut skipped,
            "baseline",
            "insufficient_baseline_samples",
            format!(
                "baseline sample_count={} is below min_baseline_samples={}",
                b.sample_count, min_samples
            ),
        );
    }

    let baseline_usable = baseline.is_some_and(|b| b.sample_count >= min_samples);
    if baseline_usable && let Some(b) = baseline {
        if let Some(max_drop) = config.max_success_rate_drop_abs {
            compare_abs_drop(
                &mut violations,
                "tool_success_rate",
                current.tool_success_rate,
                b.tool_success_rate.as_ref().map(|s| s.median),
                max_drop,
            );
        }
        if let Some(max_inc) = config.max_timeout_rate_increase_abs {
            compare_abs_increase(
                &mut violations,
                "timeout_rate",
                current.timeout_rate,
                b.timeout_rate.as_ref().map(|s| s.median),
                max_inc,
            );
        }
        if let Some(max_inc) = config.max_failure_rate_increase_abs {
            compare_abs_increase(
                &mut violations,
                "tool_failure_rate",
                current.tool_failure_rate,
                b.tool_failure_rate.as_ref().map(|s| s.median),
                max_inc,
            );
        }
        if let Some(max_ratio) = config.max_retries_increase_ratio {
            compare_increase_ratio(
                &mut violations,
                &mut skipped,
                strict,
                "retries_total",
                current.retries_total as f64,
                b.retries_total.as_ref().map(|s| s.mean),
                max_ratio,
            );
        }
        if let Some(max_ratio) = config.max_latency_avg_increase_ratio {
            compare_increase_ratio(
                &mut violations,
                &mut skipped,
                strict,
                "tool_latency_ms_avg",
                current.tool_latency_ms_avg as f64,
                b.tool_latency_ms_avg.as_ref().map(|s| s.mean),
                max_ratio,
            );
        }
    }

    let status = if violations.is_empty() {
        TrendStatus::Pass
    } else {
        TrendStatus::RegressionViolation
    };
    Ok(ReliabilityTrendEvaluation {
        status,
        strict,
        current_metrics: current.clone(),
        baseline_summary: baseline.cloned(),
        applied_regression_config: config.clone(),
        violations,
        skipped,
        sample_counts,
    })
}

fn compare_abs_drop(
    violations: &mut Vec<TrendViolation>,
    metric: &str,
    current: Option<f64>,
    baseline: Option<f64>,
    max_drop: f64,
) {
    let Some(c) = current else { return };
    let Some(b) = baseline else { return };
    let drop = b - c;
    if drop > max_drop {
        violations.push(TrendViolation {
            metric: metric.to_string(),
            reason_code: "regression_abs_drop".to_string(),
            message: format!("absolute drop {drop:.6} exceeds {max_drop:.6}"),
            current: c,
            baseline: b,
            threshold: max_drop,
            delta: drop,
        });
    }
}

fn compare_abs_increase(
    violations: &mut Vec<TrendViolation>,
    metric: &str,
    current: Option<f64>,
    baseline: Option<f64>,
    max_increase: f64,
) {
    let Some(c) = current else { return };
    let Some(b) = baseline else { return };
    let increase = c - b;
    if increase > max_increase {
        violations.push(TrendViolation {
            metric: metric.to_string(),
            reason_code: "regression_abs_increase".to_string(),
            message: format!("absolute increase {increase:.6} exceeds {max_increase:.6}"),
            current: c,
            baseline: b,
            threshold: max_increase,
            delta: increase,
        });
    }
}

fn compare_increase_ratio(
    violations: &mut Vec<TrendViolation>,
    skipped: &mut Vec<TrendSkippedCheck>,
    strict: bool,
    metric: &str,
    current: f64,
    baseline: Option<f64>,
    max_ratio: f64,
) {
    let Some(b) = baseline else {
        handle_skipped(
            strict,
            violations,
            skipped,
            metric,
            "missing_baseline_metric",
            "baseline metric is unavailable",
        );
        return;
    };
    if b <= 0.0 {
        if current <= 0.0 {
            return;
        }
        handle_skipped(
            strict,
            violations,
            skipped,
            metric,
            "baseline_zero",
            "cannot compute increase ratio when baseline is zero",
        );
        return;
    }
    let ratio = (current / b) - 1.0;
    if ratio > max_ratio {
        violations.push(TrendViolation {
            metric: metric.to_string(),
            reason_code: "regression_ratio_increase".to_string(),
            message: format!("increase ratio {ratio:.6} exceeds {max_ratio:.6}"),
            current,
            baseline: b,
            threshold: max_ratio,
            delta: ratio,
        });
    }
}

fn handle_skipped(
    strict: bool,
    violations: &mut Vec<TrendViolation>,
    skipped: &mut Vec<TrendSkippedCheck>,
    metric: &str,
    reason_code: &str,
    message: impl Into<String>,
) {
    let msg = message.into();
    if strict {
        violations.push(TrendViolation {
            metric: metric.to_string(),
            reason_code: format!("strict_{reason_code}"),
            message: format!("strict mode: {msg}"),
            current: 0.0,
            baseline: 0.0,
            threshold: 0.0,
            delta: 0.0,
        });
    } else {
        skipped.push(TrendSkippedCheck {
            metric: metric.to_string(),
            reason_code: reason_code.to_string(),
            message: msg,
        });
    }
}

fn safe_rate(numerator: u64, denominator: u64) -> Option<f64> {
    if denominator == 0 {
        return None;
    }
    Some(numerator as f64 / denominator as f64)
}

fn is_finite_nonnegative(v: f64) -> bool {
    v.is_finite() && v >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn m(
        success: u64,
        total: u64,
        failure: u64,
        retries: u64,
        timeout: u64,
        latency: u64,
    ) -> RunReliabilityMetrics {
        RunReliabilityMetrics {
            tool_calls_total: total,
            tool_calls_success: success,
            tool_calls_failure: failure,
            tool_latency_ms_total: latency.saturating_mul(total),
            tool_latency_ms_avg: latency,
            tool_latency_ms_p95: Some(latency),
            policy_denials_total: 0,
            retries_total: retries,
            timeouts_total: timeout,
        }
    }

    #[test]
    fn aggregate_baseline_has_mean_and_median() {
        let a = normalize_reliability_metrics(&m(8, 10, 2, 1, 1, 20)).expect("norm");
        let b = normalize_reliability_metrics(&m(9, 10, 1, 2, 0, 30)).expect("norm");
        let summary = aggregate_baseline_metrics(&[a, b]).expect("summary");
        assert_eq!(summary.sample_count, 2);
        assert!(summary.tool_success_rate.is_some());
        assert!(summary.tool_latency_ms_avg.is_some());
    }

    #[test]
    fn trend_pass_case() {
        let current = normalize_reliability_metrics(&m(9, 10, 1, 1, 0, 25)).expect("norm");
        let baseline = aggregate_baseline_metrics(&[
            normalize_reliability_metrics(&m(9, 10, 1, 1, 0, 24)).expect("norm"),
            normalize_reliability_metrics(&m(10, 10, 0, 1, 0, 22)).expect("norm"),
            normalize_reliability_metrics(&m(9, 10, 1, 1, 0, 23)).expect("norm"),
            normalize_reliability_metrics(&m(9, 10, 1, 1, 0, 25)).expect("norm"),
            normalize_reliability_metrics(&m(9, 10, 1, 1, 0, 24)).expect("norm"),
        ]);
        let eval = evaluate_reliability_trend(
            &current,
            baseline.as_ref(),
            &RegressionGuardConfig {
                max_success_rate_drop_abs: Some(0.1),
                max_timeout_rate_increase_abs: Some(0.1),
                max_failure_rate_increase_abs: Some(0.1),
                max_retries_increase_ratio: Some(1.0),
                max_latency_avg_increase_ratio: Some(1.0),
                min_baseline_samples: Some(5),
            },
            false,
            TrendSampleCounts {
                total: 5,
                valid: 5,
                invalid: 0,
                used: 5,
            },
        )
        .expect("eval");
        assert_eq!(eval.status, TrendStatus::Pass);
    }

    #[test]
    fn trend_violation_for_each_rule_type() {
        let current = normalize_reliability_metrics(&m(4, 10, 6, 10, 5, 120)).expect("norm");
        let baseline = aggregate_baseline_metrics(&[
            normalize_reliability_metrics(&m(9, 10, 1, 2, 1, 40)).expect("norm"),
            normalize_reliability_metrics(&m(9, 10, 1, 2, 1, 45)).expect("norm"),
            normalize_reliability_metrics(&m(9, 10, 1, 2, 1, 42)).expect("norm"),
            normalize_reliability_metrics(&m(9, 10, 1, 2, 1, 43)).expect("norm"),
            normalize_reliability_metrics(&m(9, 10, 1, 2, 1, 41)).expect("norm"),
        ]);
        let eval = evaluate_reliability_trend(
            &current,
            baseline.as_ref(),
            &RegressionGuardConfig {
                max_success_rate_drop_abs: Some(0.1),
                max_timeout_rate_increase_abs: Some(0.1),
                max_failure_rate_increase_abs: Some(0.1),
                max_retries_increase_ratio: Some(1.0),
                max_latency_avg_increase_ratio: Some(1.0),
                min_baseline_samples: Some(5),
            },
            false,
            TrendSampleCounts {
                total: 5,
                valid: 5,
                invalid: 0,
                used: 5,
            },
        )
        .expect("eval");
        assert_eq!(eval.status, TrendStatus::RegressionViolation);
        assert!(eval.violations.len() >= 5);
    }

    #[test]
    fn insufficient_baseline_skips_or_fails_by_strict() {
        let current = normalize_reliability_metrics(&m(9, 10, 1, 1, 0, 30)).expect("norm");
        let baseline =
            aggregate_baseline_metrics(&[
                normalize_reliability_metrics(&m(9, 10, 1, 1, 0, 30)).expect("norm")
            ]);
        let cfg = RegressionGuardConfig {
            max_success_rate_drop_abs: Some(0.1),
            min_baseline_samples: Some(5),
            ..Default::default()
        };
        let non_strict = evaluate_reliability_trend(
            &current,
            baseline.as_ref(),
            &cfg,
            false,
            TrendSampleCounts {
                total: 1,
                valid: 1,
                invalid: 0,
                used: 1,
            },
        )
        .expect("eval");
        assert_eq!(non_strict.status, TrendStatus::Pass);
        assert!(!non_strict.skipped.is_empty());
        let strict = evaluate_reliability_trend(
            &current,
            baseline.as_ref(),
            &cfg,
            true,
            TrendSampleCounts {
                total: 1,
                valid: 1,
                invalid: 0,
                used: 1,
            },
        )
        .expect("eval");
        assert_eq!(strict.status, TrendStatus::RegressionViolation);
    }

    #[test]
    fn parse_trend_config_from_json_and_toml() {
        let j = json!({"slo": {"trend": {"max_success_rate_drop_abs": 0.1}}});
        let cj = parse_regression_guard_config_json(&j).expect("json");
        assert_eq!(cj.max_success_rate_drop_abs, Some(0.1));
        let t = "[slo.trend]\nmax_timeout_rate_increase_abs = 0.2\n";
        let ct = parse_regression_guard_config_toml(t).expect("toml");
        assert_eq!(ct.max_timeout_rate_increase_abs, Some(0.2));
    }
}
