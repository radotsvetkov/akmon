use std::path::Path;
use std::sync::{Arc, Mutex};

use akmon_journal::{
    Event, EventKind, Hash, ObjectStore, RedbObjectStore, RedbSessionGraph, SessionGraph,
    referenced_object_hashes_for_kind,
};
use akmon_query::{journal_contains_session, open_journal_read_only};
use uuid::Uuid;

use crate::{DiffError, DiffMode};

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
}

#[cfg(test)]
mod tests {
    use akmon_journal::{
        EventKind, HashAlgorithm, MemoryObjectStore, MemorySessionGraph, ObjectStore, SessionGraph,
    };

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
}
