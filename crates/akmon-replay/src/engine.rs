use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};

use akmon_core::{
    AgentConfig, AgentEvent, InteractivePolicyReply, PolicyEngine, PolicyEngineMode, Sandbox,
};
use akmon_journal::{
    Event, EventKind, Hash, HashAlgorithm, MemoryObjectStore, MemorySessionGraph, ObjectStore,
    RedbObjectStore, RedbSessionGraph, SessionGraph, referenced_object_hashes_for_kind,
};
use akmon_models::LlmProvider;
use akmon_query::{
    AgentSession, JournalHandle, open_default_journal_handle, open_journal_read_only,
};
use akmon_tools::Tool;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    PlaybackProvider, PlaybackProviderConfig, PlaybackTool, PlaybackToolConfig, ReplayDivergence,
    ReplayDivergenceCollector, ReplayDivergenceKind, ReplayError, ReplayMode,
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
    source_index: SourceIndex,
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
    user_prompts: Vec<String>,
    source_config_hash: Hash,
}

type PlaybackProviderMap<S> = HashMap<String, Arc<PlaybackProvider<S>>>;
type PlaybackToolMap<S> = HashMap<String, Arc<PlaybackTool<S>>>;
type BuildPlaybackMapsOutput<S> = (PlaybackProviderMap<S>, PlaybackToolMap<S>, SourceIndex);

/// Layer-2 replay execution output used as Layer-3 comparison input.
#[derive(Debug)]
pub struct ReplayRunOutput {
    /// Source session id used as replay input.
    pub source_session_id: Uuid,
    /// Replay session id generated for this replay run.
    pub replay_session_id: Uuid,
    /// Effective replay mode.
    pub mode: ReplayMode,
    /// Source event history loaded for replay.
    pub source_history: Vec<(Hash, Event)>,
    /// Replay event history emitted by replay AgentSession.
    pub replay_history: Vec<(Hash, Event)>,
    /// Runtime divergences recorded by replay primitives.
    pub divergences: Vec<crate::ReplayDivergence>,
}

/// Compares source and replay histories in default mode using index lockstep.
///
/// This comparator excludes envelope-coupled fields (`parents`, `sequence`, `emitted_at`) and
/// focuses on event-kind and kind-specific semantic content references per Decision D-09 (P1/O1).
pub fn compare_default_mode(output: &ReplayRunOutput) -> Vec<ReplayDivergence> {
    let mut divergences = Vec::new();
    let shared_len = output.source_history.len().min(output.replay_history.len());
    for idx in 0..shared_len {
        let (_, source_event) = &output.source_history[idx];
        let (_, replay_event) = &output.replay_history[idx];
        compare_event_pair(source_event, replay_event, &mut divergences);
    }
    if output.source_history.len() > shared_len {
        for (_, source_event) in &output.source_history[shared_len..] {
            divergences.push(ReplayDivergence {
                event_seq: Some(source_event.sequence),
                kind: ReplayDivergenceKind::MissingReplayEvent,
                expected: format!("event at source seq {}", source_event.sequence),
                actual: "replay history ended before this event".to_owned(),
            });
        }
    }
    if output.replay_history.len() > shared_len {
        for (_, replay_event) in &output.replay_history[shared_len..] {
            divergences.push(ReplayDivergence {
                event_seq: Some(replay_event.sequence),
                kind: ReplayDivergenceKind::UnexpectedReplayEvent,
                expected: "no additional replay events".to_owned(),
                actual: format!("unexpected replay event at seq {}", replay_event.sequence),
            });
        }
    }
    if output.source_history.len() != output.replay_history.len() {
        divergences.push(ReplayDivergence {
            event_seq: None,
            kind: ReplayDivergenceKind::EventCountMismatch,
            expected: format!("source event_count={}", output.source_history.len()),
            actual: format!("replay event_count={}", output.replay_history.len()),
        });
    }
    divergences
}

fn compare_event_pair(source: &Event, replay: &Event, divergences: &mut Vec<ReplayDivergence>) {
    if std::mem::discriminant(&source.kind) != std::mem::discriminant(&replay.kind) {
        divergences.push(ReplayDivergence {
            event_seq: Some(source.sequence),
            kind: ReplayDivergenceKind::EventKindMismatch,
            expected: kind_name(&source.kind).to_owned(),
            actual: kind_name(&replay.kind).to_owned(),
        });
        return;
    }
    match (&source.kind, &replay.kind) {
        (
            EventKind::SessionStart {
                cwd_hash: source_cwd,
                config_hash: source_cfg,
            },
            EventKind::SessionStart {
                cwd_hash: replay_cwd,
                config_hash: replay_cfg,
            },
        ) => {
            compare_hash_field(
                source.sequence,
                "SessionStart.cwd_hash",
                source_cwd,
                replay_cwd,
                divergences,
            );
            compare_hash_field(
                source.sequence,
                "SessionStart.config_hash",
                source_cfg,
                replay_cfg,
                divergences,
            );
        }
        (
            EventKind::UserTurn {
                prompt_hash: source_prompt,
            },
            EventKind::UserTurn {
                prompt_hash: replay_prompt,
            },
        ) => {
            compare_hash_field(
                source.sequence,
                "UserTurn.prompt_hash",
                source_prompt,
                replay_prompt,
                divergences,
            );
        }
        (
            EventKind::ProviderCall {
                provider_id: source_provider_id,
                attempts: source_attempts,
                stream_hash: source_stream_hash,
            },
            EventKind::ProviderCall {
                provider_id: replay_provider_id,
                attempts: replay_attempts,
                stream_hash: replay_stream_hash,
            },
        ) => {
            if source_provider_id != replay_provider_id {
                divergences.push(ReplayDivergence {
                    event_seq: Some(source.sequence),
                    kind: ReplayDivergenceKind::ContentReferenceMismatch,
                    expected: format!("ProviderCall.provider_id={source_provider_id}"),
                    actual: format!("ProviderCall.provider_id={replay_provider_id}"),
                });
            }
            let source_final_response =
                source_attempts.last().and_then(|a| a.response_hash.clone());
            let replay_final_response =
                replay_attempts.last().and_then(|a| a.response_hash.clone());
            if source_final_response != replay_final_response {
                divergences.push(ReplayDivergence {
                    event_seq: Some(source.sequence),
                    kind: ReplayDivergenceKind::AssistantContentMismatch,
                    expected: format!("ProviderCall.final_response_hash={source_final_response:?}"),
                    actual: format!("ProviderCall.final_response_hash={replay_final_response:?}"),
                });
            }
            if source_stream_hash != replay_stream_hash {
                divergences.push(ReplayDivergence {
                    event_seq: Some(source.sequence),
                    kind: ReplayDivergenceKind::AssistantContentMismatch,
                    expected: format!("ProviderCall.stream_hash={source_stream_hash:?}"),
                    actual: format!("ProviderCall.stream_hash={replay_stream_hash:?}"),
                });
            }
        }
        (
            EventKind::ToolCall {
                tool_id: source_tool_id,
                input_hash: source_input_hash,
                output_hash: source_output_hash,
                side_effects_hash: source_side_effects_hash,
            },
            EventKind::ToolCall {
                tool_id: replay_tool_id,
                input_hash: replay_input_hash,
                output_hash: replay_output_hash,
                side_effects_hash: replay_side_effects_hash,
            },
        ) => {
            if source_tool_id != replay_tool_id {
                divergences.push(ReplayDivergence {
                    event_seq: Some(source.sequence),
                    kind: ReplayDivergenceKind::ContentReferenceMismatch,
                    expected: format!("ToolCall.tool_id={source_tool_id}"),
                    actual: format!("ToolCall.tool_id={replay_tool_id}"),
                });
            }
            compare_hash_field(
                source.sequence,
                "ToolCall.input_hash",
                source_input_hash,
                replay_input_hash,
                divergences,
            );
            if source_output_hash != replay_output_hash {
                divergences.push(ReplayDivergence {
                    event_seq: Some(source.sequence),
                    kind: ReplayDivergenceKind::ToolOutputMismatch,
                    expected: format!("ToolCall.output_hash={source_output_hash}"),
                    actual: format!("ToolCall.output_hash={replay_output_hash}"),
                });
            }
            if source_side_effects_hash != replay_side_effects_hash {
                divergences.push(ReplayDivergence {
                    event_seq: Some(source.sequence),
                    kind: ReplayDivergenceKind::ContentReferenceMismatch,
                    expected: format!("ToolCall.side_effects_hash={source_side_effects_hash:?}"),
                    actual: format!("ToolCall.side_effects_hash={replay_side_effects_hash:?}"),
                });
            }
        }
        (
            EventKind::RetrievalCall {
                index_id: source_index,
                query_hash: source_query_hash,
                results_hash: source_results_hash,
            },
            EventKind::RetrievalCall {
                index_id: replay_index,
                query_hash: replay_query_hash,
                results_hash: replay_results_hash,
            },
        ) => {
            if source_index != replay_index {
                divergences.push(ReplayDivergence {
                    event_seq: Some(source.sequence),
                    kind: ReplayDivergenceKind::ContentReferenceMismatch,
                    expected: format!("RetrievalCall.index_id={source_index}"),
                    actual: format!("RetrievalCall.index_id={replay_index}"),
                });
            }
            compare_hash_field(
                source.sequence,
                "RetrievalCall.query_hash",
                source_query_hash,
                replay_query_hash,
                divergences,
            );
            compare_hash_field(
                source.sequence,
                "RetrievalCall.results_hash",
                source_results_hash,
                replay_results_hash,
                divergences,
            );
        }
        (
            EventKind::PermissionGate {
                policy_id: source_policy_id,
                decision: source_decision,
                context_hash: source_context_hash,
            },
            EventKind::PermissionGate {
                policy_id: replay_policy_id,
                decision: replay_decision,
                context_hash: replay_context_hash,
            },
        ) => {
            if source_decision != replay_decision {
                divergences.push(ReplayDivergence {
                    event_seq: Some(source.sequence),
                    kind: ReplayDivergenceKind::PermissionGateDecisionMismatch,
                    expected: format!("PermissionGate.decision={source_decision}"),
                    actual: format!("PermissionGate.decision={replay_decision}"),
                });
            }
            if source_policy_id != replay_policy_id {
                divergences.push(ReplayDivergence {
                    event_seq: Some(source.sequence),
                    kind: ReplayDivergenceKind::ContentReferenceMismatch,
                    expected: format!("PermissionGate.policy_id={source_policy_id}"),
                    actual: format!("PermissionGate.policy_id={replay_policy_id}"),
                });
            }
            compare_hash_field(
                source.sequence,
                "PermissionGate.context_hash",
                source_context_hash,
                replay_context_hash,
                divergences,
            );
        }
        (
            EventKind::AssistantTurn {
                message_hash: source_message_hash,
                tool_calls_hash: source_tool_calls_hash,
            },
            EventKind::AssistantTurn {
                message_hash: replay_message_hash,
                tool_calls_hash: replay_tool_calls_hash,
            },
        ) => {
            if source_message_hash != replay_message_hash {
                divergences.push(ReplayDivergence {
                    event_seq: Some(source.sequence),
                    kind: ReplayDivergenceKind::AssistantContentMismatch,
                    expected: format!("AssistantTurn.message_hash={source_message_hash}"),
                    actual: format!("AssistantTurn.message_hash={replay_message_hash}"),
                });
            }
            if source_tool_calls_hash != replay_tool_calls_hash {
                divergences.push(ReplayDivergence {
                    event_seq: Some(source.sequence),
                    kind: ReplayDivergenceKind::ContentReferenceMismatch,
                    expected: format!("AssistantTurn.tool_calls_hash={source_tool_calls_hash:?}"),
                    actual: format!("AssistantTurn.tool_calls_hash={replay_tool_calls_hash:?}"),
                });
            }
        }
        (
            EventKind::SessionEnd {
                summary_hash: source_summary,
            },
            EventKind::SessionEnd {
                summary_hash: replay_summary,
            },
        ) => {
            if source_summary != replay_summary {
                divergences.push(ReplayDivergence {
                    event_seq: Some(source.sequence),
                    kind: ReplayDivergenceKind::ContentReferenceMismatch,
                    expected: format!("SessionEnd.summary_hash={source_summary:?}"),
                    actual: format!("SessionEnd.summary_hash={replay_summary:?}"),
                });
            }
        }
        _ => {}
    }
}

fn compare_hash_field(
    event_seq: u64,
    field_name: &str,
    source_hash: &Hash,
    replay_hash: &Hash,
    divergences: &mut Vec<ReplayDivergence>,
) {
    if source_hash != replay_hash {
        divergences.push(ReplayDivergence {
            event_seq: Some(event_seq),
            kind: ReplayDivergenceKind::ContentReferenceMismatch,
            expected: format!("{field_name}={source_hash}"),
            actual: format!("{field_name}={replay_hash}"),
        });
    }
}

fn kind_name(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::SessionStart { .. } => "SessionStart",
        EventKind::UserTurn { .. } => "UserTurn",
        EventKind::ProviderCall { .. } => "ProviderCall",
        EventKind::ToolCall { .. } => "ToolCall",
        EventKind::RetrievalCall { .. } => "RetrievalCall",
        EventKind::PermissionGate { .. } => "PermissionGate",
        EventKind::AssistantTurn { .. } => "AssistantTurn",
        EventKind::SessionEnd { .. } => "SessionEnd",
    }
}

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
        let replay_agent_config =
            reconstruct_agent_config_from_source(source.store.as_ref(), &index.source_config_hash)?;
        Ok(Self {
            source,
            config,
            divergences,
            provider_playbacks,
            tool_playbacks,
            replay_agent_config,
            source_index: index,
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

    /// Runs source user turns against playback primitives and captures replay history.
    pub async fn drive_replay(self) -> Result<ReplayRunOutput, ReplayError> {
        let provider = self.select_single_provider()?;
        let tools = self.replay_tools();
        let mut replay_config = self.replay_agent_config.clone();
        let replay_session_id = Uuid::new_v4();
        replay_config.session_id = replay_session_id;

        let replay_history = if self.config.persist {
            let journal = open_default_journal_handle(replay_session_id).map_err(|err| {
                ReplayError::SessionRunFailed {
                    reason: format!("open persist journal: {err}"),
                }
            })?;
            self.drive_with_journal(
                replay_config,
                provider,
                tools,
                journal,
                replay_session_id,
                Arc::new(Sandbox::new(std::env::current_dir().map_err(|err| {
                    ReplayError::SessionRunFailed {
                        reason: format!("resolve current_dir: {err}"),
                    }
                })?)),
            )
            .await?
        } else {
            let replay_store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
            let replay_graph = Arc::new(Mutex::new(MemorySessionGraph::open_new(
                Arc::clone(&replay_store),
                replay_session_id,
            )));
            let journal = JournalHandle::new(replay_store, replay_graph);
            self.drive_with_journal(
                replay_config,
                provider,
                tools,
                journal,
                replay_session_id,
                Arc::new(Sandbox::new(std::env::current_dir().map_err(|err| {
                    ReplayError::SessionRunFailed {
                        reason: format!("resolve current_dir: {err}"),
                    }
                })?)),
            )
            .await?
        };

        let divergences = self
            .divergences
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        Ok(ReplayRunOutput {
            source_session_id: self.source.session_id(),
            replay_session_id,
            mode: self.config.mode,
            source_history: self.source.history().to_vec(),
            replay_history,
            divergences,
        })
    }

    fn select_single_provider(&self) -> Result<Arc<dyn LlmProvider>, ReplayError> {
        match self.provider_playbacks.len() {
            1 => {
                let provider = self
                    .provider_playbacks
                    .values()
                    .next()
                    .cloned()
                    .ok_or(ReplayError::UnsupportedProviderMultiplicity { count: 0 })?;
                Ok(provider)
            }
            n => Err(ReplayError::UnsupportedProviderMultiplicity { count: n }),
        }
    }

    fn replay_tools(&self) -> Vec<Box<dyn Tool>> {
        self.tool_playbacks
            .values()
            .map(|tool| {
                let dyn_tool: Arc<dyn Tool> = tool.clone();
                Box::new(dyn_tool) as Box<dyn Tool>
            })
            .collect()
    }

    async fn drive_with_journal<RS, RG>(
        &self,
        replay_config: AgentConfig,
        provider: Arc<dyn LlmProvider>,
        tools: Vec<Box<dyn Tool>>,
        journal: JournalHandle<RS, RG>,
        replay_session_id: Uuid,
        sandbox: Arc<Sandbox>,
    ) -> Result<Vec<(Hash, Event)>, ReplayError>
    where
        RS: ObjectStore + Send + Sync + 'static,
        RG: SessionGraph + Send + 'static,
    {
        let policy = Arc::new(PolicyEngine::new(PolicyEngineMode::DenyAll));
        let replay_graph = Arc::clone(&journal.graph);
        let mut session = AgentSession::new(
            replay_config,
            policy,
            provider,
            tools,
            sandbox,
            None,
            false,
            journal,
        )
        .map_err(|err| ReplayError::SessionRunFailed {
            reason: format!("create replay session: {err}"),
        })?;
        let (event_tx, _event_rx): (mpsc::Sender<AgentEvent>, mpsc::Receiver<AgentEvent>) =
            mpsc::channel(32);
        let mut interactive_policy_rx: Option<mpsc::Receiver<InteractivePolicyReply>> = None;
        let mut question_answer_rx: Option<mpsc::Receiver<String>> = None;
        for prompt in &self.source_index.user_prompts {
            session
                .run(
                    prompt.clone(),
                    event_tx.clone(),
                    &mut interactive_policy_rx,
                    &mut question_answer_rx,
                    None,
                )
                .await
                .map_err(|err| ReplayError::SessionRunFailed {
                    reason: format!("run replay user turn: {err}"),
                })?;
        }
        session
            .end(None)
            .map_err(|err| ReplayError::SessionRunFailed {
                reason: format!("end replay session: {err}"),
            })?;
        if session.session_id() != replay_session_id {
            return Err(ReplayError::ReplaySessionMalformed {
                reason: "replay session id mismatch".to_owned(),
            });
        }
        drop(session);
        let guard = replay_graph.lock().unwrap_or_else(|p| p.into_inner());
        let history = guard
            .history()
            .map_err(|err| ReplayError::SessionRunFailed {
                reason: format!("read replay history: {err}"),
            })?;
        if !matches!(
            history.last().map(|(_, e)| &e.kind),
            Some(EventKind::SessionEnd { .. })
        ) {
            return Err(ReplayError::ReplaySessionMalformed {
                reason: "replay history missing terminal SessionEnd".to_owned(),
            });
        }
        Ok(history)
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
            user_prompts,
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

    fn hash_of(byte: u8) -> Hash {
        Hash::from_bytes(HashAlgorithm::Sha256, [byte; 32])
    }

    fn sample_event(sequence: u64, kind: EventKind) -> Event {
        Event {
            parents: Vec::new(),
            kind,
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence,
        }
    }

    fn output_for_compare(source_events: Vec<Event>, replay_events: Vec<Event>) -> ReplayRunOutput {
        ReplayRunOutput {
            source_session_id: Uuid::new_v4(),
            replay_session_id: Uuid::new_v4(),
            mode: ReplayMode::Default,
            source_history: source_events
                .into_iter()
                .map(|e| (hash_of(e.sequence as u8), e))
                .collect(),
            replay_history: replay_events
                .into_iter()
                .map(|e| (hash_of(e.sequence as u8), e))
                .collect(),
            divergences: Vec::new(),
        }
    }

    fn request_hash(store: &MemoryObjectStore) -> Hash {
        let payload = serde_json::json!({
            "provider_id":"p1",
            "messages":[],
            "config":{
                "max_tokens":1000_u32,
                "session_id":Uuid::nil(),
                "temperature":0.0_f32,
                "first_token_deadline_ms":10000_u64,
                "stream":true,
                "tools":[]
            }
        });
        put_cbor(store, &payload)
    }

    fn response_hash_text(store: &MemoryObjectStore, text: &str) -> Hash {
        put_cbor(
            store,
            &serde_json::json!({
                "text":text,
                "tool_calls":[],
                "stop_reason":"end_turn"
            }),
        )
    }

    fn response_hash_tool_use(
        store: &MemoryObjectStore,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Hash {
        put_cbor(
            store,
            &serde_json::json!({
                "text":"",
                "tool_calls":[{
                    "id":"call_1",
                    "name":tool_name,
                    "arguments":args
                }],
                "stop_reason":"tool_use"
            }),
        )
    }

    fn valid_history(store: &MemoryObjectStore) -> Vec<(Hash, Event)> {
        let cfg = AgentConfig::default();
        let config_hash = put_cbor(store, &cfg);
        let cwd_hash = put_bytes(store, b"/tmp/replay");
        let prompt_hash = put_bytes(store, b"hello");
        let req_hash = request_hash(store);
        let rsp_hash = response_hash_text(store, "ok");
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

    fn tool_flow_history(store: &MemoryObjectStore) -> Vec<(Hash, Event)> {
        let cfg = AgentConfig::default();
        let config_hash = put_cbor(store, &cfg);
        let cwd_hash = put_bytes(store, b"/tmp/replay");
        let prompt_hash = put_bytes(store, b"use a tool");
        let req1_hash = request_hash(store);
        let req2_hash = request_hash(store);
        let tool_input_hash = put_cbor(store, &serde_json::json!({"path":"Cargo.toml"}));
        let tool_output_hash = put_cbor(
            store,
            &akmon_tools::ToolOutput::Success {
                content: "tool output".to_owned(),
            },
        );
        let rsp1_hash =
            response_hash_tool_use(store, "t1", serde_json::json!({"path":"Cargo.toml"}));
        let rsp2_hash = response_hash_text(store, "done");
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
                attempts: vec![sample_attempt(req1_hash, Some(rsp1_hash))],
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
                input_hash: tool_input_hash,
                output_hash: tool_output_hash,
                side_effects_hash: None,
            },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 3,
        };
        let h3 = e3.content_hash(store.algorithm()).expect("hash3");
        let e4 = Event {
            parents: vec![h3.clone()],
            kind: EventKind::ProviderCall {
                provider_id: "p1".to_owned(),
                attempts: vec![sample_attempt(req2_hash, Some(rsp2_hash))],
                stream_hash: None,
            },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 4,
        };
        let h4 = e4.content_hash(store.algorithm()).expect("hash4");
        let e5 = Event {
            parents: vec![h4.clone()],
            kind: EventKind::SessionEnd { summary_hash: None },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 5,
        };
        let h5 = e5.content_hash(store.algorithm()).expect("hash5");
        vec![(h0, e0), (h1, e1), (h2, e2), (h3, e3), (h4, e4), (h5, e5)]
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

    #[tokio::test]
    async fn t_drive_replay_clean_session_completes() {
        let source = source_session();
        let engine = ReplayEngine::new(
            source,
            ReplayEngineConfig {
                mode: ReplayMode::Default,
                persist: false,
            },
        )
        .expect("engine");
        let out = engine.drive_replay().await.expect("drive");
        assert!(!out.replay_history.is_empty());
        assert!(matches!(
            out.replay_history.last().map(|(_, e)| &e.kind),
            Some(EventKind::SessionEnd { .. })
        ));
        assert_eq!(out.mode, ReplayMode::Default);
    }

    #[tokio::test]
    async fn t_drive_replay_captures_provider_responses() {
        let source = source_session();
        let engine = ReplayEngine::new(
            source,
            ReplayEngineConfig {
                mode: ReplayMode::Default,
                persist: false,
            },
        )
        .expect("engine");
        let out = engine.drive_replay().await.expect("drive");
        let provider_calls = out
            .replay_history
            .iter()
            .filter(|(_, e)| matches!(e.kind, EventKind::ProviderCall { .. }))
            .count();
        assert!(provider_calls >= 1);
    }

    #[tokio::test]
    async fn t_drive_replay_captures_tool_outputs() {
        let store = MemoryObjectStore::new(HashAlgorithm::Sha256);
        let history = tool_flow_history(&store);
        let source = SourceSession::new(
            Uuid::new_v4(),
            Arc::new(store),
            Arc::new(Mutex::new(akmon_journal::MemorySessionGraph::open_new(
                Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256)),
                Uuid::new_v4(),
            ))),
            history,
        );
        let engine = ReplayEngine::new(
            source,
            ReplayEngineConfig {
                mode: ReplayMode::Default,
                persist: false,
            },
        )
        .expect("engine");
        let out = engine.drive_replay().await.expect("drive");
        assert!(
            out.replay_history
                .iter()
                .any(|(_, e)| matches!(e.kind, EventKind::ToolCall { .. }))
        );
    }

    #[tokio::test]
    async fn t_drive_replay_records_divergences_on_unexpected_calls() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cfg_hash = put_cbor(store.as_ref(), &AgentConfig::default());
        let cwd_hash = put_bytes(store.as_ref(), b"/tmp/replay");
        let prompt_hash = put_bytes(store.as_ref(), b"hello");
        let req_hash = request_hash(store.as_ref());
        let rsp_hash = response_hash_tool_use(store.as_ref(), "t1", serde_json::json!({"x":1}));
        let tool_in = put_cbor(store.as_ref(), &serde_json::json!({"x":1}));
        let tool_out = put_cbor(
            store.as_ref(),
            &akmon_tools::ToolOutput::Success {
                content: "ok".to_owned(),
            },
        );
        let e0 = Event {
            parents: vec![],
            kind: EventKind::SessionStart {
                cwd_hash,
                config_hash: cfg_hash,
            },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 0,
        };
        let h0 = e0.content_hash(store.algorithm()).expect("h0");
        let e1 = Event {
            parents: vec![h0.clone()],
            kind: EventKind::UserTurn { prompt_hash },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 1,
        };
        let h1 = e1.content_hash(store.algorithm()).expect("h1");
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
        let h2 = e2.content_hash(store.algorithm()).expect("h2");
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
        let h3 = e3.content_hash(store.algorithm()).expect("h3");
        let e4 = Event {
            parents: vec![h3.clone()],
            kind: EventKind::SessionEnd { summary_hash: None },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 4,
        };
        let h4 = e4.content_hash(store.algorithm()).expect("h4");
        let history = vec![(h0, e0), (h1, e1), (h2, e2), (h3, e3), (h4, e4)];
        let graph = Arc::new(Mutex::new(akmon_journal::MemorySessionGraph::open_new(
            Arc::clone(&store),
            Uuid::new_v4(),
        )));
        let source = SourceSession::new(Uuid::new_v4(), store, graph, history);
        let engine = ReplayEngine::new(
            source,
            ReplayEngineConfig {
                mode: ReplayMode::Default,
                persist: false,
            },
        )
        .expect("engine");
        let out = engine.drive_replay().await.expect("drive");
        assert!(
            out.divergences
                .iter()
                .any(|d| { matches!(d.kind, crate::ReplayDivergenceKind::ProviderCallUnexpected) })
        );
    }

    #[tokio::test]
    async fn t_drive_replay_handles_replay_errors_gracefully() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cfg_hash = put_cbor(store.as_ref(), &AgentConfig::default());
        let cwd_hash = put_bytes(store.as_ref(), b"/tmp/replay");
        let prompt_hash = put_bytes(store.as_ref(), b"hello");
        let e0 = Event {
            parents: vec![],
            kind: EventKind::SessionStart {
                cwd_hash,
                config_hash: cfg_hash,
            },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 0,
        };
        let h0 = e0.content_hash(store.algorithm()).expect("h0");
        let e1 = Event {
            parents: vec![h0.clone()],
            kind: EventKind::UserTurn { prompt_hash },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 1,
        };
        let h1 = e1.content_hash(store.algorithm()).expect("h1");
        let e2 = Event {
            parents: vec![h1.clone()],
            kind: EventKind::SessionEnd { summary_hash: None },
            emitted_at: OffsetDateTime::UNIX_EPOCH,
            sequence: 2,
        };
        let h2 = e2.content_hash(store.algorithm()).expect("h2");
        let graph = Arc::new(Mutex::new(akmon_journal::MemorySessionGraph::open_new(
            Arc::clone(&store),
            Uuid::new_v4(),
        )));
        let source = SourceSession::new(
            Uuid::new_v4(),
            store,
            graph,
            vec![(h0, e0), (h1, e1), (h2, e2)],
        );
        let engine = ReplayEngine::new(
            source,
            ReplayEngineConfig {
                mode: ReplayMode::Default,
                persist: false,
            },
        )
        .expect("engine");
        let err = engine.drive_replay().await.expect_err("must fail");
        assert!(matches!(
            err,
            ReplayError::UnsupportedProviderMultiplicity { count: 0 }
        ));
    }

    #[test]
    fn t_compare_identical_histories_no_divergences() {
        let source = vec![
            sample_event(
                0,
                EventKind::SessionStart {
                    cwd_hash: hash_of(1),
                    config_hash: hash_of(2),
                },
            ),
            sample_event(
                1,
                EventKind::UserTurn {
                    prompt_hash: hash_of(3),
                },
            ),
            sample_event(2, EventKind::SessionEnd { summary_hash: None }),
        ];
        let out = output_for_compare(source.clone(), source);
        let diffs = compare_default_mode(&out);
        assert!(diffs.is_empty());
    }

    #[test]
    fn t_compare_detects_event_kind_mismatch() {
        let out = output_for_compare(
            vec![sample_event(
                3,
                EventKind::UserTurn {
                    prompt_hash: hash_of(7),
                },
            )],
            vec![sample_event(
                3,
                EventKind::ToolCall {
                    tool_id: "t".to_owned(),
                    input_hash: hash_of(7),
                    output_hash: hash_of(8),
                    side_effects_hash: None,
                },
            )],
        );
        let diffs = compare_default_mode(&out);
        assert!(
            diffs
                .iter()
                .any(|d| matches!(d.kind, ReplayDivergenceKind::EventKindMismatch))
        );
    }

    #[test]
    fn t_compare_detects_missing_replay_event() {
        let out = output_for_compare(
            vec![
                sample_event(
                    0,
                    EventKind::UserTurn {
                        prompt_hash: hash_of(1),
                    },
                ),
                sample_event(
                    1,
                    EventKind::UserTurn {
                        prompt_hash: hash_of(2),
                    },
                ),
            ],
            vec![sample_event(
                0,
                EventKind::UserTurn {
                    prompt_hash: hash_of(1),
                },
            )],
        );
        let diffs = compare_default_mode(&out);
        assert!(
            diffs
                .iter()
                .any(|d| matches!(d.kind, ReplayDivergenceKind::MissingReplayEvent))
        );
    }

    #[test]
    fn t_compare_detects_unexpected_replay_event() {
        let out = output_for_compare(
            vec![sample_event(
                0,
                EventKind::UserTurn {
                    prompt_hash: hash_of(1),
                },
            )],
            vec![
                sample_event(
                    0,
                    EventKind::UserTurn {
                        prompt_hash: hash_of(1),
                    },
                ),
                sample_event(1, EventKind::SessionEnd { summary_hash: None }),
            ],
        );
        let diffs = compare_default_mode(&out);
        assert!(
            diffs
                .iter()
                .any(|d| matches!(d.kind, ReplayDivergenceKind::UnexpectedReplayEvent))
        );
    }

    #[test]
    fn t_compare_detects_event_count_mismatch() {
        let out = output_for_compare(
            vec![],
            vec![sample_event(
                0,
                EventKind::SessionEnd { summary_hash: None },
            )],
        );
        let diffs = compare_default_mode(&out);
        assert!(
            diffs
                .iter()
                .any(|d| matches!(d.kind, ReplayDivergenceKind::EventCountMismatch))
        );
    }

    #[test]
    fn t_compare_detects_assistant_content_mismatch() {
        let out = output_for_compare(
            vec![sample_event(
                1,
                EventKind::AssistantTurn {
                    message_hash: hash_of(1),
                    tool_calls_hash: None,
                },
            )],
            vec![sample_event(
                1,
                EventKind::AssistantTurn {
                    message_hash: hash_of(2),
                    tool_calls_hash: None,
                },
            )],
        );
        let diffs = compare_default_mode(&out);
        assert!(
            diffs
                .iter()
                .any(|d| matches!(d.kind, ReplayDivergenceKind::AssistantContentMismatch))
        );
    }

    #[test]
    fn t_compare_detects_tool_output_mismatch() {
        let out = output_for_compare(
            vec![sample_event(
                2,
                EventKind::ToolCall {
                    tool_id: "t1".to_owned(),
                    input_hash: hash_of(1),
                    output_hash: hash_of(2),
                    side_effects_hash: None,
                },
            )],
            vec![sample_event(
                2,
                EventKind::ToolCall {
                    tool_id: "t1".to_owned(),
                    input_hash: hash_of(1),
                    output_hash: hash_of(3),
                    side_effects_hash: None,
                },
            )],
        );
        let diffs = compare_default_mode(&out);
        assert!(
            diffs
                .iter()
                .any(|d| matches!(d.kind, ReplayDivergenceKind::ToolOutputMismatch))
        );
    }

    #[test]
    fn t_compare_detects_permission_gate_decision_mismatch() {
        let out = output_for_compare(
            vec![sample_event(
                2,
                EventKind::PermissionGate {
                    policy_id: "p".to_owned(),
                    decision: "allowed".to_owned(),
                    context_hash: hash_of(7),
                },
            )],
            vec![sample_event(
                2,
                EventKind::PermissionGate {
                    policy_id: "p".to_owned(),
                    decision: "denied".to_owned(),
                    context_hash: hash_of(7),
                },
            )],
        );
        let diffs = compare_default_mode(&out);
        assert!(
            diffs.iter().any(|d| {
                matches!(d.kind, ReplayDivergenceKind::PermissionGateDecisionMismatch)
            })
        );
    }

    #[test]
    fn t_compare_no_realignment_after_divergence() {
        let source = vec![
            sample_event(
                0,
                EventKind::UserTurn {
                    prompt_hash: hash_of(1),
                },
            ),
            sample_event(
                1,
                EventKind::AssistantTurn {
                    message_hash: hash_of(2),
                    tool_calls_hash: None,
                },
            ),
            sample_event(
                2,
                EventKind::ToolCall {
                    tool_id: "t".to_owned(),
                    input_hash: hash_of(3),
                    output_hash: hash_of(4),
                    side_effects_hash: None,
                },
            ),
            sample_event(3, EventKind::SessionEnd { summary_hash: None }),
            sample_event(
                4,
                EventKind::UserTurn {
                    prompt_hash: hash_of(5),
                },
            ),
        ];
        let replay = vec![
            sample_event(
                0,
                EventKind::UserTurn {
                    prompt_hash: hash_of(1),
                },
            ),
            sample_event(
                1,
                EventKind::AssistantTurn {
                    message_hash: hash_of(2),
                    tool_calls_hash: None,
                },
            ),
            sample_event(
                2,
                EventKind::ToolCall {
                    tool_id: "t".to_owned(),
                    input_hash: hash_of(3),
                    output_hash: hash_of(4),
                    side_effects_hash: None,
                },
            ),
            sample_event(
                3,
                EventKind::PermissionGate {
                    policy_id: "p".to_owned(),
                    decision: "allowed".to_owned(),
                    context_hash: hash_of(8),
                },
            ),
            sample_event(
                4,
                EventKind::UserTurn {
                    prompt_hash: hash_of(5),
                },
            ),
        ];
        let out = output_for_compare(source, replay);
        let diffs = compare_default_mode(&out);
        let kind_mismatches = diffs
            .iter()
            .filter(|d| matches!(d.kind, ReplayDivergenceKind::EventKindMismatch))
            .count();
        assert_eq!(kind_mismatches, 1);
    }

    #[test]
    fn t_compare_excludes_timestamps_from_default_comparison() {
        let mut source_event = sample_event(
            1,
            EventKind::AssistantTurn {
                message_hash: hash_of(9),
                tool_calls_hash: None,
            },
        );
        let mut replay_event = source_event.clone();
        source_event.emitted_at = OffsetDateTime::UNIX_EPOCH;
        replay_event.emitted_at = OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(90);
        let out = output_for_compare(vec![source_event], vec![replay_event]);
        let diffs = compare_default_mode(&out);
        assert!(diffs.is_empty());
    }
}
