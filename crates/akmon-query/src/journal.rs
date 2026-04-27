//! Journal handle and default on-disk journal wiring for [`crate::session::AgentSession`].

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use akmon_core::{AgentConfig, AgentError};
use akmon_journal::{
    EventKind, HashAlgorithm, JournalError, ObjectStore, RedbObjectStore, RedbSessionGraph,
    SessionGraph,
};
use serde::Serialize;
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
}

impl<S, G> JournalHandle<S, G>
where
    S: ObjectStore,
    G: SessionGraph,
{
    /// Creates a handle from an existing store and graph mutex.
    pub fn new(store: Arc<S>, graph: Arc<Mutex<G>>) -> Self {
        Self { store, graph }
    }
}

fn canonical_cbor_bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, AgentError> {
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(value, &mut bytes).map_err(|e| AgentError::SessionFailed {
        message: format!("canonical CBOR: {e}"),
    })?;
    Ok(bytes)
}

fn default_journal_dir() -> Result<PathBuf, AgentError> {
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

/// Opens the per-user default journal (D-04) and creates a new session graph for `session_id`.
pub fn open_default_journal_handle(
    session_id: Uuid,
) -> Result<JournalHandle<RedbObjectStore, RedbSessionGraph>, AgentError> {
    let dir = default_journal_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| AgentError::SessionFailed {
        message: format!("journal mkdir {}: {e}", dir.display()),
    })?;
    let db_path = dir.join("journal.redb");
    let store = if db_path.is_file() {
        RedbObjectStore::open(db_path.as_path()).map_err(journal_err)?
    } else {
        RedbObjectStore::create(db_path.as_path(), HashAlgorithm::Sha256).map_err(journal_err)?
    };
    let store = Arc::new(store);
    let graph = RedbSessionGraph::open_new(Arc::clone(&store), session_id).map_err(journal_err)?;
    Ok(JournalHandle::new(store, Arc::new(Mutex::new(graph))))
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
