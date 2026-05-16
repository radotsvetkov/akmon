//! Merkle-linked per-session graph storage and verification.

use crate::error::{JournalError, Result};
use crate::event::{Event, EventKind, referenced_object_hashes_for_kind};
use crate::hash::{Hash, digest_bytes};
use crate::object_store::{ObjectStore, RedbObjectStore};
use redb::{Database, ReadableTable, TableDefinition};
use std::sync::Arc;

const SESSION_HEADS_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("session_heads");
const SESSION_EVENTS_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("session_events");

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct StoredEvent {
    hash: Hash,
    event: Event,
}

/// Collected verification findings for one session graph.
#[derive(Debug, Clone, Default)]
pub struct VerificationReport {
    /// Number of events walked.
    pub events_checked: u64,
    /// Number of object hash references checked.
    pub objects_checked: u64,
    /// Referenced objects that are missing from the object store.
    pub missing_objects: Vec<MissingObject>,
    /// Object bytes present but digest does not match the referenced hash (AGEF Section 13 step 5).
    pub object_hash_mismatches: Vec<Hash>,
    /// Event hashes that do not match recomputed canonical CBOR content hash.
    pub hash_mismatches: Vec<Hash>,
    /// Parent-link violations as `(event_hash, expected_parent_hash)`.
    pub broken_parent_links: Vec<(Hash, Hash)>,
    /// Sequence values where monotonic +1 invariant is violated.
    pub sequence_violations: Vec<u64>,
    /// Stored session head differs from computed terminal event hash `(stored, computed)`.
    pub head_mismatch: Option<(Hash, Hash)>,
    /// Count of [`EventKind::SessionEnd`] events in session order.
    pub session_end_count: usize,
    /// When `session_end_count == 1`, true iff that sole `SessionEnd` is the last event; otherwise `false`.
    pub session_end_is_terminal: bool,
    /// Stable list of checks attempted during verification.
    pub checks_performed: Vec<VerifyCheck>,
}

/// A missing object hash plus optional event context for where the reference was observed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MissingObject {
    /// Referenced object hash that could not be resolved.
    pub object_hash: Hash,
    /// Event hash that referenced this object, when available.
    pub referenced_by_event: Option<Hash>,
}

/// Named verification checks attempted by [`SessionGraph::verify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyCheck {
    /// Validate SessionStart/non-start parent linkage invariants.
    ParentChain,
    /// Validate event sequence monotonicity (`0..n-1`).
    Sequence,
    /// Recompute and compare event content hashes.
    EventHashRecompute,
    /// Resolve all referenced object hashes from the object store.
    ObjectPresence,
    /// Re-hash object bytes and compare to referenced hashes (AGEF Section 13 step 5).
    ObjectByteRehash,
    /// Compare stored head pointer with the computed terminal event hash.
    HeadConsistency,
    /// Validate SessionEnd count and terminal placement invariants.
    SessionEndInvariants,
}

impl VerificationReport {
    /// Returns true when there are no structural or integrity violations and SessionEnd invariants hold.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.missing_objects.is_empty()
            && self.object_hash_mismatches.is_empty()
            && self.hash_mismatches.is_empty()
            && self.broken_parent_links.is_empty()
            && self.sequence_violations.is_empty()
            && self.head_mismatch.is_none()
            && self.session_end_count == 1
            && self.session_end_is_terminal
    }
}

/// Linear session history verification: parent chain, sequence, event hashes, object closure and
/// byte-level digests, head consistency, and terminal `SessionEnd` invariants.
fn verify_history_against_store(
    history: &[(Hash, Event)],
    stored_head: Option<Hash>,
    store: &dyn ObjectStore,
) -> Result<VerificationReport> {
    let mut report = VerificationReport {
        checks_performed: vec![
            VerifyCheck::ParentChain,
            VerifyCheck::Sequence,
            VerifyCheck::EventHashRecompute,
            VerifyCheck::ObjectPresence,
            VerifyCheck::ObjectByteRehash,
            VerifyCheck::HeadConsistency,
            VerifyCheck::SessionEndInvariants,
        ],
        ..VerificationReport::default()
    };
    let mut expected_prev: Option<Hash> = None;
    let mut session_end_count = 0usize;
    let mut last_session_end_position: Option<usize> = None;

    for (idx, (stored_hash, event)) in history.iter().enumerate() {
        if matches!(event.kind, EventKind::SessionEnd { .. }) {
            session_end_count += 1;
            last_session_end_position = Some(idx);
        }

        report.events_checked += 1;
        let expected_seq = idx as u64;
        if event.sequence != expected_seq {
            report.sequence_violations.push(event.sequence);
        }

        if idx == 0 {
            if !matches!(event.kind, EventKind::SessionStart { .. }) || !event.parents.is_empty() {
                let expected = Hash::from_bytes(store.algorithm(), [0_u8; 32]);
                report
                    .broken_parent_links
                    .push((stored_hash.clone(), expected));
            }
        } else if let Some(prev_hash) = expected_prev.as_ref()
            && (event.parents.len() != 1 || event.parents.first() != Some(prev_hash))
        {
            report
                .broken_parent_links
                .push((stored_hash.clone(), prev_hash.clone()));
        }

        let recomputed = event.content_hash(store.algorithm())?;
        if recomputed != *stored_hash {
            report.hash_mismatches.push(stored_hash.clone());
        }

        for object_hash in referenced_object_hashes_for_kind(&event.kind) {
            report.objects_checked += 1;
            if !store.contains(&object_hash)? {
                report.missing_objects.push(MissingObject {
                    object_hash: object_hash.clone(),
                    referenced_by_event: Some(stored_hash.clone()),
                });
                continue;
            }
            match store.get(&object_hash)? {
                None => report.missing_objects.push(MissingObject {
                    object_hash: object_hash.clone(),
                    referenced_by_event: Some(stored_hash.clone()),
                }),
                Some(bytes) => {
                    let digest = digest_bytes(store.algorithm(), bytes.as_ref());
                    if digest != object_hash {
                        report.object_hash_mismatches.push(object_hash.clone());
                    }
                }
            }
        }

        expected_prev = Some(stored_hash.clone());
    }

    report.session_end_count = session_end_count;
    report.session_end_is_terminal = session_end_count == 1
        && last_session_end_position == Some(history.len().saturating_sub(1));

    let computed_head = history.last().map(|(hash, _)| hash.clone());
    if let (Some(stored), Some(computed)) = (stored_head, computed_head)
        && stored != computed
    {
        report.head_mismatch = Some((stored, computed));
    }

    Ok(report)
}

/// Verifies a linear `[(event_hash, event)]` history and object store using the same rules as
/// [`SessionGraph::verify`] for persisted graphs.
///
/// `declared_head` is the head hash asserted by the producer (for example from a bundle manifest);
/// when `Some`, it must equal the content hash of the terminal event or a head mismatch is recorded.
pub fn verify_linear_history_against_store(
    history: &[(Hash, Event)],
    declared_head: Option<Hash>,
    store: &dyn ObjectStore,
) -> Result<VerificationReport> {
    verify_history_against_store(history, declared_head, store)
}

fn ensure_empty_import_target(head: Option<Hash>, history_len: usize) -> Result<()> {
    if head.is_some() || history_len > 0 {
        return Err(JournalError::Verification(
            "import target session is not empty".into(),
        ));
    }
    Ok(())
}

fn verify_import_precondition(events: &[(Hash, Event)], store: &dyn ObjectStore) -> Result<()> {
    if events.is_empty() {
        return Err(JournalError::Verification(
            "cannot import empty event history".into(),
        ));
    }
    let report = verify_linear_history_against_store(events, None, store)?;
    if !report.is_clean() {
        return Err(JournalError::Verification(
            "import precondition: linear history verification not clean".into(),
        ));
    }
    Ok(())
}

/// Session graph operations.
pub trait SessionGraph: Send + Sync {
    /// Returns the session identifier.
    fn session_id(&self) -> uuid::Uuid;
    /// Appends one event kind to this session and returns its event hash.
    fn append(&mut self, kind: EventKind) -> Result<Hash>;
    /// Returns current head hash, or `None` when graph is empty.
    fn head(&self) -> Result<Option<Hash>>;
    /// Returns all events in sequence/topological order.
    fn history(&self) -> Result<Vec<(Hash, Event)>>;
    /// Verifies graph and object integrity, collecting all violations.
    ///
    /// This method intentionally treats hash-algorithm mismatches differently from ordinary
    /// tamper-evidence findings. Structural inconsistencies (missing objects, object byte digests,
    /// parent-link breaks, sequence gaps, hash mismatches, head mismatch, SessionEnd invariants) are
    /// accumulated into the report. A
    /// `HashAlgorithmMismatch` from the object store is returned as `Err(...)` because it indicates
    /// infrastructure-level corruption/configuration failure (the store and graph no longer agree
    /// on the active algorithm), not a recoverable per-event inconsistency.
    fn verify(&self) -> Result<VerificationReport>;
    /// Persists a producer-verified linear `[(event_hash, event)]` chain without recomputing hashes
    /// or mutating fields such as parents, sequence, or `emitted_at`.
    ///
    /// # Preconditions
    /// - The graph is empty: [`SessionGraph::head`] is [`None`] and [`SessionGraph::history`] is
    ///   empty (the state immediately after [`RedbSessionGraph::open_new`] /
    ///   [`MemorySessionGraph::open_new`] with no [`SessionGraph::append`] calls).
    /// - `events` is non-empty and [`verify_linear_history_against_store`] with `declared_head: None`
    ///   yields [`VerificationReport::is_clean`] against this graph's object store.
    fn import_verified_linear_history(&mut self, events: &[(Hash, Event)]) -> Result<()>;
}

/// redb-backed session graph for persisted journals.
pub struct RedbSessionGraph {
    store: Arc<RedbObjectStore>,
    session_id: uuid::Uuid,
}

impl RedbSessionGraph {
    /// Creates a new empty session graph.
    pub fn open_new(store: Arc<RedbObjectStore>, session_id: uuid::Uuid) -> Result<Self> {
        ensure_graph_tables(store.database())?;
        let key = session_key_bytes(session_id);
        let write_txn = store
            .database()
            .begin_write()
            .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
        {
            let mut heads = write_txn.open_table(SESSION_HEADS_TABLE).map_err(|err| {
                JournalError::Verification(format!("open session_heads failed: {err}"))
            })?;
            if heads
                .get(key.as_slice())
                .map_err(|err| {
                    JournalError::Verification(format!("read session_heads failed: {err}"))
                })?
                .is_some()
            {
                return Err(JournalError::Verification(format!(
                    "session already exists: {session_id}"
                )));
            }
            let none_head = postcard::to_allocvec(&Option::<Hash>::None)?;
            heads
                .insert(key.as_slice(), none_head.as_slice())
                .map_err(|err| {
                    JournalError::Verification(format!("write session_heads failed: {err}"))
                })?;
        }
        write_txn.commit().map_err(|err| {
            JournalError::Verification(format!("commit session create failed: {err}"))
        })?;
        Ok(Self { store, session_id })
    }

    /// Opens an existing session graph.
    pub fn reopen(store: Arc<RedbObjectStore>, session_id: uuid::Uuid) -> Result<Self> {
        ensure_graph_tables(store.database())?;
        let key = session_key_bytes(session_id);
        let read_txn = store
            .database()
            .begin_read()
            .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
        let heads = read_txn.open_table(SESSION_HEADS_TABLE).map_err(|err| {
            JournalError::Verification(format!("open session_heads failed: {err}"))
        })?;
        let head = heads.get(key.as_slice()).map_err(|err| {
            JournalError::Verification(format!("read session_heads failed: {err}"))
        })?;
        if head.is_none() {
            return Err(JournalError::SessionNotFound(session_id));
        }
        Ok(Self { store, session_id })
    }

    /// Test-only: overwrites the event payload at `sequence` while preserving stored hash bytes.
    ///
    /// This enables corruption fixtures where event bytes change but address metadata remains.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn overwrite_event_at_sequence_for_testing(
        &mut self,
        sequence: u64,
        event: Event,
    ) -> Result<()> {
        let key = event_key(self.session_id, sequence);
        let write_txn = self
            .store
            .database()
            .begin_write()
            .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
        {
            let mut events = write_txn.open_table(SESSION_EVENTS_TABLE).map_err(|err| {
                JournalError::Verification(format!("open session_events failed: {err}"))
            })?;
            let existing = events
                .get(key.as_slice())
                .map_err(|err| {
                    JournalError::Verification(format!("read session event failed: {err}"))
                })?
                .ok_or_else(|| {
                    JournalError::Verification(format!(
                        "session event not found at sequence {sequence}"
                    ))
                })?;
            let mut stored: StoredEvent = postcard::from_bytes(existing.value())?;
            drop(existing);
            stored.event = event;
            let bytes = postcard::to_allocvec(&stored)?;
            events
                .insert(key.as_slice(), bytes.as_slice())
                .map_err(|err| {
                    JournalError::Verification(format!("overwrite session event failed: {err}"))
                })?;
        }
        write_txn.commit().map_err(|err| {
            JournalError::Verification(format!("commit session event overwrite failed: {err}"))
        })?;
        Ok(())
    }

    /// Test-only: overwrites the stored session head hash.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn overwrite_head_for_testing(&mut self, head: Hash) -> Result<()> {
        let session_key = session_key_bytes(self.session_id);
        let head_bytes = postcard::to_allocvec(&Some(head))?;
        let write_txn = self
            .store
            .database()
            .begin_write()
            .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
        {
            let mut heads = write_txn.open_table(SESSION_HEADS_TABLE).map_err(|err| {
                JournalError::Verification(format!("open session_heads failed: {err}"))
            })?;
            heads
                .insert(session_key.as_slice(), head_bytes.as_slice())
                .map_err(|err| {
                    JournalError::Verification(format!("overwrite session head failed: {err}"))
                })?;
        }
        write_txn.commit().map_err(|err| {
            JournalError::Verification(format!("commit session head overwrite failed: {err}"))
        })?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn raw_db(&self) -> &Database {
        self.store.database()
    }
}

impl SessionGraph for RedbSessionGraph {
    fn session_id(&self) -> uuid::Uuid {
        self.session_id
    }

    fn append(&mut self, kind: EventKind) -> Result<Hash> {
        validate_event_kind_invariants(&kind)?;
        let current_head = self.head()?;
        // TODO(Item 4.x): avoid O(n) append by storing next sequence in session_heads.
        let sequence = self.history()?.len() as u64;
        let parents = match current_head {
            None => {
                if !matches!(kind, EventKind::SessionStart { .. }) {
                    return Err(JournalError::Verification(
                        "first event must be SessionStart".to_owned(),
                    ));
                }
                Vec::new()
            }
            Some(head) => {
                if matches!(kind, EventKind::SessionStart { .. }) {
                    return Err(JournalError::Verification(
                        "SessionStart cannot be appended to non-empty session".to_owned(),
                    ));
                }
                vec![head]
            }
        };
        let event = Event {
            parents,
            kind,
            emitted_at: time::OffsetDateTime::now_utc(),
            sequence,
        };
        let hash = event.content_hash(self.store.algorithm())?;
        let stored = StoredEvent {
            hash: hash.clone(),
            event,
        };
        let value = postcard::to_allocvec(&stored)?;
        let key = event_key(self.session_id, sequence);
        let session_key = session_key_bytes(self.session_id);
        let head_bytes = postcard::to_allocvec(&Some(hash.clone()))?;

        let write_txn = self
            .store
            .database()
            .begin_write()
            .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
        {
            let mut events = write_txn.open_table(SESSION_EVENTS_TABLE).map_err(|err| {
                JournalError::Verification(format!("open session_events failed: {err}"))
            })?;
            events
                .insert(key.as_slice(), value.as_slice())
                .map_err(|err| {
                    JournalError::Verification(format!("insert session event failed: {err}"))
                })?;
            let mut heads = write_txn.open_table(SESSION_HEADS_TABLE).map_err(|err| {
                JournalError::Verification(format!("open session_heads failed: {err}"))
            })?;
            heads
                .insert(session_key.as_slice(), head_bytes.as_slice())
                .map_err(|err| {
                    JournalError::Verification(format!("update session head failed: {err}"))
                })?;
        }
        write_txn.commit().map_err(|err| {
            JournalError::Verification(format!("commit session append failed: {err}"))
        })?;
        Ok(hash)
    }

    fn head(&self) -> Result<Option<Hash>> {
        let key = session_key_bytes(self.session_id);
        let read_txn = self
            .store
            .database()
            .begin_read()
            .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
        let heads = read_txn.open_table(SESSION_HEADS_TABLE).map_err(|err| {
            JournalError::Verification(format!("open session_heads failed: {err}"))
        })?;
        let value = heads.get(key.as_slice()).map_err(|err| {
            JournalError::Verification(format!("read session_heads failed: {err}"))
        })?;
        let bytes = value.ok_or(JournalError::SessionNotFound(self.session_id))?;
        let head: Option<Hash> = postcard::from_bytes(bytes.value())?;
        Ok(head)
    }

    fn history(&self) -> Result<Vec<(Hash, Event)>> {
        let read_txn = self
            .store
            .database()
            .begin_read()
            .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
        let table = read_txn.open_table(SESSION_EVENTS_TABLE).map_err(|err| {
            JournalError::Verification(format!("open session_events failed: {err}"))
        })?;
        let iter = table.iter().map_err(|err| {
            JournalError::Verification(format!("iterate session_events failed: {err}"))
        })?;
        let mut out: Vec<(u64, Hash, Event)> = Vec::new();
        for item in iter {
            let (key, value) = item.map_err(|err| {
                JournalError::Verification(format!("iterate session_events item failed: {err}"))
            })?;
            // Skip keys that don't match the current 24-byte format (UUID 16 + seq u64 8).
            // This makes the journal forward-compatible with databases written by older akmon
            // versions that used a different key layout (e.g. 32-byte keys). Rather than
            // crashing the entire session on a schema mismatch, we silently skip foreign-format
            // keys — they belong to sessions from the old binary and are not readable anyway.
            let (sid, seq) = match parse_event_key(key.value()) {
                Ok(pair) => pair,
                Err(_) => continue,
            };
            if sid != self.session_id {
                continue;
            }
            let stored: StoredEvent = postcard::from_bytes(value.value())?;
            out.push((seq, stored.hash, stored.event));
        }
        out.sort_by_key(|(seq, _, _)| *seq);
        Ok(out.into_iter().map(|(_, h, e)| (h, e)).collect())
    }

    fn verify(&self) -> Result<VerificationReport> {
        let history = self.history()?;
        let stored_head = self.head()?;
        verify_history_against_store(&history, stored_head, self.store.as_ref())
    }

    fn import_verified_linear_history(&mut self, events: &[(Hash, Event)]) -> Result<()> {
        ensure_empty_import_target(self.head()?, self.history()?.len())?;
        verify_import_precondition(events, self.store.as_ref())?;

        let session_key = session_key_bytes(self.session_id);
        let write_txn = self
            .store
            .database()
            .begin_write()
            .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
        {
            let mut heads = write_txn.open_table(SESSION_HEADS_TABLE).map_err(|err| {
                JournalError::Verification(format!("open session_heads failed: {err}"))
            })?;
            let stored_head: Option<Hash> = {
                let head_row = heads
                    .get(session_key.as_slice())
                    .map_err(|err| {
                        JournalError::Verification(format!("read session_heads failed: {err}"))
                    })?
                    .ok_or(JournalError::SessionNotFound(self.session_id))?;
                postcard::from_bytes(head_row.value())?
            };
            if stored_head.is_some() {
                return Err(JournalError::Verification(
                    "import target session is not empty".into(),
                ));
            }

            let mut events_table = write_txn.open_table(SESSION_EVENTS_TABLE).map_err(|err| {
                JournalError::Verification(format!("open session_events failed: {err}"))
            })?;
            for item in events_table.iter().map_err(|err| {
                JournalError::Verification(format!("iterate session_events failed: {err}"))
            })? {
                let (k, _) = item.map_err(|err| {
                    JournalError::Verification(format!("iterate session_events item failed: {err}"))
                })?;
                let (sid, _) = match parse_event_key(k.value()) {
                    Ok(pair) => pair,
                    Err(_) => continue, // skip keys from older akmon schema versions
                };
                if sid == self.session_id {
                    return Err(JournalError::Verification(
                        "import target session is not empty".into(),
                    ));
                }
            }

            for (seq, (hash, event)) in events.iter().enumerate() {
                let seq = u64::try_from(seq).map_err(|_| {
                    JournalError::Verification("import event index overflow".into())
                })?;
                let key = event_key(self.session_id, seq);
                let stored = StoredEvent {
                    hash: hash.clone(),
                    event: event.clone(),
                };
                let value = postcard::to_allocvec(&stored)?;
                events_table
                    .insert(key.as_slice(), value.as_slice())
                    .map_err(|err| {
                        JournalError::Verification(format!("insert session event failed: {err}"))
                    })?;
            }

            let terminal = events.last().map(|(h, _)| h.clone()).ok_or_else(|| {
                JournalError::Verification("internal: empty events after precondition".into())
            })?;
            let head_bytes = postcard::to_allocvec(&Some(terminal))?;
            heads
                .insert(session_key.as_slice(), head_bytes.as_slice())
                .map_err(|err| {
                    JournalError::Verification(format!("update session head failed: {err}"))
                })?;
        }
        write_txn.commit().map_err(|err| {
            JournalError::Verification(format!("commit session import failed: {err}"))
        })?;
        Ok(())
    }
}

/// In-memory session graph implementation for tests and consumer test utilities.
#[cfg(any(test, feature = "test-utils"))]
pub struct MemorySessionGraph {
    store: Arc<crate::object_store::MemoryObjectStore>,
    session_id: uuid::Uuid,
    events: Vec<(Hash, Event)>,
}

#[cfg(any(test, feature = "test-utils"))]
impl MemorySessionGraph {
    /// Creates a new in-memory session graph for `session_id`.
    pub fn open_new(
        store: Arc<crate::object_store::MemoryObjectStore>,
        session_id: uuid::Uuid,
    ) -> Self {
        Self {
            store,
            session_id,
            events: Vec::new(),
        }
    }

    /// Test-only: overwrites the event payload at `sequence` while preserving stored hash bytes.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn overwrite_event_at_sequence_for_testing(
        &mut self,
        sequence: u64,
        event: Event,
    ) -> Result<()> {
        let idx = usize::try_from(sequence).map_err(|_| {
            JournalError::Verification(format!("sequence {sequence} does not fit usize"))
        })?;
        let slot = self.events.get_mut(idx).ok_or_else(|| {
            JournalError::Verification(format!("session event not found at sequence {sequence}"))
        })?;
        slot.1 = event;
        Ok(())
    }

    /// Test-only: overwrites the stored session head hash.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn overwrite_head_for_testing(&mut self, head: Hash) -> Result<()> {
        if let Some((stored_head, _)) = self.events.last_mut() {
            *stored_head = head;
            Ok(())
        } else {
            Err(JournalError::Verification(
                "cannot overwrite head for empty session".to_owned(),
            ))
        }
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl SessionGraph for MemorySessionGraph {
    fn session_id(&self) -> uuid::Uuid {
        self.session_id
    }

    fn append(&mut self, kind: EventKind) -> Result<Hash> {
        validate_event_kind_invariants(&kind)?;
        let parents = if let Some((head, _)) = self.events.last() {
            if matches!(kind, EventKind::SessionStart { .. }) {
                return Err(JournalError::Verification(
                    "SessionStart cannot be appended to non-empty session".to_owned(),
                ));
            }
            vec![head.clone()]
        } else {
            if !matches!(kind, EventKind::SessionStart { .. }) {
                return Err(JournalError::Verification(
                    "first event must be SessionStart".to_owned(),
                ));
            }
            Vec::new()
        };
        let event = Event {
            parents,
            kind,
            emitted_at: time::OffsetDateTime::now_utc(),
            sequence: self.events.len() as u64,
        };
        let hash = event.content_hash(self.store.algorithm())?;
        self.events.push((hash.clone(), event));
        Ok(hash)
    }

    fn head(&self) -> Result<Option<Hash>> {
        Ok(self.events.last().map(|(h, _)| h.clone()))
    }

    fn history(&self) -> Result<Vec<(Hash, Event)>> {
        Ok(self.events.clone())
    }

    fn verify(&self) -> Result<VerificationReport> {
        let stored_head = self.head()?;
        verify_history_against_store(&self.events, stored_head, self.store.as_ref())
    }

    fn import_verified_linear_history(&mut self, events: &[(Hash, Event)]) -> Result<()> {
        ensure_empty_import_target(self.head()?, self.events.len())?;
        verify_import_precondition(events, self.store.as_ref())?;
        self.events
            .extend(events.iter().map(|(h, e)| (h.clone(), e.clone())));
        Ok(())
    }
}

fn validate_event_kind_invariants(kind: &EventKind) -> Result<()> {
    if let EventKind::ProviderCall { attempts, .. } = kind {
        if attempts.is_empty() {
            return Err(JournalError::Verification(
                "ProviderCall attempts must contain at least one attempt".to_owned(),
            ));
        }
        for (idx, attempt) in attempts.iter().enumerate() {
            let expected = (idx as u32) + 1;
            if attempt.attempt_number != expected {
                return Err(JournalError::Verification(format!(
                    "ProviderCall attempts must be 1-indexed and contiguous: expected {}, found {}",
                    expected, attempt.attempt_number
                )));
            }
            if idx > 0 && attempt.started_at < attempts[idx - 1].started_at {
                return Err(JournalError::Verification(
                    "ProviderCall attempt started_at must be non-decreasing".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn ensure_graph_tables(db: &Database) -> Result<()> {
    let write_txn = db
        .begin_write()
        .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
    {
        write_txn.open_table(SESSION_HEADS_TABLE).map_err(|err| {
            JournalError::Verification(format!("open session_heads failed: {err}"))
        })?;
        write_txn.open_table(SESSION_EVENTS_TABLE).map_err(|err| {
            JournalError::Verification(format!("open session_events failed: {err}"))
        })?;
    }
    write_txn.commit().map_err(|err| {
        JournalError::Verification(format!("commit graph table creation failed: {err}"))
    })?;
    Ok(())
}

fn session_key_bytes(session_id: uuid::Uuid) -> [u8; 16] {
    *session_id.as_bytes()
}

fn event_key(session_id: uuid::Uuid, sequence: u64) -> [u8; 24] {
    let mut key = [0_u8; 24];
    key[..16].copy_from_slice(session_id.as_bytes());
    key[16..].copy_from_slice(&sequence.to_be_bytes());
    key
}

fn parse_event_key(bytes: &[u8]) -> Result<(uuid::Uuid, u64)> {
    if bytes.len() != 24 {
        return Err(JournalError::Verification(format!(
            "invalid session event key length: {}",
            bytes.len()
        )));
    }
    let sid = uuid::Uuid::from_slice(&bytes[..16]).map_err(|err| {
        JournalError::Verification(format!("invalid session id in event key: {err}"))
    })?;
    let mut seq = [0_u8; 8];
    seq.copy_from_slice(&bytes[16..24]);
    Ok((sid, u64::from_be_bytes(seq)))
}

/// Returns `true` when `session_heads` contains a row for `session_id`.
///
/// This is true for an empty session created via [`RedbSessionGraph::open_new`] (head row stores
/// `None`) as well as for sessions with persisted events.
pub fn session_head_row_exists(store: &RedbObjectStore, session_id: uuid::Uuid) -> Result<bool> {
    let key = session_key_bytes(session_id);
    let read_txn = store
        .database()
        .begin_read()
        .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
    let heads = read_txn
        .open_table(SESSION_HEADS_TABLE)
        .map_err(|err| JournalError::Verification(format!("open session_heads failed: {err}")))?;
    let row = heads
        .get(key.as_slice())
        .map_err(|err| JournalError::Verification(format!("read session_heads failed: {err}")))?;
    Ok(row.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{AttemptRecord, AttemptStatus};
    use crate::hash::HashAlgorithm;
    use crate::object_store::MemoryObjectStore;

    fn make_hash(store: &dyn ObjectStore, seed: u8) -> Hash {
        store.put(&[seed; 32]).unwrap_or_else(|_| unreachable!())
    }

    fn session_start(store: &dyn ObjectStore) -> EventKind {
        EventKind::SessionStart {
            cwd_hash: make_hash(store, 0x11),
            config_hash: make_hash(store, 0x12),
        }
    }

    fn provider_call(store: &dyn ObjectStore) -> EventKind {
        EventKind::ProviderCall {
            provider_id: "p".to_owned(),
            attempts: vec![AttemptRecord {
                attempt_number: 1,
                started_at: time::OffsetDateTime::now_utc(),
                ended_at: time::OffsetDateTime::now_utc(),
                status: AttemptStatus::Success,
                request_hash: make_hash(store, 0x20),
                response_hash: Some(make_hash(store, 0x21)),
                stream_hash: Some(make_hash(store, 0x22)),
                error_message: None,
            }],
            stream_hash: Some(make_hash(store, 0x23)),
        }
    }

    fn ts(seconds: i64) -> time::OffsetDateTime {
        time::OffsetDateTime::from_unix_timestamp(seconds).unwrap_or_else(|_| unreachable!())
    }

    fn provider_call_with_attempts(
        store: &dyn ObjectStore,
        attempts: Vec<AttemptRecord>,
    ) -> EventKind {
        EventKind::ProviderCall {
            provider_id: "p".to_owned(),
            attempts,
            stream_hash: Some(make_hash(store, 0x23)),
        }
    }

    #[test]
    fn empty_graph_head_and_history() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("empty.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let graph = RedbSessionGraph::open_new(store, uuid::Uuid::new_v4())
            .unwrap_or_else(|_| unreachable!());
        assert!(graph.head().unwrap_or_else(|_| unreachable!()).is_none());
        assert!(
            graph
                .history()
                .unwrap_or_else(|_| unreachable!())
                .is_empty()
        );
    }

    #[test]
    fn linear_append_history_head_and_sequence() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("linear.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let sid = uuid::Uuid::new_v4();
        let mut graph =
            RedbSessionGraph::open_new(Arc::clone(&store), sid).unwrap_or_else(|_| unreachable!());

        let h0 = graph
            .append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(EventKind::UserTurn {
                prompt_hash: make_hash(store.as_ref(), 0x30),
            })
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(provider_call(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        let h3 = graph
            .append(EventKind::SessionEnd {
                summary_hash: Some(make_hash(store.as_ref(), 0x31)),
            })
            .unwrap_or_else(|_| unreachable!());
        let history = graph.history().unwrap_or_else(|_| unreachable!());
        assert_eq!(history.len(), 4);
        assert_eq!(history[0].1.sequence, 0);
        assert_eq!(history[1].1.sequence, 1);
        assert_eq!(history[2].1.sequence, 2);
        assert_eq!(history[3].1.sequence, 3);
        assert_eq!(graph.head().unwrap_or_else(|_| unreachable!()), Some(h3));
        assert_eq!(history[0].0, h0);
    }

    #[test]
    fn session_start_enforcement() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let mut graph = MemorySessionGraph::open_new(store.clone(), uuid::Uuid::new_v4());
        let start = graph.append(session_start(store.as_ref()));
        assert!(start.is_ok());
        let second_start = graph.append(session_start(store.as_ref()));
        assert!(second_start.is_err());

        let mut graph2 = MemorySessionGraph::open_new(store, uuid::Uuid::new_v4());
        let bad_first = graph2.append(EventKind::UserTurn {
            prompt_hash: Hash::from_bytes(HashAlgorithm::Sha256, [0xAA; 32]),
        });
        assert!(bad_first.is_err());
    }

    #[test]
    fn reopen_roundtrip() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("reopen.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let sid = uuid::Uuid::new_v4();
        {
            let mut graph = RedbSessionGraph::open_new(Arc::clone(&store), sid)
                .unwrap_or_else(|_| unreachable!());
            graph
                .append(session_start(store.as_ref()))
                .unwrap_or_else(|_| unreachable!());
            graph
                .append(EventKind::UserTurn {
                    prompt_hash: make_hash(store.as_ref(), 0x40),
                })
                .unwrap_or_else(|_| unreachable!());
        }
        let reopened = RedbSessionGraph::reopen(store, sid).unwrap_or_else(|_| unreachable!());
        assert_eq!(
            reopened.history().unwrap_or_else(|_| unreachable!()).len(),
            2
        );
        assert!(reopened.head().unwrap_or_else(|_| unreachable!()).is_some());
    }

    #[test]
    fn multiple_sessions_same_database_are_isolated() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("multi.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let sid_a = uuid::Uuid::new_v4();
        let sid_b = uuid::Uuid::new_v4();
        let mut a = RedbSessionGraph::open_new(Arc::clone(&store), sid_a)
            .unwrap_or_else(|_| unreachable!());
        let mut b = RedbSessionGraph::open_new(Arc::clone(&store), sid_b)
            .unwrap_or_else(|_| unreachable!());

        a.append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        a.append(EventKind::SessionEnd { summary_hash: None })
            .unwrap_or_else(|_| unreachable!());
        b.append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());

        let ra =
            RedbSessionGraph::reopen(Arc::clone(&store), sid_a).unwrap_or_else(|_| unreachable!());
        let rb = RedbSessionGraph::reopen(store, sid_b).unwrap_or_else(|_| unreachable!());
        assert_eq!(ra.history().unwrap_or_else(|_| unreachable!()).len(), 2);
        assert_eq!(rb.history().unwrap_or_else(|_| unreachable!()).len(), 1);
    }

    #[test]
    fn verify_clean_linear_graph() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("verify_clean.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let sid = uuid::Uuid::new_v4();
        let mut graph =
            RedbSessionGraph::open_new(store.clone(), sid).unwrap_or_else(|_| unreachable!());
        graph
            .append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(EventKind::UserTurn {
                prompt_hash: make_hash(store.as_ref(), 0x50),
            })
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .unwrap_or_else(|_| unreachable!());
        let report = graph.verify().unwrap_or_else(|_| unreachable!());
        assert_eq!(report.hash_mismatches.len(), 0);
        assert_eq!(report.broken_parent_links.len(), 0);
        assert_eq!(report.sequence_violations.len(), 0);
        assert!(report.head_mismatch.is_none());
        assert_eq!(report.missing_objects.len(), 0);
        assert_eq!(report.object_hash_mismatches.len(), 0);
        assert_eq!(report.session_end_count, 1);
        assert!(report.session_end_is_terminal);
        assert!(report.is_clean());
        assert_eq!(report.events_checked, 3);
        assert!(report.objects_checked >= 3);
    }

    #[test]
    fn append_rejects_provider_call_with_empty_attempts() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let mut graph = MemorySessionGraph::open_new(store.clone(), uuid::Uuid::new_v4());
        graph
            .append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        let result = graph.append(provider_call_with_attempts(store.as_ref(), Vec::new()));
        assert!(result.is_err());
    }

    #[test]
    fn append_rejects_provider_call_with_non_contiguous_attempt_numbers() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let mut graph = MemorySessionGraph::open_new(store.clone(), uuid::Uuid::new_v4());
        graph
            .append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        let attempts = vec![
            AttemptRecord {
                attempt_number: 1,
                started_at: ts(10),
                ended_at: ts(11),
                status: AttemptStatus::RateLimited,
                request_hash: make_hash(store.as_ref(), 0x70),
                response_hash: None,
                stream_hash: None,
                error_message: Some("429".to_owned()),
            },
            AttemptRecord {
                attempt_number: 3,
                started_at: ts(12),
                ended_at: ts(13),
                status: AttemptStatus::Success,
                request_hash: make_hash(store.as_ref(), 0x71),
                response_hash: Some(make_hash(store.as_ref(), 0x72)),
                stream_hash: None,
                error_message: None,
            },
        ];
        let result = graph.append(provider_call_with_attempts(store.as_ref(), attempts));
        assert!(result.is_err());
    }

    #[test]
    fn append_rejects_provider_call_with_decreasing_started_at() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let mut graph = MemorySessionGraph::open_new(store.clone(), uuid::Uuid::new_v4());
        graph
            .append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        let attempts = vec![
            AttemptRecord {
                attempt_number: 1,
                started_at: ts(20),
                ended_at: ts(21),
                status: AttemptStatus::RateLimited,
                request_hash: make_hash(store.as_ref(), 0x73),
                response_hash: None,
                stream_hash: None,
                error_message: Some("429".to_owned()),
            },
            AttemptRecord {
                attempt_number: 2,
                started_at: ts(19),
                ended_at: ts(22),
                status: AttemptStatus::Success,
                request_hash: make_hash(store.as_ref(), 0x74),
                response_hash: Some(make_hash(store.as_ref(), 0x75)),
                stream_hash: None,
                error_message: None,
            },
        ];
        let result = graph.append(provider_call_with_attempts(store.as_ref(), attempts));
        assert!(result.is_err());
    }

    #[test]
    fn verify_detects_corruption_all_four_cases() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("verify_corrupt.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let sid = uuid::Uuid::new_v4();
        let mut graph =
            RedbSessionGraph::open_new(Arc::clone(&store), sid).unwrap_or_else(|_| unreachable!());
        graph
            .append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        let _h1 = graph
            .append(EventKind::UserTurn {
                prompt_hash: make_hash(store.as_ref(), 0x60),
            })
            .unwrap_or_else(|_| unreachable!());
        let h2 = graph
            .append(EventKind::AssistantTurn {
                message_hash: make_hash(store.as_ref(), 0x61),
                tool_calls_hash: None,
            })
            .unwrap_or_else(|_| unreachable!());

        // Case 1: delete referenced object -> missing_objects.
        let missing_hash = make_hash(store.as_ref(), 0x61);
        {
            let tx = graph
                .raw_db()
                .begin_write()
                .unwrap_or_else(|_| unreachable!());
            {
                let mut objects = tx
                    .open_table(redb::TableDefinition::<&[u8], &[u8]>::new("objects"))
                    .unwrap_or_else(|_| unreachable!());
                objects
                    .remove(missing_hash.bytes.as_slice())
                    .unwrap_or_else(|_| unreachable!());
            }
            tx.commit().unwrap_or_else(|_| unreachable!());
        }

        // Case 2: mutate stored bytes without updating stored hash -> hash_mismatches.
        {
            let tx = graph
                .raw_db()
                .begin_write()
                .unwrap_or_else(|_| unreachable!());
            {
                let mut events = tx
                    .open_table(SESSION_EVENTS_TABLE)
                    .unwrap_or_else(|_| unreachable!());
                let key = event_key(sid, 2);
                let existing = events
                    .get(key.as_slice())
                    .unwrap_or_else(|_| unreachable!())
                    .unwrap_or_else(|| unreachable!());
                let mut stored: StoredEvent =
                    postcard::from_bytes(existing.value()).unwrap_or_else(|_| unreachable!());
                drop(existing);
                stored.event.sequence = 22;
                let bytes = postcard::to_allocvec(&stored).unwrap_or_else(|_| unreachable!());
                events
                    .insert(key.as_slice(), bytes.as_slice())
                    .unwrap_or_else(|_| unreachable!());
            }
            tx.commit().unwrap_or_else(|_| unreachable!());
        }

        // Case 3 + 4: skip sequence number and wrong parent.
        {
            let tx = graph
                .raw_db()
                .begin_write()
                .unwrap_or_else(|_| unreachable!());
            {
                let mut events = tx
                    .open_table(SESSION_EVENTS_TABLE)
                    .unwrap_or_else(|_| unreachable!());
                let bad_event = Event {
                    parents: vec![Hash::from_bytes(HashAlgorithm::Sha256, [0xEF; 32])],
                    kind: EventKind::SessionEnd { summary_hash: None },
                    emitted_at: time::OffsetDateTime::now_utc(),
                    sequence: 4,
                };
                let bad_hash = bad_event
                    .content_hash(HashAlgorithm::Sha256)
                    .unwrap_or_else(|_| unreachable!());
                let stored = StoredEvent {
                    hash: bad_hash,
                    event: bad_event,
                };
                let bytes = postcard::to_allocvec(&stored).unwrap_or_else(|_| unreachable!());
                let key = event_key(sid, 4);
                events
                    .insert(key.as_slice(), bytes.as_slice())
                    .unwrap_or_else(|_| unreachable!());
            }
            {
                let mut heads = tx
                    .open_table(SESSION_HEADS_TABLE)
                    .unwrap_or_else(|_| unreachable!());
                let head = postcard::to_allocvec(&Some(h2)).unwrap_or_else(|_| unreachable!());
                heads
                    .insert(session_key_bytes(sid).as_slice(), head.as_slice())
                    .unwrap_or_else(|_| unreachable!());
            }
            tx.commit().unwrap_or_else(|_| unreachable!());
        }

        let report = graph.verify().unwrap_or_else(|_| unreachable!());
        assert!(!report.missing_objects.is_empty());
        assert!(!report.hash_mismatches.is_empty());
        assert!(!report.sequence_violations.is_empty());
        assert!(!report.broken_parent_links.is_empty());
        assert!(report.head_mismatch.is_some());
    }

    #[test]
    fn verify_detects_head_mismatch_when_head_record_is_corrupted() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("verify_head_corrupt.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let sid = uuid::Uuid::new_v4();
        let mut graph =
            RedbSessionGraph::open_new(Arc::clone(&store), sid).unwrap_or_else(|_| unreachable!());
        graph
            .append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        let computed_end = graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .unwrap_or_else(|_| unreachable!());

        let corrupted_stored = Hash::from_bytes(HashAlgorithm::Sha256, [0xCC; 32]);
        {
            let tx = graph
                .raw_db()
                .begin_write()
                .unwrap_or_else(|_| unreachable!());
            {
                let mut heads = tx
                    .open_table(SESSION_HEADS_TABLE)
                    .unwrap_or_else(|_| unreachable!());
                let head = postcard::to_allocvec(&Some(corrupted_stored.clone()))
                    .unwrap_or_else(|_| unreachable!());
                heads
                    .insert(session_key_bytes(sid).as_slice(), head.as_slice())
                    .unwrap_or_else(|_| unreachable!());
            }
            tx.commit().unwrap_or_else(|_| unreachable!());
        }

        let report = graph.verify().unwrap_or_else(|_| unreachable!());
        assert_eq!(report.head_mismatch, Some((corrupted_stored, computed_end)));
    }

    #[test]
    fn t_verify_detects_corrupted_object_bytes() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("verify_object_corrupt.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let sid = uuid::Uuid::new_v4();
        let mut graph =
            RedbSessionGraph::open_new(Arc::clone(&store), sid).unwrap_or_else(|_| unreachable!());
        graph
            .append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        let prompt = make_hash(store.as_ref(), 0x80);
        graph
            .append(EventKind::UserTurn {
                prompt_hash: prompt.clone(),
            })
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .unwrap_or_else(|_| unreachable!());
        store
            .overwrite_object_bytes_for_testing(&prompt, b"tampered-prompt")
            .unwrap_or_else(|_| unreachable!());
        let report = graph.verify().unwrap_or_else(|_| unreachable!());
        assert!(
            report.object_hash_mismatches.contains(&prompt),
            "expected object hash mismatch for {:?}",
            report.object_hash_mismatches
        );
        assert!(!report.is_clean());
    }

    #[test]
    fn t_verify_flags_missing_session_end() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let mut graph = MemorySessionGraph::open_new(store.clone(), uuid::Uuid::new_v4());
        graph
            .append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(EventKind::UserTurn {
                prompt_hash: make_hash(store.as_ref(), 0x81),
            })
            .unwrap_or_else(|_| unreachable!());
        let report = graph.verify().unwrap_or_else(|_| unreachable!());
        assert_eq!(report.session_end_count, 0);
        assert!(!report.session_end_is_terminal);
        assert!(!report.is_clean());
    }

    #[test]
    fn t_verify_flags_duplicate_session_end() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let mut graph = MemorySessionGraph::open_new(store.clone(), uuid::Uuid::new_v4());
        graph
            .append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .unwrap_or_else(|_| unreachable!());
        let report = graph.verify().unwrap_or_else(|_| unreachable!());
        assert_eq!(report.session_end_count, 2);
        assert!(!report.session_end_is_terminal);
        assert!(!report.is_clean());
    }

    #[test]
    fn t_verify_flags_session_end_not_terminal() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let mut graph = MemorySessionGraph::open_new(store.clone(), uuid::Uuid::new_v4());
        graph
            .append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(EventKind::UserTurn {
                prompt_hash: make_hash(store.as_ref(), 0x82),
            })
            .unwrap_or_else(|_| unreachable!());
        let report = graph.verify().unwrap_or_else(|_| unreachable!());
        assert_eq!(report.session_end_count, 1);
        assert!(!report.session_end_is_terminal);
        assert!(!report.is_clean());
    }

    #[test]
    fn t_verify_passes_for_correct_session() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let mut graph = MemorySessionGraph::open_new(store.clone(), uuid::Uuid::new_v4());
        graph
            .append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(EventKind::UserTurn {
                prompt_hash: make_hash(store.as_ref(), 0x83),
            })
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(provider_call(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(EventKind::AssistantTurn {
                message_hash: make_hash(store.as_ref(), 0x84),
                tool_calls_hash: None,
            })
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(EventKind::SessionEnd {
                summary_hash: Some(make_hash(store.as_ref(), 0x85)),
            })
            .unwrap_or_else(|_| unreachable!());
        let report = graph.verify().unwrap_or_else(|_| unreachable!());
        assert!(report.object_hash_mismatches.is_empty());
        assert_eq!(report.session_end_count, 1);
        assert!(report.session_end_is_terminal);
        assert!(report.is_clean());
    }

    fn linear_history_three_events_redb(
        store: &Arc<RedbObjectStore>,
        sid: uuid::Uuid,
    ) -> Vec<(Hash, Event)> {
        let mut g =
            RedbSessionGraph::open_new(Arc::clone(store), sid).unwrap_or_else(|_| unreachable!());
        g.append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        g.append(EventKind::UserTurn {
            prompt_hash: make_hash(store.as_ref(), 0xA0),
        })
        .unwrap_or_else(|_| unreachable!());
        g.append(EventKind::SessionEnd { summary_hash: None })
            .unwrap_or_else(|_| unreachable!());
        g.history().unwrap_or_else(|_| unreachable!())
    }

    #[test]
    fn t_session_exists_returns_true_for_empty_open_new_session() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("reopen_empty.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let sid = uuid::Uuid::new_v4();
        {
            let _g = RedbSessionGraph::open_new(Arc::clone(&store), sid)
                .unwrap_or_else(|_| unreachable!());
        }
        assert!(
            session_head_row_exists(store.as_ref(), sid).unwrap_or_else(|_| unreachable!()),
            "expected session_heads row after open_new"
        );
    }

    #[test]
    fn t_session_exists_returns_false_for_unknown_session() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("unknown_sid.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let sid_a = uuid::Uuid::new_v4();
        RedbSessionGraph::open_new(Arc::clone(&store), sid_a).unwrap_or_else(|_| unreachable!());
        let sid_other = uuid::Uuid::new_v4();
        assert!(
            !session_head_row_exists(store.as_ref(), sid_other).unwrap_or_else(|_| unreachable!())
        );
    }

    #[test]
    fn t_session_exists_returns_true_for_session_with_events() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("with_events.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let sid = uuid::Uuid::new_v4();
        let mut graph =
            RedbSessionGraph::open_new(Arc::clone(&store), sid).unwrap_or_else(|_| unreachable!());
        graph
            .append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(EventKind::SessionEnd { summary_hash: None })
            .unwrap_or_else(|_| unreachable!());
        assert!(session_head_row_exists(store.as_ref(), sid).unwrap_or_else(|_| unreachable!()));
    }

    #[test]
    fn import_verified_linear_history_redb_roundtrip_second_session() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("import_two_sids.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let sid_src = uuid::Uuid::new_v4();
        let events = linear_history_three_events_redb(&store, sid_src);
        let sid_dst = uuid::Uuid::new_v4();
        let mut dst = RedbSessionGraph::open_new(Arc::clone(&store), sid_dst)
            .unwrap_or_else(|_| unreachable!());
        dst.import_verified_linear_history(&events)
            .unwrap_or_else(|_| unreachable!());
        assert!(dst.verify().unwrap_or_else(|_| unreachable!()).is_clean());
        assert_eq!(dst.history().unwrap_or_else(|_| unreachable!()), events);
    }

    #[test]
    fn import_verified_linear_history_redb_rejects_non_empty_target() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("import_nonempty.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let sid_src = uuid::Uuid::new_v4();
        let events = linear_history_three_events_redb(&store, sid_src);
        let sid_dst = uuid::Uuid::new_v4();
        let mut dst = RedbSessionGraph::open_new(Arc::clone(&store), sid_dst)
            .unwrap_or_else(|_| unreachable!());
        dst.append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        let err = dst
            .import_verified_linear_history(&events)
            .expect_err("non-empty target");
        assert!(
            err.to_string().contains("not empty"),
            "unexpected err: {err}"
        );
    }

    #[test]
    fn import_verified_linear_history_redb_rejects_bad_content_hash_tuple() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("import_bad_hash.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let sid_src = uuid::Uuid::new_v4();
        let mut events = linear_history_three_events_redb(&store, sid_src);
        events[0].0 = Hash::from_bytes(HashAlgorithm::Sha256, [0xEE; 32]);
        let sid_dst = uuid::Uuid::new_v4();
        let mut dst = RedbSessionGraph::open_new(Arc::clone(&store), sid_dst)
            .unwrap_or_else(|_| unreachable!());
        let err = dst
            .import_verified_linear_history(&events)
            .expect_err("bad hash");
        assert!(
            err.to_string().contains("not clean"),
            "unexpected err: {err}"
        );
    }

    #[test]
    fn import_verified_linear_history_redb_rejects_empty_events() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("import_empty_slice.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let sid_dst = uuid::Uuid::new_v4();
        let mut dst = RedbSessionGraph::open_new(Arc::clone(&store), sid_dst)
            .unwrap_or_else(|_| unreachable!());
        let err = dst
            .import_verified_linear_history(&[])
            .expect_err("empty slice");
        assert!(err.to_string().contains("empty"), "unexpected err: {err}");
    }

    #[test]
    fn import_verified_linear_history_memory_roundtrip() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let sid_src = uuid::Uuid::new_v4();
        let mut src = MemorySessionGraph::open_new(Arc::clone(&store), sid_src);
        src.append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        src.append(EventKind::UserTurn {
            prompt_hash: make_hash(store.as_ref(), 0xB1),
        })
        .unwrap_or_else(|_| unreachable!());
        src.append(EventKind::SessionEnd { summary_hash: None })
            .unwrap_or_else(|_| unreachable!());
        let events = src.history().unwrap_or_else(|_| unreachable!());
        let sid_dst = uuid::Uuid::new_v4();
        let mut dst = MemorySessionGraph::open_new(store, sid_dst);
        dst.import_verified_linear_history(&events)
            .unwrap_or_else(|_| unreachable!());
        assert!(dst.verify().unwrap_or_else(|_| unreachable!()).is_clean());
        assert_eq!(dst.history().unwrap_or_else(|_| unreachable!()), events);
    }

    #[test]
    fn import_verified_linear_history_second_call_fails() {
        let store = Arc::new(MemoryObjectStore::new(HashAlgorithm::Sha256));
        let sid_src = uuid::Uuid::new_v4();
        let mut src = MemorySessionGraph::open_new(Arc::clone(&store), sid_src);
        src.append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        src.append(EventKind::SessionEnd { summary_hash: None })
            .unwrap_or_else(|_| unreachable!());
        let events = src.history().unwrap_or_else(|_| unreachable!());
        let sid_dst = uuid::Uuid::new_v4();
        let mut dst = MemorySessionGraph::open_new(store, sid_dst);
        dst.import_verified_linear_history(&events)
            .unwrap_or_else(|_| unreachable!());
        let err = dst
            .import_verified_linear_history(&events)
            .expect_err("second import");
        assert!(
            err.to_string().contains("not empty"),
            "unexpected err: {err}"
        );
    }

    /// Regression test: history() and import_verified_linear_history() must survive a journal
    /// database that contains rows with non-24-byte keys (written by an older akmon version).
    ///
    /// Root cause observed in production: /tmp/agentpack-stage is volume-mounted into every
    /// smolvm task VM and persists across runs. An older akmon binary wrote 32-byte event keys;
    /// when akmon 2.0.0 opened the same journal.redb it crashed immediately on the first
    /// history() call because parse_event_key() hard-failed on the unexpected length.
    #[test]
    fn history_skips_foreign_key_lengths() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("foreign_keys.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );

        // Create a normal session and append two events.
        let sid = uuid::Uuid::new_v4();
        let mut graph =
            RedbSessionGraph::open_new(Arc::clone(&store), sid).unwrap_or_else(|_| unreachable!());
        graph
            .append(session_start(store.as_ref()))
            .unwrap_or_else(|_| unreachable!());
        graph
            .append(EventKind::UserTurn {
                prompt_hash: make_hash(store.as_ref(), 0x55),
            })
            .unwrap_or_else(|_| unreachable!());

        // Inject a row with a 32-byte key directly into SESSION_EVENTS_TABLE, simulating a
        // stale entry written by an older akmon binary with a different key layout.
        {
            let write_txn = store
                .database()
                .begin_write()
                .unwrap_or_else(|_| unreachable!());
            {
                let mut events_table = write_txn
                    .open_table(SESSION_EVENTS_TABLE)
                    .unwrap_or_else(|_| unreachable!());
                let foreign_key = [0xFFu8; 32]; // 32-byte key — unrecognized format
                let dummy_value = [0u8; 4];
                events_table
                    .insert(foreign_key.as_slice(), dummy_value.as_slice())
                    .unwrap_or_else(|_| unreachable!());
            }
            write_txn.commit().unwrap_or_else(|_| unreachable!());
        }

        // Reopen the session. history() must succeed and return only the two valid events,
        // skipping the foreign-key row entirely.
        let reopened =
            RedbSessionGraph::reopen(Arc::clone(&store), sid).unwrap_or_else(|_| unreachable!());
        let history = reopened
            .history()
            .expect("history() must not fail on foreign key lengths");
        assert_eq!(
            history.len(),
            2,
            "expected 2 valid events, got {}",
            history.len()
        );

        // Also verify that session_head_row_exists works correctly with foreign-key rows
        // present — the foreign rows must not be mistaken for a real session's events.
        assert!(
            session_head_row_exists(&store, sid).unwrap_or_else(|_| unreachable!()),
            "real session must still be found"
        );
        let unknown_sid = uuid::Uuid::new_v4();
        assert!(
            !session_head_row_exists(&store, unknown_sid).unwrap_or_else(|_| unreachable!()),
            "unknown session must not be found"
        );
    }
}
