use std::path::{Path, PathBuf};
use std::sync::Arc;

use akmon_journal::{
    EventKind, Hash, HashAlgorithm, JournalError, ObjectStore, RedbObjectStore, RedbSessionGraph,
    SessionGraph,
};
use uuid::Uuid;

/// Resolves the `akmon` binary path for CLI integration tests.
///
/// Cargo usually provides `CARGO_BIN_EXE_akmon`; some CI invocations of targeted
/// integration tests may not, so this falls back to `target/<profile>/akmon`.
#[must_use]
pub fn akmon_bin_path() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_akmon") {
        return PathBuf::from(path);
    }
    let exe = std::env::current_exe().expect("current_exe");
    let deps_dir = exe.parent().expect("test binary dir");
    let target_dir = deps_dir.parent().expect("target dir");
    let candidate = target_dir.join(format!("akmon{}", std::env::consts::EXE_SUFFIX));
    if candidate.is_file() {
        return candidate;
    }
    panic!(
        "akmon test binary not found: CARGO_BIN_EXE_akmon unset and fallback path missing at {}",
        candidate.display()
    );
}

/// Session fixture paths and key hashes useful for corruption tests.
pub struct SessionFixture {
    /// Journal database path (`journal.redb`) under test tempdir.
    pub journal_db_path: PathBuf,
    /// User-turn event hash (sequence 1 in standard fixtures).
    pub user_event_hash: Hash,
    /// Prompt object hash referenced by the user-turn event.
    pub prompt_hash: Hash,
}

/// Returns `<dir>/journal.redb`.
#[must_use]
pub fn journal_db_path(dir: &Path) -> PathBuf {
    dir.join("journal.redb")
}

/// Stores bytes and returns content hash.
pub fn put_bytes(store: &RedbObjectStore, bytes: &[u8]) -> Hash {
    store.put(bytes).expect("put bytes")
}

/// Creates a clean session: SessionStart -> UserTurn -> SessionEnd.
pub fn create_clean_session(journal_dir: &Path, session_id: Uuid) -> SessionFixture {
    std::fs::create_dir_all(journal_dir).expect("mkdir journal dir");
    let db_path = journal_db_path(journal_dir);
    let store = Arc::new(
        RedbObjectStore::create(db_path.as_path(), HashAlgorithm::Sha256).expect("create store"),
    );
    let mut graph = RedbSessionGraph::open_new(Arc::clone(&store), session_id).expect("open graph");
    graph
        .append(EventKind::SessionStart {
            cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
            config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
        })
        .expect("append start");
    let user_event_hash = graph
        .append(EventKind::UserTurn {
            prompt_hash: put_bytes(store.as_ref(), b"hello"),
        })
        .expect("append user");
    let prompt_hash = graph
        .history()
        .expect("history")
        .iter()
        .find_map(|(_, ev)| match &ev.kind {
            EventKind::UserTurn { prompt_hash } => Some(prompt_hash.clone()),
            _ => None,
        })
        .expect("prompt hash");
    graph
        .append(EventKind::SessionEnd {
            summary_hash: Some(put_bytes(store.as_ref(), b"summary")),
        })
        .expect("append end");
    SessionFixture {
        journal_db_path: db_path,
        user_event_hash,
        prompt_hash,
    }
}

/// Creates a session missing SessionEnd.
pub fn create_session_missing_end(journal_dir: &Path, session_id: Uuid) -> SessionFixture {
    std::fs::create_dir_all(journal_dir).expect("mkdir journal dir");
    let db_path = journal_db_path(journal_dir);
    let store = Arc::new(
        RedbObjectStore::create(db_path.as_path(), HashAlgorithm::Sha256).expect("create store"),
    );
    let mut graph = RedbSessionGraph::open_new(Arc::clone(&store), session_id).expect("open graph");
    graph
        .append(EventKind::SessionStart {
            cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
            config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
        })
        .expect("append start");
    let user_event_hash = graph
        .append(EventKind::UserTurn {
            prompt_hash: put_bytes(store.as_ref(), b"hello"),
        })
        .expect("append user");
    let prompt_hash = graph
        .history()
        .expect("history")
        .iter()
        .find_map(|(_, ev)| match &ev.kind {
            EventKind::UserTurn { prompt_hash } => Some(prompt_hash.clone()),
            _ => None,
        })
        .expect("prompt hash");
    SessionFixture {
        journal_db_path: db_path,
        user_event_hash,
        prompt_hash,
    }
}

/// Creates a session with duplicate SessionEnd events.
pub fn create_session_duplicate_end(journal_dir: &Path, session_id: Uuid) -> SessionFixture {
    let fixture = create_clean_session(journal_dir, session_id);
    let store =
        Arc::new(RedbObjectStore::open(fixture.journal_db_path.as_path()).expect("reopen store"));
    let mut graph = RedbSessionGraph::reopen(Arc::clone(&store), session_id).expect("reopen graph");
    graph
        .append(EventKind::SessionEnd {
            summary_hash: Some(put_bytes(store.as_ref(), b"summary-2")),
        })
        .expect("append duplicate end");
    fixture
}

/// Creates a session with non-terminal SessionEnd.
pub fn create_session_end_not_terminal(journal_dir: &Path, session_id: Uuid) -> SessionFixture {
    std::fs::create_dir_all(journal_dir).expect("mkdir journal dir");
    let db_path = journal_db_path(journal_dir);
    let store = Arc::new(
        RedbObjectStore::create(db_path.as_path(), HashAlgorithm::Sha256).expect("create store"),
    );
    let mut graph = RedbSessionGraph::open_new(Arc::clone(&store), session_id).expect("open graph");
    graph
        .append(EventKind::SessionStart {
            cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
            config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
        })
        .expect("append start");
    graph
        .append(EventKind::SessionEnd {
            summary_hash: Some(put_bytes(store.as_ref(), b"summary")),
        })
        .expect("append end");
    let user_event_hash = graph
        .append(EventKind::UserTurn {
            prompt_hash: put_bytes(store.as_ref(), b"after-end"),
        })
        .expect("append user after end");
    let prompt_hash = graph
        .history()
        .expect("history")
        .iter()
        .find_map(|(_, ev)| match &ev.kind {
            EventKind::UserTurn { prompt_hash } => Some(prompt_hash.clone()),
            _ => None,
        })
        .expect("prompt hash");
    SessionFixture {
        journal_db_path: db_path,
        user_event_hash,
        prompt_hash,
    }
}

/// Corrupts event bytes at sequence 1 while preserving stored event hash.
pub fn create_session_event_hash_mismatch(journal_dir: &Path, session_id: Uuid) -> SessionFixture {
    let fixture = create_clean_session(journal_dir, session_id);
    let store =
        Arc::new(RedbObjectStore::open(fixture.journal_db_path.as_path()).expect("reopen store"));
    let mut graph = RedbSessionGraph::reopen(Arc::clone(&store), session_id).expect("reopen graph");
    let history = graph.history().expect("history");
    let mut event = history[1].1.clone();
    event.kind = EventKind::AssistantTurn {
        message_hash: fixture.prompt_hash.clone(),
        tool_calls_hash: None,
    };
    graph
        .overwrite_event_at_sequence_for_testing(1, event)
        .expect("overwrite event");
    fixture
}

/// Corrupts parent link at sequence 1 while preserving stored event hash.
pub fn create_session_parent_chain_break(journal_dir: &Path, session_id: Uuid) -> SessionFixture {
    let fixture = create_clean_session(journal_dir, session_id);
    let store =
        Arc::new(RedbObjectStore::open(fixture.journal_db_path.as_path()).expect("reopen store"));
    let mut graph = RedbSessionGraph::reopen(Arc::clone(&store), session_id).expect("reopen graph");
    let history = graph.history().expect("history");
    let mut event = history[1].1.clone();
    event.parents = vec![Hash::from_bytes(HashAlgorithm::Sha256, [0xAA; 32])];
    graph
        .overwrite_event_at_sequence_for_testing(1, event)
        .expect("overwrite event");
    fixture
}

/// Corrupts stored head pointer.
pub fn create_session_head_mismatch(journal_dir: &Path, session_id: Uuid) -> SessionFixture {
    let fixture = create_clean_session(journal_dir, session_id);
    let store =
        Arc::new(RedbObjectStore::open(fixture.journal_db_path.as_path()).expect("reopen store"));
    let mut graph = RedbSessionGraph::reopen(Arc::clone(&store), session_id).expect("reopen graph");
    graph
        .overwrite_head_for_testing(Hash::from_bytes(HashAlgorithm::Sha256, [0xCC; 32]))
        .expect("overwrite head");
    fixture
}

/// Creates one session with both a missing-object and duplicate-SessionEnd violation.
pub fn create_session_multi_violation(journal_dir: &Path, session_id: Uuid) -> SessionFixture {
    let fixture = create_session_duplicate_end(journal_dir, session_id);
    let store =
        Arc::new(RedbObjectStore::open(fixture.journal_db_path.as_path()).expect("reopen store"));
    store
        .remove_object_for_testing(&fixture.prompt_hash)
        .expect("remove prompt object");
    fixture
}

/// Opens a redb-backed journal handle against `journal_dir` and `session_id`.
pub fn open_journal_handle(
    journal_dir: &Path,
    session_id: Uuid,
) -> Result<akmon_query::JournalHandle<RedbObjectStore, RedbSessionGraph>, JournalError> {
    std::fs::create_dir_all(journal_dir).map_err(|e| JournalError::Verification(e.to_string()))?;
    let db_path = journal_db_path(journal_dir);
    let store = Arc::new(RedbObjectStore::create(
        db_path.as_path(),
        HashAlgorithm::Sha256,
    )?);
    let graph = RedbSessionGraph::open_new(Arc::clone(&store), session_id)?;
    Ok(akmon_query::JournalHandle::new(
        store,
        Arc::new(std::sync::Mutex::new(graph)),
    ))
}

/// Corrupts one object payload for existing `fixture`.
pub fn corrupt_fixture_object_bytes(fixture: &SessionFixture) {
    let store =
        Arc::new(RedbObjectStore::open(fixture.journal_db_path.as_path()).expect("reopen store"));
    store
        .overwrite_object_bytes_for_testing(&fixture.prompt_hash, b"corrupted-object")
        .expect("overwrite object");
}
