//! `akmon scout` read-only context dossier generation.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::Utc;
use serde::{Deserialize, Serialize};

/// `akmon scout` command arguments.
#[derive(Debug, Clone, clap::Args)]
pub struct ScoutArgs {
    /// Question or task to scout.
    #[arg(long = "task")]
    pub task: String,
    /// Output dossier path (defaults to `<project>/.akmon/context/scout-<timestamp>.json`).
    #[arg(long = "out", value_name = "PATH")]
    pub out: Option<PathBuf>,
    /// Maximum number of files to scan.
    #[arg(long = "max-files", default_value_t = 200)]
    pub max_files: usize,
}

/// One candidate file and why it is relevant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoutCandidateFile {
    /// Sandbox-relative path.
    pub path: String,
    /// Short explanation for inclusion.
    pub rationale: String,
}

/// One structured scout dossier payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoutDossier {
    /// Stable schema version string.
    pub schema_version: String,
    /// Original user task.
    pub task: String,
    /// Canonical project root path.
    pub project_root: String,
    /// Sorted set of scanned directory paths.
    pub scanned_paths: Vec<String>,
    /// Sorted likely entrypoint files.
    pub key_entrypoints: Vec<String>,
    /// Sorted candidate files with rationale.
    pub candidate_files: Vec<ScoutCandidateFile>,
    /// Sorted related test files.
    pub related_tests: Vec<String>,
    /// Relevant execution constraints and budget posture.
    pub constraints: Vec<String>,
    /// Open questions that remain after bounded scouting.
    pub unresolved_questions: Vec<String>,
    /// Confidence in findings.
    pub confidence: String,
    /// Number of files considered.
    pub files_scanned: usize,
    /// Configured max-file bound.
    pub max_files: usize,
    /// True when scan stopped due to bounds.
    pub truncated: bool,
    /// Optional wall-clock generation timestamp (RFC3339).
    pub generated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScannedRepo {
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    truncated: bool,
}

/// Runs the scout command and optionally writes dossier JSON.
pub fn run_scout(
    args: ScoutArgs,
    project_root: &Path,
    json_output: bool,
    max_budget_usd: Option<f64>,
) -> ExitCode {
    if args.task.trim().is_empty() {
        emit_error(
            "scout task must be non-empty",
            json_output,
            Some(project_root),
            args.out.as_deref(),
        );
        return ExitCode::from(2);
    }
    if args.max_files == 0 {
        emit_error(
            "max-files must be >= 1",
            json_output,
            Some(project_root),
            args.out.as_deref(),
        );
        return ExitCode::from(2);
    }
    if let Some(v) = max_budget_usd
        && v < 0.0
    {
        emit_error(
            "max-budget-usd must be >= 0",
            json_output,
            Some(project_root),
            args.out.as_deref(),
        );
        return ExitCode::from(2);
    }

    let scanned = match scan_repo_files(project_root, args.max_files) {
        Ok(s) => s,
        Err(e) => {
            emit_error(
                &format!("scout failed to scan project: {e}"),
                json_output,
                Some(project_root),
                args.out.as_deref(),
            );
            return ExitCode::from(1);
        }
    };

    let dossier = build_dossier(
        project_root,
        args.task.as_str(),
        &scanned,
        args.max_files,
        max_budget_usd,
    );

    let out_path = args
        .out
        .clone()
        .unwrap_or_else(|| default_scout_output_path(project_root));
    if let Err(e) = write_dossier_json(&out_path, &dossier) {
        emit_error(
            &format!("failed to write dossier {}: {e}", out_path.display()),
            json_output,
            Some(project_root),
            Some(&out_path),
        );
        return ExitCode::from(1);
    }

    if json_output {
        match serde_json::to_string_pretty(&dossier) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                emit_error(
                    &format!("failed to serialize dossier JSON: {e}"),
                    json_output,
                    Some(project_root),
                    Some(&out_path),
                );
                return ExitCode::from(1);
            }
        }
    } else {
        println!(
            "scout: dossier written ({}) files_scanned={} truncated={} confidence={}",
            out_path.display(),
            dossier.files_scanned,
            dossier.truncated,
            dossier.confidence
        );
    }

    ExitCode::SUCCESS
}

fn emit_error(message: &str, json_output: bool, project_root: Option<&Path>, out: Option<&Path>) {
    if json_output {
        let payload = serde_json::json!({
            "ok": false,
            "error": message,
            "project_root": project_root.map(|p| p.display().to_string()),
            "out": out.map(|p| p.display().to_string()),
        });
        println!("{payload}");
    } else {
        eprintln!("akmon scout: {message}");
    }
}

fn default_scout_output_path(project_root: &Path) -> PathBuf {
    let ts = Utc::now().format("%Y%m%dT%H%M%SZ");
    project_root
        .join(".akmon")
        .join("context")
        .join(format!("scout-{ts}.json"))
}

fn write_dossier_json(path: &Path, dossier: &ScoutDossier) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let payload = serde_json::to_string_pretty(dossier).map_err(std::io::Error::other)?;
    std::fs::write(path, payload)
}

fn scan_repo_files(project_root: &Path, max_files: usize) -> std::io::Result<ScannedRepo> {
    let mut files = Vec::new();
    let mut dirs = BTreeSet::new();
    let mut stack = vec![project_root.to_path_buf()];
    let mut truncated = false;

    while let Some(dir) = stack.pop() {
        dirs.insert(dir.clone());
        let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(&dir)?.flatten().collect();
        entries.sort_by_key(|a| a.file_name());
        for entry in entries {
            let path = entry.path();
            let ty = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            let rel = match path.strip_prefix(project_root) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if should_skip_path(rel) {
                continue;
            }
            if ty.is_dir() {
                stack.push(path);
                stack.sort();
                continue;
            }
            if !ty.is_file() {
                continue;
            }
            files.push(path);
            if files.len() >= max_files {
                truncated = true;
                break;
            }
        }
        if truncated {
            break;
        }
    }

    let mut dirs_vec: Vec<PathBuf> = dirs.into_iter().collect();
    dirs_vec.sort();
    files.sort();
    Ok(ScannedRepo {
        files,
        dirs: dirs_vec,
        truncated,
    })
}

fn should_skip_path(rel: &Path) -> bool {
    let mut comps = rel.components();
    let Some(first) = comps.next() else {
        return false;
    };
    let first = first.as_os_str().to_string_lossy();
    matches!(
        first.as_ref(),
        ".git" | "target" | "node_modules" | ".next" | "dist" | "build"
    )
}

fn build_dossier(
    project_root: &Path,
    task: &str,
    scanned: &ScannedRepo,
    max_files: usize,
    max_budget_usd: Option<f64>,
) -> ScoutDossier {
    let task_tokens = tokenize(task);

    let mut scanned_paths: Vec<String> = scanned
        .dirs
        .iter()
        .filter_map(|p| p.strip_prefix(project_root).ok())
        .map(rel_to_slash)
        .collect();
    scanned_paths.sort();
    scanned_paths.dedup();

    let mut key_entrypoints = Vec::new();
    let mut related_tests = Vec::new();
    let mut candidates = Vec::new();

    for file in &scanned.files {
        let rel = match file.strip_prefix(project_root) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rel_s = rel_to_slash(rel);
        let lower = rel_s.to_ascii_lowercase();

        if is_key_entrypoint(rel_s.as_str()) {
            key_entrypoints.push(rel_s.clone());
        }
        if lower.contains("test") || lower.contains("/tests/") || lower.ends_with("_test.rs") {
            related_tests.push(rel_s.clone());
        }

        let mut score = 0usize;
        let mut reasons: Vec<String> = Vec::new();
        let matched: Vec<&str> = task_tokens
            .iter()
            .filter(|t| lower.contains(t.as_str()))
            .map(String::as_str)
            .collect();
        if !matched.is_empty() {
            score = score.saturating_add(2 + matched.len());
            reasons.push(format!("path matches task terms: {}", matched.join(", ")));
        }
        if is_key_entrypoint(rel_s.as_str()) {
            score = score.saturating_add(1);
            reasons.push("project entrypoint".to_string());
        }
        if lower.contains("policy") || lower.contains("reliab") || lower.contains("audit") {
            score = score.saturating_add(1);
            reasons.push("governance/reliability signal".to_string());
        }
        if score > 0 {
            candidates.push((score, rel_s, reasons.join("; ")));
        }
    }

    key_entrypoints.sort();
    key_entrypoints.dedup();
    related_tests.sort();
    related_tests.dedup();

    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let keep = candidates.len().min(20);
    let mut candidate_files: Vec<ScoutCandidateFile> = candidates
        .into_iter()
        .take(keep)
        .map(|(_, path, rationale)| ScoutCandidateFile { path, rationale })
        .collect();
    candidate_files.sort_by(|a, b| a.path.cmp(&b.path));

    let mut constraints = vec![
        "scout mode is read-only: no write/edit/patch/apply_patch/shell side effects".to_string(),
        format!(
            "bounded scan: max_files={} files_scanned={}",
            max_files,
            scanned.files.len()
        ),
        "network fetch disabled for scout analysis".to_string(),
    ];
    if let Some(v) = max_budget_usd {
        constraints.push(format!(
            "budget cap requested: ${v:.4}; scout analysis model spend is $0.0000"
        ));
    }
    if scanned.truncated {
        constraints.push(
            "truncation: max-files limit reached; dossier may omit additional relevant files"
                .to_string(),
        );
    }
    constraints.sort();

    let mut unresolved_questions = Vec::new();
    if scanned.truncated {
        unresolved_questions
            .push("Would increasing --max-files reveal additional candidate files?".to_string());
    }
    if candidate_files.is_empty() {
        unresolved_questions.push(
            "No strong candidate files found from task terms; refine task wording.".to_string(),
        );
    }
    unresolved_questions.sort();

    let confidence = if candidate_files.len() >= 6 && !scanned.truncated {
        "high"
    } else if candidate_files.len() >= 3 {
        "medium"
    } else {
        "low"
    }
    .to_string();

    ScoutDossier {
        schema_version: "context_scout.v1".to_string(),
        task: task.to_string(),
        project_root: project_root.display().to_string(),
        scanned_paths,
        key_entrypoints,
        candidate_files,
        related_tests,
        constraints,
        unresolved_questions,
        confidence,
        files_scanned: scanned.files.len(),
        max_files,
        truncated: scanned.truncated,
        generated_at: Some(Utc::now().to_rfc3339()),
    }
}

fn tokenize(task: &str) -> Vec<String> {
    let mut out: Vec<String> = task
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .filter(|s| s.len() >= 3)
        .map(|s| s.to_ascii_lowercase())
        .collect();
    out.sort();
    out.dedup();
    out
}

fn rel_to_slash(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

fn is_key_entrypoint(rel: &str) -> bool {
    matches!(
        rel,
        "Cargo.toml"
            | "README.md"
            | "AKMON.md"
            | "docs/src/SUMMARY.md"
            | ".github/workflows/ci.yml"
            | ".github/workflows/release.yml"
    ) || rel.ends_with("/src/main.rs")
        || rel.ends_with("/src/lib.rs")
        || rel.ends_with("/main.rs")
        || rel.ends_with("/lib.rs")
}

/// Loads and validates dossier JSON from disk.
pub fn load_dossier(path: &Path) -> Result<ScoutDossier, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read dossier {}: {e}", path.display()))?;
    let mut parsed: ScoutDossier = serde_json::from_str(&raw)
        .map_err(|e| format!("invalid dossier JSON {}: {e}", path.display()))?;
    if parsed.schema_version.trim().is_empty() {
        return Err("dossier missing schema_version".to_string());
    }
    parsed.scanned_paths.sort();
    parsed.scanned_paths.dedup();
    parsed.key_entrypoints.sort();
    parsed.key_entrypoints.dedup();
    parsed.related_tests.sort();
    parsed.related_tests.dedup();
    parsed.candidate_files.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.rationale.cmp(&b.rationale))
    });
    parsed.constraints.sort();
    parsed.constraints.dedup();
    parsed.unresolved_questions.sort();
    parsed.unresolved_questions.dedup();
    Ok(parsed)
}

/// Builds a compact prompt block from a dossier for follow-up implementation runs.
pub fn dossier_prompt_block(dossier: &ScoutDossier) -> String {
    let mut out = String::new();
    out.push_str("=== Context Scout Dossier ===\n");
    out.push_str(&format!("schema_version: {}\n", dossier.schema_version));
    out.push_str(&format!("task: {}\n", dossier.task));
    out.push_str(&format!("confidence: {}\n", dossier.confidence));
    out.push_str(&format!(
        "bounds: files_scanned={} max_files={} truncated={}\n",
        dossier.files_scanned, dossier.max_files, dossier.truncated
    ));
    out.push_str("key_entrypoints:\n");
    for p in &dossier.key_entrypoints {
        out.push_str(&format!("- {p}\n"));
    }
    out.push_str("candidate_files:\n");
    for c in dossier.candidate_files.iter().take(12) {
        out.push_str(&format!("- {} ({})\n", c.path, c.rationale));
    }
    if !dossier.related_tests.is_empty() {
        out.push_str("related_tests:\n");
        for t in dossier.related_tests.iter().take(10) {
            out.push_str(&format!("- {t}\n"));
        }
    }
    if !dossier.constraints.is_empty() {
        out.push_str("constraints:\n");
        for c in &dossier.constraints {
            out.push_str(&format!("- {c}\n"));
        }
    }
    if !dossier.unresolved_questions.is_empty() {
        out.push_str("unresolved_questions:\n");
        for q in &dossier.unresolved_questions {
            out.push_str(&format!("- {q}\n"));
        }
    }
    out.push_str("=== End Context Scout Dossier ===");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, content).expect("write");
    }

    #[test]
    fn scout_schema_is_valid_and_sorted() {
        let dir = tempfile::tempdir().expect("tmp");
        write_file(&dir.path().join("Cargo.toml"), "[workspace]\n");
        write_file(
            &dir.path().join("crates/akmon-cli/src/main.rs"),
            "fn main() {}\n",
        );
        write_file(
            &dir.path().join("crates/akmon-cli/tests/scout_test.rs"),
            "#[test] fn t() {}\n",
        );
        let scanned = scan_repo_files(dir.path(), 50).expect("scan");
        let dossier = build_dossier(dir.path(), "scout cli main", &scanned, 50, Some(1.0));
        assert_eq!(dossier.schema_version, "context_scout.v1");
        assert!(dossier.scanned_paths.windows(2).all(|w| w[0] <= w[1]));
        assert!(dossier.key_entrypoints.windows(2).all(|w| w[0] <= w[1]));
        assert!(dossier.related_tests.windows(2).all(|w| w[0] <= w[1]));
        assert!(
            dossier
                .candidate_files
                .windows(2)
                .all(|w| w[0].path <= w[1].path)
        );
    }

    #[test]
    fn scan_truncation_sets_indicator() {
        let dir = tempfile::tempdir().expect("tmp");
        for i in 0..5 {
            write_file(&dir.path().join(format!("src/f{i}.rs")), "fn x() {}\n");
        }
        let scanned = scan_repo_files(dir.path(), 2).expect("scan");
        assert!(scanned.truncated);
        let dossier = build_dossier(dir.path(), "src", &scanned, 2, None);
        assert!(dossier.truncated);
        assert!(
            dossier
                .constraints
                .iter()
                .any(|c| c.contains("truncation: max-files limit reached"))
        );
    }

    #[test]
    fn load_dossier_rejects_malformed_json() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path().join("bad.json");
        write_file(&p, "{bad");
        let err = load_dossier(&p).expect_err("must fail");
        assert!(err.contains("invalid dossier JSON"));
    }

    #[test]
    fn dossier_prompt_block_contains_key_sections() {
        let dossier = ScoutDossier {
            schema_version: "context_scout.v1".into(),
            task: "task".into(),
            project_root: "/tmp/repo".into(),
            scanned_paths: vec!["src".into()],
            key_entrypoints: vec!["Cargo.toml".into()],
            candidate_files: vec![ScoutCandidateFile {
                path: "src/main.rs".into(),
                rationale: "path matches task terms: main".into(),
            }],
            related_tests: vec!["tests/main_test.rs".into()],
            constraints: vec!["read-only".into()],
            unresolved_questions: vec![],
            confidence: "medium".into(),
            files_scanned: 10,
            max_files: 20,
            truncated: false,
            generated_at: None,
        };
        let block = dossier_prompt_block(&dossier);
        assert!(block.contains("Context Scout Dossier"));
        assert!(block.contains("candidate_files:"));
        assert!(block.contains("src/main.rs"));
    }

    #[test]
    fn scout_has_read_only_constraints() {
        let dir = tempfile::tempdir().expect("tmp");
        write_file(&dir.path().join("Cargo.toml"), "[workspace]\n");
        let scanned = scan_repo_files(dir.path(), 20).expect("scan");
        let dossier = build_dossier(dir.path(), "policy", &scanned, 20, None);
        assert!(
            dossier
                .constraints
                .iter()
                .any(|c| c.contains("read-only") && c.contains("no write/edit/patch"))
        );
        assert!(
            dossier
                .constraints
                .iter()
                .all(|c| !c.contains("write_file invoked"))
        );
    }
}
