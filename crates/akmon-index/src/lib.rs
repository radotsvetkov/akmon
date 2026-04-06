//! Semantic indexing for a project tree: chunk text files, embed with [fastembed], and search by vector similarity.
//!
//! The walker respects `.gitignore`, `.git/info/exclude`, global gitignore, and per-directory `.akmonignore`
//! (same syntax as `.gitignore`), and skips common dependency and cache directory names.
//!
//! Persisted indices use [`bincode`] under `.akmon/index.bin` (see the `akmon-cli` `--index` flag).

#![warn(missing_docs)]

mod error;
mod persist;
mod search;
#[cfg(feature = "semantic-index")]
mod indexer;

pub use error::IndexError;
pub use persist::{load_index, save_index};
pub use search::{cosine_similarity, semantic_search};
#[cfg(feature = "semantic-index")]
pub use indexer::Indexer;

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One line-bounded slice of a source file and its dense embedding vector.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileChunk {
    /// Sandbox-relative path using `/` separators.
    pub path: String,
    /// First line number in the file (1-based, inclusive).
    pub start_line: usize,
    /// Last line number in the file (1-based, inclusive).
    pub end_line: usize,
    /// Raw chunk text (UTF-8) that was embedded.
    pub content: String,
    /// L2-normalized embedding (model-dependent dimensionality).
    pub embedding: Vec<f32>,
}

/// One hit from [`semantic_search`], sorted by descending [`IndexEntry::score`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IndexEntry {
    /// File path (same convention as [`FileChunk::path`]).
    pub path: String,
    /// First line in file (1-based, inclusive).
    pub start_line: usize,
    /// Last line in file (1-based, inclusive).
    pub end_line: usize,
    /// Chunk text that was embedded for this row.
    pub content: String,
    /// Cosine similarity score in \[0, 1\] for normalized embeddings.
    pub score: f32,
}

/// Serialized semantic index for a single project checkout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepoIndex {
    chunks: Vec<FileChunk>,
    /// Canonical project root used when the index was built.
    pub project_root: PathBuf,
    /// UTC timestamp when indexing finished.
    pub indexed_at: DateTime<Utc>,
    /// Number of source files that contributed at least one chunk.
    pub file_count: usize,
    /// Total stored chunks (equals [`FileChunk`] count).
    pub chunk_count: usize,
}

impl RepoIndex {
    /// Builds an index from precomputed chunks (tests, fixtures, or custom pipelines).
    pub fn from_parts(
        chunks: Vec<FileChunk>,
        project_root: PathBuf,
        indexed_at: DateTime<Utc>,
        file_count: usize,
    ) -> Self {
        let chunk_count = chunks.len();
        Self {
            chunks,
            project_root,
            indexed_at,
            file_count,
            chunk_count,
        }
    }

    /// Borrow all [`FileChunk`] rows (immutable).
    pub fn chunks(&self) -> &[FileChunk] {
        &self.chunks
    }

    /// `true` when there are no chunks.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }
}
