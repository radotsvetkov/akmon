use serde::{Deserialize, Serialize};

use crate::{DiffComparison, DiffDivergence, DiffMode, StructuralBreak};

const AGEF_VERSION: &str = "0.1";

/// Final diff report schema emitted by diff orchestration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffReportV1 {
    /// Akmon crate version producing this report.
    pub akmon_version: String,
    /// AGEF version targeted by diff semantics.
    pub agef_version: String,
    /// Session A UUID (hyphenated).
    pub session_a_id: String,
    /// Session B UUID (hyphenated).
    pub session_b_id: String,
    /// Diff mode used for comparison.
    pub mode: DiffMode,
    /// Number of events lockstep-compared.
    pub events_compared: usize,
    /// Session A event count.
    pub session_a_event_count: usize,
    /// Session B event count.
    pub session_b_event_count: usize,
    /// Structural break metadata, if comparison stopped early.
    pub structural_break: Option<StructuralBreak>,
    /// Total number of divergences.
    pub divergence_count: usize,
    /// Divergence list in emitted order.
    pub divergences: Vec<DiffDivergence>,
    /// True when no divergences and no structural break occurred.
    pub matches: bool,
}

impl DiffReportV1 {
    /// Builds a report from a finalized comparison artifact.
    #[must_use]
    pub fn from_comparison(
        comparison: DiffComparison,
        session_a_event_count: usize,
        session_b_event_count: usize,
    ) -> Self {
        let divergence_count = comparison.divergences.len();
        let matches = divergence_count == 0 && comparison.structural_break.is_none();
        Self {
            akmon_version: env!("CARGO_PKG_VERSION").to_owned(),
            agef_version: AGEF_VERSION.to_owned(),
            session_a_id: comparison.session_a_id,
            session_b_id: comparison.session_b_id,
            mode: comparison.mode,
            events_compared: comparison.events_compared,
            session_a_event_count,
            session_b_event_count,
            structural_break: comparison.structural_break,
            divergence_count,
            divergences: comparison.divergences,
            matches,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{DiffDivergenceKind, DiffMode};

    use super::*;

    fn sample_comparison(with_divergence: bool) -> DiffComparison {
        let mut comparison = DiffComparison::new("a".to_owned(), "b".to_owned(), DiffMode::Default);
        comparison.events_compared = 2;
        if with_divergence {
            comparison.structural_break = Some(StructuralBreak {
                position: 2,
                expected: "AssistantTurn".to_owned(),
                actual: "ToolCall".to_owned(),
            });
            comparison.divergences.push(crate::DiffDivergence {
                position: Some(2),
                kind: DiffDivergenceKind::StructuralBreakAtPosition,
                field: None,
                expected: "AssistantTurn".to_owned(),
                actual: "ToolCall".to_owned(),
                resolved: None,
                resolved_skip_reason: None,
            });
        }
        comparison
    }

    #[test]
    fn t_report_round_trip_json() {
        let report = DiffReportV1::from_comparison(sample_comparison(false), 2, 2);
        let encoded = serde_json::to_string(&report).expect("serialize");
        let decoded: DiffReportV1 = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, report);
    }

    #[test]
    fn t_report_json_shape_contains_expected_keys() {
        let report = DiffReportV1::from_comparison(sample_comparison(false), 2, 2);
        let value = serde_json::to_value(report).expect("serialize");
        assert!(value.get("akmon_version").is_some());
        assert!(value.get("agef_version").is_some());
        assert_eq!(
            value.get("mode").and_then(serde_json::Value::as_str),
            Some("default")
        );
        assert_eq!(
            value.get("matches").and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn t_report_matches_false_when_divergence_present() {
        let report = DiffReportV1::from_comparison(sample_comparison(true), 3, 3);
        assert!(!report.matches);
        assert_eq!(report.divergence_count, 1);
        assert!(report.structural_break.is_some());
    }
}
