use std::sync::{Arc, Mutex};

use akmon_core::Permission;
use akmon_journal::{Event, EventKind, Hash, ObjectStore, digest_bytes};
use akmon_tools::{McpPolicyContext, Tool, ToolContext, ToolOutput};
use async_trait::async_trait;

use crate::{
    ReplayDivergence, ReplayDivergenceCollector, ReplayDivergenceKind, ReplayError, ReplayMode,
};

/// Playback-tool construction options.
#[derive(Debug, Clone)]
pub struct PlaybackToolConfig {
    /// Replay behavior mode.
    ///
    /// The mode field MUST match the corresponding mode of any [`crate::PlaybackProvider`] used
    /// in the same replay run. The ReplayEngine constructs both with the same mode value;
    /// constructing them with mismatched modes (for example, for testing) results in undefined
    /// replay semantics.
    pub mode: ReplayMode,
    /// Tool identifier to extract from source `ToolCall` events.
    pub tool_id: String,
    /// Description returned by [`Tool::description`].
    pub description: String,
}

#[derive(Debug, Clone)]
struct ToolState {
    cursor: usize,
}

#[derive(Debug, Clone)]
struct RecordedToolCall {
    event_seq: u64,
    input_hash: Hash,
    output_hash: Hash,
    side_effects_hash: Option<Hash>,
}

/// Replays recorded tool calls through the [`Tool`] trait.
pub struct PlaybackTool<S: ObjectStore> {
    calls: Vec<RecordedToolCall>,
    store: Arc<S>,
    divergences: ReplayDivergenceCollector,
    state: Mutex<ToolState>,
    config: PlaybackToolConfig,
}

impl<S: ObjectStore> PlaybackTool<S> {
    /// Builds tool playback state from source history.
    pub fn from_history(
        events: &[(Hash, Event)],
        store: Arc<S>,
        config: PlaybackToolConfig,
        divergences: ReplayDivergenceCollector,
    ) -> Result<Self, ReplayError> {
        if events.is_empty() {
            return Err(ReplayError::EmptySource);
        }
        let mut calls = Vec::new();
        for (_, event) in events {
            if let EventKind::ToolCall {
                tool_id,
                input_hash,
                output_hash,
                side_effects_hash,
            } = &event.kind
            {
                if tool_id != &config.tool_id {
                    continue;
                }
                ensure_present(store.as_ref(), input_hash)?;
                ensure_present(store.as_ref(), output_hash)?;
                if let Some(h) = side_effects_hash.as_ref() {
                    ensure_present(store.as_ref(), h)?;
                }
                calls.push(RecordedToolCall {
                    event_seq: event.sequence,
                    input_hash: input_hash.clone(),
                    output_hash: output_hash.clone(),
                    side_effects_hash: side_effects_hash.clone(),
                });
            }
        }
        if calls.is_empty() {
            return Err(ReplayError::NoMatchingCalls(config.tool_id.clone()));
        }
        Ok(Self {
            calls,
            store,
            divergences,
            state: Mutex::new(ToolState { cursor: 0 }),
            config,
        })
    }

    /// Current tool-call cursor index.
    pub fn cursor(&self) -> usize {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).cursor
    }

    /// Remaining recorded tool calls.
    pub fn remaining(&self) -> usize {
        let cursor = self.cursor();
        self.calls.len().saturating_sub(cursor)
    }
}

#[async_trait]
impl<S: ObjectStore + 'static> Tool for PlaybackTool<S> {
    fn name(&self) -> &str {
        self.config.tool_id.as_str()
    }

    fn description(&self) -> &str {
        self.config.description.as_str()
    }

    fn required_permissions(&self) -> &[Permission] {
        &[]
    }

    fn mcp_policy_context(&self) -> Option<McpPolicyContext> {
        None
    }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        let call = {
            let mut guard = self.state.lock().unwrap_or_else(|p| p.into_inner());
            let Some(call) = self.calls.get(guard.cursor).cloned() else {
                record_divergence(
                    &self.divergences,
                    ReplayDivergence {
                        event_seq: None,
                        kind: ReplayDivergenceKind::ToolCallUnexpected,
                        expected: "recorded tool call available".to_owned(),
                        actual: "tool called after recorded sequence exhausted".to_owned(),
                    },
                );
                return ToolOutput::Error {
                    code: akmon_tools::ToolErrorCode::InvalidArgs,
                    message: "replay tool cursor exhausted".to_owned(),
                };
            };
            guard.cursor = guard.cursor.saturating_add(1);
            call
        };
        let actual_input_hash = match input_hash(self.store.as_ref(), &args) {
            Ok(hash) => hash,
            Err(message) => {
                return ToolOutput::Error {
                    code: akmon_tools::ToolErrorCode::InvalidArgs,
                    message,
                };
            }
        };
        if actual_input_hash != call.input_hash {
            record_divergence(
                &self.divergences,
                ReplayDivergence {
                    event_seq: Some(call.event_seq),
                    kind: ReplayDivergenceKind::ToolInputMismatch,
                    expected: call.input_hash.to_hex(),
                    actual: actual_input_hash.to_hex(),
                },
            );
            if matches!(self.config.mode, ReplayMode::Strict) {
                return ToolOutput::Error {
                    code: akmon_tools::ToolErrorCode::InvalidArgs,
                    message: "strict replay: tool input mismatch".to_owned(),
                };
            }
        }
        let _ = &call.side_effects_hash;
        read_output(self.store.as_ref(), &call.output_hash).unwrap_or(ToolOutput::Error {
            code: akmon_tools::ToolErrorCode::NotFound,
            message: "recorded tool output missing".to_owned(),
        })
    }
}

fn ensure_present<S: ObjectStore>(store: &S, hash: &Hash) -> Result<(), ReplayError> {
    match store.contains(hash) {
        Ok(true) => Ok(()),
        Ok(false) => Err(ReplayError::MissingSourceObject(hash.clone())),
        Err(err) => Err(ReplayError::StoreReadFailed {
            hash: hash.clone(),
            reason: err.to_string(),
        }),
    }
}

fn input_hash<S: ObjectStore>(store: &S, args: &serde_json::Value) -> Result<Hash, String> {
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(args, &mut bytes)
        .map_err(|err| format!("replay tool input encode failed: {err}"))?;
    Ok(digest_bytes(store.algorithm(), &bytes))
}

fn read_output<S: ObjectStore>(store: &S, hash: &Hash) -> Option<ToolOutput> {
    let bytes = store.get(hash).ok()??;
    ciborium::de::from_reader(bytes.as_ref()).ok()
}

fn record_divergence(collector: &ReplayDivergenceCollector, divergence: ReplayDivergence) {
    let mut guard = collector.lock().unwrap_or_else(|p| p.into_inner());
    guard.push(divergence);
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_core::{PolicyEngine, PolicyEngineMode, Sandbox};
    use akmon_journal::{HashAlgorithm, MemoryObjectStore, ObjectStore};
    use std::sync::{Arc, Mutex};
    use time::OffsetDateTime;

    fn collector() -> ReplayDivergenceCollector {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn tool_config(mode: ReplayMode) -> PlaybackToolConfig {
        PlaybackToolConfig {
            mode,
            tool_id: "mock_tool".to_owned(),
            description: "mock tool replay".to_owned(),
        }
    }

    fn ctx() -> ToolContext {
        ToolContext::new(
            Sandbox::new(std::env::temp_dir()),
            Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll)),
        )
    }

    fn sample_event(input_hash: Hash, output_hash: Hash) -> Event {
        Event {
            parents: vec![],
            kind: EventKind::ToolCall {
                tool_id: "mock_tool".to_owned(),
                input_hash,
                output_hash,
                side_effects_hash: None,
            },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 2,
        }
    }

    fn put_cbor<S: ObjectStore, T: serde::Serialize>(store: &S, value: &T) -> Hash {
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(value, &mut bytes).expect("encode");
        store.put(&bytes).expect("put")
    }

    #[tokio::test]
    async fn playback_tool_returns_recorded_output_for_matching_input() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let args = serde_json::json!({"path":"a.txt"});
        let input_hash = put_cbor(store.as_ref(), &args);
        let expected = ToolOutput::Success {
            content: "ok".to_owned(),
        };
        let output_hash = put_cbor(store.as_ref(), &expected);
        let playback = PlaybackTool::from_history(
            &[(
                Hash::from_bytes(HashAlgorithm::Sha256, [9_u8; 32]),
                sample_event(input_hash, output_hash),
            )],
            store,
            tool_config(ReplayMode::Default),
            collector(),
        )
        .expect("construct");
        let got = playback.execute(args, &ctx()).await;
        assert_eq!(got, expected);
    }

    #[tokio::test]
    async fn playback_tool_reports_input_mismatch_default_mode_and_continues() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let recorded_args = serde_json::json!({"path":"a.txt"});
        let replay_args = serde_json::json!({"path":"b.txt"});
        let input_hash = put_cbor(store.as_ref(), &recorded_args);
        let expected = ToolOutput::Success {
            content: "ok".to_owned(),
        };
        let output_hash = put_cbor(store.as_ref(), &expected);
        let divergences = collector();
        let playback = PlaybackTool::from_history(
            &[(
                Hash::from_bytes(HashAlgorithm::Sha256, [10_u8; 32]),
                sample_event(input_hash, output_hash),
            )],
            store,
            tool_config(ReplayMode::Default),
            Arc::clone(&divergences),
        )
        .expect("construct");
        let got = playback.execute(replay_args, &ctx()).await;
        assert_eq!(got, expected);
        let guard = divergences.lock().unwrap_or_else(|p| p.into_inner());
        assert!(
            guard
                .iter()
                .any(|d| matches!(d.kind, ReplayDivergenceKind::ToolInputMismatch))
        );
    }

    #[tokio::test]
    async fn playback_tool_reports_input_mismatch_strict_mode_and_fails() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let recorded_args = serde_json::json!({"path":"a.txt"});
        let replay_args = serde_json::json!({"path":"b.txt"});
        let input_hash = put_cbor(store.as_ref(), &recorded_args);
        let expected = ToolOutput::Success {
            content: "ok".to_owned(),
        };
        let output_hash = put_cbor(store.as_ref(), &expected);
        let playback = PlaybackTool::from_history(
            &[(
                Hash::from_bytes(HashAlgorithm::Sha256, [11_u8; 32]),
                sample_event(input_hash, output_hash),
            )],
            store,
            tool_config(ReplayMode::Strict),
            collector(),
        )
        .expect("construct");
        let got = playback.execute(replay_args, &ctx()).await;
        assert!(matches!(got, ToolOutput::Error { .. }));
    }

    #[tokio::test]
    async fn playback_tool_cursor_exhaustion_reports_divergence() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let args = serde_json::json!({"path":"a.txt"});
        let input_hash = put_cbor(store.as_ref(), &args);
        let output_hash = put_cbor(
            store.as_ref(),
            &ToolOutput::Success {
                content: "ok".to_owned(),
            },
        );
        let divergences = collector();
        let playback = PlaybackTool::from_history(
            &[(
                Hash::from_bytes(HashAlgorithm::Sha256, [12_u8; 32]),
                sample_event(input_hash, output_hash),
            )],
            store,
            tool_config(ReplayMode::Default),
            Arc::clone(&divergences),
        )
        .expect("construct");
        let _ = playback.execute(args.clone(), &ctx()).await;
        let _ = playback.execute(args, &ctx()).await;
        let guard = divergences.lock().unwrap_or_else(|p| p.into_inner());
        assert!(
            guard
                .iter()
                .any(|d| matches!(d.kind, ReplayDivergenceKind::ToolCallUnexpected))
        );
    }
}
