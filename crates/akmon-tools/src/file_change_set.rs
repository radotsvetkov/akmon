//! Standardized machine-readable diff payload for file-modifying tools.

use serde::{Deserialize, Serialize};

/// One file diff entry in a [`FileChangeSet`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    /// Sandbox-relative path.
    pub path: String,
    /// Unified diff text (`--- a/...`, `+++ b/...`, hunks).
    pub diff: String,
    /// Added lines in hunks.
    pub lines_added: usize,
    /// Removed lines in hunks.
    pub lines_removed: usize,
    /// Approximate changed lines (`max(added, removed)`).
    pub lines_changed: usize,
}

/// Totals across all files in a [`FileChangeSet`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChangeSummary {
    /// Number of files changed.
    pub files_changed: usize,
    /// Total added lines.
    pub lines_added: usize,
    /// Total removed lines.
    pub lines_removed: usize,
    /// Total changed lines.
    pub lines_changed: usize,
}

/// Risk indicator derived from change shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeRisk {
    Low,
    Medium,
    High,
}

/// Whether proposed changes were persisted or only validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSetMode {
    /// Changes were written to disk.
    Applied,
    /// Changes were validated and diffed but not written.
    DryRun,
}

/// Standard output payload for `edit`, `write_file`, `patch`, `apply_patch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChangeSet {
    /// Stable payload discriminator for consumers.
    #[serde(rename = "type")]
    pub payload_type: String,
    /// Apply mode for this result.
    pub mode: ChangeSetMode,
    /// Canonical list of changed files.
    pub changes: Vec<FileChange>,
    /// Backward-compatible alias for older consumers.
    pub files: Vec<FileChange>,
    /// Aggregate summary.
    pub summary: FileChangeSummary,
    /// Heuristic risk classification.
    pub risk: ChangeRisk,
}

impl FileChangeSet {
    /// Build a payload from per-file diffs and line counts.
    pub fn from_files(mode: ChangeSetMode, changes: Vec<FileChange>) -> Self {
        let summary = FileChangeSummary {
            files_changed: changes.len(),
            lines_added: changes.iter().map(|f| f.lines_added).sum(),
            lines_removed: changes.iter().map(|f| f.lines_removed).sum(),
            lines_changed: changes.iter().map(|f| f.lines_changed).sum(),
        };
        let risk = classify_risk(&summary);
        Self {
            payload_type: "file_change_set".to_string(),
            mode,
            files: changes.clone(),
            changes,
            summary,
            risk,
        }
    }
}

/// Count added/removed/changed lines from a unified diff text.
pub fn diff_stats_from_unified(diff: &str) -> (usize, usize, usize) {
    let mut added = 0usize;
    let mut removed = 0usize;
    for line in diff.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            added += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            removed += 1;
        }
    }
    (added, removed, added.max(removed))
}

/// Classify risk from aggregate change volume.
///
/// Thresholds:
/// - `low`: <= 25 changed lines and <= 1 file
/// - `medium`: <= 200 changed lines and <= 5 files
/// - `high`: everything above
pub fn classify_risk(summary: &FileChangeSummary) -> ChangeRisk {
    if summary.lines_changed <= 25 && summary.files_changed <= 1 {
        ChangeRisk::Low
    } else if summary.lines_changed <= 200 && summary.files_changed <= 5 {
        ChangeRisk::Medium
    } else {
        ChangeRisk::High
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_low_risk() {
        let summary = FileChangeSummary {
            files_changed: 1,
            lines_added: 3,
            lines_removed: 2,
            lines_changed: 5,
        };
        assert_eq!(classify_risk(&summary), ChangeRisk::Low);
    }

    #[test]
    fn classifies_medium_risk() {
        let summary = FileChangeSummary {
            files_changed: 3,
            lines_added: 80,
            lines_removed: 60,
            lines_changed: 140,
        };
        assert_eq!(classify_risk(&summary), ChangeRisk::Medium);
    }

    #[test]
    fn classifies_high_risk() {
        let summary = FileChangeSummary {
            files_changed: 8,
            lines_added: 400,
            lines_removed: 200,
            lines_changed: 450,
        };
        assert_eq!(classify_risk(&summary), ChangeRisk::High);
    }

    #[test]
    fn payload_includes_type_mode_and_changes() {
        let payload = FileChangeSet::from_files(
            ChangeSetMode::DryRun,
            vec![FileChange {
                path: "src/main.rs".into(),
                diff: "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\n".into(),
                lines_added: 1,
                lines_removed: 1,
                lines_changed: 1,
            }],
        );
        let v = serde_json::to_value(payload).expect("serialize");
        assert_eq!(v["type"], "file_change_set");
        assert_eq!(v["mode"], "dry_run");
        assert_eq!(v["summary"]["files_changed"], 1);
        assert!(v["changes"].is_array());
        assert!(v["files"].is_array());
    }
}
