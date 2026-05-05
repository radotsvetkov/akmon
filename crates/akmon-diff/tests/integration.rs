//! End-to-end diff tests against real on-disk `RedbObjectStore` journals (Item 6.2 layer 7).

use std::path::Path;
use std::sync::{Arc, Mutex};

use akmon_diff::{DiffDivergenceKind, DiffEngine, DiffError, load_source_session_from_journal};
use akmon_journal::{
    AttemptRecord, AttemptStatus, EventKind, Hash, HashAlgorithm, ObjectStore, RedbObjectStore,
    RedbSessionGraph, SessionGraph,
};
use akmon_query::{JournalHandle, journal_db_path};
use tempfile::tempdir;
use time::OffsetDateTime;
use uuid::Uuid;

fn put(store: &RedbObjectStore, bytes: &[u8]) -> Hash {
    store.put(bytes).expect("put object")
}

fn open_writable_journal(
    dir: &Path,
    session_id: Uuid,
) -> JournalHandle<RedbObjectStore, RedbSessionGraph> {
    std::fs::create_dir_all(dir).expect("mkdir journal dir");
    let db_path = journal_db_path(dir);
    let store = Arc::new(
        RedbObjectStore::create(db_path.as_path(), HashAlgorithm::Sha256).expect("create store"),
    );
    let graph = RedbSessionGraph::open_new(Arc::clone(&store), session_id).expect("open graph");
    JournalHandle::new(store, Arc::new(Mutex::new(graph)))
}

/// Builds event kinds in lockstep order; each builder sees the same store the graph appends to.
fn create_test_journal_with_session(
    dir: &Path,
    session_id: Uuid,
    event_builders: &[fn(&RedbObjectStore) -> EventKind],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let journal = open_writable_journal(dir, session_id);
    let store = journal.store.clone();
    let mut graph = journal.graph.lock().unwrap_or_else(|p| p.into_inner());
    for builder in event_builders {
        graph
            .append(builder(store.as_ref()))
            .map_err(|e| format!("append: {e}"))?;
    }
    Ok(())
}

fn event_session_start(store: &RedbObjectStore) -> EventKind {
    EventKind::SessionStart {
        cwd_hash: put(store, b"/workspace"),
        config_hash: put(store, br#"{"model":"x"}"#),
    }
}

fn event_user_turn(store: &RedbObjectStore) -> EventKind {
    EventKind::UserTurn {
        prompt_hash: put(store, b"Explain this change."),
    }
}

fn event_user_turn_prompt(store: &RedbObjectStore, prompt: &[u8]) -> EventKind {
    EventKind::UserTurn {
        prompt_hash: put(store, prompt),
    }
}

fn event_provider_call(store: &RedbObjectStore) -> EventKind {
    EventKind::ProviderCall {
        provider_id: "anthropic".to_owned(),
        attempts: vec![AttemptRecord {
            attempt_number: 1,
            started_at: OffsetDateTime::now_utc(),
            ended_at: OffsetDateTime::now_utc(),
            status: AttemptStatus::Success,
            request_hash: put(store, b"request-1"),
            response_hash: Some(put(store, b"response-1")),
            stream_hash: None,
            error_message: None,
        }],
        stream_hash: None,
    }
}

fn event_assistant_turn(store: &RedbObjectStore) -> EventKind {
    EventKind::AssistantTurn {
        message_hash: put(store, b"Here is the explanation."),
        tool_calls_hash: None,
    }
}

fn event_tool_call(store: &RedbObjectStore) -> EventKind {
    EventKind::ToolCall {
        tool_id: "integration_tool".to_owned(),
        input_hash: put(store, b"tool-in"),
        output_hash: put(store, b"tool-out"),
        side_effects_hash: None,
    }
}

fn event_retrieval_call(store: &RedbObjectStore) -> EventKind {
    EventKind::RetrievalCall {
        index_id: "idx-1".to_owned(),
        query_hash: put(store, b"retrieval-query"),
        results_hash: put(store, b"retrieval-results"),
    }
}

fn event_permission_gate(store: &RedbObjectStore) -> EventKind {
    EventKind::PermissionGate {
        policy_id: "policy-1".to_owned(),
        decision: "allowed".to_owned(),
        context_hash: put(store, b"gate-ctx"),
    }
}

fn event_session_end(store: &RedbObjectStore) -> EventKind {
    EventKind::SessionEnd {
        summary_hash: Some(put(store, b"summary")),
    }
}

/// One representative ordering that exercises every `EventKind` variant on a linear session.
fn full_session_builders() -> &'static [fn(&RedbObjectStore) -> EventKind] {
    &[
        event_session_start,
        event_user_turn,
        event_provider_call,
        event_assistant_turn,
        event_tool_call,
        event_retrieval_call,
        event_permission_gate,
        event_session_end,
    ]
}

fn write_minimal_three_event_session_with_prompt(
    dir: &Path,
    session_id: Uuid,
    prompt: &[u8],
) -> Hash {
    let journal = open_writable_journal(dir, session_id);
    let store = journal.store.clone();
    let mut graph = journal.graph.lock().unwrap_or_else(|p| p.into_inner());
    let cwd = put(store.as_ref(), b"/tmp");
    let cfg = put(store.as_ref(), b"cfg");
    let prompt_hash = put(store.as_ref(), prompt);
    let summary = put(store.as_ref(), b"done");
    graph
        .append(EventKind::SessionStart {
            cwd_hash: cwd,
            config_hash: cfg,
        })
        .expect("SessionStart");
    graph
        .append(EventKind::UserTurn {
            prompt_hash: prompt_hash.clone(),
        })
        .expect("UserTurn");
    graph
        .append(EventKind::SessionEnd {
            summary_hash: Some(summary),
        })
        .expect("SessionEnd");
    prompt_hash
}

fn write_multi_turn_chain(dir: &Path, session_id: Uuid, turns: usize) {
    let journal = open_writable_journal(dir, session_id);
    let store = journal.store.clone();
    let mut graph = journal.graph.lock().unwrap_or_else(|p| p.into_inner());
    graph
        .append(event_session_start(store.as_ref()))
        .expect("SessionStart");
    for i in 0..turns {
        let prompt = format!("prompt turn {i}");
        graph
            .append(event_user_turn_prompt(store.as_ref(), prompt.as_bytes()))
            .expect("UserTurn");
        graph
            .append(event_assistant_turn(store.as_ref()))
            .expect("AssistantTurn");
    }
    graph
        .append(event_session_end(store.as_ref()))
        .expect("SessionEnd");
}

#[test]
fn t_diff_full_session_via_real_journal() {
    let tmp_a = tempdir().expect("tempdir a");
    let tmp_b = tempdir().expect("tempdir b");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    create_test_journal_with_session(tmp_a.path(), sid_a, full_session_builders())
        .expect("journal a");
    create_test_journal_with_session(tmp_b.path(), sid_b, full_session_builders())
        .expect("journal b");

    let source_a = load_source_session_from_journal(tmp_a.path(), sid_a).expect("source a");
    let source_b = load_source_session_from_journal(tmp_b.path(), sid_b).expect("source b");
    let report = DiffEngine::new(source_a, source_b)
        .expect("engine")
        .run_to_report()
        .expect("report");
    assert!(report.matches, "{report:?}");
    assert!(report.divergences.is_empty());
    assert!(report.structural_break.is_none());
}

#[test]
fn t_diff_detects_field_divergence_via_real_journal() {
    let tmp_a = tempdir().expect("tempdir a");
    let tmp_b = tempdir().expect("tempdir b");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    write_minimal_three_event_session_with_prompt(tmp_a.path(), sid_a, b"hello A");
    write_minimal_three_event_session_with_prompt(tmp_b.path(), sid_b, b"hello B");

    let source_a = load_source_session_from_journal(tmp_a.path(), sid_a).expect("source a");
    let source_b = load_source_session_from_journal(tmp_b.path(), sid_b).expect("source b");
    let report = DiffEngine::new(source_a, source_b)
        .expect("engine")
        .run_to_report()
        .expect("report");
    assert!(!report.matches);
    assert_eq!(report.divergences.len(), 1);
    assert_eq!(report.divergence_count, 1);
    assert_eq!(
        report.divergences[0].kind,
        DiffDivergenceKind::ContentReferenceDifference
    );
    assert_eq!(report.divergences[0].field.as_deref(), Some("prompt_hash"));
    assert!(report.structural_break.is_none());
}

#[test]
fn t_diff_detects_structural_break_via_real_journal() {
    let tmp_a = tempdir().expect("tempdir a");
    let tmp_b = tempdir().expect("tempdir b");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let builders_a: &[fn(&RedbObjectStore) -> EventKind] = &[
        event_session_start,
        event_user_turn,
        event_assistant_turn,
        event_session_end,
    ];
    let builders_b: &[fn(&RedbObjectStore) -> EventKind] = &[
        event_session_start,
        event_user_turn,
        event_provider_call,
        event_session_end,
    ];
    create_test_journal_with_session(tmp_a.path(), sid_a, builders_a).expect("journal a");
    create_test_journal_with_session(tmp_b.path(), sid_b, builders_b).expect("journal b");

    let source_a = load_source_session_from_journal(tmp_a.path(), sid_a).expect("source a");
    let source_b = load_source_session_from_journal(tmp_b.path(), sid_b).expect("source b");
    let report = DiffEngine::new(source_a, source_b)
        .expect("engine")
        .run_to_report()
        .expect("report");
    assert!(report.structural_break.is_some());
    assert!(report.divergence_count >= 1);
    assert!(!report.matches);
}

#[test]
fn t_diff_with_resolve_real_journal() {
    let tmp_a = tempdir().expect("tempdir a");
    let tmp_b = tempdir().expect("tempdir b");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let journal_a = open_writable_journal(tmp_a.path(), sid_a);
    let mut graph_a = journal_a.graph.lock().unwrap_or_else(|p| p.into_inner());
    let cwd_a = put(journal_a.store.as_ref(), b"/session-a");
    let cfg_a = put(journal_a.store.as_ref(), b"cfg");
    let prompt_a = put(journal_a.store.as_ref(), b"hello");
    let summary_a = put(journal_a.store.as_ref(), b"done");
    graph_a
        .append(EventKind::SessionStart {
            cwd_hash: cwd_a,
            config_hash: cfg_a,
        })
        .expect("start a");
    graph_a
        .append(EventKind::UserTurn {
            prompt_hash: prompt_a,
        })
        .expect("user a");
    graph_a
        .append(EventKind::SessionEnd {
            summary_hash: Some(summary_a),
        })
        .expect("end a");
    // Drop handle (and release redb lock) before opening the same path read-only via load_*.
    drop(graph_a);
    drop(journal_a);

    let journal_b = open_writable_journal(tmp_b.path(), sid_b);
    let mut graph_b = journal_b.graph.lock().unwrap_or_else(|p| p.into_inner());
    let cwd_b = put(journal_b.store.as_ref(), b"/session-b");
    let cfg_b = put(journal_b.store.as_ref(), b"cfg");
    let prompt_b = put(journal_b.store.as_ref(), b"hello");
    let summary_b = put(journal_b.store.as_ref(), b"done");
    graph_b
        .append(EventKind::SessionStart {
            cwd_hash: cwd_b,
            config_hash: cfg_b,
        })
        .expect("start b");
    graph_b
        .append(EventKind::UserTurn {
            prompt_hash: prompt_b,
        })
        .expect("user b");
    graph_b
        .append(EventKind::SessionEnd {
            summary_hash: Some(summary_b),
        })
        .expect("end b");
    drop(graph_b);
    drop(journal_b);

    let source_a = load_source_session_from_journal(tmp_a.path(), sid_a).expect("source a");
    let source_b = load_source_session_from_journal(tmp_b.path(), sid_b).expect("source b");
    let report = DiffEngine::new(source_a, source_b)
        .expect("engine")
        .run_with_resolve_to_report()
        .expect("report");
    assert_eq!(report.divergences.len(), 1);
    let div = &report.divergences[0];
    assert_eq!(div.kind, DiffDivergenceKind::SessionStartCwdDifference);
    assert_eq!(div.field.as_deref(), Some("cwd_hash"));
    let resolved = div.resolved.as_ref().expect("resolved");
    assert_eq!(resolved.a_size_bytes, b"/session-a".len());
    assert_eq!(resolved.b_size_bytes, b"/session-b".len());
    assert!(resolved.a_preview.as_ref().is_some_and(|s| !s.is_empty()));
    assert!(resolved.b_preview.as_ref().is_some_and(|s| !s.is_empty()));
}

#[test]
fn t_diff_handles_multi_turn_session() {
    let tmp_a = tempdir().expect("tempdir a");
    let tmp_b = tempdir().expect("tempdir b");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let turns = 4usize;
    write_multi_turn_chain(tmp_a.path(), sid_a, turns);
    write_multi_turn_chain(tmp_b.path(), sid_b, turns);

    let source_a = load_source_session_from_journal(tmp_a.path(), sid_a).expect("source a");
    let source_b = load_source_session_from_journal(tmp_b.path(), sid_b).expect("source b");
    let expected_len = 1 + turns * 2 + 1;
    assert_eq!(source_a.history().len(), expected_len);
    let report = DiffEngine::new(source_a, source_b)
        .expect("engine")
        .run_to_report()
        .expect("report");
    assert!(report.matches, "{report:?}");
    assert!(report.divergences.is_empty());
    assert_eq!(report.events_compared, expected_len);
}

#[test]
fn t_diff_completes_with_more_than_32_events() {
    let tmp_a = tempdir().expect("tempdir a");
    let tmp_b = tempdir().expect("tempdir b");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let turns = 17usize;
    write_multi_turn_chain(tmp_a.path(), sid_a, turns);
    write_multi_turn_chain(tmp_b.path(), sid_b, turns);

    let source_a = load_source_session_from_journal(tmp_a.path(), sid_a).expect("source a");
    let source_b = load_source_session_from_journal(tmp_b.path(), sid_b).expect("source b");
    let history_len = source_a.history().len();
    assert!(history_len > 32);
    let report = DiffEngine::new(source_a, source_b)
        .expect("engine")
        .run_to_report()
        .expect("report");
    assert!(report.matches, "{report:?}");
    assert_eq!(report.events_compared, history_len);
}

#[test]
fn t_diff_handles_missing_source_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    write_minimal_three_event_session_with_prompt(tmp.path(), sid, b"only");

    let missing = Uuid::new_v4();
    let err = match load_source_session_from_journal(tmp.path(), missing) {
        Ok(_) => panic!("expected SourceSessionMissing"),
        Err(e) => e,
    };
    match err {
        DiffError::SourceSessionMissing { session_id } => {
            assert_eq!(session_id, missing.to_string());
        }
        other => panic!("unexpected err: {other:?}"),
    }
}

#[test]
fn t_diff_handles_corrupted_session() {
    let tmp_a = tempdir().expect("tempdir a");
    let tmp_b = tempdir().expect("tempdir b");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let prompt_hash = write_minimal_three_event_session_with_prompt(tmp_a.path(), sid_a, b"x");
    write_minimal_three_event_session_with_prompt(tmp_b.path(), sid_b, b"x");

    {
        let store = RedbObjectStore::open(journal_db_path(tmp_a.path()).as_path()).expect("reopen");
        store
            .remove_object_for_testing(&prompt_hash)
            .expect("remove prompt object");
    }

    let source_a = load_source_session_from_journal(tmp_a.path(), sid_a).expect("source a");
    let source_b = load_source_session_from_journal(tmp_b.path(), sid_b).expect("source b");
    let err = match DiffEngine::new(source_a, source_b) {
        Ok(_) => panic!("expected engine new to fail"),
        Err(e) => e,
    };
    assert!(
        matches!(err, DiffError::StoreAccessFailed { .. }),
        "expected StoreAccessFailed from precondition validation, got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("missing referenced object"),
        "unexpected: {msg}"
    );
}
