//! Detect and read AI-tool context files under a project root for import into `AKMON.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Maximum file size (bytes) to read; larger files are skipped.
pub const CONTEXT_FILE_MAX_BYTES: usize = 50 * 1024;

/// Known origin of a context snippet on disk (IDE or agent product).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolOrigin {
    /// Anthropic Claude Code (`CLAUDE.md`, `.claude/CLAUDE.md`).
    ClaudeCode,
    /// OpenAI Codex-style exports (`akmon export --tool codex`).
    Codex,
    /// Cursor `.cursor/rules/*.mdc`.
    Cursor,
    /// Legacy Cursor `.cursorrules`.
    CursorLegacy,
    /// Gemini CLI `GEMINI.md`.
    GeminiCli,
    /// Amazon Kiro steering and spec paths under `.kiro/`.
    Kiro,
    /// Windsurf `.windsurfrules` or `.windsurf/rules/`.
    Windsurf,
    /// Aider `.aider.conf.yml`.
    Aider,
    /// GitHub Copilot instructions under `.github/`.
    GitHubCopilot,
    /// Cline `.clinerules`.
    Cline,
    /// Roo Code `.roo/rules/*.md`.
    RooCode,
    /// Shared or unknown files (`AGENTS.md`, `llms.txt`, …).
    Generic,
}

impl ToolOrigin {
    /// Human-readable product name for UI and prompts.
    pub fn display_name(&self) -> &'static str {
        match self {
            ToolOrigin::ClaudeCode => "Claude Code",
            ToolOrigin::Codex => "Codex",
            ToolOrigin::Cursor => "Cursor",
            ToolOrigin::CursorLegacy => "Cursor",
            ToolOrigin::GeminiCli => "Gemini CLI",
            ToolOrigin::Kiro => "Kiro",
            ToolOrigin::Windsurf => "Windsurf",
            ToolOrigin::Aider => "Aider",
            ToolOrigin::GitHubCopilot => "GitHub Copilot",
            ToolOrigin::Cline => "Cline",
            ToolOrigin::RooCode => "Roo Code",
            ToolOrigin::Generic => "Generic",
        }
    }

    /// Stable CLI flag value for `akmon import --from` / `akmon export --tool`.
    pub fn cli_name(&self) -> &'static str {
        match self {
            ToolOrigin::ClaudeCode => "claude-code",
            ToolOrigin::Codex => "codex",
            ToolOrigin::Cursor => "cursor",
            ToolOrigin::CursorLegacy => "cursor-legacy",
            ToolOrigin::GeminiCli => "gemini",
            ToolOrigin::Kiro => "kiro",
            ToolOrigin::Windsurf => "windsurf",
            ToolOrigin::Aider => "aider",
            ToolOrigin::GitHubCopilot => "copilot",
            ToolOrigin::Cline => "cline",
            ToolOrigin::RooCode => "roo",
            ToolOrigin::Generic => "generic",
        }
    }
}

/// One context file discovered under the project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextFile {
    /// Detected tool origin.
    pub tool: ToolOrigin,
    /// Path relative to the scan root (POSIX-style `/`).
    pub path: String,
    /// Normalized text contents (e.g. Cursor `.mdc` frontmatter stripped when applicable).
    pub content: String,
    /// Byte length on disk before optional normalization ([`ContextScan::primary_tool`] uses this).
    pub size_bytes: usize,
}

/// Result of [`scan_context_files`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextScan {
    /// Readable context files at or under `root` (never includes `AKMON.md`).
    pub files: Vec<ContextFile>,
    /// `true` when `AKMON.md` exists at the root.
    pub has_akmon_md: bool,
    /// Tool with the largest combined [`ContextFile::size_bytes`] for that tool, if any.
    pub primary_tool: Option<ToolOrigin>,
}

fn rel_path_str(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn read_limited(path: &Path) -> Option<(String, usize)> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let len = usize::try_from(meta.len()).unwrap_or(usize::MAX);
    if len > CONTEXT_FILE_MAX_BYTES {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    Some((content, len))
}

/// Removes leading `---` … `---` YAML block used in Cursor `.mdc` rules.
pub fn strip_mdc_style_frontmatter(content: &str) -> String {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content.to_string();
    }
    let after_open = trimmed[3..].strip_prefix('\n').unwrap_or(&trimmed[3..]);
    if let Some(end_rel) = after_open.find("\n---") {
        let after_delim = &after_open[end_rel + 4..];
        let body = after_delim.strip_prefix('\n').unwrap_or(after_delim);
        return body.to_string();
    }
    content.to_string()
}

fn push_single(root: &Path, files: &mut Vec<ContextFile>, rel: &str, tool: ToolOrigin) {
    let path = root.join(rel);
    let Some((content, len)) = read_limited(&path) else {
        return;
    };
    files.push(ContextFile {
        tool,
        path: rel_path_str(root, &path),
        size_bytes: len,
        content,
    });
}

fn glob_collect(root: &Path, pattern: &str) -> Vec<PathBuf> {
    let full = root.join(pattern);
    let Ok(g) = glob::glob(&full.to_string_lossy()) else {
        return Vec::new();
    };
    g.filter_map(std::result::Result::ok).collect()
}

fn push_glob_files(
    root: &Path,
    files: &mut Vec<ContextFile>,
    pattern: &str,
    tool: ToolOrigin,
    strip_frontmatter: bool,
) {
    for path in glob_collect(root, pattern) {
        let Some((mut content, len)) = read_limited(&path) else {
            continue;
        };
        if strip_frontmatter {
            content = strip_mdc_style_frontmatter(&content);
        }
        files.push(ContextFile {
            tool,
            path: rel_path_str(root, &path),
            size_bytes: len,
            content,
        });
    }
}

/// Tool with the largest combined [`ContextFile::size_bytes`] in `files`, if any.
pub fn primary_tool_from_files(files: &[ContextFile]) -> Option<ToolOrigin> {
    let mut totals: HashMap<ToolOrigin, usize> = HashMap::new();
    for f in files {
        *totals.entry(f.tool).or_insert(0) += f.size_bytes;
    }
    totals
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(t, _)| t)
}

fn dedup_paths(mut files: Vec<ContextFile>) -> Vec<ContextFile> {
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    files
}

/// Walks `root` for known AI context paths and reads small files into [`ContextScan`].
///
/// `AKMON.md` is noted in [`ContextScan::has_akmon_md`] only and never appears in [`ContextScan::files`].
pub fn scan_context_files(root: &Path) -> ContextScan {
    let has_akmon_md = root.join("AKMON.md").is_file();
    let mut files: Vec<ContextFile> = Vec::new();

    push_single(root, &mut files, "AGENTS.md", ToolOrigin::Generic);
    push_single(root, &mut files, "llms.txt", ToolOrigin::Generic);
    push_single(root, &mut files, "CLAUDE.md", ToolOrigin::ClaudeCode);
    push_single(root, &mut files, ".claude/CLAUDE.md", ToolOrigin::ClaudeCode);
    push_single(root, &mut files, ".cursorrules", ToolOrigin::CursorLegacy);
    push_glob_files(
        root,
        &mut files,
        ".cursor/rules/*.mdc",
        ToolOrigin::Cursor,
        true,
    );
    push_single(root, &mut files, "GEMINI.md", ToolOrigin::GeminiCli);
    push_glob_files(
        root,
        &mut files,
        ".kiro/steering/*.md",
        ToolOrigin::Kiro,
        false,
    );
    push_glob_files(
        root,
        &mut files,
        ".kiro/specs/**/requirements.md",
        ToolOrigin::Kiro,
        false,
    );
    push_glob_files(
        root,
        &mut files,
        ".kiro/specs/**/design.md",
        ToolOrigin::Kiro,
        false,
    );
    push_single(root, &mut files, ".windsurfrules", ToolOrigin::Windsurf);
    push_glob_files(
        root,
        &mut files,
        ".windsurf/rules/*",
        ToolOrigin::Windsurf,
        false,
    );
    push_single(root, &mut files, ".aider.conf.yml", ToolOrigin::Aider);
    push_single(
        root,
        &mut files,
        ".github/copilot-instructions.md",
        ToolOrigin::GitHubCopilot,
    );
    push_glob_files(
        root,
        &mut files,
        ".github/instructions/*.instructions.md",
        ToolOrigin::GitHubCopilot,
        false,
    );
    push_single(root, &mut files, ".clinerules", ToolOrigin::Cline);
    push_glob_files(root, &mut files, ".roo/rules/*.md", ToolOrigin::RooCode, false);

    let files = dedup_paths(files);
    let primary_tool = primary_tool_from_files(&files);

    ContextScan {
        files,
        has_akmon_md,
        primary_tool,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(p: &Path, body: &str) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::File::create(p)
            .and_then(|mut f| f.write_all(body.as_bytes()))
            .expect("write");
    }

    #[test]
    fn scan_finds_claude_md() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        write(&root.join("CLAUDE.md"), "hi");
        let s = scan_context_files(root);
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].tool, ToolOrigin::ClaudeCode);
        assert!(!s.has_akmon_md);
    }

    #[test]
    fn scan_finds_cursorrules_as_legacy() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        write(&root.join(".cursorrules"), "rules");
        let s = scan_context_files(root);
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].tool, ToolOrigin::CursorLegacy);
    }

    #[test]
    fn scan_cursor_mdc_strips_frontmatter() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        let p = root.join(".cursor/rules/r.mdc");
        write(
            &p,
            "---\nfoo: bar\n---\n\n# Body\n",
        );
        let s = scan_context_files(root);
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].tool, ToolOrigin::Cursor);
        assert!(s.files[0].content.contains("# Body"));
        assert!(!s.files[0].content.contains("foo: bar"));
    }

    #[test]
    fn scan_kiro_steering() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        write(&root.join(".kiro/steering/tech.md"), "steer");
        let s = scan_context_files(root);
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].tool, ToolOrigin::Kiro);
    }

    #[test]
    fn scan_clinerules() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        write(&root.join(".clinerules"), "x");
        let s = scan_context_files(root);
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].tool, ToolOrigin::Cline);
    }

    #[test]
    fn scan_empty_dir() {
        let dir = tempfile::tempdir().expect("tmp");
        let s = scan_context_files(dir.path());
        assert!(s.files.is_empty());
        assert!(!s.has_akmon_md);
        assert_eq!(s.primary_tool, None);
    }

    #[test]
    fn has_akmon_md_and_not_in_files() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        write(&root.join("AKMON.md"), "ak");
        write(&root.join("CLAUDE.md"), "cl");
        let s = scan_context_files(root);
        assert!(s.has_akmon_md);
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].path, "CLAUDE.md");
    }

    #[test]
    fn primary_tool_largest_combined() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        write(&root.join("CLAUDE.md"), "abc");
        write(&root.join("GEMINI.md"), "abcdef");
        let s = scan_context_files(root);
        assert_eq!(s.primary_tool, Some(ToolOrigin::GeminiCli));
    }

    #[test]
    fn scan_roo_rules_md() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        write(&root.join(".roo/rules/hi.md"), "roo");
        let s = scan_context_files(root);
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].tool, ToolOrigin::RooCode);
    }

    #[test]
    fn skips_large_files() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        let big = "x".repeat(CONTEXT_FILE_MAX_BYTES + 1024);
        write(&root.join("CLAUDE.md"), &big);
        let s = scan_context_files(root);
        assert!(s.files.is_empty());
    }
}
