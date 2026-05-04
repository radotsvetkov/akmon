//! Content-addressed object store implementations.

use crate::error::{JournalError, Result};
use crate::hash::{Hash, HashAlgorithm, SHA256_LEN, digest_bytes};
use crate::journal_meta::JournalMeta;
use bytes::Bytes;
use redb::{Database, ReadableTable, TableDefinition};
use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

const JOURNAL_META_TABLE: TableDefinition<u32, &[u8]> = TableDefinition::new("journal_meta");
const OBJECTS_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("objects");
const JOURNAL_META_KEY: u32 = 0;

/// Immutable content-addressed blob store.
pub trait ObjectStore: Send + Sync {
    /// Returns the configured hash algorithm for this store.
    fn algorithm(&self) -> HashAlgorithm;
    /// Inserts object bytes and returns their address hash.
    fn put(&self, bytes: &[u8]) -> Result<Hash>;
    /// Retrieves object bytes by hash.
    fn get(&self, hash: &Hash) -> Result<Option<Bytes>>;
    /// Checks whether a hash exists in the store.
    fn contains(&self, hash: &Hash) -> Result<bool>;
    /// Iterates all stored object hashes.
    fn iter_hashes(&self) -> Result<Box<dyn Iterator<Item = Hash> + '_>>;
}

/// In-memory object store used for tests.
pub struct MemoryObjectStore {
    algorithm: HashAlgorithm,
    objects: RwLock<HashMap<[u8; SHA256_LEN], Bytes>>,
}

impl MemoryObjectStore {
    /// Creates a new in-memory object store.
    pub fn new(algorithm: HashAlgorithm) -> Self {
        Self {
            algorithm,
            objects: RwLock::new(HashMap::new()),
        }
    }

    /// Test-only: replaces bytes at `hash` without updating the key (object corruption simulation).
    #[cfg(test)]
    pub fn overwrite_object_bytes_for_testing(
        &self,
        hash: &Hash,
        corrupt_bytes: &[u8],
    ) -> Result<()> {
        if hash.algorithm != self.algorithm {
            return Err(JournalError::HashAlgorithmMismatch {
                expected: self.algorithm,
                found: hash.algorithm,
            });
        }
        let mut guard = self.objects.write().map_err(|_| {
            JournalError::Verification("memory object store lock poisoned".to_owned())
        })?;
        guard.insert(hash.bytes, Bytes::copy_from_slice(corrupt_bytes));
        Ok(())
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.objects.read().map(|g| g.len()).unwrap_or(0)
    }
}

impl ObjectStore for MemoryObjectStore {
    fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }

    fn put(&self, bytes: &[u8]) -> Result<Hash> {
        let hash = digest_bytes(self.algorithm, bytes);
        let mut guard = self.objects.write().map_err(|_| {
            JournalError::Verification("memory object store lock poisoned".to_owned())
        })?;
        guard
            .entry(hash.bytes)
            .or_insert_with(|| Bytes::copy_from_slice(bytes));
        Ok(hash)
    }

    fn get(&self, hash: &Hash) -> Result<Option<Bytes>> {
        if hash.algorithm != self.algorithm {
            return Err(JournalError::HashAlgorithmMismatch {
                expected: self.algorithm,
                found: hash.algorithm,
            });
        }
        let guard = self.objects.read().map_err(|_| {
            JournalError::Verification("memory object store lock poisoned".to_owned())
        })?;
        Ok(guard.get(&hash.bytes).cloned())
    }

    fn contains(&self, hash: &Hash) -> Result<bool> {
        if hash.algorithm != self.algorithm {
            return Err(JournalError::HashAlgorithmMismatch {
                expected: self.algorithm,
                found: hash.algorithm,
            });
        }
        let guard = self.objects.read().map_err(|_| {
            JournalError::Verification("memory object store lock poisoned".to_owned())
        })?;
        Ok(guard.contains_key(&hash.bytes))
    }

    fn iter_hashes(&self) -> Result<Box<dyn Iterator<Item = Hash> + '_>> {
        let guard = self.objects.read().map_err(|_| {
            JournalError::Verification("memory object store lock poisoned".to_owned())
        })?;
        let hashes: Vec<Hash> = guard
            .keys()
            .copied()
            .map(|bytes| Hash::from_bytes(self.algorithm, bytes))
            .collect();
        Ok(Box::new(hashes.into_iter()))
    }
}

/// redb-backed object store implementation.
///
/// Concurrency findings (verified against redb 2.6.3 docs):
/// - `Database` supports concurrent reads and a single in-progress writer.
/// - `begin_write()` blocks until the current writer finishes (no busy error on write contention).
/// - redb reports `DatabaseError::DatabaseAlreadyOpen` when the same database file is already opened
///   by another process.
/// - This type is `Send + Sync` (validated in compile-time tests) and can be shared via `Arc`.
///
/// `get()` returns `Bytes::copy_from_slice(...)` from redb's access guard, which is one copy and
/// keeps implementation simple/safe for now.
pub struct RedbObjectStore {
    db: Database,
    algorithm: HashAlgorithm,
}

impl RedbObjectStore {
    /// Creates a new journal database at `path` and persists metadata.
    pub fn create(path: &Path, algorithm: HashAlgorithm) -> Result<Self> {
        let db = Database::create(path)
            .map_err(|err| JournalError::Verification(format!("redb create failed: {err}")))?;
        let meta = read_meta(&db)?;
        if meta.is_some() {
            return Err(JournalError::Verification(
                "journal metadata already exists; refusing create on initialized journal"
                    .to_owned(),
            ));
        }
        write_meta(
            &db,
            &JournalMeta::new(algorithm, time::OffsetDateTime::now_utc().unix_timestamp()),
        )?;
        let store = Self { db, algorithm };
        store.ensure_objects_table()?;
        Ok(store)
    }

    /// Opens an existing journal database and loads persisted metadata.
    pub fn open(path: &Path) -> Result<Self> {
        let db = Database::open(path)
            .map_err(|err| JournalError::Verification(format!("redb open failed: {err}")))?;
        let meta = read_meta(&db)?.ok_or_else(|| {
            JournalError::Verification(
                "journal metadata missing; expected initialized journal".to_owned(),
            )
        })?;
        if meta.schema_version != JournalMeta::SCHEMA_VERSION {
            return Err(JournalError::Verification(format!(
                "unsupported journal metadata schema: expected {}, found {}",
                JournalMeta::SCHEMA_VERSION,
                meta.schema_version
            )));
        }
        let store = Self {
            db,
            algorithm: meta.hash_algorithm,
        };
        store.ensure_objects_table()?;
        Ok(store)
    }

    fn ensure_objects_table(&self) -> Result<()> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
        {
            write_txn.open_table(OBJECTS_TABLE).map_err(|err| {
                JournalError::Verification(format!("open objects table failed: {err}"))
            })?;
        }
        write_txn.commit().map_err(|err| {
            JournalError::Verification(format!("commit objects table open failed: {err}"))
        })?;
        Ok(())
    }

    pub(crate) fn database(&self) -> &Database {
        &self.db
    }

    /// Test-only: replaces bytes at `hash` without updating the key (object corruption simulation).
    #[cfg(test)]
    pub fn overwrite_object_bytes_for_testing(
        &self,
        hash: &Hash,
        corrupt_bytes: &[u8],
    ) -> Result<()> {
        if hash.algorithm != self.algorithm {
            return Err(JournalError::HashAlgorithmMismatch {
                expected: self.algorithm,
                found: hash.algorithm,
            });
        }
        let write_txn = self
            .db
            .begin_write()
            .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
        {
            let mut table = write_txn.open_table(OBJECTS_TABLE).map_err(|err| {
                JournalError::Verification(format!("open objects table failed: {err}"))
            })?;
            table
                .insert(hash.bytes.as_slice(), corrupt_bytes)
                .map_err(|err| {
                    JournalError::Verification(format!("object overwrite failed: {err}"))
                })?;
        }
        write_txn.commit().map_err(|err| {
            JournalError::Verification(format!("object overwrite commit failed: {err}"))
        })?;
        Ok(())
    }
}

impl ObjectStore for RedbObjectStore {
    fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }

    fn put(&self, bytes: &[u8]) -> Result<Hash> {
        let hash = digest_bytes(self.algorithm, bytes);
        let write_txn = self
            .db
            .begin_write()
            .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
        {
            let mut table = write_txn.open_table(OBJECTS_TABLE).map_err(|err| {
                JournalError::Verification(format!("open objects table failed: {err}"))
            })?;
            if table
                .get(hash.bytes.as_slice())
                .map_err(|err| JournalError::Verification(format!("object lookup failed: {err}")))?
                .is_none()
            {
                table.insert(hash.bytes.as_slice(), bytes).map_err(|err| {
                    JournalError::Verification(format!("object insert failed: {err}"))
                })?;
            }
        }
        write_txn
            .commit()
            .map_err(|err| JournalError::Verification(format!("object commit failed: {err}")))?;
        Ok(hash)
    }

    fn get(&self, hash: &Hash) -> Result<Option<Bytes>> {
        if hash.algorithm != self.algorithm {
            return Err(JournalError::HashAlgorithmMismatch {
                expected: self.algorithm,
                found: hash.algorithm,
            });
        }
        let read_txn = self
            .db
            .begin_read()
            .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
        let table = read_txn.open_table(OBJECTS_TABLE).map_err(|err| {
            JournalError::Verification(format!("open objects table failed: {err}"))
        })?;
        let value = table
            .get(hash.bytes.as_slice())
            .map_err(|err| JournalError::Verification(format!("object lookup failed: {err}")))?;
        Ok(value.map(|v| Bytes::copy_from_slice(v.value())))
    }

    fn contains(&self, hash: &Hash) -> Result<bool> {
        if hash.algorithm != self.algorithm {
            return Err(JournalError::HashAlgorithmMismatch {
                expected: self.algorithm,
                found: hash.algorithm,
            });
        }
        let read_txn = self
            .db
            .begin_read()
            .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
        let table = read_txn.open_table(OBJECTS_TABLE).map_err(|err| {
            JournalError::Verification(format!("open objects table failed: {err}"))
        })?;
        Ok(table
            .get(hash.bytes.as_slice())
            .map_err(|err| JournalError::Verification(format!("object lookup failed: {err}")))?
            .is_some())
    }

    fn iter_hashes(&self) -> Result<Box<dyn Iterator<Item = Hash> + '_>> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
        let table = read_txn.open_table(OBJECTS_TABLE).map_err(|err| {
            JournalError::Verification(format!("open objects table failed: {err}"))
        })?;
        let iter = table
            .iter()
            .map_err(|err| JournalError::Verification(format!("object iteration failed: {err}")))?;
        let mut out = Vec::new();
        for item in iter {
            let (key, _) = item.map_err(|err| {
                JournalError::Verification(format!("object iteration item failed: {err}"))
            })?;
            let key_bytes = key.value();
            if key_bytes.len() != SHA256_LEN {
                return Err(JournalError::Verification(format!(
                    "objects table key length mismatch: expected {SHA256_LEN}, found {}",
                    key_bytes.len()
                )));
            }
            let mut arr = [0_u8; SHA256_LEN];
            arr.copy_from_slice(key_bytes);
            out.push(Hash::from_bytes(self.algorithm, arr));
        }
        Ok(Box::new(out.into_iter()))
    }
}

fn write_meta(db: &Database, meta: &JournalMeta) -> Result<()> {
    let bytes = postcard::to_allocvec(meta)?;
    let write_txn = db
        .begin_write()
        .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
    {
        let mut table = write_txn.open_table(JOURNAL_META_TABLE).map_err(|err| {
            JournalError::Verification(format!("open journal_meta table failed: {err}"))
        })?;
        table
            .insert(&JOURNAL_META_KEY, bytes.as_slice())
            .map_err(|err| {
                JournalError::Verification(format!("journal_meta insert failed: {err}"))
            })?;
    }
    write_txn
        .commit()
        .map_err(|err| JournalError::Verification(format!("journal_meta commit failed: {err}")))?;
    Ok(())
}

fn read_meta(db: &Database) -> Result<Option<JournalMeta>> {
    let read_txn = db
        .begin_read()
        .map_err(|err| JournalError::StorageTx(Box::new(err)))?;
    let table = match read_txn.open_table(JOURNAL_META_TABLE) {
        Ok(table) => table,
        Err(_) => return Ok(None),
    };
    let value = table
        .get(&JOURNAL_META_KEY)
        .map_err(|err| JournalError::Verification(format!("journal_meta read failed: {err}")))?;
    match value {
        Some(v) => {
            let meta: JournalMeta = postcard::from_bytes(v.value())?;
            Ok(Some(meta))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn payload(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| seed.wrapping_add(i as u8)).collect()
    }

    #[test]
    fn memory_store_put_get_contains_and_unknown() {
        let store = MemoryObjectStore::new(HashAlgorithm::Sha256);
        let before = store.contains(&digest_bytes(HashAlgorithm::Sha256, b"missing"));
        assert!(before.is_ok());
        assert!(!before.unwrap_or(false));

        let bytes = payload(128, 0x11);
        let hash = store.put(&bytes).unwrap_or_else(|_| unreachable!());
        let contains = store.contains(&hash).unwrap_or_else(|_| unreachable!());
        assert!(contains);
        let got = store.get(&hash).unwrap_or_else(|_| unreachable!());
        assert!(got.is_some());
        assert_eq!(
            got.unwrap_or_else(|| unreachable!()).as_ref(),
            bytes.as_slice()
        );

        let unknown = Hash::from_bytes(HashAlgorithm::Sha256, [0xEE; 32]);
        let missing = store.get(&unknown).unwrap_or_else(|_| unreachable!());
        assert!(missing.is_none());
    }

    #[test]
    fn memory_store_deduplicates() {
        let store = MemoryObjectStore::new(HashAlgorithm::Sha256);
        let bytes = payload(64, 0x33);
        let h1 = store.put(&bytes).unwrap_or_else(|_| unreachable!());
        let h2 = store.put(&bytes).unwrap_or_else(|_| unreachable!());
        assert_eq!(h1, h2);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn memory_store_iter_hashes_distinct_count() {
        let store = MemoryObjectStore::new(HashAlgorithm::Sha256);
        let p1 = store
            .put(&payload(32, 1))
            .unwrap_or_else(|_| unreachable!());
        let p2 = store
            .put(&payload(32, 2))
            .unwrap_or_else(|_| unreachable!());
        let p3 = store
            .put(&payload(32, 3))
            .unwrap_or_else(|_| unreachable!());
        let mut hashes: Vec<Hash> = store
            .iter_hashes()
            .unwrap_or_else(|_| unreachable!())
            .collect();
        hashes.sort_by_key(Hash::to_hex);
        assert_eq!(hashes.len(), 3);
        assert!(hashes.contains(&p1));
        assert!(hashes.contains(&p2));
        assert!(hashes.contains(&p3));
    }

    #[test]
    fn redb_create_open_and_meta_behavior() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("journal.redb");
        {
            let created = RedbObjectStore::create(path.as_path(), HashAlgorithm::Blake3);
            assert!(created.is_ok());
        }

        let created_again = RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256);
        assert!(created_again.is_err());

        let opened = RedbObjectStore::open(path.as_path());
        assert!(opened.is_ok());
        assert_eq!(
            opened.unwrap_or_else(|_| unreachable!()).algorithm(),
            HashAlgorithm::Blake3
        );
    }

    #[test]
    fn redb_open_refuses_missing_meta() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("journal_missing_meta.redb");
        let db = Database::create(path.as_path()).unwrap_or_else(|_| unreachable!());
        let tx = db.begin_write().unwrap_or_else(|_| unreachable!());
        tx.commit().unwrap_or_else(|_| unreachable!());

        let opened = RedbObjectStore::open(path.as_path());
        assert!(opened.is_err());
    }

    #[test]
    fn redb_algorithm_consistency_and_bytes_get() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let p1 = tmp.path().join("a.redb");
        let p2 = tmp.path().join("b.redb");
        let s1 = RedbObjectStore::create(p1.as_path(), HashAlgorithm::Sha256)
            .unwrap_or_else(|_| unreachable!());
        let s2 = RedbObjectStore::create(p2.as_path(), HashAlgorithm::Blake3)
            .unwrap_or_else(|_| unreachable!());

        let bytes = payload(77, 0x4A);
        let h1 = s1.put(&bytes).unwrap_or_else(|_| unreachable!());
        let h2 = s2.put(&bytes).unwrap_or_else(|_| unreachable!());
        assert_eq!(h1.algorithm, HashAlgorithm::Sha256);
        assert_eq!(h2.algorithm, HashAlgorithm::Blake3);
        assert_ne!(h1.bytes, h2.bytes);
        assert_eq!(s1.algorithm(), HashAlgorithm::Sha256);
        assert_eq!(s2.algorithm(), HashAlgorithm::Blake3);
        let got = s1.get(&h1).unwrap_or_else(|_| unreachable!());
        assert!(got.is_some());
        assert_eq!(
            got.unwrap_or_else(|| unreachable!()).as_ref(),
            bytes.as_slice()
        );
    }

    #[test]
    fn redb_concurrency_smoke_test() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| unreachable!());
        let path = tmp.path().join("concurrent.redb");
        let store = Arc::new(
            RedbObjectStore::create(path.as_path(), HashAlgorithm::Sha256)
                .unwrap_or_else(|_| unreachable!()),
        );
        let left = Arc::clone(&store);
        let right = Arc::clone(&store);

        let t1 = std::thread::spawn(move || {
            for idx in 0..50_u8 {
                let bytes = payload(64, idx);
                let h = left.put(&bytes).unwrap_or_else(|_| unreachable!());
                let got = left.get(&h).unwrap_or_else(|_| unreachable!());
                assert!(got.is_some());
            }
        });
        let t2 = std::thread::spawn(move || {
            for idx in 100..150_u8 {
                let bytes = payload(64, idx);
                let h = right.put(&bytes).unwrap_or_else(|_| unreachable!());
                let got = right.get(&h).unwrap_or_else(|_| unreachable!());
                assert!(got.is_some());
            }
        });

        t1.join().unwrap_or_else(|_| unreachable!());
        t2.join().unwrap_or_else(|_| unreachable!());
        let hashes: Vec<Hash> = store
            .iter_hashes()
            .unwrap_or_else(|_| unreachable!())
            .collect();
        assert!(hashes.len() >= 100);
    }

    #[test]
    fn redb_database_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Database>();
    }
}
