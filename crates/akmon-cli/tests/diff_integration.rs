//! End-to-end `akmon diff` tests against real on-disk journals (Item 6.3 layer 4).
//!
//! Uses [`std::process::Command`] and [`common::akmon_bin_path`] — same pattern as
//! `replay_integration.rs` (no `assert_cmd` crate).
//!
//! **Redb locking:** Each journal directory must be written through **one** [`RedbObjectStore`]
//! handle when creating multiple sessions; reopening `journal.redb` in-process for the second
//! session hits `Database already open` on some platforms. Drop the store handle **before**
//! spawning `akmon diff`, or the child cannot acquire the database lock.
//!
//! **Resolve + missing object:** `DiffEngine::new` validates that every hash referenced by each
//! session exists in the store, so removing a referenced object before `akmon diff` fails with
//! exit code **3** (no report JSON). The `resolved_skip_reason = "object missing from store"` path
//! is covered in `akmon-diff` unit tests (`comparison` / `resolve`); reproducing it here would
//! require bypassing that validation. CLI integration instead covers **`resolved_skip_reason`** via
//! a **non-dereferenceable** field divergence (`ProviderCall` `provider_id`) under `--resolve`.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use akmon_diff::{DiffReportV1, RESOLVE_SKIP_NOT_DEREFERENCABLE};
use akmon_journal::{
    AttemptRecord, AttemptStatus, EventKind, HashAlgorithm, RedbObjectStore, RedbSessionGraph,
    SessionGraph,
};
use akmon_query::journal_db_path;
use tempfile::tempdir;
use time::OffsetDateTime;
use uuid::Uuid;

#[allow(dead_code)]
mod common;
use common::{akmon_bin_path, put_bytes};

fn open_store(dir: &Path) -> Arc<RedbObjectStore> {
    std::fs::create_dir_all(dir).expect("mkdir journal dir");
    let db = journal_db_path(dir);
    Arc::new(if db.is_file() {
        RedbObjectStore::open(db.as_path()).expect("open store")
    } else {
        RedbObjectStore::create(db.as_path(), HashAlgorithm::Sha256).expect("create store")
    })
}

fn attempt_record(store: &Arc<RedbObjectStore>, req: &[u8], resp: &[u8]) -> AttemptRecord {
    AttemptRecord {
        attempt_number: 1,
        started_at: OffsetDateTime::now_utc(),
        ended_at: OffsetDateTime::now_utc(),
        status: AttemptStatus::Success,
        request_hash: put_bytes(store.as_ref(), req),
        response_hash: Some(put_bytes(store.as_ref(), resp)),
        stream_hash: None,
        error_message: None,
    }
}

/// SessionStart → UserTurn → SessionEnd (three events).
fn append_minimal_session(store: &Arc<RedbObjectStore>, session_id: Uuid, prompt: &[u8]) {
    let mut graph = RedbSessionGraph::open_new(Arc::clone(store), session_id).expect("graph");
    graph
        .append(EventKind::SessionStart {
            cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
            config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
        })
        .expect("SessionStart");
    graph
        .append(EventKind::UserTurn {
            prompt_hash: put_bytes(store.as_ref(), prompt),
        })
        .expect("UserTurn");
    graph
        .append(EventKind::SessionEnd {
            summary_hash: Some(put_bytes(store.as_ref(), b"summary")),
        })
        .expect("SessionEnd");
}

fn append_minimal_session_custom_cwd(
    store: &Arc<RedbObjectStore>,
    session_id: Uuid,
    cwd: &[u8],
    prompt: &[u8],
) {
    let mut graph = RedbSessionGraph::open_new(Arc::clone(store), session_id).expect("graph");
    graph
        .append(EventKind::SessionStart {
            cwd_hash: put_bytes(store.as_ref(), cwd),
            config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
        })
        .expect("SessionStart");
    graph
        .append(EventKind::UserTurn {
            prompt_hash: put_bytes(store.as_ref(), prompt),
        })
        .expect("UserTurn");
    graph
        .append(EventKind::SessionEnd {
            summary_hash: Some(put_bytes(store.as_ref(), b"summary")),
        })
        .expect("SessionEnd");
}

fn append_provider_session(store: &Arc<RedbObjectStore>, session_id: Uuid, provider_id: &str) {
    let mut graph = RedbSessionGraph::open_new(Arc::clone(store), session_id).expect("graph");
    graph
        .append(EventKind::SessionStart {
            cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
            config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
        })
        .expect("SessionStart");
    graph
        .append(EventKind::UserTurn {
            prompt_hash: put_bytes(store.as_ref(), b"hello"),
        })
        .expect("UserTurn");
    graph
        .append(EventKind::ProviderCall {
            provider_id: provider_id.to_owned(),
            attempts: vec![attempt_record(store, b"req", b"resp")],
            stream_hash: None,
        })
        .expect("ProviderCall");
    graph
        .append(EventKind::SessionEnd {
            summary_hash: Some(put_bytes(store.as_ref(), b"summary")),
        })
        .expect("SessionEnd");
}

fn append_structural_session_assistant(store: &Arc<RedbObjectStore>, session_id: Uuid) {
    let mut graph = RedbSessionGraph::open_new(Arc::clone(store), session_id).expect("graph");
    graph
        .append(EventKind::SessionStart {
            cwd_hash: put_bytes(store.as_ref(), b"/workspace"),
            config_hash: put_bytes(store.as_ref(), br#"{"model":"x"}"#),
        })
        .expect("SessionStart");
    graph
        .append(EventKind::UserTurn {
            prompt_hash: put_bytes(store.as_ref(), b"hello"),
        })
        .expect("UserTurn");
    graph
        .append(EventKind::AssistantTurn {
            message_hash: put_bytes(store.as_ref(), b"assistant-msg"),
            tool_calls_hash: None,
        })
        .expect("AssistantTurn");
    graph
        .append(EventKind::SessionEnd {
            summary_hash: Some(put_bytes(store.as_ref(), b"summary")),
        })
        .expect("SessionEnd");
}

/// One session only; closes store before return.
fn write_single_minimal_session(dir: &Path, session_id: Uuid, prompt: &[u8]) {
    let store = open_store(dir);
    append_minimal_session(&store, session_id, prompt);
}

fn run_diff_with(
    journal_dir: &Path,
    session_a: Uuid,
    session_b: Uuid,
    extra: &[&str],
) -> std::process::Output {
    let bin = akmon_bin_path();
    let mut cmd = Command::new(bin);
    cmd.args([
        "diff",
        &session_a.to_string(),
        &session_b.to_string(),
        "--journal",
        &journal_dir.display().to_string(),
    ]);
    cmd.args(extra);
    cmd.output().expect("akmon diff")
}

#[test]
fn t_diff_exit_zero_on_match() {
    let tmp = tempdir().expect("tempdir");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let prompt = b"same-prompt";
    let store = open_store(tmp.path());
    append_minimal_session(&store, sid_a, prompt);
    append_minimal_session(&store, sid_b, prompt);
    drop(store);
    let out = run_diff_with(tmp.path(), sid_a, sid_b, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("matches: yes"), "{stdout}");
}

#[test]
fn t_diff_exit_one_on_diverge() {
    let tmp = tempdir().expect("tempdir");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let store = open_store(tmp.path());
    append_minimal_session(&store, sid_a, b"hello-a");
    append_minimal_session(&store, sid_b, b"hello-b");
    drop(store);
    let out = run_diff_with(tmp.path(), sid_a, sid_b, &[]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("matches: no"), "{stdout}");
    assert!(stdout.contains("divergences:"), "{stdout}");
}

#[test]
fn t_diff_exit_one_on_structural_break() {
    let tmp = tempdir().expect("tempdir");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let store = open_store(tmp.path());
    append_structural_session_assistant(&store, sid_a);
    append_provider_session(&store, sid_b, "anthropic");
    drop(store);
    let out = run_diff_with(tmp.path(), sid_a, sid_b, &[]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("structural break:"), "{stdout}");
}

#[test]
fn t_diff_exit_three_on_missing_session() {
    let tmp = tempdir().expect("tempdir");
    let sid_a = Uuid::new_v4();
    let missing = Uuid::new_v4();
    write_single_minimal_session(tmp.path(), sid_a, b"x");
    let out = run_diff_with(tmp.path(), sid_a, missing, &[]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&missing.to_string()) || stderr.contains("akmon: diff:"),
        "{stderr}"
    );
}

#[test]
fn t_diff_exit_three_on_missing_journal() {
    let tmp = tempdir().expect("tempdir");
    let nowhere = tmp.path().join("no-journal-here");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let out = run_diff_with(nowhere.as_path(), sid_a, sid_b, &[]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("akmon: diff:"), "{stderr}");
}

#[test]
fn t_diff_exit_two_on_invalid_session_id() {
    let tmp = tempdir().expect("tempdir");
    let sid_b = Uuid::new_v4();
    write_single_minimal_session(tmp.path(), sid_b, b"x");
    let bin = akmon_bin_path();
    let out = Command::new(bin)
        .args([
            "diff",
            "not-a-uuid",
            &sid_b.to_string(),
            "--journal",
            &tmp.path().display().to_string(),
        ])
        .output()
        .expect("diff");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn t_diff_human_format_passing_session() {
    let tmp = tempdir().expect("tempdir");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let store = open_store(tmp.path());
    append_minimal_session(&store, sid_a, b"hello");
    append_minimal_session(&store, sid_b, b"hello");
    drop(store);
    let out = run_diff_with(tmp.path(), sid_a, sid_b, &[]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("diff: comparing"));
    assert!(stdout.contains(&sid_a.to_string()));
    assert!(stdout.contains(&sid_b.to_string()));
    assert!(stdout.contains("mode: default"));
    assert!(stdout.contains("events compared:"));
    assert!(stdout.contains("session A events:"));
    assert!(stdout.contains("session B events:"));
    assert!(stdout.contains("divergence count:"));
    assert!(stdout.contains("matches: yes"));
}

#[test]
fn t_diff_human_format_failing_session() {
    let tmp = tempdir().expect("tempdir");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let store = open_store(tmp.path());
    append_minimal_session(&store, sid_a, b"a");
    append_minimal_session(&store, sid_b, b"b");
    drop(store);
    let out = run_diff_with(tmp.path(), sid_a, sid_b, &[]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("divergences:"));
    assert!(stdout.contains("prompt_hash") || stdout.contains("content_reference"));
    assert!(stdout.contains("expected:"));
    assert!(stdout.contains("actual:"));
}

#[test]
fn t_diff_json_format_passing_session() {
    let tmp = tempdir().expect("tempdir");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let store = open_store(tmp.path());
    append_minimal_session(&store, sid_a, b"hello");
    append_minimal_session(&store, sid_b, b"hello");
    drop(store);
    let out = run_diff_with(tmp.path(), sid_a, sid_b, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(0));
    let report: DiffReportV1 = serde_json::from_slice(&out.stdout).expect("json");
    assert!(report.matches);
    assert!(report.divergences.is_empty());
}

#[test]
fn t_diff_json_format_failing_session() {
    let tmp = tempdir().expect("tempdir");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let store = open_store(tmp.path());
    append_minimal_session(&store, sid_a, b"a");
    append_minimal_session(&store, sid_b, b"b");
    drop(store);
    let out = run_diff_with(tmp.path(), sid_a, sid_b, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(1));
    let report: DiffReportV1 = serde_json::from_slice(&out.stdout).expect("json");
    assert!(!report.matches);
    assert!(!report.divergences.is_empty());
}

#[test]
fn t_diff_resolve_populates_resolved_field() {
    let tmp = tempdir().expect("tempdir");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let store = open_store(tmp.path());
    append_minimal_session_custom_cwd(&store, sid_a, b"/session-a", b"hello");
    append_minimal_session_custom_cwd(&store, sid_b, b"/session-b", b"hello");
    drop(store);
    let out = run_diff_with(tmp.path(), sid_a, sid_b, &["--resolve", "--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: DiffReportV1 = serde_json::from_slice(&out.stdout).expect("json");
    let div = report.divergences.first().expect("divergence");
    let resolved = div.resolved.as_ref().expect("resolved");
    assert_eq!(resolved.a_size_bytes, b"/session-a".len());
    assert_eq!(resolved.b_size_bytes, b"/session-b".len());
    assert!(!resolved.bytes_match);
}

#[test]
fn t_diff_resolve_skip_reason_for_non_dereferenceable_field() {
    let tmp = tempdir().expect("tempdir");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let store = open_store(tmp.path());
    append_provider_session(&store, sid_a, "anthropic");
    append_provider_session(&store, sid_b, "openai");
    drop(store);
    let out = run_diff_with(tmp.path(), sid_a, sid_b, &["--resolve", "--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: DiffReportV1 = serde_json::from_slice(&out.stdout).expect("json");
    let div = report.divergences.first().expect("divergence");
    assert!(div.resolved.is_none());
    assert_eq!(
        div.resolved_skip_reason.as_deref(),
        Some(RESOLVE_SKIP_NOT_DEREFERENCABLE)
    );
}

#[test]
fn t_diff_referenced_object_removed_exit_three_even_with_resolve() {
    let tmp = tempdir().expect("tempdir");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let store = open_store(tmp.path());
    append_minimal_session(&store, sid_a, b"ok-a");
    append_minimal_session(&store, sid_b, b"ok-b");
    drop(store);

    let db_path = journal_db_path(tmp.path());
    let prompt_hash = {
        let store = Arc::new(RedbObjectStore::open(db_path.as_path()).expect("open"));
        let graph = RedbSessionGraph::reopen(Arc::clone(&store), sid_b).expect("reopen");
        let history = graph.history().expect("history");
        let (_, ev) = &history[1];
        match &ev.kind {
            EventKind::UserTurn { prompt_hash } => prompt_hash.clone(),
            _ => panic!("expected UserTurn at seq 1"),
        }
    };
    {
        let store = Arc::new(RedbObjectStore::open(db_path.as_path()).expect("open"));
        store
            .remove_object_for_testing(&prompt_hash)
            .expect("remove prompt");
    }
    let out = run_diff_with(tmp.path(), sid_a, sid_b, &["--resolve", "--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
async fn t_diff_journal_flag_overrides_default() {
    let tmp = tempdir().expect("tempdir");
    let custom_journal = tmp.path().join("custom-journal");
    std::fs::create_dir_all(&custom_journal).expect("mkdir");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let store = open_store(custom_journal.as_path());
    append_minimal_session(&store, sid_a, b"hello");
    append_minimal_session(&store, sid_b, b"hello");
    drop(store);

    let default_state = tmp.path().join("xdg-state-empty");
    std::fs::create_dir_all(&default_state).expect("mkdir default");
    let bin = akmon_bin_path();
    let out = Command::new(bin)
        .args([
            "diff",
            &sid_a.to_string(),
            &sid_b.to_string(),
            "--journal",
            &custom_journal.display().to_string(),
        ])
        .env("XDG_STATE_HOME", &default_state)
        .output()
        .expect("diff");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}
