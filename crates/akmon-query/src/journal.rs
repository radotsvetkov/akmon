//! Journal handle and default on-disk journal wiring for [`crate::session::AgentSession`].

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use akmon_core::{AgentConfig, AgentError};
use akmon_journal::{
    EventKind, HashAlgorithm, JournalError, ObjectStore, RedbObjectStore, RedbSessionGraph,
    SessionGraph, session_head_row_exists,
};
use akmon_models::ModelToolCall;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

/// Shared object store and mutex-protected session graph for one agent session.
pub struct JournalHandle<S, G>
where
    S: ObjectStore,
    G: SessionGraph,
{
    /// Content-addressed object store for journal blobs.
    pub store: Arc<S>,
    /// Merkle session graph; mutex matches [`akmon_models::journaling::JournalingProvider`] patterns.
    pub graph: Arc<Mutex<G>>,
    /// When true, the graph was reopened for resume and [`emit_session_start`] must be skipped.
    pub resumed: bool,
}

impl<S, G> JournalHandle<S, G>
where
    S: ObjectStore,
    G: SessionGraph,
{
    /// Creates a handle from an existing store and graph mutex.
    pub fn new(store: Arc<S>, graph: Arc<Mutex<G>>) -> Self {
        Self {
            store,
            graph,
            resumed: false,
        }
    }

    /// Like [`Self::new`] but marks the handle as a resumed session (skip [`emit_session_start`]).
    pub fn resumed(store: Arc<S>, graph: Arc<Mutex<G>>) -> Self {
        Self {
            store,
            graph,
            resumed: true,
        }
    }
}

fn canonical_cbor_bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, AgentError> {
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(value, &mut bytes).map_err(|e| AgentError::SessionFailed {
        message: format!("canonical CBOR: {e}"),
    })?;
    Ok(bytes)
}

/// Resolves the per-user journal directory (Decision D-04).
pub fn default_journal_dir() -> Result<PathBuf, AgentError> {
    #[cfg(windows)]
    {
        let base = std::env::var("LOCALAPPDATA").map_err(|_| AgentError::SessionFailed {
            message: "LOCALAPPDATA is unset; cannot resolve journal directory".into(),
        })?;
        Ok(PathBuf::from(base).join("akmon").join("journal"))
    }
    #[cfg(not(windows))]
    {
        let base = std::env::var("XDG_STATE_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".local").join("state"))
            })
            .ok_or_else(|| AgentError::SessionFailed {
                message: "cannot resolve journal directory (set XDG_STATE_HOME or HOME)".into(),
            })?;
        Ok(base.join("akmon").join("journal"))
    }
}

/// Resolves the journal database file path inside a journal directory.
#[must_use]
pub fn journal_db_path(journal_dir: &std::path::Path) -> PathBuf {
    journal_dir.join("journal.redb")
}

/// Opens the per-user default journal (D-04) and creates a new session graph for `session_id`.
pub fn open_default_journal_handle(
    session_id: Uuid,
) -> Result<JournalHandle<RedbObjectStore, RedbSessionGraph>, AgentError> {
    open_or_resume_default_journal_handle(session_id, false)
}

/// Opens the default journal, creating a new session graph or reopening an existing one when `resume`.
///
/// When `resume` is true and `session_id` already exists in `session_heads`, the graph is reopened
/// and the returned handle has [`JournalHandle::resumed`] set. When `resume` is false, [`RedbSessionGraph::open_new`]
/// is used (fails if the session already exists).
pub fn open_or_resume_default_journal_handle(
    session_id: Uuid,
    resume: bool,
) -> Result<JournalHandle<RedbObjectStore, RedbSessionGraph>, AgentError> {
    let dir = default_journal_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| AgentError::SessionFailed {
        message: format!("journal mkdir {}: {e}", dir.display()),
    })?;
    let db_path = journal_db_path(&dir);
    let store = if db_path.is_file() {
        RedbObjectStore::open(db_path.as_path()).map_err(journal_err)?
    } else {
        RedbObjectStore::create(db_path.as_path(), HashAlgorithm::Sha256).map_err(journal_err)?
    };
    let store = Arc::new(store);
    let exists = session_head_row_exists(store.as_ref(), session_id).map_err(journal_err)?;
    if resume && exists {
        let graph =
            RedbSessionGraph::reopen(Arc::clone(&store), session_id).map_err(journal_err)?;
        Ok(JournalHandle::resumed(store, Arc::new(Mutex::new(graph))))
    } else {
        let graph =
            RedbSessionGraph::open_new(Arc::clone(&store), session_id).map_err(journal_err)?;
        Ok(JournalHandle::new(store, Arc::new(Mutex::new(graph))))
    }
}

/// Opens an existing journal + session graph for read-only operations by `session_id`.
///
/// This function never creates a journal or session. It returns an error when the database file is
/// missing/unreadable or when `session_id` is not present in the session graph.
pub fn open_journal_read_only(
    journal_dir: &std::path::Path,
    session_id: Uuid,
) -> Result<JournalHandle<RedbObjectStore, RedbSessionGraph>, AgentError> {
    let db_path = journal_db_path(journal_dir);
    let store = Arc::new(RedbObjectStore::open(db_path.as_path()).map_err(journal_err)?);
    let graph = RedbSessionGraph::reopen(Arc::clone(&store), session_id).map_err(journal_err)?;
    Ok(JournalHandle::new(store, Arc::new(Mutex::new(graph))))
}

/// Returns whether `session_id` has a row in the journal database `session_heads` table.
///
/// When `journal.redb` is missing, returns `Ok(false)` (no session can exist). Opens the database
/// read-only and performs a single key probe.
pub fn journal_contains_session(
    journal_dir: &std::path::Path,
    session_id: Uuid,
) -> Result<bool, AgentError> {
    let db_path = journal_db_path(journal_dir);
    if !db_path.is_file() {
        return Ok(false);
    }
    let store = RedbObjectStore::open(db_path.as_path()).map_err(journal_err)?;
    session_head_row_exists(&store, session_id).map_err(journal_err)
}

fn journal_err(e: JournalError) -> AgentError {
    AgentError::SessionFailed {
        message: e.to_string(),
    }
}

/// Writes [`SessionStart`](EventKind::SessionStart) evidence objects and appends the event.
pub(crate) fn emit_session_start<S, G>(
    journal: &JournalHandle<S, G>,
    config: &AgentConfig,
) -> Result<(), AgentError>
where
    S: ObjectStore,
    G: SessionGraph,
{
    let cwd = std::env::current_dir().map_err(|e| AgentError::SessionFailed {
        message: format!("current_dir: {e}"),
    })?;
    let cwd_bytes = cwd.to_string_lossy().as_bytes().to_vec();
    let cwd_hash = journal.store.put(&cwd_bytes).map_err(journal_err)?;
    let config_bytes = canonical_cbor_bytes(config)?;
    let config_hash = journal.store.put(&config_bytes).map_err(journal_err)?;
    let mut guard = journal
        .graph
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .append(EventKind::SessionStart {
            cwd_hash,
            config_hash,
        })
        .map_err(journal_err)?;
    Ok(())
}

/// Stores `prompt` as raw UTF-8 bytes and appends [`UserTurn`](EventKind::UserTurn).
///
/// The blob is hashed from UTF-8 octets only (no CBOR envelope): the content address is the
/// literal task bytes, which is deterministic and matches “hash what the model saw as user text”.
pub(crate) fn emit_user_turn<S, G>(
    journal: &JournalHandle<S, G>,
    prompt: &str,
) -> Result<(), AgentError>
where
    S: ObjectStore,
    G: SessionGraph,
{
    let prompt_bytes = prompt.as_bytes().to_vec();
    let prompt_hash = journal.store.put(&prompt_bytes).map_err(journal_err)?;
    let mut guard = journal
        .graph
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .append(EventKind::UserTurn { prompt_hash })
        .map_err(journal_err)?;
    Ok(())
}

/// CBOR-backed permission decision context (minimal v2.0.0 shape: tool id, args, human-readable path).
#[derive(Serialize)]
struct PermissionGateContext<'a> {
    tool_name: &'a str,
    tool_input: &'a Value,
    decision_path: &'a str,
}

/// Serializes [`PermissionGateContext`] to canonical CBOR for [`emit_permission_gate`].
pub(crate) fn permission_gate_context_cbor(
    tool_name: &str,
    tool_input: &Value,
    decision_path: &str,
) -> Result<Vec<u8>, AgentError> {
    canonical_cbor_bytes(&PermissionGateContext {
        tool_name,
        tool_input,
        decision_path,
    })
}

/// Appends [`EventKind::PermissionGate`] after storing `context_bytes` in the journal object store.
pub(crate) fn emit_permission_gate<S, G>(
    journal: &JournalHandle<S, G>,
    policy_id: &str,
    decision: &str,
    context_bytes: &[u8],
) -> Result<(), AgentError>
where
    S: ObjectStore,
    G: SessionGraph,
{
    let context_hash = journal.store.put(context_bytes).map_err(journal_err)?;
    let mut guard = journal
        .graph
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .append(EventKind::PermissionGate {
            policy_id: policy_id.to_owned(),
            decision: decision.to_owned(),
            context_hash,
        })
        .map_err(journal_err)?;
    Ok(())
}

/// Canonical CBOR of model-requested tool calls for [`emit_assistant_turn`] `tool_calls_hash`.
pub(crate) fn assistant_tool_calls_cbor(calls: &[ModelToolCall]) -> Result<Vec<u8>, AgentError> {
    canonical_cbor_bytes(calls)
}

/// Stores `message_bytes` as raw UTF-8 (parallel to [`emit_user_turn`]: assistant text only; role is implied by [`EventKind::AssistantTurn`]) and appends [`EventKind::AssistantTurn`].
pub(crate) fn emit_assistant_turn<S, G>(
    journal: &JournalHandle<S, G>,
    message_bytes: &[u8],
    tool_calls_bytes: Option<&[u8]>,
) -> Result<(), AgentError>
where
    S: ObjectStore,
    G: SessionGraph,
{
    let message_hash = journal.store.put(message_bytes).map_err(journal_err)?;
    let tool_calls_hash = match tool_calls_bytes {
        Some(b) => Some(journal.store.put(b).map_err(journal_err)?),
        None => None,
    };
    let mut guard = journal
        .graph
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .append(EventKind::AssistantTurn {
            message_hash,
            tool_calls_hash,
        })
        .map_err(journal_err)?;
    Ok(())
}

/// Appends [`SessionEnd`](EventKind::SessionEnd).
pub(crate) fn append_session_end<S, G>(
    journal: &JournalHandle<S, G>,
    summary_hash: Option<akmon_journal::Hash>,
) -> Result<(), AgentError>
where
    S: ObjectStore,
    G: SessionGraph,
{
    let mut guard = journal
        .graph
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .append(EventKind::SessionEnd { summary_hash })
        .map_err(journal_err)?;
    Ok(())
}

#[cfg(test)]
mod journal_contains_session_tests {
    use super::*;
    use tempfile::tempdir;
    use uuid::Uuid;

    #[test]
    fn journal_contains_session_false_when_database_missing() {
        let tmp = tempdir().unwrap();
        let sid = Uuid::new_v4();
        assert!(
            !journal_contains_session(tmp.path(), sid).expect("probe"),
            "missing journal.redb implies no session rows"
        );
    }
}
