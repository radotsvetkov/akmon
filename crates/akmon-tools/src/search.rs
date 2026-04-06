//! Search for regex patterns in UTF-8 text files under the sandbox (read-only).

use std::fs::{File, canonicalize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use akmon_core::{Permission, SandboxError};
use async_trait::async_trait;
use glob::Pattern as GlobPattern;
use regex::RegexBuilder;
use serde_json::{Value as JsonValue, json};
use walkdir::WalkDir;

use crate::Tool;
use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};

/// Default maximum number of matching lines returned in one tool call.
pub const DEFAULT_MAX_SEARCH_RESULTS: usize = 50;
/// Default maximum file size (bytes) to read for searching.
pub const DEFAULT_MAX_SEARCH_FILE_BYTES: usize = 1024 * 1024;
/// Bytes read from the start of a file to detect embedded nulls (binary heuristic).
const BINARY_SNIFF_LEN: usize = 8192;

fn search_permissions() -> &'static [Permission] {
    static CELL: OnceLock<[Permission; 1]> = OnceLock::new();
    CELL.get_or_init(|| {
        [Permission::ReadFile {
            path: PathBuf::new(),
        }]
    })
    .as_slice()
}

/// Recursively searches UTF-8 text files under a sandbox path for a regex pattern.
pub struct SearchTool {
    max_results: usize,
    max_file_size_bytes: usize,
}

impl SearchTool {
    /// Builds a searcher with [`DEFAULT_MAX_SEARCH_RESULTS`] and [`DEFAULT_MAX_SEARCH_FILE_BYTES`].
    pub fn new() -> Self {
        Self {
            max_results: DEFAULT_MAX_SEARCH_RESULTS,
            max_file_size_bytes: DEFAULT_MAX_SEARCH_FILE_BYTES,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_limits(max_results: usize, max_file_size_bytes: usize) -> Self {
        Self {
            max_results,
            max_file_size_bytes,
        }
    }
}

/// Same as [`SearchTool::new`].
impl Default for SearchTool {
    fn default() -> Self {
        Self::new()
    }
}

fn path_under_sandbox(file: &Path, sandbox_root: &Path) -> bool {
    match canonicalize(file) {
        Ok(c) => c.starts_with(sandbox_root),
        Err(_) => false,
    }
}

fn relative_path_display(file: &Path, sandbox_root: &Path) -> Option<String> {
    let c = canonicalize(file).ok()?;
    let root = canonicalize(sandbox_root).ok()?;
    c.strip_prefix(&root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
}

/// Returns `true` if the first up to 8KB of the file contains a NUL byte.
fn file_looks_binary(path: &Path, meta_len: u64) -> std::io::Result<bool> {
    let mut f = File::open(path)?;
    let take = (BINARY_SNIFF_LEN as u64).min(meta_len) as usize;
    let mut buf = vec![0u8; take];
    let n = f.read(&mut buf)?;
    buf.truncate(n);
    Ok(buf.contains(&0))
}

fn read_file_capped(path: &Path, max: usize) -> std::io::Result<Vec<u8>> {
    let f = File::open(path)?;
    let mut buf = Vec::new();
    f.take(max as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

fn file_name_matches(glob: &GlobPattern, path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| glob.matches(name))
}

struct SearchRun {
    pattern: String,
    results: Vec<JsonValue>,
    truncated: bool,
}

fn run_search(
    pattern_str: &str,
    case_sensitive: bool,
    start: PathBuf,
    sandbox_root: PathBuf,
    file_glob: Option<GlobPattern>,
    max_results: usize,
    max_file_size_bytes: usize,
) -> Result<SearchRun, String> {
    let root_canonical = canonicalize(&sandbox_root).map_err(|e| e.to_string())?;

    let re = RegexBuilder::new(pattern_str)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|e| format!("invalid regex pattern: {e}"))?;

    let mut results: Vec<JsonValue> = Vec::new();
    let mut truncated = false;

    'walk: for entry in WalkDir::new(&start).follow_links(false) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !entry.file_type().is_file() {
            continue;
        }
        if !path_under_sandbox(path, &root_canonical) {
            continue;
        }

        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let len = meta.len() as usize;
        if len > max_file_size_bytes {
            continue;
        }

        if let Some(ref g) = file_glob
            && !file_name_matches(g, path)
        {
            continue;
        }

        match file_looks_binary(path, meta.len()) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(_) => continue,
        }

        let bytes = match read_file_capped(path, max_file_size_bytes) {
            Ok(b) => b,
            Err(_) => continue,
        };

        let text = match std::str::from_utf8(&bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let rel = match relative_path_display(path, &sandbox_root) {
            Some(r) => r,
            None => continue,
        };

        let lines: Vec<&str> = text.lines().collect();
        let line_count = lines.len();

        for (i, line) in lines.iter().enumerate() {
            if re.is_match(line) {
                let context_before = if i > 0 {
                    (*lines[i - 1]).to_string()
                } else {
                    String::new()
                };
                let context_after = if i + 1 < line_count {
                    (*lines[i + 1]).to_string()
                } else {
                    String::new()
                };

                results.push(json!({
                    "file": rel,
                    "line": i + 1,
                    "content": *line,
                    "context_before": context_before,
                    "context_after": context_after,
                }));

                if results.len() >= max_results {
                    truncated = true;
                    break 'walk;
                }
            }
        }
    }

    Ok(SearchRun {
        pattern: pattern_str.to_string(),
        results,
        truncated,
    })
}

/// Registers the `search` tool: regex search over UTF-8 text files under the sandbox root.
#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }

    fn description(&self) -> &str {
        "Search for a pattern in files within the project sandbox. Use this to find function definitions, variable names, imports, or any text pattern. Always search before editing. On success, the JSON field total_matches is the number of result rows returned in this response (same as the length of results), not necessarily the total count of matches in the codebase; when truncated is true, more matches may exist beyond the limit."
    }

    fn required_permissions(&self) -> &[Permission] {
        search_permissions()
    }

    fn parameters_schema(&self) -> JsonValue {
        serde_json::json!({
            "type": "object",
            "description": "Arguments for search. Successful tool output JSON includes total_matches: the count of entries in results for this response only; when truncated is true, additional matches may exist in the tree beyond that count.",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Text or regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search in. Use '.' for entire project. Default: '.'"
                },
                "file_pattern": {
                    "type": "string",
                    "description": "Glob pattern to filter files. Example: '*.rs' or '*.toml'. Default: all files"
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Default false"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: JsonValue, ctx: &ToolContext) -> ToolOutput {
        let pattern_str = match args.get("pattern").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "missing or empty \"pattern\" string".into(),
                };
            }
        };

        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(".");

        let case_sensitive = args
            .get("case_sensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let file_glob = match args.get("file_pattern").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => match GlobPattern::new(s) {
                Ok(p) => Some(p),
                Err(e) => {
                    return ToolOutput::Error {
                        code: ToolErrorCode::InvalidArgs,
                        message: format!("invalid file_pattern glob: {e}"),
                    };
                }
            },
            _ => None,
        };

        let resolved = match ctx.resolve_path(path_str) {
            Ok(p) => p,
            Err(SandboxError::PathEscape { .. }) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::PathEscape,
                    message: format!("path escapes sandbox: {path_str}"),
                };
            }
            Err(SandboxError::Canonicalize(e)) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    return ToolOutput::Error {
                        code: ToolErrorCode::NotFound,
                        message: format!("path not found: {path_str}"),
                    };
                }
                return ToolOutput::Error {
                    code: ToolErrorCode::PermissionDenied,
                    message: format!("failed to resolve path: {e}"),
                };
            }
        };

        let start = resolved.clone();
        let sandbox_root = ctx.primary_root();
        let pattern_owned = pattern_str.to_string();
        let max_results = self.max_results;
        let max_file_size = self.max_file_size_bytes;

        let outcome = tokio::task::spawn_blocking(move || {
            run_search(
                &pattern_owned,
                case_sensitive,
                start,
                sandbox_root,
                file_glob,
                max_results,
                max_file_size,
            )
        })
        .await;

        let run = match outcome {
            Ok(Ok(r)) => r,
            Ok(Err(msg)) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: msg,
                };
            }
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::PermissionDenied,
                    message: format!("search task failed: {e}"),
                };
            }
        };

        // total_matches = number of rows in `results` for this response; when `truncated` is true,
        // more matches may exist in the project than this count.
        let total_matches = run.results.len();
        let payload = json!({
            "pattern": run.pattern,
            "total_matches": total_matches,
            "truncated": run.truncated,
            "results": run.results,
        });

        match serde_json::to_string(&payload) {
            Ok(content) => ToolOutput::Success { content },
            Err(e) => ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: format!("serialize search results: {e}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_core::PolicyEngine;
    use serde_json::json;
    use std::sync::Arc;

    fn ctx(dir: &std::path::Path) -> ToolContext {
        let sandbox = akmon_core::Sandbox::new(dir);
        let policy = Arc::new(PolicyEngine::new(akmon_core::PolicyEngineMode::DenyAll));
        ToolContext::new(sandbox, policy)
    }

    #[tokio::test]
    async fn finds_known_string() {
        let dir = tempfile::tempdir().expect("tmp");
        let f = dir.path().join("a.txt");
        std::fs::write(&f, "hello\nworld\n").expect("write");
        let tool = SearchTool::new();
        let out = tool
            .execute(json!({"pattern": "world", "path": "."}), &ctx(dir.path()))
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success: {out:?}");
        };
        let v: JsonValue = serde_json::from_str(&content).expect("json");
        assert_eq!(v["total_matches"], 1);
        assert_eq!(v["results"][0]["content"], "world");
    }

    #[tokio::test]
    async fn file_pattern_filters() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("a.rs"), "fn foo() {}\n").expect("w");
        std::fs::write(dir.path().join("b.txt"), "fn foo() {}\n").expect("w");
        let tool = SearchTool::new();
        let out = tool
            .execute(
                json!({
                    "pattern": "fn foo",
                    "path": ".",
                    "file_pattern": "*.rs"
                }),
                &ctx(dir.path()),
            )
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success");
        };
        let v: JsonValue = serde_json::from_str(&content).expect("json");
        assert_eq!(v["total_matches"], 1);
        let file = v["results"][0]["file"].as_str().expect("file path");
        assert!(file.ends_with("a.rs"));
    }

    #[tokio::test]
    async fn no_match_empty_results() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("x.txt"), "abc\n").expect("w");
        let tool = SearchTool::new();
        let out = tool
            .execute(json!({"pattern": "zzz", "path": "."}), &ctx(dir.path()))
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success");
        };
        let v: JsonValue = serde_json::from_str(&content).expect("json");
        assert_eq!(v["total_matches"], 0);
        assert_eq!(v["results"].as_array().map(|a| a.len()), Some(0));
    }

    #[tokio::test]
    async fn respects_max_results() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut s = String::new();
        for i in 0..10 {
            s.push_str(&format!("line{i} unique\n"));
        }
        std::fs::write(dir.path().join("t.txt"), s).expect("w");
        let tool = SearchTool::with_limits(3, DEFAULT_MAX_SEARCH_FILE_BYTES);
        let out = tool
            .execute(
                json!({"pattern": r"line\d+ unique", "path": "."}),
                &ctx(dir.path()),
            )
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success");
        };
        let v: JsonValue = serde_json::from_str(&content).expect("json");
        assert_eq!(v["total_matches"], 3);
        assert_eq!(v["truncated"], true);
    }

    #[tokio::test]
    async fn rejects_escape_path() {
        let dir = tempfile::tempdir().expect("tmp");
        let inner = dir.path().join("inner");
        std::fs::create_dir_all(&inner).expect("mkdir");
        std::fs::write(dir.path().join("outside.txt"), "x").expect("write");
        let tool = SearchTool::new();
        let out = tool
            .execute(
                json!({"pattern": "x", "path": "../outside.txt"}),
                &ctx(&inner),
            )
            .await;
        let ToolOutput::Error { code, .. } = out else {
            panic!("expected error");
        };
        assert_eq!(code, ToolErrorCode::PathEscape);
    }

    #[tokio::test]
    async fn skips_binary_file() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut bin = vec![b'h', b'i', 0, b'x'];
        bin.extend(std::iter::repeat_n(b'a', 100));
        std::fs::write(dir.path().join("b.bin"), bin).expect("w");
        std::fs::write(dir.path().join("ok.txt"), "hi\n").expect("w");
        let tool = SearchTool::new();
        let out = tool
            .execute(json!({"pattern": "hi", "path": "."}), &ctx(dir.path()))
            .await;
        let ToolOutput::Success { content } = out else {
            panic!("expected success");
        };
        let v: JsonValue = serde_json::from_str(&content).expect("json");
        assert_eq!(v["total_matches"], 1);
        let file = v["results"][0]["file"].as_str().expect("file path");
        assert!(file.contains("ok.txt"));
    }
}
