use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use akmon_journal::{
    EventKind, HashAlgorithm, ObjectStore, RedbObjectStore, RedbSessionGraph, SessionGraph,
};
use tempfile::tempdir;
use uuid::Uuid;

fn journal_db_path(dir: &Path) -> std::path::PathBuf {
    dir.join("journal.redb")
}

fn put_bytes(store: &RedbObjectStore, bytes: &[u8]) -> akmon_journal::Hash {
    store.put(bytes).expect("put bytes")
}

fn create_clean_session(journal_dir: &Path, session_id: Uuid) {
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
        .append(EventKind::UserTurn {
            prompt_hash: put_bytes(store.as_ref(), b"hello"),
        })
        .expect("append user");
    graph
        .append(EventKind::SessionEnd {
            summary_hash: Some(put_bytes(store.as_ref(), b"summary")),
        })
        .expect("append end");
}

fn run_verify(journal_dir: &Path, session_id: Uuid) -> std::process::Output {
    let bin = std::env::var("CARGO_BIN_EXE_akmon").expect("CARGO_BIN_EXE_akmon");
    Command::new(bin)
        .args([
            "verify",
            &session_id.to_string(),
            "--journal",
            &journal_dir.display().to_string(),
        ])
        .output()
        .expect("run verify")
}

#[test]
fn t_verify_passes_for_clean_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let out = run_verify(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("verified: session"));
    assert!(stderr.contains("SessionEnd: present and terminal"));
}

#[test]
fn t_verify_fails_for_corrupted_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);

    let db_path = journal_db_path(tmp.path());
    let store = Arc::new(RedbObjectStore::open(db_path.as_path()).expect("reopen store"));
    let graph = RedbSessionGraph::reopen(Arc::clone(&store), sid).expect("reopen graph");
    let history = graph.history().expect("history");
    let prompt_hash = history
        .iter()
        .find_map(|(_, ev)| match &ev.kind {
            EventKind::UserTurn { prompt_hash } => Some(prompt_hash.clone()),
            _ => None,
        })
        .expect("prompt hash");
    store
        .overwrite_object_bytes_for_testing(&prompt_hash, b"corrupted-object")
        .expect("overwrite object");
    drop(graph);
    drop(store);

    let out = run_verify(tmp.path(), sid);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("verification failed: session"));
    assert!(stderr.contains("object hash mismatches"));
}

#[test]
fn t_verify_fails_for_missing_session() {
    let tmp = tempdir().expect("tempdir");
    let sid = Uuid::new_v4();
    create_clean_session(tmp.path(), sid);
    let missing = Uuid::new_v4();
    let out = run_verify(tmp.path(), missing);
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot open journal"));
}

#[test]
fn t_verify_fails_for_missing_journal() {
    let tmp = tempdir().expect("tempdir");
    let missing_dir = tmp.path().join("does-not-exist");
    let sid = Uuid::new_v4();
    let out = run_verify(missing_dir.as_path(), sid);
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot open journal"));
}
