//! Walk the tree, chunk text, embed, and (de)serialize [`RepoIndex`](crate::RepoIndex).

use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};

use akmon_core::Sandbox;
use chrono::Utc;
use dunce::canonicalize;
use fastembed::TextEmbedding;
use ignore::{DirEntry, Error as IgnoreError, WalkBuilder};

use crate::{FileChunk, RepoIndex};

/// Sandbox-relative path and line-bounded text slice before embedding (1-based line numbers).
type ChunkMetaRow = (String, usize, usize, String);

/// Directory names (final path component) to prune entirely — never descend.
const SKIP_DIR_NAMES: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".akmon",
    ".cargo",
    ".rustup",
    ".fastembed",
    "__pycache__",
    ".venv",
    "venv",
    "dist",
    "build",
    "vendor",
    "third_party",
    ".cache",
];

/// Extensions never indexed (lowercase, no dot), in addition to the allowlist in [`Indexer::extensions`].
const BLOCKED_EXTENSIONS: &[&str] = &[
    "lock", "bin", "so", "dylib", "dll", "a", "rlib", "wasm", "pb",
];

use crate::error::IndexError;

fn map_ignore_err(e: IgnoreError) -> IndexError {
    match e {
        IgnoreError::Io(ioe) => IndexError::Io(ioe),
        other => IndexError::Io(std::io::Error::other(other.to_string())),
    }
}

fn skip_directory_name(name: &OsStr) -> bool {
    let s = name.to_string_lossy();
    SKIP_DIR_NAMES.iter().any(|&n| n == s.as_ref())
}

/// True if the file should be skipped before the extension allowlist (lockfiles, minified assets, blocked ext).
fn is_blocked_source_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return true;
    };
    let name_lc = name.to_ascii_lowercase();
    if name_lc.ends_with(".min.js") || name_lc.ends_with(".min.css") {
        return true;
    }
    if matches!(
        name_lc.as_str(),
        "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "npm-shrinkwrap.json"
            | "poetry.lock"
            | "pipfile.lock"
    ) {
        return true;
    }
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let ext_lc = ext.to_ascii_lowercase();
    BLOCKED_EXTENSIONS.contains(&ext_lc.as_str())
}

/// Configuration for filesystem walk, chunking, and file caps.
#[derive(Debug, Clone)]
pub struct Indexer {
    /// Lines per chunk (before overlap).
    pub chunk_size: usize,
    /// Lines shared between consecutive chunks.
    pub chunk_overlap: usize,
    /// File extensions to include (lowercase, without dot); see also built-in extension blocklist.
    pub extensions: Vec<String>,
    /// Skip files larger than this many bytes.
    pub max_file_bytes: usize,
    /// Maximum source files to read and embed; walk stops once this many files are indexed.
    pub max_files: usize,
}

impl Default for Indexer {
    fn default() -> Self {
        Self {
            chunk_size: 50,
            chunk_overlap: 10,
            extensions: vec![
                "rs".into(),
                "toml".into(),
                "md".into(),
                "txt".into(),
                "json".into(),
                "yaml".into(),
                "yml".into(),
                "py".into(),
                "js".into(),
                "ts".into(),
            ],
            max_file_bytes: 1024 * 1024,
            max_files: 500,
        }
    }
}

/// How often to print scan progress (`N` indexed source files) to stderr.
const SCAN_PROGRESS_EVERY_FILES: usize = 20;
/// Passages per [`TextEmbedding::embed`] call so work yields visible progress and stays bounded in memory.
const EMBEDDING_BATCH_PASSAGES: usize = 32;

impl Indexer {
    /// Splits `lines` into overlapping windows. Each tuple is `(start_line, end_line, text)` with **1-based** line numbers.
    pub fn chunk_lines(&self, lines: &[String]) -> Vec<(usize, usize, String)> {
        let chunk_size = self.chunk_size.max(1);
        let overlap = self.chunk_overlap.min(chunk_size.saturating_sub(1));
        let step = chunk_size.saturating_sub(overlap).max(1);
        let n = lines.len();
        if n == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < n {
            let end = (i + chunk_size).min(n);
            let chunk_text = lines[i..end].join("\n");
            if chunk_text.len() >= 10 {
                let start_line = i + 1;
                let end_line = end;
                out.push((start_line, end_line, chunk_text));
            }
            if end >= n {
                break;
            }
            i += step;
        }
        out
    }

    /// Build a [`RepoIndex`] under `project_root`, respecting [`Sandbox`] boundaries.
    ///
    /// File walk and embedding run on the blocking pool. The embedder is behind [`Mutex`] because
    /// [`TextEmbedding::embed`](fastembed::TextEmbedding::embed) requires `&mut self`.
    pub async fn build_index(
        &self,
        project_root: &Path,
        embedder: Arc<Mutex<TextEmbedding>>,
        sandbox: &Sandbox,
    ) -> Result<RepoIndex, IndexError> {
        let root_canon = canonicalize(project_root)?;
        let indexer = self.clone();
        let sandbox = sandbox.clone();
        let emb = Arc::clone(&embedder);

        tokio::task::spawn_blocking(move || {
            eprintln!("akmon: scanning the repository for indexable source files…");
            let (mut chunks_meta, file_count, hit_cap) =
                indexer.collect_file_chunks(&root_canon, &sandbox)?;

            if hit_cap {
                eprintln!(
                    "akmon: index cap reached ({} files) — use .akmonignore to exclude paths",
                    indexer.max_files
                );
            }

            if chunks_meta.is_empty() {
                eprintln!("akmon: index scan finished — no chunks to embed");
                return Ok(RepoIndex {
                    chunks: Vec::new(),
                    project_root: root_canon,
                    indexed_at: Utc::now(),
                    file_count,
                    chunk_count: 0,
                });
            }

            let n_passages = chunks_meta.len();
            eprintln!(
                "akmon: scan finished — {file_count} files, {n_passages} chunks; embedding in batches of {EMBEDDING_BATCH_PASSAGES}…"
            );

            let passages: Vec<String> = chunks_meta
                .iter()
                .map(|(_, _, _, t)| format!("passage: {t}"))
                .collect();

            let mut embeddings: Vec<Vec<f32>> = Vec::with_capacity(n_passages);
            for (i, batch) in passages.chunks(EMBEDDING_BATCH_PASSAGES).enumerate() {
                let done = i * EMBEDDING_BATCH_PASSAGES;
                let end = done + batch.len();
                eprintln!("akmon: embedding chunks {}–{} of {n_passages}…", done + 1, end);
                let mut guard = emb
                    .lock()
                    .map_err(|e| IndexError::Embedding(format!("embedder lock: {e}")))?;
                let batch_out = guard
                    .embed(batch, Some(EMBEDDING_BATCH_PASSAGES))
                    .map_err(|e| IndexError::Embedding(e.to_string()))?;
                if batch_out.len() != batch.len() {
                    return Err(IndexError::Embedding(format!(
                        "embedding batch size {} != expected {}",
                        batch_out.len(),
                        batch.len()
                    )));
                }
                embeddings.extend(batch_out);
            }

            if embeddings.len() != chunks_meta.len() {
                return Err(IndexError::Embedding(format!(
                    "embedding count {} != chunk count {}",
                    embeddings.len(),
                    chunks_meta.len()
                )));
            }

            let mut chunks = Vec::with_capacity(chunks_meta.len());
            for ((path, start, end, content), embedding) in
                chunks_meta.drain(..).zip(embeddings.into_iter())
            {
                chunks.push(FileChunk {
                    path,
                    start_line: start,
                    end_line: end,
                    content,
                    embedding,
                });
            }

            let chunk_count = chunks.len();
            Ok(RepoIndex {
                chunks,
                project_root: root_canon,
                indexed_at: Utc::now(),
                file_count,
                chunk_count,
            })
        })
        .await
        .map_err(|e| IndexError::Embedding(e.to_string()))?
    }

    /// Walk and chunk files (crate-visible for unit tests).
    pub(crate) fn collect_file_chunks(
        &self,
        root_canon: &Path,
        sandbox: &Sandbox,
    ) -> Result<(Vec<ChunkMetaRow>, usize, bool), IndexError> {
        let mut metas: Vec<ChunkMetaRow> = Vec::new();
        let mut file_count = 0usize;
        let mut hit_cap = false;

        let mut builder = WalkBuilder::new(root_canon);
        builder.follow_links(false);
        builder.hidden(false);
        builder.git_ignore(true);
        builder.git_global(true);
        builder.git_exclude(true);
        // Apply `.gitignore` rules even when the tree has no `.git` directory (e.g. export tarballs).
        builder.require_git(false);
        builder.add_custom_ignore_filename(".akmonignore");
        builder.filter_entry(|entry: &DirEntry| {
            if entry.depth() == 0 {
                return true;
            }
            !skip_directory_name(entry.file_name())
        });

        let walker = builder.build();

        for result in walker {
            let entry = result.map_err(map_ignore_err)?;
            if let Some(err) = entry.error() {
                return Err(map_ignore_err(err.clone()));
            }
            let Some(ft) = entry.file_type() else {
                continue;
            };
            if !ft.is_file() {
                continue;
            }
            let path = entry.path();
            let meta = match fs::metadata(path) {
                Ok(m) => m,
                Err(e) => return Err(IndexError::Io(e)),
            };
            if meta.len() > self.max_file_bytes as u64 {
                continue;
            }

            if is_blocked_source_file(path) {
                continue;
            }

            let ext_ok = path
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| {
                    let xl = x.to_ascii_lowercase();
                    self.extensions.iter().any(|e| e == &xl)
                })
                .unwrap_or(false);
            if !ext_ok {
                continue;
            }

            let rel = match relative_posix_path(path, root_canon) {
                Some(r) => r,
                None => continue,
            };

            if sandbox.resolve(&rel).is_err() {
                continue;
            }

            if file_count >= self.max_files {
                hit_cap = true;
                break;
            }

            let sniff = read_file_prefix(path, 8192.min(meta.len() as usize))?;
            if sniff.contains(&0u8) {
                continue;
            }

            let bytes = read_file_capped(path, self.max_file_bytes)?;
            let text = match String::from_utf8(bytes) {
                Ok(t) => t,
                Err(_) => continue,
            };

            file_count += 1;
            let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
            for (start_line, end_line, chunk_text) in self.chunk_lines(&lines) {
                metas.push((rel.clone(), start_line, end_line, chunk_text));
            }

            if file_count > 0 && file_count % SCAN_PROGRESS_EVERY_FILES == 0 {
                eprintln!(
                    "akmon: indexing… {file_count} files read, {} chunks collected so far",
                    metas.len()
                );
            }
        }

        Ok((metas, file_count, hit_cap))
    }
}

fn read_file_prefix(path: &Path, max: usize) -> Result<Vec<u8>, IndexError> {
    let mut f = File::open(path)?;
    let mut buf = vec![0u8; max];
    let n = f.read(&mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

fn read_file_capped(path: &Path, max: usize) -> Result<Vec<u8>, IndexError> {
    let f = File::open(path)?;
    let mut buf = Vec::new();
    f.take(max as u64)
        .read_to_end(&mut buf)
        .map_err(IndexError::Io)?;
    Ok(buf)
}

fn relative_posix_path(path: &Path, root: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
}

#[cfg(all(test, feature = "semantic-index"))]
mod tests {
    use super::*;
    use akmon_core::Sandbox;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn chunk_splits_correctly() {
        let idx = Indexer::default();
        let lines: Vec<String> = (1..=100)
            .map(|i| format!("line{i}"))
            .collect();
        let chunks = idx.chunk_lines(&lines);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].0, 1);
        assert_eq!(chunks[0].1, 50);
        assert_eq!(chunks[1].0, 41);
        assert_eq!(chunks[1].1, 90);
        assert_eq!(chunks[2].0, 81);
        assert_eq!(chunks[2].1, 100);
    }

    #[test]
    fn blocked_min_js_and_lock_ext() {
        assert!(is_blocked_source_file(Path::new("foo.min.js")));
        assert!(is_blocked_source_file(Path::new("X.MIN.CSS")));
        assert!(is_blocked_source_file(Path::new("libfoo.so")));
        assert!(is_blocked_source_file(Path::new("Cargo.lock")));
        assert!(is_blocked_source_file(Path::new("package-lock.json")));
        assert!(!is_blocked_source_file(Path::new("src/main.rs")));
    }

    #[test]
    fn max_files_stops_collection() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        for i in 0..5 {
            let p = root.join(format!("f{i}.rs"));
            let mut f = File::create(&p).expect("create");
            writeln!(f, "// line\nlet x = 1;\n").expect("write");
        }
        let sandbox = Sandbox::new(root.to_path_buf());
        let mut idx = Indexer::default();
        idx.max_files = 2;
        let (metas, file_count, hit_cap) = idx
            .collect_file_chunks(root, &sandbox)
            .expect("collect");
        assert!(hit_cap);
        assert_eq!(file_count, 2);
        assert!(!metas.is_empty());
    }

    #[test]
    fn skips_vendor_directory_by_name() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("vendor/stuff")).expect("mkdir");
        let hidden = root.join("vendor/stuff/hidden.rs");
        let mut f = File::create(&hidden).expect("create");
        writeln!(f, "// secret\nfn x() {{}}\n").expect("write");
        let visible = root.join("ok.rs");
        let mut f2 = File::create(&visible).expect("create");
        writeln!(f2, "// ok\nfn y() {{}}\n").expect("write");

        let sandbox = Sandbox::new(root.to_path_buf());
        let idx = Indexer::default();
        let (metas, file_count, _) = idx.collect_file_chunks(root, &sandbox).expect("collect");
        assert_eq!(file_count, 1);
        assert_eq!(metas.len(), 1);
        assert!(metas[0].0.starts_with("ok.rs") || metas[0].0 == "ok.rs");
    }
}
