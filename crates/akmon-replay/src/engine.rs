use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};

use akmon_core::AgentConfig;
use akmon_journal::{
    Event, EventKind, Hash, ObjectStore, RedbObjectStore, RedbSessionGraph, SessionGraph,
    referenced_object_hashes_for_kind,
};
use akmon_query::open_journal_read_only;
use uuid::Uuid;

use crate::{
    PlaybackProvider, PlaybackProviderConfig, PlaybackTool, PlaybackToolConfig,
    ReplayDivergenceCollector, ReplayError, ReplayMode,
};

/// Replay engine setup and orchestration state (Layer 1: loading and setup only).
pub struct ReplayEngine<S, G>
where
    S: ObjectStore + Send + Sync + 'static,
    G: SessionGraph + Send + 'static,
{
    source: SourceSession<S, G>,
    config: ReplayEngineConfig,
    divergences: ReplayDivergenceCollector,
    provider_playbacks: HashMap<String, Arc<PlaybackProvider<S>>>,
    tool_playbacks: HashMap<String, Arc<PlaybackTool<S>>>,
    replay_agent_config: AgentConfig,
}

/// Replay engine configuration.
#[derive(Debug, Clone)]
pub struct ReplayEngineConfig {
    /// Replay comparison and mismatch mode.
    pub mode: ReplayMode,
    /// Whether replay output should be persisted as a new session.
    pub persist: bool,
}

/// Loaded source-session material used by replay setup and execution.
pub struct SourceSession<S, G>
where
    S: ObjectStore,
    G: SessionGraph,
{
    session_id: Uuid,
    store: Arc<S>,
    graph: Arc<Mutex<G>>,
    history: Vec<(Hash, Event)>,
}

impl<S, G> SourceSession<S, G>
where
    S: ObjectStore,
    G: SessionGraph,
{
    /// Creates a source-session container from loaded components.
    pub fn new(
        session_id: Uuid,
        store: Arc<S>,
        graph: Arc<Mutex<G>>,
        history: Vec<(Hash, Event)>,
    ) -> Self {
        Self {
            session_id,
            store,
            graph,
            history,
        }
    }

    /// Source session UUID.
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// Source event history in sequence order.
    pub fn history(&self) -> &[(Hash, Event)] {
        &self.history
    }

    /// Source object store.
    pub fn store(&self) -> &Arc<S> {
        &self.store
    }

    /// Source graph handle.
    pub fn graph(&self) -> &Arc<Mutex<G>> {
        &self.graph
    }
}

#[derive(Debug, Clone)]
struct SourceIndex {
    provider_ids: BTreeSet<String>,
    tool_ids: BTreeSet<String>,
    user_prompts: Vec<String>,
    source_head: Hash,
    source_config_hash: Hash,
}

type PlaybackProviderMap<S> = HashMap<String, Arc<PlaybackProvider<S>>>;
type PlaybackToolMap<S> = HashMap<String, Arc<PlaybackTool<S>>>;
type BuildPlaybackMapsOutput<S> = (PlaybackProviderMap<S>, PlaybackToolMap<S>, SourceIndex);

/// Load a source session from an on-disk journal directory.
///
/// This loader is concrete to the production storage backend (Redb). Tests that need to exercise
/// [`ReplayEngine`] with a different store backend (for example, `MemoryObjectStore`) should
/// construct [`SourceSession`] directly via [`SourceSession::new`] (or test-only helpers),
/// bypassing this loader.
pub fn load_source_session_from_journal(
    journal_dir: &Path,
    session_id: Uuid,
) -> Result<SourceSession<RedbObjectStore, RedbSessionGraph>, ReplayError> {
    let handle = open_journal_read_only(journal_dir, session_id).map_err(|err| {
        ReplayError::MalformedSourceEvent {
            event_seq: 0,
            reason: err.to_string(),
        }
    })?;
    let history = {
        let guard = handle.graph.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .history()
            .map_err(|err| ReplayError::MalformedSourceEvent {
                event_seq: 0,
                reason: err.to_string(),
            })?
    };
    Ok(SourceSession::new(
        session_id,
        Arc::clone(&handle.store),
        Arc::clone(&handle.graph),
        history,
    ))
}

impl<S, G> ReplayEngine<S, G>
where
    S: ObjectStore + Send + Sync + 'static,
    G: SessionGraph + Send + 'static,
{
    /// Builds replay-engine setup state from a loaded source session.
    pub fn new(
        source: SourceSession<S, G>,
        config: ReplayEngineConfig,
    ) -> Result<Self, ReplayError> {
        Self::validate_source_preconditions(source.history(), source.store.as_ref())?;
        let divergences: ReplayDivergenceCollector = Arc::new(Mutex::new(Vec::new()));
        let (provider_playbacks, tool_playbacks, index) = build_playback_maps(
            source.history(),
            Arc::clone(source.store()),
            config.mode,
            Arc::clone(&divergences),
        )?;
        let _ = (
            &index.provider_ids,
            &index.tool_ids,
            &index.user_prompts,
            &index.source_head,
        );
        let replay_agent_config =
            reconstruct_agent_config_from_source(source.store.as_ref(), &index.source_config_hash)?;
        Ok(Self {
            source,
            config,
            divergences,
            provider_playbacks,
            tool_playbacks,
            replay_agent_config,
        })
    }

    fn validate_source_preconditions(
        history: &[(Hash, Event)],
        store: &dyn ObjectStore,
    ) -> Result<(), ReplayError> {
        if history.is_empty() {
            return Err(ReplayError::EmptySource);
        }
        let Some((_, first)) = history.first() else {
            return Err(ReplayError::EmptySource);
        };
        if !matches!(first.kind, EventKind::SessionStart { .. }) {
            return Err(ReplayError::MalformedSourceEvent {
                event_seq: first.sequence,
                reason: "first event must be SessionStart".to_owned(),
            });
        }
        let session_end_positions: Vec<u64> = history
            .iter()
            .filter_map(|(_, e)| {
                matches!(e.kind, EventKind::SessionEnd { .. }).then_some(e.sequence)
            })
            .collect();
        if session_end_positions.len() != 1 {
            return Err(ReplayError::MalformedSourceEvent {
                event_seq: 0,
                reason: format!(
                    "source must contain exactly one SessionEnd (found {})",
                    session_end_positions.len()
                ),
            });
        }
        let expected_terminal = history.last().map(|(_, e)| e.sequence).unwrap_or(0);
        if session_end_positions[0] != expected_terminal {
            return Err(ReplayError::MalformedSourceEvent {
                event_seq: session_end_positions[0],
                reason: "SessionEnd must be terminal".to_owned(),
            });
        }
        for (_, event) in history {
            for object_hash in referenced_object_hashes_for_kind(&event.kind) {
                match store.contains(&object_hash) {
                    Ok(true) => {}
                    Ok(false) => return Err(ReplayError::MissingSourceObject(object_hash)),
                    Err(err) => {
                        return Err(ReplayError::StoreReadFailed {
                            hash: object_hash,
                            reason: err.to_string(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Loaded source session state.
    pub fn source(&self) -> &SourceSession<S, G> {
        &self.source
    }

    /// Replay setup config.
    pub fn config(&self) -> &ReplayEngineConfig {
        &self.config
    }

    /// Shared divergence collector used by playback primitives.
    pub fn divergences(&self) -> &ReplayDivergenceCollector {
        &self.divergences
    }

    /// Provider playback map keyed by source `provider_id`.
    pub fn provider_playbacks(&self) -> &HashMap<String, Arc<PlaybackProvider<S>>> {
        &self.provider_playbacks
    }

    /// Tool playback map keyed by source `tool_id`.
    pub fn tool_playbacks(&self) -> &HashMap<String, Arc<PlaybackTool<S>>> {
        &self.tool_playbacks
    }

    /// Agent config reconstructed from source `SessionStart.config_hash`.
    pub fn replay_agent_config(&self) -> &AgentConfig {
        &self.replay_agent_config
    }
}

fn build_playback_maps<S: ObjectStore + Send + Sync + 'static>(
    history: &[(Hash, Event)],
    store: Arc<S>,
    mode: ReplayMode,
    divergences: ReplayDivergenceCollector,
) -> Result<BuildPlaybackMapsOutput<S>, ReplayError> {
    let mut provider_ids = BTreeSet::new();
    let mut tool_ids = BTreeSet::new();
    let mut user_prompts = Vec::new();
    let mut source_config_hash: Option<Hash> = None;
    for (_, event) in history {
        match &event.kind {
            EventKind::SessionStart { config_hash, .. } => {
                if source_config_hash.is_none() {
                    source_config_hash = Some(config_hash.clone());
                }
            }
            EventKind::UserTurn { prompt_hash } => {
                let prompt_bytes = store
                    .get(prompt_hash)
                    .map_err(|err| ReplayError::StoreReadFailed {
                        hash: prompt_hash.clone(),
                        reason: err.to_string(),
                    })?
                    .ok_or_else(|| ReplayError::MissingSourceObject(prompt_hash.clone()))?;
                let prompt = std::str::from_utf8(prompt_bytes.as_ref()).map_err(|err| {
                    ReplayError::MalformedSourceEvent {
                        event_seq: event.sequence,
                        reason: format!("UserTurn prompt bytes are not UTF-8: {err}"),
                    }
                })?;
                user_prompts.push(prompt.to_owned());
            }
            EventKind::ProviderCall { provider_id, .. } => {
                provider_ids.insert(provider_id.clone());
            }
            EventKind::ToolCall { tool_id, .. } => {
                tool_ids.insert(tool_id.clone());
            }
            _ => {}
        }
    }
    let source_head = history
        .last()
        .map(|(h, _)| h.clone())
        .ok_or(ReplayError::EmptySource)?;
    let source_config_hash =
        source_config_hash.ok_or_else(|| ReplayError::MalformedSourceEvent {
            event_seq: 0,
            reason: "SessionStart with config_hash is required".to_owned(),
        })?;

    let mut provider_playbacks = HashMap::new();
    for provider_id in &provider_ids {
        let provider = PlaybackProvider::from_history(
            history,
            Arc::clone(&store),
            PlaybackProviderConfig {
                mode,
                provider_id: provider_id.clone(),
                provider_name: provider_id.clone(),
                model_id: provider_id.clone(),
                context_window_tokens: 200_000,
            },
            Arc::clone(&divergences),
        )
        .map_err(|err| match err {
            ReplayError::NoMatchingCalls(_) => ReplayError::MissingProviderForReplay {
                provider_id: provider_id.clone(),
            },
            other => other,
        })?;
        provider_playbacks.insert(provider_id.clone(), Arc::new(provider));
    }

    let mut tool_playbacks = HashMap::new();
    for tool_id in &tool_ids {
        let tool = PlaybackTool::from_history(
            history,
            Arc::clone(&store),
            PlaybackToolConfig {
                mode,
                tool_id: tool_id.clone(),
                description: format!("replay playback tool for {tool_id}"),
            },
            Arc::clone(&divergences),
        )
        .map_err(|err| match err {
            ReplayError::NoMatchingCalls(_) => ReplayError::MissingToolForReplay {
                tool_id: tool_id.clone(),
            },
            other => other,
        })?;
        tool_playbacks.insert(tool_id.clone(), Arc::new(tool));
    }

    Ok((
        provider_playbacks,
        tool_playbacks,
        SourceIndex {
            provider_ids,
            tool_ids,
            user_prompts,
            source_head,
            source_config_hash,
        },
    ))
}

fn reconstruct_agent_config_from_source<S: ObjectStore>(
    store: &S,
    config_hash: &Hash,
) -> Result<AgentConfig, ReplayError> {
    let bytes = store
        .get(config_hash)
        .map_err(|err| ReplayError::StoreReadFailed {
            hash: config_hash.clone(),
            reason: err.to_string(),
        })?
        .ok_or_else(|| ReplayError::MissingSourceObject(config_hash.clone()))?;
    ciborium::de::from_reader(bytes.as_ref()).map_err(|err| ReplayError::MalformedSourceConfig {
        config_hash: config_hash.clone(),
        reason: err.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_journal::{AttemptRecord, AttemptStatus, HashAlgorithm, MemoryObjectStore};
    use time::OffsetDateTime;

    fn put_bytes(store: &MemoryObjectStore, bytes: &[u8]) -> Hash {
        store.put(bytes).expect("put bytes")
    }

    fn put_cbor<T: serde::Serialize>(store: &MemoryObjectStore, value: &T) -> Hash {
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(value, &mut bytes).expect("encode");
        store.put(&bytes).expect("put cbor")
    }

    fn sample_attempt(request_hash: Hash, response_hash: Option<Hash>) -> AttemptRecord {
        AttemptRecord {
            attempt_number: 1,
            started_at: OffsetDateTime::UNIX_EPOCH,
            ended_at: OffsetDateTime::UNIX_EPOCH,
            status: AttemptStatus::Success,
            request_hash,
            response_hash,
            stream_hash: None,
            error_message: None,
        }
    }

    fn valid_history(store: &MemoryObjectStore) -> Vec<(Hash, Event)> {
        let cfg = AgentConfig::default();
        let config_hash = put_cbor(store, &cfg);
        let cwd_hash = put_bytes(store, b"/tmp/replay");
        let prompt_hash = put_bytes(store, b"hello");
        let req_hash = put_cbor(
            store,
            &serde_json::json!({"provider_id":"p1","messages":[],"config":{}}),
        );
        let rsp_hash = put_cbor(
            store,
            &serde_json::json!({"text":"ok","tool_calls":[],"stop_reason":"end_turn"}),
        );
        let tool_in = put_cbor(store, &serde_json::json!({"x":1}));
        let tool_out = put_cbor(
            store,
            &akmon_tools::ToolOutput::Success {
                content: "ok".to_owned(),
            },
        );
        let e0 = Event {
            parents: vec![],
            kind: EventKind::SessionStart {
                cwd_hash,
                config_hash,
            },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 0,
        };
        let h0 = e0.content_hash(store.algorithm()).expect("hash0");
        let e1 = Event {
            parents: vec![h0.clone()],
            kind: EventKind::UserTurn { prompt_hash },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 1,
        };
        let h1 = e1.content_hash(store.algorithm()).expect("hash1");
        let e2 = Event {
            parents: vec![h1.clone()],
            kind: EventKind::ProviderCall {
                provider_id: "p1".to_owned(),
                attempts: vec![sample_attempt(req_hash, Some(rsp_hash))],
                stream_hash: None,
            },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 2,
        };
        let h2 = e2.content_hash(store.algorithm()).expect("hash2");
        let e3 = Event {
            parents: vec![h2.clone()],
            kind: EventKind::ToolCall {
                tool_id: "t1".to_owned(),
                input_hash: tool_in,
                output_hash: tool_out,
                side_effects_hash: None,
            },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 3,
        };
        let h3 = e3.content_hash(store.algorithm()).expect("hash3");
        let e4 = Event {
            parents: vec![h3.clone()],
            kind: EventKind::SessionEnd { summary_hash: None },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 4,
        };
        let h4 = e4.content_hash(store.algorithm()).expect("hash4");
        vec![(h0, e0), (h1, e1), (h2, e2), (h3, e3), (h4, e4)]
    }

    fn source_session() -> SourceSession<MemoryObjectStore, akmon_journal::MemorySessionGraph> {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let history = valid_history(store.as_ref());
        let graph = Arc::new(Mutex::new(akmon_journal::MemorySessionGraph::open_new(
            Arc::clone(&store),
            Uuid::new_v4(),
        )));
        SourceSession::new(Uuid::new_v4(), store, graph, history)
    }

    #[test]
    fn preconditions_reject_empty_source() {
        let store = MemoryObjectStore::new(HashAlgorithm::Sha256);
        let err = ReplayEngine::<MemoryObjectStore, akmon_journal::MemorySessionGraph>::validate_source_preconditions(&[], &store)
            .expect_err("must fail");
        assert!(matches!(err, ReplayError::EmptySource));
    }

    #[test]
    fn preconditions_reject_non_terminal_session_end() {
        let store = MemoryObjectStore::new(HashAlgorithm::Sha256);
        let mut history = valid_history(&store);
        history.swap(3, 4);
        let err = ReplayEngine::<MemoryObjectStore, akmon_journal::MemorySessionGraph>::validate_source_preconditions(
            &history,
            &store,
        )
        .expect_err("must fail");
        assert!(matches!(err, ReplayError::MalformedSourceEvent { .. }));
    }

    #[test]
    fn preconditions_reject_missing_referenced_object() {
        let store = MemoryObjectStore::new(HashAlgorithm::Sha256);
        let mut history = valid_history(&store);
        if let EventKind::ToolCall { output_hash, .. } = &mut history[3].1.kind {
            *output_hash = Hash::from_bytes(HashAlgorithm::Sha256, [0xEE_u8; 32]);
        }
        let err = ReplayEngine::<MemoryObjectStore, akmon_journal::MemorySessionGraph>::validate_source_preconditions(
            &history,
            &store,
        )
        .expect_err("must fail");
        assert!(matches!(err, ReplayError::MissingSourceObject(_)));
    }

    #[test]
    fn replay_engine_new_builds_playback_maps_and_config() {
        let source = source_session();
        let engine = ReplayEngine::new(
            source,
            ReplayEngineConfig {
                mode: ReplayMode::Default,
                persist: false,
            },
        )
        .expect("new");
        assert_eq!(engine.provider_playbacks().len(), 1);
        assert_eq!(engine.tool_playbacks().len(), 1);
    }

    #[test]
    fn reconstruct_agent_config_rejects_malformed_config_bytes() {
        let store = MemoryObjectStore::new(HashAlgorithm::Sha256);
        let cfg_hash = put_bytes(&store, b"not-cbor");
        let err = reconstruct_agent_config_from_source(&store, &cfg_hash).expect_err("must fail");
        assert!(matches!(err, ReplayError::MalformedSourceConfig { .. }));
    }
}
