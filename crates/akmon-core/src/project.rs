//! Project type detection and file scaffolding for `AKMON.md` initialization.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// High-level classification inferred from repository markers at the project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectType {
    /// Rust workspace member or package (`Cargo.toml`).
    Rust {
        /// `package.edition` when present (e.g. `"2021"`).
        edition: String,
        /// `package.name`.
        name: String,
        /// Number of explicit `[[bin]]` targets in `Cargo.toml`.
        bin_count: usize,
        /// `true` when a `[lib]` section exists or `src/lib.rs` is present.
        lib: bool,
    },
    /// Node.js project (`package.json`).
    Node {
        /// `package.json` `name` field when present.
        name: String,
        /// First matching framework among supported dependencies, if any.
        framework: Option<String>,
    },
    /// Python project (`pyproject.toml`, `setup.py`, or `requirements.txt`).
    Python {
        /// Project name when discoverable from packaging metadata.
        name: String,
        /// Inferred framework (`fastapi`, `flask`, `django`) from dependencies.
        framework: Option<String>,
    },
    /// Go module (`go.mod`).
    Go {
        /// Module path from the `module` directive.
        name: String,
    },
    /// Fallback: non-hidden file names directly under the root directory.
    Generic {
        /// File names only (not directories), excluding dotfiles.
        files: Vec<String>,
    },
}

/// Summary returned by [`detect_project`]: type, markers, and environment flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSummary {
    /// Inferred [`ProjectType`].
    pub project_type: ProjectType,
    /// Root directory that was inspected (same as the `root` argument, normalized).
    pub root: PathBuf,
    /// Notable paths (relative to `root`) for context and prompts.
    pub key_files: Vec<String>,
    /// First ~200 bytes of `README.md` when present (lossy UTF-8).
    pub description: Option<String>,
    /// Whether `.git` exists under `root`.
    pub has_git: bool,
    /// Whether `AKMON.md` exists under `root`.
    pub has_akmon_md: bool,
}

/// Programming language selector for [`scaffold_project`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaffoldLanguage {
    /// Rust (`cargo`-style layout).
    Rust,
    /// Node.js (`package.json`).
    Node,
    /// Python (`pyproject.toml` / `src`).
    Python,
    /// Go (`go.mod`).
    Go,
    /// README + `.gitignore` only.
    Generic,
}

/// Project shape selector for [`scaffold_project`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaffoldKind {
    /// Command-line binary.
    Cli,
    /// Library crate / package.
    Lib,
    /// Web front-end or server (language-specific).
    Web,
    /// HTTP API service (language-specific).
    Api,
    /// Minimal generic layout.
    Generic,
}

/// Records paths created under the target directory (relative POSIX-style strings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaffoldReport {
    /// Relative paths written (files only).
    pub files_created: Vec<String>,
}

fn read_readme_excerpt(root: &Path) -> Option<String> {
    let p = root.join("README.md");
    let mut buf = fs::read(&p).ok()?;
    buf.truncate(buf.len().min(200));
    Some(String::from_utf8_lossy(&buf).trim().to_string())
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

fn list_root_files(root: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for ent in fs::read_dir(root)? {
        let ent = ent?;
        let name = ent.file_name().to_string_lossy().into_owned();
        if is_hidden(&name) {
            continue;
        }
        let meta = ent.metadata()?;
        if meta.is_file() {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

fn parse_cargo(root: &Path) -> io::Result<Option<ProjectType>> {
    let path = root.join("Cargo.toml");
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)?;
    let value: toml::Value = raw.parse().map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Cargo.toml parse error: {e}"),
        )
    })?;
    let pkg = value.get("package");
    let name = pkg
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let edition = pkg
        .and_then(|p| p.get("edition"))
        .and_then(|v| v.as_str())
        .unwrap_or("2021")
        .to_string();
    let bin_count = value
        .get("bin")
        .and_then(|b| b.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let has_lib_table = value.get("lib").is_some();
    let lib = has_lib_table || root.join("src/lib.rs").is_file();
    Ok(Some(ProjectType::Rust {
        edition,
        name,
        bin_count,
        lib,
    }))
}

fn first_dep_framework(deps: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let obj = deps.as_object()?;
    for k in keys {
        if obj.contains_key(*k) {
            return Some((*k).to_string());
        }
    }
    None
}

fn parse_package_json(root: &Path) -> io::Result<Option<ProjectType>> {
    let path = root.join("package.json");
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)?;
    let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("package.json parse error: {e}"),
        )
    })?;
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();
    let deps = v
        .get("dependencies")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let dev = v
        .get("devDependencies")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let framework = first_dep_framework(&deps, &["next", "react", "express", "vue", "svelte"])
        .or_else(|| first_dep_framework(&dev, &["next", "react", "express", "vue", "svelte"]));
    Ok(Some(ProjectType::Node { name, framework }))
}

fn parse_pyproject_name_and_framework(root: &Path) -> io::Result<Option<(String, Option<String>)>> {
    let path = root.join("pyproject.toml");
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)?;
    let value: toml::Value = raw.parse().map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("pyproject.toml parse error: {e}"),
        )
    })?;
    let name = value
        .get("project")
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            root.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        });
    let mut fw = None;
    if let Some(deps) = value
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
    {
        for item in deps {
            let s = item.as_str().unwrap_or("");
            let lower = s.to_lowercase();
            if lower.contains("fastapi") {
                fw = Some("fastapi".into());
                break;
            }
            if lower.contains("flask") {
                fw = Some("flask".into());
                break;
            }
            if lower.contains("django") {
                fw = Some("django".into());
                break;
            }
        }
    }
    Ok(Some((name, fw)))
}

fn scan_requirements_for_framework(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    for line in raw.lines() {
        let lower = line.to_lowercase();
        if lower.contains("fastapi") {
            return Some("fastapi".into());
        }
        if lower.contains("flask") {
            return Some("flask".into());
        }
        if lower.contains("django") {
            return Some("django".into());
        }
    }
    None
}

fn parse_python_project(root: &Path) -> io::Result<Option<ProjectType>> {
    if root.join("pyproject.toml").is_file()
        && let Some((name, mut fw)) = parse_pyproject_name_and_framework(root)?
    {
        if fw.is_none() {
            fw = scan_requirements_for_framework(&root.join("requirements.txt"));
        }
        return Ok(Some(ProjectType::Python {
            name,
            framework: fw,
        }));
    }
    if root.join("setup.py").is_file() || root.join("requirements.txt").is_file() {
        let name = root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let framework = scan_requirements_for_framework(&root.join("requirements.txt"));
        return Ok(Some(ProjectType::Python { name, framework }));
    }
    Ok(None)
}

fn parse_go_mod(root: &Path) -> io::Result<Option<ProjectType>> {
    let path = root.join("go.mod");
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)?;
    let name = raw
        .lines()
        .find_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("module ")?;
            Some(rest.split_whitespace().next().unwrap_or(rest).to_string())
        })
        .unwrap_or_else(|| "unknown".into());
    Ok(Some(ProjectType::Go { name }))
}

/// Inspects `root` and returns a [`ProjectSummary`] (never fails on unknown layouts — uses [`ProjectType::Generic`]).
pub fn detect_project(root: &Path) -> io::Result<ProjectSummary> {
    let root = root.to_path_buf();
    let has_git = root.join(".git").exists();
    let has_akmon_md = root.join("AKMON.md").is_file();
    let description = read_readme_excerpt(&root);

    let project_type = if let Some(t) = parse_cargo(&root)? {
        t
    } else if let Some(t) = parse_package_json(&root)? {
        t
    } else if let Some(t) = parse_python_project(&root)? {
        t
    } else if let Some(t) = parse_go_mod(&root)? {
        t
    } else {
        let files = list_root_files(&root)?;
        ProjectType::Generic { files }
    };

    let mut key_files = Vec::new();
    match &project_type {
        ProjectType::Rust { .. } => {
            push_if_file(&root, "Cargo.toml", &mut key_files);
            push_if_file(&root, "README.md", &mut key_files);
            push_if_file(&root, "src/main.rs", &mut key_files);
            push_if_file(&root, "src/lib.rs", &mut key_files);
        }
        ProjectType::Node { .. } => {
            push_if_file(&root, "package.json", &mut key_files);
            push_if_file(&root, "README.md", &mut key_files);
            push_if_file(&root, "src/index.js", &mut key_files);
        }
        ProjectType::Python { .. } => {
            push_if_file(&root, "pyproject.toml", &mut key_files);
            push_if_file(&root, "setup.py", &mut key_files);
            push_if_file(&root, "requirements.txt", &mut key_files);
            push_if_file(&root, "README.md", &mut key_files);
            push_if_file(&root, "src/main.py", &mut key_files);
        }
        ProjectType::Go { .. } => {
            push_if_file(&root, "go.mod", &mut key_files);
            push_if_file(&root, "README.md", &mut key_files);
        }
        ProjectType::Generic { files } => {
            for f in files.iter().take(12) {
                key_files.push(f.clone());
            }
        }
    }

    Ok(ProjectSummary {
        project_type,
        root,
        key_files,
        description,
        has_git,
        has_akmon_md,
    })
}

fn push_if_file(root: &Path, rel: &str, out: &mut Vec<String>) {
    if root.join(rel).is_file() {
        out.push(rel.to_string());
    }
}

/// Default markdown H1 title for a generated `AKMON.md` (package/module name or directory).
pub fn suggested_akmon_title(summary: &ProjectSummary) -> String {
    match &summary.project_type {
        ProjectType::Rust { name, .. }
        | ProjectType::Node { name, .. }
        | ProjectType::Python { name, .. }
        | ProjectType::Go { name, .. } => name.clone(),
        ProjectType::Generic { .. } => summary
            .root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Project")
            .to_string(),
    }
}

/// Human-oriented type label for prompts and CLI output.
pub fn project_type_label(summary: &ProjectSummary) -> String {
    match &summary.project_type {
        ProjectType::Rust {
            name,
            lib,
            bin_count,
            ..
        } => {
            if *lib && *bin_count == 0 {
                format!("Rust library ({name})")
            } else if *bin_count > 0 || summary.root.join("src/main.rs").is_file() {
                format!("Rust application ({name})")
            } else {
                format!("Rust project ({name})")
            }
        }
        ProjectType::Node { name, framework } => {
            if let Some(f) = framework {
                format!("Node.js project ({name}, {f})")
            } else {
                format!("Node.js project ({name})")
            }
        }
        ProjectType::Python { name, framework } => {
            if let Some(f) = framework {
                format!("Python project ({name}, {f})")
            } else {
                format!("Python project ({name})")
            }
        }
        ProjectType::Go { name } => format!("Go module ({name})"),
        ProjectType::Generic { .. } => "Generic / unknown".to_string(),
    }
}

/// Counts common source files for status output (best-effort, shallow).
pub fn count_source_files_for_summary(summary: &ProjectSummary) -> usize {
    let root = &summary.root;
    match &summary.project_type {
        ProjectType::Rust { .. } => count_by_ext(root.join("src"), "rs", 10_000),
        ProjectType::Node { .. } => {
            let n = count_exts_under_dir(&root.join("src"), &["js", "ts", "mjs", "cjs"], 10_000);
            if n > 0 {
                n
            } else {
                count_exts_under_dir(root, &["js", "ts", "mjs", "cjs"], 500)
            }
        }
        ProjectType::Python { .. } => count_by_ext(root.join("src"), "py", 10_000),
        ProjectType::Go { .. } => count_by_ext(root.to_path_buf(), "go", 10_000),
        ProjectType::Generic { .. } => 0,
    }
}

fn count_exts_under_dir(dir: &Path, exts: &[&str], cap: usize) -> usize {
    if !dir.is_dir() {
        return 0;
    }
    let mut n = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(read) = fs::read_dir(&d) else {
            continue;
        };
        for ent in read.flatten() {
            if n >= cap {
                return n;
            }
            let path = ent.path();
            let name = ent.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                if name == "target" || name == "node_modules" || name == "vendor" {
                    continue;
                }
                stack.push(path);
            } else if path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|e| exts.iter().any(|x| x == &e))
            {
                n += 1;
            }
        }
    }
    n
}

fn count_by_ext(dir: PathBuf, ext: &str, cap: usize) -> usize {
    if !dir.is_dir() {
        return 0;
    }
    let mut n = 0usize;
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        let Ok(read) = fs::read_dir(&d) else {
            continue;
        };
        for ent in read.flatten() {
            if n >= cap {
                return n;
            }
            let path = ent.path();
            let name = ent.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                if name == "target" || name == "node_modules" || name == "vendor" {
                    continue;
                }
                stack.push(path);
            } else if path.extension().and_then(|s| s.to_str()) == Some(ext) {
                n += 1;
            }
        }
    }
    n
}

/// Builds a structured plain-text brief for the model that will author `AKMON.md`.
pub fn format_project_context_for_init(summary: &ProjectSummary) -> String {
    let mut s = String::new();
    s.push_str("Project root: ");
    s.push_str(&summary.root.to_string_lossy());
    s.push('\n');
    s.push_str("Type: ");
    s.push_str(&project_type_label(summary));
    s.push('\n');

    match &summary.project_type {
        ProjectType::Rust {
            name,
            edition,
            bin_count,
            lib,
        } => {
            s.push_str("Package name: ");
            s.push_str(name);
            s.push('\n');
            s.push_str("Edition: ");
            s.push_str(edition);
            s.push('\n');
            s.push_str("Explicit [[bin]] targets: ");
            s.push_str(&bin_count.to_string());
            s.push('\n');
            s.push_str("Library crate: ");
            s.push_str(if *lib { "yes" } else { "no" });
            s.push('\n');
            let mut eps = Vec::new();
            if summary.root.join("src/main.rs").is_file() {
                eps.push("src/main.rs");
            }
            if summary.root.join("src/lib.rs").is_file() {
                eps.push("src/lib.rs");
            }
            if !eps.is_empty() {
                s.push_str("Entry points: ");
                s.push_str(&eps.join(", "));
                s.push('\n');
            }
        }
        ProjectType::Node { name, framework } => {
            s.push_str("Package name: ");
            s.push_str(name);
            s.push('\n');
            if let Some(f) = framework {
                s.push_str("Detected framework dependency: ");
                s.push_str(f);
                s.push('\n');
            }
        }
        ProjectType::Python { name, framework } => {
            s.push_str("Project name: ");
            s.push_str(name);
            s.push('\n');
            if let Some(f) = framework {
                s.push_str("Detected framework: ");
                s.push_str(f);
                s.push('\n');
            }
        }
        ProjectType::Go { name } => {
            s.push_str("Module: ");
            s.push_str(name);
            s.push('\n');
        }
        ProjectType::Generic { files } => {
            s.push_str("Root files: ");
            s.push_str(&files.join(", "));
            s.push('\n');
        }
    }

    if !summary.key_files.is_empty() {
        s.push_str("Key files: ");
        s.push_str(&summary.key_files.join(", "));
        s.push('\n');
    }

    s.push_str("Has git: ");
    s.push_str(if summary.has_git { "true" } else { "false" });
    s.push('\n');
    s.push_str("Has AKMON.md: ");
    s.push_str(if summary.has_akmon_md {
        "true"
    } else {
        "false"
    });
    s.push('\n');

    if let Some(ex) = &summary.description
        && !ex.is_empty()
    {
        s.push_str("README excerpt: ");
        s.push_str(ex);
        s.push('\n');
    }

    s.push('\n');
    s.push_str(&crate::lang_profile::format_project_intelligence_for_root(
        &summary.root,
    ));

    s
}

/// Slugifies a natural-language task for `.akmon/plans/{timestamp}-{slug}.md` filenames.
pub fn task_slug_for_plan_filename(task: &str) -> String {
    let folded: String = task
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = folded.trim_matches('-');
    let parts: Vec<&str> = trimmed
        .split('-')
        .filter(|s| !s.is_empty())
        .take(12)
        .collect();
    let joined = parts.join("-");
    let base = if joined.is_empty() {
        "task".to_string()
    } else {
        joined
    };
    base.chars().take(80).collect()
}

/// Writes markdown `body` to `<project_root>/.akmon/plans/{unix_timestamp}-{slug}.md`.
pub fn save_plan_markdown(project_root: &Path, task: &str, body: &str) -> io::Result<PathBuf> {
    let plans_dir = project_root.join(".akmon").join("plans");
    fs::create_dir_all(&plans_dir)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let slug = task_slug_for_plan_filename(task);
    let path = plans_dir.join(format!("{ts}-{slug}.md"));
    fs::write(&path, body)?;
    Ok(path)
}

fn write_file(root: &Path, rel: &str, body: &str, created: &mut Vec<String>) -> io::Result<()> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, body)?;
    created.push(rel.replace('\\', "/"));
    Ok(())
}

/// Creates starter files for `akmon new` under `root` (must already be the new project directory).
pub fn scaffold_project(
    root: &Path,
    project_name: &str,
    lang: ScaffoldLanguage,
    kind: ScaffoldKind,
) -> io::Result<ScaffoldReport> {
    let mut files_created = Vec::new();

    let rust_gitignore = "/target\nCargo.lock\n.DS_Store\n";
    let node_gitignore = "node_modules/\n.DS_Store\n";
    let py_gitignore = "__pycache__/\n.venv/\n*.pyc\n.DS_Store\n";
    let go_gitignore = "bin/\n.DS_Store\n";

    match (lang, kind) {
        (ScaffoldLanguage::Rust, ScaffoldKind::Cli) => {
            let cargo = format!(
                r#"[package]
name = "{project_name}"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "{project_name}"
path = "src/main.rs"

[dependencies]
clap = {{ version = "4", features = ["derive"] }}
"#
            );
            let main_rs = r#"fn main() {
    // Generated by Akmon
    println!("Hello from Akmon scaffold");
}
"#;
            write_file(root, "Cargo.toml", &cargo, &mut files_created)?;
            write_file(root, "src/main.rs", main_rs, &mut files_created)?;
            write_file(root, ".gitignore", rust_gitignore, &mut files_created)?;
            write_file(
                root,
                "README.md",
                &format!("# {project_name}\n\nRust CLI scaffold (Akmon).\n"),
                &mut files_created,
            )?;
        }
        (ScaffoldLanguage::Rust, ScaffoldKind::Lib) => {
            let cargo = format!(
                r#"[package]
name = "{project_name}"
version = "0.1.0"
edition = "2021"

[lib]
name = "{project_name}"
path = "src/lib.rs"
"#
            );
            let lib_rs = r#"//! Generated by Akmon

/// Returns a greeting string.
pub fn hello() -> &'static str {
    "hello"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        assert_eq!(hello(), "hello");
    }
}
"#;
            write_file(root, "Cargo.toml", &cargo, &mut files_created)?;
            write_file(root, "src/lib.rs", lib_rs, &mut files_created)?;
            write_file(root, ".gitignore", rust_gitignore, &mut files_created)?;
            write_file(
                root,
                "README.md",
                &format!("# {project_name}\n\nRust library scaffold (Akmon).\n"),
                &mut files_created,
            )?;
        }
        (ScaffoldLanguage::Node, ScaffoldKind::Web) => {
            let pkg = format!(
                r#"{{
  "name": "{project_name}",
  "version": "0.1.0",
  "private": true,
  "scripts": {{
    "start": "node src/index.js"
  }}
}}
"#
            );
            let idx = r#"// Generated by Akmon
const http = require("http");
const port = process.env.PORT || 3000;
const server = http.createServer((_req, res) => {
  res.writeHead(200, { "Content-Type": "text/plain" });
  res.end("ok\n");
});
server.listen(port, () => {
  console.log(`listening on ${port}`);
});
"#;
            write_file(root, "package.json", &pkg, &mut files_created)?;
            write_file(root, "src/index.js", idx, &mut files_created)?;
            write_file(root, ".gitignore", node_gitignore, &mut files_created)?;
            write_file(
                root,
                "README.md",
                &format!("# {project_name}\n\nNode web scaffold (Akmon). Run `npm start`.\n"),
                &mut files_created,
            )?;
        }
        (ScaffoldLanguage::Python, ScaffoldKind::Api) => {
            let pyproject = format!(
                r#"[project]
name = "{project_name}"
version = "0.1.0"
requires-python = ">=3.10"
dependencies = [
  "fastapi>=0.100",
  "uvicorn[standard]>=0.22",
]

[build-system]
requires = ["setuptools>=61"]
build-backend = "setuptools.build_meta"
"#
            );
            let main_py = r#"""Generated by Akmon — minimal FastAPI app."""
from fastapi import FastAPI

app = FastAPI()


@app.get("/")
def read_root():
    return {"status": "ok"}
"#;
            let req = "fastapi>=0.100\nuvicorn[standard]>=0.22\n";
            write_file(root, "pyproject.toml", &pyproject, &mut files_created)?;
            write_file(root, "src/__init__.py", "", &mut files_created)?;
            write_file(root, "src/main.py", main_py, &mut files_created)?;
            write_file(root, "requirements.txt", req, &mut files_created)?;
            write_file(root, ".gitignore", py_gitignore, &mut files_created)?;
            write_file(
                root,
                "README.md",
                &format!(
                    "# {project_name}\n\nPython API scaffold (Akmon). Try `uvicorn src.main:app`.\n"
                ),
                &mut files_created,
            )?;
        }
        (ScaffoldLanguage::Go, ScaffoldKind::Cli) => {
            let module_path = project_name.replace('-', "_");
            let gomod = format!("module {module_path}\n\ngo 1.22\n");
            let main_go = r#"package main

import "fmt"

func main() {
	// Generated by Akmon
	fmt.Println("Hello from Akmon scaffold")
}
"#;
            write_file(root, "go.mod", &gomod, &mut files_created)?;
            write_file(root, "main.go", main_go, &mut files_created)?;
            write_file(root, ".gitignore", go_gitignore, &mut files_created)?;
            write_file(
                root,
                "README.md",
                &format!("# {project_name}\n\nGo CLI scaffold (Akmon).\n"),
                &mut files_created,
            )?;
        }
        _ => {
            write_file(root, ".gitignore", "*.log\n.DS_Store\n", &mut files_created)?;
            write_file(
                root,
                "README.md",
                &format!("# {project_name}\n\nMinimal scaffold (Akmon).\n"),
                &mut files_created,
            )?;
        }
    }

    Ok(ScaffoldReport { files_created })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn detect_project_rust_from_cargo_toml() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let mut f = fs::File::create(root.join("Cargo.toml")).expect("cargo");
        writeln!(
            f,
            r#"[package]
name = "my-tool"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "my-tool"
path = "src/main.rs"
"#
        )
        .expect("write");
        fs::create_dir_all(root.join("src")).expect("src");
        fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("main");
        let s = detect_project(root).expect("detect");
        match s.project_type {
            ProjectType::Rust {
                ref name,
                ref edition,
                bin_count,
                lib,
            } => {
                assert_eq!(name, "my-tool");
                assert_eq!(edition, "2024");
                assert_eq!(bin_count, 1);
                assert!(!lib);
            }
            ref other => panic!("expected Rust, got {other:?}"),
        }
    }

    #[test]
    fn detect_project_node_from_package_json() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        fs::write(
            root.join("package.json"),
            r#"{"name":"webapp","dependencies":{"react":"^18"}}"#,
        )
        .expect("write");
        let s = detect_project(root).expect("detect");
        match s.project_type {
            ProjectType::Node {
                ref name,
                ref framework,
            } => {
                assert_eq!(name, "webapp");
                assert_eq!(framework.as_deref(), Some("react"));
            }
            ref other => panic!("expected Node, got {other:?}"),
        }
    }

    #[test]
    fn detect_project_generic_empty_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = detect_project(tmp.path()).expect("detect");
        match s.project_type {
            ProjectType::Generic { files } => assert!(files.is_empty()),
            ref other => panic!("expected Generic, got {other:?}"),
        }
    }

    #[test]
    fn detect_project_has_git_true() {
        let tmp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(tmp.path().join(".git")).expect("git");
        let s = detect_project(tmp.path()).expect("detect");
        assert!(s.has_git);
    }

    #[test]
    fn detect_project_has_git_false() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let s = detect_project(tmp.path()).expect("detect");
        assert!(!s.has_git);
    }

    #[test]
    fn format_project_context_non_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        fs::write(tmp.path().join("package.json"), r#"{"name":"x"}"#).expect("pj");
        let s = detect_project(tmp.path()).expect("detect");
        let ctx = format_project_context_for_init(&s);
        assert!(ctx.len() > 20);
        assert!(ctx.contains("Project root:"));
    }

    #[test]
    fn scaffold_rust_cli_expected_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let r = scaffold_project(tmp.path(), "foo", ScaffoldLanguage::Rust, ScaffoldKind::Cli)
            .expect("scaffold");
        let mut names: Vec<_> = r.files_created.iter().map(String::as_str).collect();
        names.sort();
        assert!(names.contains(&"Cargo.toml"));
        assert!(names.contains(&"src/main.rs"));
        assert!(names.contains(&".gitignore"));
        assert!(names.contains(&"README.md"));
    }

    #[test]
    fn scaffold_rust_lib_expected_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let r = scaffold_project(tmp.path(), "bar", ScaffoldLanguage::Rust, ScaffoldKind::Lib)
            .expect("scaffold");
        assert!(r.files_created.iter().any(|p| p == "src/lib.rs"));
        assert!(!r.files_created.iter().any(|p| p == "src/main.rs"));
    }
}
