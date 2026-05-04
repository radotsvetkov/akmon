use akmon_journal::{
    AttemptRecord, AttemptStatus, EventKind, HashAlgorithm, ObjectStore, RedbObjectStore,
    RedbSessionGraph, SessionGraph,
};
use std::sync::Arc;

fn put_bytes(store: &RedbObjectStore, bytes: &[u8]) -> akmon_journal::Hash {
    store.put(bytes).unwrap_or_else(|_| unreachable!())
}

#[test]
fn full_journal_roundtrip() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
    let path = tmp.path().join("journal.redb");
    let store = Arc::new(
        RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
            .unwrap_or_else(|_| unreachable!()),
    );
    let session_id = uuid::Uuid::new_v4();
    let session_end_hash = {
        let mut graph = RedbSessionGraph::open_new(Arc::clone(&store), session_id)
            .unwrap_or_else(|_| unreachable!());

        graph
            .append(EventKind::SessionStart {
                cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
                config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
            })
            .unwrap_or_else(|_| unreachable!());

        graph
            .append(EventKind::UserTurn {
                prompt_hash: put_bytes(store.as_ref(), b"Explain this change."),
            })
            .unwrap_or_else(|_| unreachable!());

        graph
            .append(EventKind::ProviderCall {
                provider_id: "anthropic".to_owned(),
                attempts: vec![
                    AttemptRecord {
                        attempt_number: 1,
                        started_at: time::OffsetDateTime::now_utc(),
                        ended_at: time::OffsetDateTime::now_utc(),
                        status: AttemptStatus::RateLimited,
                        request_hash: put_bytes(store.as_ref(), b"request-1"),
                        response_hash: None,
                        stream_hash: None,
                        error_message: Some("429".to_owned()),
                    },
                    AttemptRecord {
                        attempt_number: 2,
                        started_at: time::OffsetDateTime::now_utc(),
                        ended_at: time::OffsetDateTime::now_utc(),
                        status: AttemptStatus::Success,
                        request_hash: put_bytes(store.as_ref(), b"request-2"),
                        response_hash: Some(put_bytes(store.as_ref(), b"response-2")),
                        stream_hash: Some(put_bytes(store.as_ref(), b"stream-2")),
                        error_message: None,
                    },
                ],
                stream_hash: Some(put_bytes(store.as_ref(), b"provider-stream")),
            })
            .unwrap_or_else(|_| unreachable!());

        graph
            .append(EventKind::AssistantTurn {
                message_hash: put_bytes(store.as_ref(), b"Here is the explanation."),
                tool_calls_hash: None,
            })
            .unwrap_or_else(|_| unreachable!());

        let end_hash = graph
            .append(EventKind::SessionEnd {
                summary_hash: Some(put_bytes(store.as_ref(), b"summary")),
            })
            .unwrap_or_else(|_| unreachable!());

        let verify = graph.verify().unwrap_or_else(|_| unreachable!());
        assert!(verify.missing_objects.is_empty());
        assert!(verify.object_hash_mismatches.is_empty());
        assert!(verify.hash_mismatches.is_empty());
        assert!(verify.broken_parent_links.is_empty());
        assert!(verify.sequence_violations.is_empty());
        assert!(verify.is_clean());
        end_hash
    };

    drop(store);

    let reopened_store =
        Arc::new(RedbObjectStore::open(path.as_path()).unwrap_or_else(|_| unreachable!()));
    let reopened =
        RedbSessionGraph::reopen(reopened_store, session_id).unwrap_or_else(|_| unreachable!());
    let history = reopened.history().unwrap_or_else(|_| unreachable!());
    assert_eq!(history.len(), 5);
    assert!(matches!(history[0].1.kind, EventKind::SessionStart { .. }));
    assert!(matches!(history[1].1.kind, EventKind::UserTurn { .. }));
    assert!(matches!(history[2].1.kind, EventKind::ProviderCall { .. }));
    assert!(matches!(history[3].1.kind, EventKind::AssistantTurn { .. }));
    assert!(matches!(history[4].1.kind, EventKind::SessionEnd { .. }));
    assert_eq!(
        reopened.head().unwrap_or_else(|_| unreachable!()),
        Some(session_end_hash)
    );
}
