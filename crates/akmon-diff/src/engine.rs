use std::path::Path;
use std::sync::{Arc, Mutex};

use akmon_journal::{
    Event, EventKind, Hash, ObjectStore, RedbObjectStore, RedbSessionGraph, SessionGraph,
    referenced_object_hashes_for_kind,
};
use akmon_query::{journal_contains_session, open_journal_read_only};
use uuid::Uuid;

use crate::resolve::ResolveContext;
use crate::{DiffComparison, DiffError, DiffMode, DiffReportV1};

/// Loaded source-session material used by diff setup and execution.
#[derive(Debug)]
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

/// Load one source session from an on-disk journal directory.
pub fn load_source_session_from_journal(
    journal_dir: &Path,
    session_id: Uuid,
) -> Result<SourceSession<RedbObjectStore, RedbSessionGraph>, DiffError> {
    let exists = journal_contains_session(journal_dir, session_id).map_err(|err| {
        DiffError::StoreAccessFailed {
            source: Box::new(std::io::Error::other(err.to_string())),
        }
    })?;
    if !exists {
        return Err(DiffError::SourceSessionMissing {
            session_id: session_id.to_string(),
        });
    }
    let handle = open_journal_read_only(journal_dir, session_id).map_err(|err| {
        DiffError::SourceSessionLoadFailed {
            session_id: session_id.to_string(),
            source: Box::new(std::io::Error::other(err.to_string())),
        }
    })?;
    let history = {
        let guard = handle.graph.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .history()
            .map_err(|err| DiffError::SourceSessionLoadFailed {
                session_id: session_id.to_string(),
                source: Box::new(std::io::Error::other(err.to_string())),
            })?
    };
    Ok(SourceSession::new(
        session_id,
        Arc::clone(&handle.store),
        Arc::clone(&handle.graph),
        history,
    ))
}

/// Diff engine setup and source state.
#[derive(Debug)]
pub struct DiffEngine<S, G>
where
    S: ObjectStore + Send + Sync + 'static,
    G: SessionGraph + Send + 'static,
{
    source_a: SourceSession<S, G>,
    source_b: SourceSession<S, G>,
    mode: DiffMode,
}

impl<S, G> DiffEngine<S, G>
where
    S: ObjectStore + Send + Sync + 'static,
    G: SessionGraph + Send + 'static,
{
    /// Creates a diff engine with two validated source sessions.
    pub fn new(
        source_a: SourceSession<S, G>,
        source_b: SourceSession<S, G>,
    ) -> Result<Self, DiffError> {
        Self::validate_source_preconditions("A", source_a.history(), source_a.store.as_ref())?;
        Self::validate_source_preconditions("B", source_b.history(), source_b.store.as_ref())?;
        Ok(Self {
            source_a,
            source_b,
            mode: DiffMode::Default,
        })
    }

    fn validate_source_preconditions(
        label: &str,
        history: &[(Hash, Event)],
        store: &dyn ObjectStore,
    ) -> Result<(), DiffError> {
        if history.is_empty() {
            return Err(DiffError::SourcePreconditionViolated {
                session_label: label.to_owned(),
                violation: "history is empty".to_owned(),
            });
        }
        let Some((_, first)) = history.first() else {
            return Err(DiffError::SourcePreconditionViolated {
                session_label: label.to_owned(),
                violation: "history is empty".to_owned(),
            });
        };
        if !matches!(first.kind, EventKind::SessionStart { .. }) {
            return Err(DiffError::SourcePreconditionViolated {
                session_label: label.to_owned(),
                violation: format!(
                    "first event must be SessionStart (sequence={})",
                    first.sequence
                ),
            });
        }
        let session_end_positions: Vec<u64> = history
            .iter()
            .filter_map(|(_, event)| {
                matches!(event.kind, EventKind::SessionEnd { .. }).then_some(event.sequence)
            })
            .collect();
        if session_end_positions.len() != 1 {
            return Err(DiffError::SourcePreconditionViolated {
                session_label: label.to_owned(),
                violation: format!(
                    "must contain exactly one SessionEnd (found {})",
                    session_end_positions.len()
                ),
            });
        }
        let expected_terminal = history.last().map(|(_, event)| event.sequence).unwrap_or(0);
        if session_end_positions[0] != expected_terminal {
            return Err(DiffError::SourcePreconditionViolated {
                session_label: label.to_owned(),
                violation: format!(
                    "SessionEnd must be terminal (session_end_at={} terminal={expected_terminal})",
                    session_end_positions[0]
                ),
            });
        }
        for (_, event) in history {
            for object_hash in referenced_object_hashes_for_kind(&event.kind) {
                match store.contains(&object_hash) {
                    Ok(true) => {}
                    Ok(false) => {
                        return Err(DiffError::StoreAccessFailed {
                            source: Box::new(std::io::Error::other(format!(
                                "source {label} missing referenced object {object_hash}"
                            ))),
                        });
                    }
                    Err(err) => {
                        return Err(DiffError::StoreAccessFailed {
                            source: Box::new(std::io::Error::other(format!(
                                "source {label} store access for {object_hash}: {err}"
                            ))),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Source session A.
    pub fn source_a(&self) -> &SourceSession<S, G> {
        &self.source_a
    }

    /// Source session B.
    pub fn source_b(&self) -> &SourceSession<S, G> {
        &self.source_b
    }

    /// Effective diff mode.
    pub fn mode(&self) -> DiffMode {
        self.mode
    }

    /// Runs diff comparison and returns the intermediate comparison artifact.
    ///
    /// Lockstep structural walk stops at the first structural break. When event
    /// kinds match at a position, field-level comparison runs for that pair.
    pub fn run(&self) -> Result<DiffComparison, DiffError> {
        self.run_inner(None)
    }

    /// Like [`Self::run`], but loads object bytes for hash-backed field divergences (opt-in resolve).
    pub fn run_with_resolve(&self) -> Result<DiffComparison, DiffError> {
        let ctx = ResolveContext {
            store_a: self.source_a.store().as_ref(),
            store_b: self.source_b.store().as_ref(),
        };
        self.run_inner(Some(ctx))
    }

    /// Runs comparison and builds a [`DiffReportV1`] with per-session event counts from source histories.
    pub fn run_to_report(&self) -> Result<DiffReportV1, DiffError> {
        let comparison = self.run()?;
        Ok(DiffReportV1::from_comparison(
            comparison,
            self.source_a.history().len(),
            self.source_b.history().len(),
        ))
    }

    /// Like [`Self::run_to_report`], using [`Self::run_with_resolve`] for comparison.
    pub fn run_with_resolve_to_report(&self) -> Result<DiffReportV1, DiffError> {
        let comparison = self.run_with_resolve()?;
        Ok(DiffReportV1::from_comparison(
            comparison,
            self.source_a.history().len(),
            self.source_b.history().len(),
        ))
    }

    fn run_inner(&self, resolve: Option<ResolveContext<'_>>) -> Result<DiffComparison, DiffError> {
        let mut comparison = DiffComparison::new(
            self.source_a.session_id().to_string(),
            self.source_b.session_id().to_string(),
            self.mode,
        );
        let history_a = self.source_a.history();
        let history_b = self.source_b.history();
        let max_len = history_a.len().max(history_b.len());
        for position in 0..max_len {
            match (history_a.get(position), history_b.get(position)) {
                (Some((_, event_a)), Some((_, event_b))) => {
                    if !same_event_kind_variant(&event_a.kind, &event_b.kind) {
                        let expected = kind_label(&event_a.kind).to_owned();
                        let actual = kind_label(&event_b.kind).to_owned();
                        comparison.structural_break = Some(crate::StructuralBreak {
                            position: position as u64,
                            expected: expected.clone(),
                            actual: actual.clone(),
                        });
                        comparison.divergences.push(crate::DiffDivergence {
                            position: Some(position as u64),
                            kind: crate::DiffDivergenceKind::EventKindMismatchAtPosition,
                            field: None,
                            expected,
                            actual,
                            resolved: None,
                            resolved_skip_reason: None,
                        });
                        break;
                    }
                    comparison
                        .divergences
                        .extend(field_differences_at_position(event_a, event_b, resolve)?);
                    comparison.events_compared = position + 1;
                }
                (Some(_), None) | (None, Some(_)) => {
                    let expected = history_a.len().to_string();
                    let actual = history_b.len().to_string();
                    comparison.structural_break = Some(crate::StructuralBreak {
                        position: position as u64,
                        expected: format!("len={expected}"),
                        actual: format!("len={actual}"),
                    });
                    comparison.divergences.push(crate::DiffDivergence {
                        position: Some(position as u64),
                        kind: crate::DiffDivergenceKind::EventCountMismatch,
                        field: None,
                        expected,
                        actual,
                        resolved: None,
                        resolved_skip_reason: None,
                    });
                    break;
                }
                (None, None) => unreachable!(),
            }
        }
        Ok(comparison)
    }
}

fn field_differences_at_position(
    event_a: &Event,
    event_b: &Event,
    resolve: Option<ResolveContext<'_>>,
) -> Result<Vec<crate::DiffDivergence>, DiffError> {
    match (&event_a.kind, &event_b.kind) {
        (EventKind::SessionStart { .. }, EventKind::SessionStart { .. }) => {
            crate::compare_session_start(event_a, event_b, resolve)
        }
        (EventKind::UserTurn { .. }, EventKind::UserTurn { .. }) => {
            crate::compare_user_turn(event_a, event_b, resolve)
        }
        (EventKind::ProviderCall { .. }, EventKind::ProviderCall { .. }) => {
            crate::compare_provider_call(event_a, event_b, resolve)
        }
        (EventKind::ToolCall { .. }, EventKind::ToolCall { .. }) => {
            crate::compare_tool_call(event_a, event_b, resolve)
        }
        (EventKind::RetrievalCall { .. }, EventKind::RetrievalCall { .. }) => {
            crate::compare_retrieval_call(event_a, event_b, resolve)
        }
        (EventKind::PermissionGate { .. }, EventKind::PermissionGate { .. }) => {
            crate::compare_permission_gate(event_a, event_b, resolve)
        }
        (EventKind::AssistantTurn { .. }, EventKind::AssistantTurn { .. }) => {
            crate::compare_assistant_turn(event_a, event_b, resolve)
        }
        (EventKind::SessionEnd { .. }, EventKind::SessionEnd { .. }) => {
            crate::compare_session_end(event_a, event_b, resolve)
        }
        _ => Ok(Vec::new()),
    }
}

fn same_event_kind_variant(a: &EventKind, b: &EventKind) -> bool {
    matches!(
        (a, b),
        (
            EventKind::SessionStart { .. },
            EventKind::SessionStart { .. }
        ) | (EventKind::UserTurn { .. }, EventKind::UserTurn { .. })
            | (
                EventKind::ProviderCall { .. },
                EventKind::ProviderCall { .. }
            )
            | (EventKind::ToolCall { .. }, EventKind::ToolCall { .. })
            | (
                EventKind::RetrievalCall { .. },
                EventKind::RetrievalCall { .. }
            )
            | (
                EventKind::PermissionGate { .. },
                EventKind::PermissionGate { .. }
            )
            | (
                EventKind::AssistantTurn { .. },
                EventKind::AssistantTurn { .. }
            )
            | (EventKind::SessionEnd { .. }, EventKind::SessionEnd { .. })
    )
}

fn kind_label(kind: &EventKind) -> &'static str {
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

#[cfg(test)]
mod tests {
    use akmon_journal::{
        AttemptRecord, AttemptStatus, EventKind, HashAlgorithm, MemoryObjectStore,
        MemorySessionGraph, ObjectStore, SessionGraph,
    };
    use time::OffsetDateTime;

    use crate::DiffDivergenceKind;

    use super::*;

    fn setup_source_with_history(
        store: Arc<MemoryObjectStore>,
        kinds: &[EventKind],
    ) -> SourceSession<MemoryObjectStore, MemorySessionGraph> {
        let session_id = Uuid::new_v4();
        let mut graph = MemorySessionGraph::open_new(Arc::clone(&store), session_id);
        for kind in kinds {
            graph.append(kind.clone()).expect("append");
        }
        let history = graph.history().expect("history");
        SourceSession::new(session_id, store, Arc::new(Mutex::new(graph)), history)
    }

    fn minimal_valid_source() -> SourceSession<MemoryObjectStore, MemorySessionGraph> {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_hash = store.put(b"/tmp").expect("cwd put");
        let config_hash = store.put(b"config").expect("config put");
        let prompt_hash = store.put(b"hello").expect("prompt put");
        let summary_hash = store.put(b"done").expect("summary put");
        setup_source_with_history(
            Arc::clone(&store),
            &[
                EventKind::SessionStart {
                    cwd_hash,
                    config_hash,
                },
                EventKind::UserTurn { prompt_hash },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_hash),
                },
            ],
        )
    }

    fn provider_call(store: &MemoryObjectStore) -> EventKind {
        let request_hash = store.put(b"provider-request").expect("request put");
        let response_hash = store.put(b"provider-response").expect("response put");
        EventKind::ProviderCall {
            provider_id: "anthropic".to_owned(),
            attempts: vec![AttemptRecord {
                attempt_number: 1,
                started_at: OffsetDateTime::UNIX_EPOCH,
                ended_at: OffsetDateTime::UNIX_EPOCH,
                status: AttemptStatus::Success,
                request_hash,
                response_hash: Some(response_hash),
                stream_hash: None,
                error_message: None,
            }],
            stream_hash: None,
        }
    }

    fn assistant_turn(store: &MemoryObjectStore) -> EventKind {
        let message_hash = store.put(b"assistant").expect("assistant put");
        EventKind::AssistantTurn {
            message_hash,
            tool_calls_hash: None,
        }
    }

    fn permission_gate(store: &MemoryObjectStore) -> EventKind {
        let context_hash = store.put(b"permission-context").expect("context put");
        EventKind::PermissionGate {
            policy_id: "policy".to_owned(),
            decision: "allowed".to_owned(),
            context_hash,
        }
    }

    fn permission_gate_decision(store: &MemoryObjectStore, decision: &str) -> EventKind {
        let context_hash = store.put(b"permission-context").expect("context put");
        EventKind::PermissionGate {
            policy_id: "policy".to_owned(),
            decision: decision.to_owned(),
            context_hash,
        }
    }

    fn tool_call_event(store: &MemoryObjectStore, input: &[u8], output: &[u8]) -> EventKind {
        EventKind::ToolCall {
            tool_id: "tool".to_owned(),
            input_hash: store.put(input).expect("input"),
            output_hash: store.put(output).expect("output"),
            side_effects_hash: None,
        }
    }

    fn retrieval_call(store: &MemoryObjectStore, query: &[u8], results: &[u8]) -> EventKind {
        EventKind::RetrievalCall {
            index_id: "index".to_owned(),
            query_hash: store.put(query).expect("query"),
            results_hash: store.put(results).expect("results"),
        }
    }

    fn provider_call_with_response(store: &MemoryObjectStore, response: &[u8]) -> EventKind {
        let request_hash = store.put(b"provider-request").expect("request put");
        let response_hash = store.put(response).expect("response put");
        EventKind::ProviderCall {
            provider_id: "anthropic".to_owned(),
            attempts: vec![AttemptRecord {
                attempt_number: 1,
                started_at: OffsetDateTime::UNIX_EPOCH,
                ended_at: OffsetDateTime::UNIX_EPOCH,
                status: AttemptStatus::Success,
                request_hash,
                response_hash: Some(response_hash),
                stream_hash: None,
                error_message: None,
            }],
            stream_hash: None,
        }
    }

    #[test]
    fn t_preconditions_reject_empty_source_a() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let graph = Arc::new(Mutex::new(MemorySessionGraph::open_new(
            Arc::clone(&store),
            Uuid::new_v4(),
        )));
        let empty_a = SourceSession::new(Uuid::new_v4(), store, graph, Vec::new());
        let valid_b = minimal_valid_source();
        let err = match DiffEngine::new(empty_a, valid_b) {
            Ok(_) => panic!("must fail"),
            Err(err) => err,
        };
        assert!(matches!(err, DiffError::SourcePreconditionViolated { .. }));
        assert!(
            err.to_string()
                .contains("source A precondition violated: history is empty")
        );
    }

    #[test]
    fn t_preconditions_reject_empty_source_b() {
        let valid_a = minimal_valid_source();
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let graph = Arc::new(Mutex::new(MemorySessionGraph::open_new(
            Arc::clone(&store),
            Uuid::new_v4(),
        )));
        let empty_b = SourceSession::new(Uuid::new_v4(), store, graph, Vec::new());
        let err = match DiffEngine::new(valid_a, empty_b) {
            Ok(_) => panic!("must fail"),
            Err(err) => err,
        };
        assert!(matches!(err, DiffError::SourcePreconditionViolated { .. }));
        assert!(
            err.to_string()
                .contains("source B precondition violated: history is empty")
        );
    }

    #[test]
    fn t_preconditions_reject_non_terminal_session_end_source_a() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_hash = store.put(b"/tmp").expect("cwd put");
        let config_hash = store.put(b"config").expect("config put");
        let prompt_hash = store.put(b"hello").expect("prompt put");
        let source_a = setup_source_with_history(
            Arc::clone(&store),
            &[
                EventKind::SessionStart {
                    cwd_hash,
                    config_hash,
                },
                EventKind::SessionEnd { summary_hash: None },
                EventKind::UserTurn { prompt_hash },
            ],
        );
        let valid_b = minimal_valid_source();
        let err = match DiffEngine::new(source_a, valid_b) {
            Ok(_) => panic!("must fail"),
            Err(err) => err,
        };
        assert!(matches!(err, DiffError::SourcePreconditionViolated { .. }));
        assert!(
            err.to_string()
                .contains("source A precondition violated: SessionEnd must be terminal")
        );
    }

    #[test]
    fn t_preconditions_reject_non_terminal_session_end_source_b() {
        let valid_a = minimal_valid_source();
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_hash = store.put(b"/tmp").expect("cwd put");
        let config_hash = store.put(b"config").expect("config put");
        let prompt_hash = store.put(b"hello").expect("prompt put");
        let source_b = setup_source_with_history(
            Arc::clone(&store),
            &[
                EventKind::SessionStart {
                    cwd_hash,
                    config_hash,
                },
                EventKind::SessionEnd { summary_hash: None },
                EventKind::UserTurn { prompt_hash },
            ],
        );
        let err = match DiffEngine::new(valid_a, source_b) {
            Ok(_) => panic!("must fail"),
            Err(err) => err,
        };
        assert!(matches!(err, DiffError::SourcePreconditionViolated { .. }));
        assert!(
            err.to_string()
                .contains("source B precondition violated: SessionEnd must be terminal")
        );
    }

    #[test]
    fn t_preconditions_reject_missing_referenced_object() {
        let source_a = minimal_valid_source();
        let source_b = minimal_valid_source();
        let prompt_hash = match &source_a.history()[1].1.kind {
            EventKind::UserTurn { prompt_hash } => prompt_hash.clone(),
            _ => panic!("unexpected event kind"),
        };
        source_a
            .store()
            .remove_object_for_testing(&prompt_hash)
            .expect("remove");
        let err = match DiffEngine::new(source_a, source_b) {
            Ok(_) => panic!("must fail"),
            Err(err) => err,
        };
        assert!(matches!(err, DiffError::StoreAccessFailed { .. }));
        assert!(err.to_string().contains("missing referenced object"));
    }

    #[test]
    fn t_new_succeeds_with_two_valid_source_sessions() {
        let source_a = minimal_valid_source();
        let source_b = minimal_valid_source();
        let engine = DiffEngine::new(source_a, source_b).expect("valid sources");
        assert_eq!(engine.mode(), DiffMode::Default);
        assert!(!engine.source_a().history().is_empty());
        assert!(!engine.source_b().history().is_empty());
    }

    #[test]
    fn t_run_identical_histories_no_divergences() {
        let source_a = minimal_valid_source();
        let source_b = minimal_valid_source();
        let engine = DiffEngine::new(source_a, source_b).expect("valid sources");
        let out = engine.run().expect("run succeeds");
        assert!(out.events_compared > 0);
        assert!(out.divergences.is_empty());
        assert!(out.structural_break.is_none());
        assert_eq!(out.mode, DiffMode::Default);
    }

    #[test]
    fn t_run_returns_correct_session_ids() {
        let source_a = minimal_valid_source();
        let source_b = minimal_valid_source();
        let expected_a = source_a.session_id().to_string();
        let expected_b = source_b.session_id().to_string();
        let engine = DiffEngine::new(source_a, source_b).expect("valid sources");
        let out = engine.run().expect("run succeeds");
        assert_eq!(out.session_a_id, expected_a);
        assert_eq!(out.session_b_id, expected_b);
    }

    #[test]
    fn t_run_records_events_compared() {
        let source_a = minimal_valid_source();
        let source_b = minimal_valid_source();
        let expected_n = source_a.history().len().min(source_b.history().len());
        let engine = DiffEngine::new(source_a, source_b).expect("valid sources");
        let out = engine.run().expect("run succeeds");
        assert_eq!(out.events_compared, expected_n);
    }

    #[test]
    fn t_run_detects_event_count_mismatch_a_longer() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/tmp").expect("cwd");
        let cfg_a = store_a.put(b"cfg").expect("cfg");
        let prompt_a = store_a.put(b"prompt").expect("prompt");
        let summary_a = store_a.put(b"summary").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: cfg_a,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_a,
                },
                assistant_turn(store_a.as_ref()),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/tmp").expect("cwd");
        let cfg_b = store_b.put(b"cfg").expect("cfg");
        let prompt_b = store_b.put(b"prompt").expect("prompt");
        let summary_b = store_b.put(b"summary").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: cfg_b,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_b,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run()
            .expect("run");
        assert_eq!(out.events_compared, 2);
        assert_eq!(out.structural_break.as_ref().map(|b| b.position), Some(2));
        assert_eq!(out.divergences.len(), 1);
        assert_eq!(
            out.divergences[0].kind,
            DiffDivergenceKind::EventKindMismatchAtPosition
        );
    }

    #[test]
    fn t_run_detects_event_kind_mismatch() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/tmp").expect("cwd");
        let cfg_a = store_a.put(b"cfg").expect("cfg");
        let summary_a = store_a.put(b"summary").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: cfg_a,
                },
                assistant_turn(store_a.as_ref()),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/tmp").expect("cwd");
        let cfg_b = store_b.put(b"cfg").expect("cfg");
        let summary_b = store_b.put(b"summary").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: cfg_b,
                },
                provider_call(store_b.as_ref()),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run()
            .expect("run");
        assert_eq!(out.events_compared, 1);
        assert_eq!(out.structural_break.as_ref().map(|b| b.position), Some(1));
        assert_eq!(out.divergences.len(), 1);
        assert_eq!(
            out.divergences[0].kind,
            DiffDivergenceKind::EventKindMismatchAtPosition
        );
        assert_eq!(out.divergences[0].expected, "AssistantTurn");
        assert_eq!(out.divergences[0].actual, "ProviderCall");
    }

    #[test]
    fn t_run_stops_at_first_structural_break() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/tmp").expect("cwd");
        let cfg_a = store_a.put(b"cfg").expect("cfg");
        let summary_a = store_a.put(b"summary").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: cfg_a,
                },
                assistant_turn(store_a.as_ref()),
                permission_gate(store_a.as_ref()),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/tmp").expect("cwd");
        let cfg_b = store_b.put(b"cfg").expect("cfg");
        let summary_b = store_b.put(b"summary").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: cfg_b,
                },
                provider_call(store_b.as_ref()),
                EventKind::UserTurn {
                    prompt_hash: store_b.put(b"later-diff").expect("prompt"),
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run()
            .expect("run");
        assert_eq!(out.events_compared, 1);
        assert_eq!(out.structural_break.as_ref().map(|b| b.position), Some(1));
        assert_eq!(out.divergences.len(), 1);
    }

    #[test]
    fn t_run_events_compared_reflects_walker_progress() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/tmp").expect("cwd");
        let cfg_a = store_a.put(b"cfg").expect("cfg");
        let summary_a = store_a.put(b"summary").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: cfg_a,
                },
                EventKind::UserTurn {
                    prompt_hash: store_a.put(b"prompt-a").expect("prompt"),
                },
                assistant_turn(store_a.as_ref()),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/tmp").expect("cwd");
        let cfg_b = store_b.put(b"cfg").expect("cfg");
        let summary_b = store_b.put(b"summary").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: cfg_b,
                },
                EventKind::UserTurn {
                    prompt_hash: store_b.put(b"prompt-b").expect("prompt"),
                },
                provider_call(store_b.as_ref()),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );
        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run()
            .expect("run");
        assert_eq!(out.events_compared, 2);
    }

    #[test]
    fn t_run_kind_labels_stable_across_calls() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/tmp").expect("cwd");
        let cfg_a = store_a.put(b"cfg").expect("cfg");
        let summary_a = store_a.put(b"summary").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: cfg_a,
                },
                assistant_turn(store_a.as_ref()),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/tmp").expect("cwd");
        let cfg_b = store_b.put(b"cfg").expect("cfg");
        let summary_b = store_b.put(b"summary").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: cfg_b,
                },
                provider_call(store_b.as_ref()),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let engine = DiffEngine::new(source_a, source_b).expect("valid");
        let first = engine.run().expect("first");
        let second = engine.run().expect("second");
        assert_eq!(first, second);
    }

    #[test]
    fn t_run_field_divergence_session_start_cwd() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/a").expect("cwd");
        let config_a = store_a.put(b"cfg").expect("cfg");
        let prompt_a = store_a.put(b"hello").expect("prompt");
        let summary_a = store_a.put(b"done").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: config_a,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_a,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/b").expect("cwd");
        let config_b = store_b.put(b"cfg").expect("cfg");
        let prompt_b = store_b.put(b"hello").expect("prompt");
        let summary_b = store_b.put(b"done").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: config_b,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_b,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run()
            .expect("run");
        assert_eq!(out.events_compared, 3);
        assert!(out.structural_break.is_none());
        assert_eq!(out.divergences.len(), 1);
        assert_eq!(
            out.divergences[0].kind,
            DiffDivergenceKind::SessionStartCwdDifference
        );
    }

    #[test]
    fn t_run_field_divergence_user_turn_prompt() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/tmp").expect("cwd");
        let config_a = store_a.put(b"cfg").expect("cfg");
        let prompt_a = store_a.put(b"prompt-a").expect("prompt");
        let summary_a = store_a.put(b"done").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: config_a,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_a,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/tmp").expect("cwd");
        let config_b = store_b.put(b"cfg").expect("cfg");
        let prompt_b = store_b.put(b"prompt-b").expect("prompt");
        let summary_b = store_b.put(b"done").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: config_b,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_b,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run()
            .expect("run");
        assert_eq!(out.events_compared, 3);
        assert!(out.structural_break.is_none());
        assert_eq!(out.divergences.len(), 1);
        assert_eq!(
            out.divergences[0].kind,
            DiffDivergenceKind::ContentReferenceDifference
        );
        assert_eq!(out.divergences[0].field.as_deref(), Some("prompt_hash"));
    }

    #[test]
    fn t_run_field_divergence_provider_call_response() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/tmp").expect("cwd");
        let config_a = store_a.put(b"cfg").expect("cfg");
        let summary_a = store_a.put(b"done").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: config_a,
                },
                provider_call_with_response(store_a.as_ref(), b"resp-a"),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/tmp").expect("cwd");
        let config_b = store_b.put(b"cfg").expect("cfg");
        let summary_b = store_b.put(b"done").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: config_b,
                },
                provider_call_with_response(store_b.as_ref(), b"resp-b"),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run()
            .expect("run");
        assert_eq!(out.events_compared, 3);
        assert!(out.structural_break.is_none());
        assert_eq!(out.divergences.len(), 1);
        assert_eq!(
            out.divergences[0].kind,
            DiffDivergenceKind::ProviderCallResponseDifference
        );
    }

    #[test]
    fn t_run_field_divergence_tool_call_input_output() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/tmp").expect("cwd");
        let config_a = store_a.put(b"cfg").expect("cfg");
        let summary_a = store_a.put(b"done").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: config_a,
                },
                tool_call_event(store_a.as_ref(), b"in-a", b"out-shared"),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/tmp").expect("cwd");
        let config_b = store_b.put(b"cfg").expect("cfg");
        let summary_b = store_b.put(b"done").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: config_b,
                },
                tool_call_event(store_b.as_ref(), b"in-b", b"out-b"),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run()
            .expect("run");
        assert_eq!(out.events_compared, 3);
        assert!(out.structural_break.is_none());
        assert_eq!(out.divergences.len(), 2);
        assert_eq!(
            out.divergences[0].kind,
            DiffDivergenceKind::ToolCallInputDifference
        );
        assert_eq!(
            out.divergences[1].kind,
            DiffDivergenceKind::ToolCallOutputDifference
        );
    }

    #[test]
    fn t_run_field_divergence_assistant_message() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/tmp").expect("cwd");
        let config_a = store_a.put(b"cfg").expect("cfg");
        let summary_a = store_a.put(b"done").expect("summary");
        let msg_a = store_a.put(b"msg-a").expect("msg");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: config_a,
                },
                EventKind::AssistantTurn {
                    message_hash: msg_a,
                    tool_calls_hash: None,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/tmp").expect("cwd");
        let config_b = store_b.put(b"cfg").expect("cfg");
        let summary_b = store_b.put(b"done").expect("summary");
        let msg_b = store_b.put(b"msg-b").expect("msg");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: config_b,
                },
                EventKind::AssistantTurn {
                    message_hash: msg_b,
                    tool_calls_hash: None,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run()
            .expect("run");
        assert_eq!(out.events_compared, 3);
        assert!(out.structural_break.is_none());
        assert_eq!(out.divergences.len(), 1);
        assert_eq!(
            out.divergences[0].kind,
            DiffDivergenceKind::AssistantContentDifference
        );
    }

    #[test]
    fn t_run_field_divergence_permission_decision() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/tmp").expect("cwd");
        let config_a = store_a.put(b"cfg").expect("cfg");
        let summary_a = store_a.put(b"done").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: config_a,
                },
                permission_gate_decision(store_a.as_ref(), "allowed"),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/tmp").expect("cwd");
        let config_b = store_b.put(b"cfg").expect("cfg");
        let summary_b = store_b.put(b"done").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: config_b,
                },
                permission_gate_decision(store_b.as_ref(), "denied"),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run()
            .expect("run");
        assert_eq!(out.events_compared, 3);
        assert!(out.structural_break.is_none());
        assert_eq!(out.divergences.len(), 1);
        assert_eq!(
            out.divergences[0].kind,
            DiffDivergenceKind::PermissionGateDecisionDifference
        );
    }

    #[test]
    fn t_run_field_divergence_session_end_summary() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/tmp").expect("cwd");
        let config_a = store_a.put(b"cfg").expect("cfg");
        let prompt_a = store_a.put(b"hello").expect("prompt");
        let summary_a = store_a.put(b"summary-a").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: config_a,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_a,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/tmp").expect("cwd");
        let config_b = store_b.put(b"cfg").expect("cfg");
        let prompt_b = store_b.put(b"hello").expect("prompt");
        let summary_b = store_b.put(b"summary-b").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: config_b,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_b,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run()
            .expect("run");
        assert_eq!(out.events_compared, 3);
        assert!(out.structural_break.is_none());
        assert_eq!(out.divergences.len(), 1);
        assert_eq!(
            out.divergences[0].kind,
            DiffDivergenceKind::SessionEndDifference
        );
    }

    #[test]
    fn t_run_field_divergence_retrieval_call() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/tmp").expect("cwd");
        let config_a = store_a.put(b"cfg").expect("cfg");
        let summary_a = store_a.put(b"done").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: config_a,
                },
                retrieval_call(store_a.as_ref(), b"q-a", b"results"),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/tmp").expect("cwd");
        let config_b = store_b.put(b"cfg").expect("cfg");
        let summary_b = store_b.put(b"done").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: config_b,
                },
                retrieval_call(store_b.as_ref(), b"q-b", b"results"),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run()
            .expect("run");
        assert_eq!(out.events_compared, 3);
        assert!(out.structural_break.is_none());
        assert_eq!(out.divergences.len(), 1);
        assert_eq!(
            out.divergences[0].kind,
            DiffDivergenceKind::ContentReferenceDifference
        );
        assert_eq!(out.divergences[0].field.as_deref(), Some("query_hash"));
    }

    #[test]
    fn t_run_field_divergences_accumulate_across_positions() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cfg_a = store_a.put(b"cfg").expect("cfg");
        let cwd_a = store_a.put(b"/a").expect("cwd");
        let prompt_a = store_a.put(b"p1").expect("p");
        let summary_a = store_a.put(b"done").expect("s");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: cfg_a,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_a,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cfg_b = store_b.put(b"cfg").expect("cfg");
        let cwd_b = store_b.put(b"/b").expect("cwd");
        let prompt_b = store_b.put(b"p2").expect("p");
        let summary_b = store_b.put(b"done").expect("s");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: cfg_b,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_b,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run()
            .expect("run");
        assert_eq!(out.divergences.len(), 2);
        assert_eq!(
            out.divergences[0].kind,
            DiffDivergenceKind::SessionStartCwdDifference
        );
        assert_eq!(
            out.divergences[1].kind,
            DiffDivergenceKind::ContentReferenceDifference
        );
        assert_eq!(out.divergences[1].field.as_deref(), Some("prompt_hash"));
        assert!(out.structural_break.is_none());
    }

    #[test]
    fn t_run_structural_break_after_field_divergences() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cfg_a = store_a.put(b"cfg").expect("cfg");
        let cwd_a = store_a.put(b"/a").expect("cwd");
        let prompt_a = store_a.put(b"hello").expect("p");
        let summary_a = store_a.put(b"done").expect("s");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: cfg_a,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_a,
                },
                assistant_turn(store_a.as_ref()),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cfg_b = store_b.put(b"cfg").expect("cfg");
        let cwd_b = store_b.put(b"/b").expect("cwd");
        let prompt_b = store_b.put(b"hello").expect("p");
        let summary_b = store_b.put(b"done").expect("s");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: cfg_b,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_b,
                },
                provider_call(store_b.as_ref()),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run()
            .expect("run");
        assert_eq!(out.events_compared, 2);
        assert_eq!(out.structural_break.as_ref().map(|b| b.position), Some(2));
        assert_eq!(out.divergences.len(), 2);
        assert_eq!(
            out.divergences[0].kind,
            DiffDivergenceKind::SessionStartCwdDifference
        );
        assert_eq!(
            out.divergences[1].kind,
            DiffDivergenceKind::EventKindMismatchAtPosition
        );
    }

    #[test]
    fn t_run_field_divergences_dont_affect_events_compared() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cfg_a = store_a.put(b"cfg").expect("cfg");
        let cwd_a = store_a.put(b"/a").expect("cwd");
        let prompt_a = store_a.put(b"p1").expect("p");
        let summary_a = store_a.put(b"done").expect("s");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: cfg_a,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_a,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cfg_b = store_b.put(b"cfg").expect("cfg");
        let cwd_b = store_b.put(b"/b").expect("cwd");
        let prompt_b = store_b.put(b"p2").expect("p");
        let summary_b = store_b.put(b"done").expect("s");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: cfg_b,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_b,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run()
            .expect("run");
        assert_eq!(out.events_compared, 3);
        assert!(!out.divergences.is_empty());
        assert!(out.structural_break.is_none());
    }

    #[test]
    fn t_run_resolved_matches_run_when_identical_sessions() {
        let source_a = minimal_valid_source();
        let source_b = minimal_valid_source();
        let engine = DiffEngine::new(source_a, source_b).expect("valid");
        let plain = engine.run().expect("run");
        let resolved = engine.run_with_resolve().expect("resolved");
        assert_eq!(plain, resolved);
    }

    #[test]
    fn t_run_resolved_populates_resolved_on_field_divergence() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/a").expect("cwd");
        let config_a = store_a.put(b"cfg").expect("cfg");
        let prompt_a = store_a.put(b"hello").expect("prompt");
        let summary_a = store_a.put(b"done").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: config_a,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_a,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/b").expect("cwd");
        let config_b = store_b.put(b"cfg").expect("cfg");
        let prompt_b = store_b.put(b"hello").expect("prompt");
        let summary_b = store_b.put(b"done").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: config_b,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_b,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run_with_resolve()
            .expect("run");
        assert_eq!(out.divergences.len(), 1);
        assert!(out.divergences[0].resolved.is_some());
        assert!(out.divergences[0].resolved_skip_reason.is_none());
    }

    #[test]
    fn t_run_resolved_no_divergences_leaves_resolved_none() {
        let source_a = minimal_valid_source();
        let source_b = minimal_valid_source();
        let out = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run_with_resolve()
            .expect("run");
        assert!(out.divergences.is_empty());
    }

    #[test]
    fn t_run_to_report_identical_sessions() {
        let source_a = minimal_valid_source();
        let source_b = minimal_valid_source();
        let n = source_a.history().len();
        let engine = DiffEngine::new(source_a, source_b).expect("valid");
        let report = engine.run_to_report().expect("report");
        assert!(report.matches);
        assert!(report.divergences.is_empty());
        assert!(report.structural_break.is_none());
        assert_eq!(report.divergence_count, 0);
        assert_eq!(report.events_compared, n);
    }

    #[test]
    fn t_run_to_report_diverging_sessions() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/tmp").expect("cwd");
        let config_a = store_a.put(b"cfg").expect("cfg");
        let summary_a = store_a.put(b"done").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: config_a,
                },
                permission_gate_decision(store_a.as_ref(), "allowed"),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/tmp").expect("cwd");
        let config_b = store_b.put(b"cfg").expect("cfg");
        let summary_b = store_b.put(b"done").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: config_b,
                },
                permission_gate_decision(store_b.as_ref(), "denied"),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let report = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run_to_report()
            .expect("report");
        assert!(!report.matches);
        assert_eq!(report.divergence_count, report.divergences.len());
        assert_eq!(report.divergences.len(), 1);
        assert!(report.structural_break.is_none());
    }

    #[test]
    fn t_run_to_report_structural_break() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/tmp").expect("cwd");
        let cfg_a = store_a.put(b"cfg").expect("cfg");
        let summary_a = store_a.put(b"summary").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: cfg_a,
                },
                assistant_turn(store_a.as_ref()),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/tmp").expect("cwd");
        let cfg_b = store_b.put(b"cfg").expect("cfg");
        let summary_b = store_b.put(b"summary").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: cfg_b,
                },
                provider_call(store_b.as_ref()),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let report = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run_to_report()
            .expect("report");
        assert!(report.structural_break.is_some());
        assert!(report.divergence_count >= 1);
    }

    #[test]
    fn t_run_with_resolve_to_report_includes_resolved() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/a").expect("cwd");
        let config_a = store_a.put(b"cfg").expect("cfg");
        let prompt_a = store_a.put(b"hello").expect("prompt");
        let summary_a = store_a.put(b"done").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: config_a,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_a,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/b").expect("cwd");
        let config_b = store_b.put(b"cfg").expect("cfg");
        let prompt_b = store_b.put(b"hello").expect("prompt");
        let summary_b = store_b.put(b"done").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: config_b,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_b,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let report = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run_with_resolve_to_report()
            .expect("report");
        assert_eq!(report.divergences.len(), 1);
        assert!(report.divergences[0].resolved.is_some());
    }

    #[test]
    fn t_run_with_resolve_to_report_serializes_resolved_in_json() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/a").expect("cwd");
        let config_a = store_a.put(b"cfg").expect("cfg");
        let prompt_a = store_a.put(b"hello").expect("prompt");
        let summary_a = store_a.put(b"done").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: config_a,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_a,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/b").expect("cwd");
        let config_b = store_b.put(b"cfg").expect("cfg");
        let prompt_b = store_b.put(b"hello").expect("prompt");
        let summary_b = store_b.put(b"done").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: config_b,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_b,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        let report = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run_with_resolve_to_report()
            .expect("report");
        let value: serde_json::Value = serde_json::to_value(&report).expect("serialize");
        let resolved = &value["divergences"][0]["resolved"];
        assert!(!resolved.is_null(), "expected non-null resolved in JSON");
    }

    #[test]
    fn t_run_to_report_session_event_counts_correct() {
        let store_a = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_a = store_a.put(b"/tmp").expect("cwd");
        let cfg_a = store_a.put(b"cfg").expect("cfg");
        let prompt_a = store_a.put(b"hi").expect("prompt");
        let summary_a = store_a.put(b"done").expect("summary");
        let source_a = setup_source_with_history(
            Arc::clone(&store_a),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_a,
                    config_hash: cfg_a,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_a,
                },
                assistant_turn(store_a.as_ref()),
                EventKind::SessionEnd {
                    summary_hash: Some(summary_a),
                },
            ],
        );

        let store_b = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let cwd_b = store_b.put(b"/tmp").expect("cwd");
        let cfg_b = store_b.put(b"cfg").expect("cfg");
        let prompt_b = store_b.put(b"hi").expect("prompt");
        let summary_b = store_b.put(b"done").expect("summary");
        let source_b = setup_source_with_history(
            Arc::clone(&store_b),
            &[
                EventKind::SessionStart {
                    cwd_hash: cwd_b,
                    config_hash: cfg_b,
                },
                EventKind::UserTurn {
                    prompt_hash: prompt_b,
                },
                EventKind::SessionEnd {
                    summary_hash: Some(summary_b),
                },
            ],
        );

        assert_eq!(source_a.history().len(), 4);
        assert_eq!(source_b.history().len(), 3);

        let report = DiffEngine::new(source_a, source_b)
            .expect("valid")
            .run_to_report()
            .expect("report");
        assert_eq!(report.session_a_event_count, 4);
        assert_eq!(report.session_b_event_count, 3);
    }
}
