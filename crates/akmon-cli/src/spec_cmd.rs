//! `akmon spec` — multi-phase structured specs under `.akmon/specs/<feature>/`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Args;
use tokio::process::Command;

use crate::cli_forward::forward_cli_for_child_process;
use crate::Cli;

/// One phase of the spec workflow (parsed from trailing CLI words).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpecPhase {
    Requirements,
    Design,
    Tasks,
    Implement,
}

/// CLI arguments for `akmon spec <name> …` (multi-document workflow under `.akmon/specs/`).
#[derive(Args, Debug, Clone)]
pub struct SpecCmd {
    /// Directory name under `.akmon/specs/` (no path separators).
    pub feature_name: String,
    /// Requirements description words, or a single phase keyword (`design`, `tasks`, `implement`).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub rest: Vec<String>,
    /// Emphasize architecture-driving constraints in the requirements pass.
    #[arg(long = "design-first")]
    pub design_first: bool,
    /// Hint to the model that this is a greenfield effort.
    #[arg(long = "from-scratch")]
    pub from_scratch: bool,
}

/// Parses `rest` into a phase and optional requirements description.
pub(crate) fn parse_spec_phase(rest: &[String]) -> (SpecPhase, Option<String>) {
    match rest.first().map(|s| s.as_str()) {
        Some("design") if rest.len() == 1 => (SpecPhase::Design, None),
        Some("tasks") if rest.len() == 1 => (SpecPhase::Tasks, None),
        Some("implement") if rest.len() == 1 => (SpecPhase::Implement, None),
        _ => {
            let desc = if rest.is_empty() {
                None
            } else {
                Some(rest.join(" "))
            };
            (SpecPhase::Requirements, desc)
        }
    }
}

fn validate_feature_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("feature name must not be empty".into());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("feature name must not contain path separators or '..'".into());
    }
    Ok(())
}

fn spec_root(project_root: &Path, feature: &str) -> PathBuf {
    project_root
        .join(".akmon")
        .join("specs")
        .join(feature)
}

/// Locates the first unchecked task line (`- [ ]` … `T-`).
pub(crate) fn first_unchecked_task_line(tasks_md: &str) -> Option<String> {
    for line in tasks_md.lines() {
        let t = line.trim_start();
        if t.starts_with("- [ ]") && t.contains("T-") {
            return Some(line.to_string());
        }
    }
    None
}

/// Marks the first matching unchecked task line as completed (`- [x]`).
pub(crate) fn check_off_first_unchecked_task(tasks_md: &str) -> Option<String> {
    let mut out: Vec<String> = Vec::new();
    let mut done = false;
    for line in tasks_md.lines() {
        let t = line.trim_start();
        if !done && t.starts_with("- [ ]") && t.contains("T-") {
            let trimmed = line.trim_start();
            let rest = trimmed.strip_prefix("- [ ]").unwrap_or(trimmed);
            out.push(format!("- [x]{rest}"));
            done = true;
        } else {
            out.push(line.to_string());
        }
    }
    if done {
        Some(out.join("\n"))
    } else {
        None
    }
}

fn requirements_prompt(
    feature: &str,
    description: &str,
    out_path: &Path,
    design_first: bool,
    from_scratch: bool,
) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Generate requirements.md for the feature `{feature}`.\n\n\
         Description from the user:\n{description}\n\n\
         You MUST write the complete file using the write_file tool to this exact path:\n{}\n\n",
        out_path.display()
    ));
    if design_first {
        s.push_str("Emphasize constraints that will drive architecture; you may briefly note architectural risks.\n\n");
    }
    if from_scratch {
        s.push_str("Context: greenfield / from-scratch — no existing code obligations unless the open repo suggests reuse.\n\n");
    }
    s.push_str(
        "Format as user stories with acceptance criteria.\n\n\
         Structure:\n\
         # Requirements: {feature}\n\n\
         ## Overview\n\
         One paragraph describing what this is and why it exists.\n\n\
         ## User stories\n\
         For each requirement:\n\
         ### US-{{N}}: {{title}}\n\
         As a {{role}}, I want {{capability}} so that {{benefit}}.\n\n\
         Acceptance criteria:\n\
         - GIVEN {{context}} WHEN {{action}} THEN {{outcome}}\n\
         - (3–5 criteria per story)\n\n\
         ## Out of scope\n\
         Explicitly list what this does NOT do.\n\n\
         ## Open questions\n\
         Things that need clarification before design can begin.\n\n\
         Use only the write_file tool to create the document; path must match exactly.",
    );
    s.replace("{feature}", feature)
}

fn design_prompt(feature: &str, req_path: &Path, out_path: &Path, from_scratch: bool) -> String {
    let mut s = format!(
        "Generate design.md for the feature `{feature}`.\n\
         Read requirements from: {}\n\
         and explore the existing codebase as needed.\n\n\
         You MUST write the complete file using the write_file tool to:\n{}\n\n",
        req_path.display(),
        out_path.display()
    );
    if from_scratch {
        s.push_str("Context: greenfield — propose a clean architecture without assuming legacy integration unless requirements say so.\n\n");
    }
    s.push_str(
        "Structure:\n\
         # Design: {feature}\n\n\
         ## Architecture\n\
         How does this fit into the existing system? Which crates/modules are affected?\n\n\
         ## New components\n\
         For each new file or module:\n\
         ### {{component-name}}\n\
         - File: {{exact path}}\n\
         - Purpose: {{one sentence}}\n\
         - Key types/functions:\n\
           - {{TypeName}}: {{description}}\n\
           - {{fn_name}}({{args}}) -> {{return}}: {{description}}\n\n\
         ## Modified components\n\
         For each existing file that changes:\n\
         ### {{file-path}}\n\
         - Current behavior: …\n\
         - Required changes: …\n\
         - Specific lines/functions: …\n\n\
         ## Data flow\n\
         How does data move through the system?\n\n\
         ## Error handling\n\
         What errors can occur and how are they handled?\n\n\
         ## Testing strategy\n\
         What tests are needed? Unit, integration, property-based?\n\n\
         Use only the write_file tool; path must match exactly.",
    );
    s.replace("{feature}", feature)
}

fn tasks_prompt(feature: &str, req_path: &Path, design_path: &Path, out_path: &Path) -> String {
    format!(
        "Generate tasks.md for the feature `{feature}`.\n\
         Read requirements from: {}\n\
         and design from: {}\n\n\
         You MUST write the complete file using the write_file tool to:\n{}\n\n\
         Produce a checkbox list in dependency order:\n\n\
         # Tasks: {feature}\n\n\
         ## Phase 1: Foundation\n\
         - [ ] T-01: …\n\
           Depends on: nothing\n\
           Estimated: small (< 50 lines)\n\n\
         (add further phases: Integration, Tests, etc.)\n\n\
         ## Done criteria\n\
         Map back to acceptance criteria from requirements.\n\n\
         Use only the write_file tool; path must match exactly.",
        req_path.display(),
        design_path.display(),
        out_path.display(),
    )
}

fn implement_prompt(
    feature: &str,
    task_line: &str,
    req_path: &Path,
    design_path: &Path,
    tasks_path: &Path,
) -> String {
    format!(
        "You are implementing ONE task from the spec `{feature}`.\n\n\
         Task line from tasks.md (implement this and only this):\n{task_line}\n\n\
         Read for context:\n- {}\n- {}\n- {}\n\n\
         Implement the task completely. When finished, update tasks.md: change ONLY this task's line from `- [ ]` to `- [x]` (leave all other lines unchanged).\n\
         Use read_file, write_file, edit, patch, search, shell (if allowed), and git tools as needed.\n",
        req_path.display(),
        design_path.display(),
        tasks_path.display(),
    )
}

async fn run_akmon_child(project_root: &Path, cli: &Cli, task: String, auto_commit: bool) -> ExitCode {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("akmon: spec: cannot resolve current executable: {e}");
            return ExitCode::from(2);
        }
    };
    let mut cmd = Command::new(exe);
    cmd.current_dir(project_root);
    forward_cli_for_child_process(&mut cmd, cli, auto_commit);
    cmd.arg("--task").arg(task);
    match cmd.status().await {
        Ok(s) => {
            if s.success() {
                ExitCode::SUCCESS
            } else {
                let code = s.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(1);
                ExitCode::from(code)
            }
        }
        Err(e) => {
            eprintln!("akmon: spec: failed to spawn agent child: {e}");
            ExitCode::from(2)
        }
    }
}

/// Executes one phase of the spec workflow by spawning a child `akmon --task …` with forwarded CLI flags.
pub async fn run_spec(cli: &Cli, project_root: &Path, cmd: SpecCmd) -> ExitCode {
    if let Err(e) = validate_feature_name(&cmd.feature_name) {
        eprintln!("akmon: spec: {e}");
        return ExitCode::from(2);
    }

    let (phase, desc_opt) = parse_spec_phase(&cmd.rest);
    let root = spec_root(project_root, &cmd.feature_name);
    let req = root.join("requirements.md");
    let design = root.join("design.md");
    let tasks = root.join("tasks.md");

    if let Err(e) = std::fs::create_dir_all(&root) {
        eprintln!("akmon: spec: cannot create {}: {e}", root.display());
        return ExitCode::from(2);
    }

    match phase {
        SpecPhase::Requirements => {
            let Some(description) = desc_opt.filter(|s| !s.trim().is_empty()) else {
                eprintln!(
                    "akmon: spec: provide a short description after the feature name, e.g.:\n  akmon spec my-feature \"rate-limit HTTP fetches to N per minute\""
                );
                return ExitCode::from(2);
            };
            let prompt = requirements_prompt(
                &cmd.feature_name,
                &description,
                &req,
                cmd.design_first,
                cmd.from_scratch,
            );
            eprintln!(
                "akmon: spec: generating requirements → {}",
                req.display()
            );
            let code = run_akmon_child(project_root, cli, prompt, false).await;
            if code == ExitCode::SUCCESS {
                eprintln!(
                    "Review requirements.md and run:\n  akmon spec {} design",
                    cmd.feature_name
                );
            }
            code
        }
        SpecPhase::Design => {
            if !req.is_file() {
                eprintln!(
                    "akmon: spec: missing {} — run requirements first.",
                    req.display()
                );
                return ExitCode::from(2);
            }
            let prompt = design_prompt(
                &cmd.feature_name,
                &req,
                &design,
                cmd.from_scratch,
            );
            eprintln!("akmon: spec: generating design → {}", design.display());
            let code = run_akmon_child(project_root, cli, prompt, false).await;
            if code == ExitCode::SUCCESS {
                eprintln!(
                    "Review design.md and run:\n  akmon spec {} tasks",
                    cmd.feature_name
                );
            }
            code
        }
        SpecPhase::Tasks => {
            if !req.is_file() || !design.is_file() {
                eprintln!(
                    "akmon: spec: need {} and {} — complete earlier phases first.",
                    req.display(),
                    design.display()
                );
                return ExitCode::from(2);
            }
            let prompt = tasks_prompt(&cmd.feature_name, &req, &design, &tasks);
            eprintln!("akmon: spec: generating tasks → {}", tasks.display());
            let code = run_akmon_child(project_root, cli, prompt, false).await;
            if code == ExitCode::SUCCESS {
                eprintln!(
                    "Review tasks.md and run:\n  akmon spec {} implement",
                    cmd.feature_name
                );
            }
            code
        }
        SpecPhase::Implement => {
            if !tasks.is_file() {
                eprintln!(
                    "akmon: spec: missing {} — run the tasks phase first.",
                    tasks.display()
                );
                return ExitCode::from(2);
            }
            let tasks_body = match std::fs::read_to_string(&tasks) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("akmon: spec: read {}: {e}", tasks.display());
                    return ExitCode::from(2);
                }
            };
            let Some(task_line_before) = first_unchecked_task_line(&tasks_body) else {
                eprintln!("akmon: spec: no unchecked tasks (`- [ ]` with `T-`) in tasks.md.");
                return ExitCode::SUCCESS;
            };
            let prompt = implement_prompt(
                &cmd.feature_name,
                &task_line_before,
                &req,
                &design,
                &tasks,
            );
            eprintln!("akmon: spec: implementing: {}", task_line_before.trim());
            let code = run_akmon_child(project_root, cli, prompt, cli.auto_commit).await;
            if code == ExitCode::SUCCESS {
                let updated = match std::fs::read_to_string(&tasks) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("akmon: spec: re-read {}: {e}", tasks.display());
                        return ExitCode::from(2);
                    }
                };
                let next_first = first_unchecked_task_line(&updated).map(|s| s.trim().to_string());
                let still_same = next_first.as_deref() == Some(task_line_before.trim());
                if still_same {
                    if let Some(new_body) = check_off_first_unchecked_task(&updated) {
                        if let Err(e) = std::fs::write(&tasks, new_body) {
                            eprintln!("akmon: spec: could not update tasks.md: {e}");
                            return ExitCode::from(2);
                        }
                    }
                }
                eprintln!(
                    "Review changes and run again for the next task:\n  akmon spec {} implement",
                    cmd.feature_name
                );
            }
            code
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_trailing_design() {
        let (p, d) = parse_spec_phase(&["design".into()]);
        assert_eq!(p, SpecPhase::Design);
        assert!(d.is_none());
    }

    #[test]
    fn parse_description_joins_words() {
        let (p, d) = parse_spec_phase(&["foo".into(), "bar".into()]);
        assert_eq!(p, SpecPhase::Requirements);
        assert_eq!(d.as_deref(), Some("foo bar"));
    }

    #[test]
    fn first_unchecked_finds_line() {
        let md = "# T\n\n- [x] T-01: a\n- [ ] T-02: b\n";
        assert_eq!(
            first_unchecked_task_line(md).as_deref(),
            Some("- [ ] T-02: b")
        );
    }

    #[test]
    fn check_off_updates_first_only() {
        let md = "- [ ] T-01: a\n- [ ] T-02: b\n";
        let out = check_off_first_unchecked_task(md).expect("updated");
        assert!(out.contains("- [x] T-01: a"));
        assert!(out.contains("- [ ] T-02: b"));
    }
}
