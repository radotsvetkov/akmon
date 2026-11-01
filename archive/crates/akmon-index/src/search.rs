//! Vector similarity helpers for dense embeddings.

use crate::{IndexEntry, RepoIndex};

/// Cosine similarity of two same-length vectors. Returns `0.0` if either slice is empty or lengths differ.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let da = na.sqrt();
    let db = nb.sqrt();
    if da == 0.0 || db == 0.0 {
        return 0.0;
    }
    dot / (da * db)
}

/// Ranks [`RepoIndex`] chunks by cosine similarity to `query_embedding` and returns the top `top_k` hits.
pub fn semantic_search(
    index: &RepoIndex,
    query_embedding: &[f32],
    top_k: usize,
) -> Vec<IndexEntry> {
    if top_k == 0 || query_embedding.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<IndexEntry> = index
        .chunks()
        .iter()
        .filter_map(|c| {
            if c.embedding.is_empty() {
                return None;
            }
            let score = cosine_similarity(query_embedding, &c.embedding);
            Some(IndexEntry {
                path: c.path.clone(),
                start_line: c.start_line,
                end_line: c.end_line,
                content: c.content.clone(),
                score,
            })
        })
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(top_k.min(scored.len()));
    scored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileChunk;
    use chrono::Utc;
    use std::path::PathBuf;

    #[test]
    fn cosine_similarity_identical_is_one() {
        let v = vec![0.6_f32, 0.8_f32];
        let s = cosine_similarity(&v, &v);
        assert!((s - 1.0).abs() < 1e-5, "got {s}");
    }

    #[test]
    fn cosine_similarity_orthogonal_is_zero() {
        let a = [1.0_f32, 0.0_f32];
        let b = [0.0_f32, 1.0_f32];
        let s = cosine_similarity(&a, &b);
        assert!((s - 0.0).abs() < 1e-5, "got {s}");
    }

    #[test]
    fn cosine_similarity_empty_or_mismatch_returns_zero() {
        assert_eq!(cosine_similarity(&[], &[1.0]), 0.0);
        assert_eq!(cosine_similarity(&[1.0], &[]), 0.0);
        assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0]), 0.0);
    }

    #[test]
    fn semantic_search_top_k_orders_by_score() {
        let index = RepoIndex::from_parts(
            vec![
                FileChunk {
                    path: "a.rs".into(),
                    start_line: 1,
                    end_line: 1,
                    content: "a".into(),
                    embedding: vec![1.0_f32, 0.0_f32],
                },
                FileChunk {
                    path: "b.rs".into(),
                    start_line: 1,
                    end_line: 1,
                    content: "b".into(),
                    embedding: vec![0.0_f32, 1.0_f32],
                },
                FileChunk {
                    path: "c.rs".into(),
                    start_line: 1,
                    end_line: 1,
                    content: "c".into(),
                    embedding: vec![0.70710677_f32, 0.70710677_f32],
                },
            ],
            PathBuf::from("/tmp"),
            Utc::now(),
            3,
        );
        let q = vec![1.0_f32, 0.0_f32];
        let hits = semantic_search(&index, &q, 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].path, "a.rs");
        assert!(hits[0].score >= hits[1].score);
    }
}
