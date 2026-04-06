//! Natural-language codebase search backed by a pre-built [`akmon_index::RepoIndex`].

use std::sync::{Arc, Mutex};

use akmon_index::{semantic_search, RepoIndex};
use async_trait::async_trait;
use fastembed::TextEmbedding;
use serde_json::{json, Value as JsonValue};
use tokio::sync::RwLock;

use akmon_core::Permission;
use std::path::PathBuf;

use crate::context::ToolContext;
use crate::output::{ToolErrorCode, ToolOutput};
use crate::Tool;

fn read_permissions() -> &'static [Permission] {
    use std::sync::OnceLock;
    static CELL: OnceLock<[Permission; 1]> = OnceLock::new();
    CELL.get_or_init(|| {
        [Permission::ReadFile {
            path: PathBuf::from("."),
        }]
    })
    .as_slice()
}

/// `semantic_search` tool: queries an in-memory [`RepoIndex`] with the same embedding model used at index time.
pub struct SemanticSearchTool {
    index: Arc<RwLock<Option<RepoIndex>>>,
    embedder: Option<Arc<Mutex<TextEmbedding>>>,
}

impl SemanticSearchTool {
    /// Wraps shared index storage and an optional embedder (missing when model init failed).
    pub fn new(
        index: Arc<RwLock<Option<RepoIndex>>>,
        embedder: Option<Arc<Mutex<TextEmbedding>>>,
    ) -> Self {
        Self { index, embedder }
    }

    fn preview(s: &str, max: usize) -> String {
        if s.len() <= max {
            s.to_string()
        } else {
            format!("{}…", &s[..max])
        }
    }
}

#[async_trait]
impl Tool for SemanticSearchTool {
    fn name(&self) -> &str {
        "semantic_search"
    }

    fn description(&self) -> &str {
        "Search the codebase using natural language. Use for conceptual searches like 'error handling' or 'policy evaluation'. For exact string matches use the search tool."
    }

    fn required_permissions(&self) -> &[Permission] {
        read_permissions()
    }

    fn parameters_schema(&self) -> JsonValue {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language question or phrase describing what to find"
                },
                "top_k": {
                    "type": "integer",
                    "description": "Maximum hits to return (default 5, max 20)",
                    "minimum": 1,
                    "maximum": 20
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: JsonValue, ctx: &ToolContext) -> ToolOutput {
        let _ = ctx;
        let query = match args.get("query").and_then(|q| q.as_str()) {
            Some(q) if !q.trim().is_empty() => q.trim().to_string(),
            _ => {
                return ToolOutput::Error {
                    code: ToolErrorCode::InvalidArgs,
                    message: "missing or empty query".into(),
                };
            }
        };

        let top_k = args
            .get("top_k")
            .and_then(|v| v.as_u64())
            .map(|u| u as usize)
            .unwrap_or(5)
            .clamp(1, 20);

        let Some(emb) = self.embedder.as_ref() else {
            return ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: "semantic index unavailable: embedding model failed to load".into(),
            };
        };

        let guard = self.index.read().await;
        let Some(index) = guard.as_ref() else {
            return ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: "semantic index not ready — indexing in progress".into(),
            };
        };

        let prefixed = format!("query: {query}");
        let emb = Arc::clone(emb);
        let vectors = match tokio::task::spawn_blocking(move || {
            let mut guard = emb
                .lock()
                .map_err(|e| format!("embedder lock: {e}"))?;
            guard
                .embed(vec![prefixed], None)
                .map_err(|e| e.to_string())
        })
        .await
        {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::PermissionDenied,
                    message: format!("embedding failed: {e}"),
                };
            }
            Err(e) => {
                return ToolOutput::Error {
                    code: ToolErrorCode::PermissionDenied,
                    message: format!("embedding task: {e}"),
                };
            }
        };
        let Some(qvec) = vectors.into_iter().next() else {
            return ToolOutput::Error {
                code: ToolErrorCode::PermissionDenied,
                message: "embedding produced no vector".into(),
            };
        };

        let hits = semantic_search(index, &qvec, top_k);
        let results: Vec<JsonValue> = hits
            .iter()
            .map(|h| {
                json!({
                    "file": h.path,
                    "start_line": h.start_line,
                    "end_line": h.end_line,
                    "score": h.score,
                    "preview": Self::preview(&h.content, 2000),
                })
            })
            .collect();

        let payload = json!({
            "query": query,
            "results": results,
        });

        match serde_json::to_string_pretty(&payload) {
            Ok(s) => ToolOutput::Success { content: s },
            Err(e) => ToolOutput::Error {
                code: ToolErrorCode::InvalidArgs,
                message: format!("json: {e}"),
            },
        }
    }
}

#[cfg(all(test, feature = "semantic-index"))]
mod tests {
    use super::*;
    use akmon_core::{PolicyEngine, PolicyEngineMode, Sandbox};
    use chrono::Utc;

    fn test_ctx() -> ToolContext {
        let tmp = tempfile::tempdir().expect("tmp");
        ToolContext::new(
            Sandbox::new(tmp.path()),
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
        )
    }

    #[tokio::test]
    async fn unavailable_when_embedder_missing() {
        let slot = Arc::new(RwLock::new(None));
        let t = SemanticSearchTool::new(slot, None);
        let out = t.execute(json!({"query": "x"}), &test_ctx()).await;
        match out {
            ToolOutput::Error { message, .. } => {
                assert!(message.contains("unavailable"));
            }
            _ => panic!("expected error"),
        }
    }

    #[tokio::test]
    async fn not_ready_when_index_none() {
        let Ok(model) = TextEmbedding::try_new(fastembed::TextInitOptions::default()) else {
            return;
        };
        let slot = Arc::new(RwLock::new(None));
        let t = SemanticSearchTool::new(slot, Some(Arc::new(Mutex::new(model))));
        let out = t.execute(json!({"query": "x"}), &test_ctx()).await;
        match out {
            ToolOutput::Error { message, .. } => {
                assert!(message.contains("indexing in progress"));
            }
            _ => panic!("expected error"),
        }
    }

    #[tokio::test]
    async fn returns_hits_with_synthetic_index() {
        let index = RepoIndex::from_parts(
            vec![akmon_index::FileChunk {
                path: "p.rs".into(),
                start_line: 1,
                end_line: 2,
                content: "policy evaluation".into(),
                embedding: vec![1.0_f32, 0.0_f32],
            }],
            std::path::PathBuf::from("/r"),
            Utc::now(),
            1,
        );
        let slot = Arc::new(RwLock::new(Some(index)));
        let Ok(model) = TextEmbedding::try_new(fastembed::TextInitOptions::default()) else {
            return;
        };
        let t = SemanticSearchTool::new(slot, Some(Arc::new(Mutex::new(model))));
        let out = t
            .execute(json!({"query": "policy", "top_k": 1}), &test_ctx())
            .await;
        match out {
            ToolOutput::Success { content } => {
                assert!(content.contains("p.rs"));
                assert!(content.contains("results"));
            }
            ToolOutput::Error { message, .. } => {
                panic!("unexpected error: {message}");
            }
        }
    }
}
