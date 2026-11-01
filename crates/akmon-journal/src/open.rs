//! Thin helpers for opening an existing on-disk journal directory for reading.
//!
//! These live in the substrate crate so evidence-side consumers (diff, replay, verify tooling)
//! can open a journal without depending on the agent stack. They never create a journal or a
//! session: a missing database or session is reported as an error (or `false`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::Result;
use crate::object_store::RedbObjectStore;
use crate::session_graph::{RedbSessionGraph, session_head_row_exists};

/// File name of the redb database inside a journal directory.
pub const JOURNAL_DB_FILE_NAME: &str = "journal.redb";

/// Resolves the journal database file path inside a journal directory.
#[must_use]
pub fn journal_db_path(journal_dir: &Path) -> PathBuf {
    journal_dir.join(JOURNAL_DB_FILE_NAME)
}

/// Opens an existing journal and session graph for `session_id`.
///
/// Never creates a journal or session. Returns an error when the database file is missing or
/// unreadable, or when `session_id` is not present in the session graph
/// ([`crate::JournalError::SessionNotFound`]).
pub fn open_journal_read_only(
    journal_dir: &Path,
    session_id: uuid::Uuid,
) -> Result<(Arc<RedbObjectStore>, RedbSessionGraph)> {
    let db_path = journal_db_path(journal_dir);
    let store = Arc::new(RedbObjectStore::open(db_path.as_path())?);
    let graph = RedbSessionGraph::reopen(Arc::clone(&store), session_id)?;
    Ok((store, graph))
}

/// Returns whether `session_id` has a row in the journal `session_heads` table.
///
/// When `journal.redb` is missing, returns `Ok(false)` (no session can exist). The database
/// handle is dropped before returning, so callers may open the journal again afterwards.
pub fn journal_contains_session(journal_dir: &Path, session_id: uuid::Uuid) -> Result<bool> {
    let db_path = journal_db_path(journal_dir);
    if !db_path.is_file() {
        return Ok(false);
    }
    let store = RedbObjectStore::open(db_path.as_path())?;
    session_head_row_exists(&store, session_id)
}

/// Returns whether two journal directory paths name the same existing directory.
///
/// Both paths are canonicalized (resolving symlinks, `.` and `..`). A path that does not exist
/// (or cannot be resolved) is never the same as another, since it cannot hold a live journal.
#[must_use]
pub fn same_journal_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::HashAlgorithm;
    use crate::{JournalError, SessionGraph};

    #[test]
    fn db_path_appends_file_name() {
        let p = journal_db_path(Path::new("/x/y"));
        assert_eq!(p, Path::new("/x/y").join("journal.redb"));
    }

    #[test]
    fn contains_session_false_when_database_missing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!journal_contains_session(tmp.path(), uuid::Uuid::new_v4()).unwrap());
    }

    #[test]
    fn open_read_only_errors_when_database_missing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(open_journal_read_only(tmp.path(), uuid::Uuid::new_v4()).is_err());
    }

    #[test]
    fn open_read_only_round_trip_and_missing_session() {
        let tmp = tempfile::tempdir().unwrap();
        let sid = uuid::Uuid::new_v4();
        {
            let store = Arc::new(
                RedbObjectStore::create(&journal_db_path(tmp.path()), HashAlgorithm::Sha256)
                    .unwrap(),
            );
            let _graph = RedbSessionGraph::open_new(Arc::clone(&store), sid).unwrap();
        }
        assert!(journal_contains_session(tmp.path(), sid).unwrap());
        let other = uuid::Uuid::new_v4();
        assert!(!journal_contains_session(tmp.path(), other).unwrap());
        {
            let (_store, graph) = open_journal_read_only(tmp.path(), sid).unwrap();
            assert!(graph.history().unwrap().is_empty());
        }
        match open_journal_read_only(tmp.path(), other) {
            Err(JournalError::SessionNotFound(id)) => assert_eq!(id, other),
            Err(e) => panic!("unexpected error: {e}"),
            Ok(_) => panic!("expected SessionNotFound"),
        }
    }

    #[test]
    fn same_journal_dir_resolves_aliases() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("journal");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        assert!(same_journal_dir(&dir, &dir));
        assert!(same_journal_dir(&dir, &dir.join("sub").join("..")));
        assert!(same_journal_dir(&dir, &dir.join(".")));
        assert!(!same_journal_dir(&dir, &dir.join("sub")));
        assert!(!same_journal_dir(&dir, &tmp.path().join("missing")));
        #[cfg(unix)]
        {
            let link = tmp.path().join("link");
            std::os::unix::fs::symlink(&dir, &link).unwrap();
            assert!(same_journal_dir(&dir, &link));
        }
    }
}
