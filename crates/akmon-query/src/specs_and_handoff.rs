//! Load `.akmon/specs` and `HANDOFF.md` into prompts; write handoff on session end.

use std::fs;
use std::path::{Path, PathBuf};

use crate::session::AgentSession;

const SPECS_DIR: &str = ".akmon/specs";
const HANDOFF_FILE: &str = ".akmon/HANDOFF.md";

/// Project-relative specs directory.
#[must_use]
pub fn specs_dir(project_root: &Path) -> PathBuf {
    project_root.join(SPECS_DIR)
}

/// Path to `.akmon/HANDOFF.md`.
#[must_use]
pub fn handoff_path(project_root: &Path) -> PathBuf {
    project_root.join(HANDOFF_FILE)
}

/// Markdown block listing every `*.md` in `.akmon/specs/` for system injection.
#[must_use]
pub fn load_specs_block_for_prompt(project_root: &Path) -> Option<String> {
    let dir = specs_dir(project_root);
    if !dir.is_dir() {
        return None;
    }
    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();
    if entries.is_empty() {
        return None;
    }
    entries.sort();
    let mut out = String::from("## Saved specs (.akmon/specs)\n\n");
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        let body = fs::read_to_string(&p).unwrap_or_default();
        let trimmed = body.trim_end();
        out.push_str(&format!("### {name}\n\n{trimmed}\n\n"));
    }
    Some(out)
}

/// Handoff preamble for system injection (first lines of [`HANDOFF_FILE`] if present).
#[must_use]
pub fn load_handoff_block_for_prompt(project_root: &Path) -> Option<String> {
    let p = handoff_path(project_root);
    let s = fs::read_to_string(&p).ok()?;
    if s.trim().is_empty() {
        return None;
    }
    let trimmed = s.trim_end();
    Some(format!(
        "## Previous session handoff (.akmon/HANDOFF.md)\n\n{trimmed}\n",
    ))
}

/// Minimum completed user turns (`AgentSession::run` calls) before writing handoff.
pub const MIN_USER_TURNS_FOR_HANDOFF: u32 = 2;

/// Whether [`write_handoff_file`] should emit a file for this session state.
#[must_use]
pub fn should_write_handoff(session: &AgentSession) -> bool {
    session.user_turns_finished >= MIN_USER_TURNS_FOR_HANDOFF
        && (!session.modified_paths.is_empty() || session.last_assistant_snippet.is_some())
}

/// Writes `.akmon/HANDOFF.md` when [`should_write_handoff`] is true.
pub fn write_handoff_file(
    session: &AgentSession,
    project_root: &Path,
    model_label: &str,
) -> std::io::Result<()> {
    if !should_write_handoff(session) {
        return Ok(());
    }
    let dir = project_root.join(".akmon");
    fs::create_dir_all(&dir)?;
    let path = handoff_path(project_root);
    let mut body = String::new();
    body.push_str(&format!("**Model:** {model_label}\n\n"));
    let user_turns = session.user_turns_finished;
    body.push_str(&format!(
        "**Completed user turns this session:** {user_turns}\n\n",
    ));
    if !session.modified_paths.is_empty() {
        body.push_str("**Files touched:**\n");
        for p in &session.modified_paths {
            let path = p.display();
            body.push_str(&format!("- {path}\n"));
        }
        body.push('\n');
    }
    if let Some(snippet) = &session.last_assistant_snippet {
        body.push_str("**Last assistant summary:**\n\n");
        body.push_str(snippet);
        body.push('\n');
    }
    fs::write(&path, body)
}

/// Deletes `*.md` under `.akmon/specs/` (used by `/clear --hard`).
pub fn clear_specs_dir(project_root: &Path) -> std::io::Result<()> {
    let dir = specs_dir(project_root);
    if !dir.is_dir() {
        return Ok(());
    }
    for e in fs::read_dir(&dir)? {
        let e = e?;
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) == Some("md") {
            let _ = fs::remove_file(p);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn load_specs_joins_markdown_files() {
        let root = tempdir().expect("tmp");
        let specs = root.path().join(".akmon/specs");
        fs::create_dir_all(&specs).expect("mkdir");
        fs::write(specs.join("a.md"), "line a").expect("w");
        fs::write(specs.join("b.md"), "line b").expect("w");

        let block = load_specs_block_for_prompt(root.path()).expect("specs");
        assert!(block.contains("a.md"));
        assert!(block.contains("line a"));
        assert!(block.contains("b.md"));
    }

    #[test]
    fn load_handoff_trimmed() {
        let root = tempdir().expect("tmp");
        fs::create_dir_all(root.path().join(".akmon")).expect("d");
        let p = handoff_path(root.path());
        fs::write(&p, "handoff body\n").expect("w");
        let h = load_handoff_block_for_prompt(root.path()).expect("h");
        assert!(h.contains("handoff body"));
        assert!(h.contains("Previous session handoff"));
    }

    #[test]
    fn clear_specs_removes_only_md() {
        let root = tempdir().expect("tmp");
        let specs = root.path().join(".akmon/specs");
        fs::create_dir_all(&specs).expect("mkdir");
        fs::write(specs.join("x.md"), "x").expect("w");
        fs::write(specs.join("keep.txt"), "t").expect("w");

        clear_specs_dir(root.path()).expect("clear");
        assert!(!specs.join("x.md").exists());
        assert!(specs.join("keep.txt").is_file());
    }
}
