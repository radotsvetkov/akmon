//! Journal metadata persisted in the redb backing store.

use crate::hash::HashAlgorithm;
use serde::{Deserialize, Serialize};

/// Persisted metadata for a journal database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalMeta {
    /// AGEF format version this journal targets.
    pub agef_version: String,
    /// Hash algorithm selected when the journal was created.
    pub hash_algorithm: HashAlgorithm,
    /// Journal creation timestamp in unix epoch seconds.
    pub created_at: i64,
    /// Metadata schema version for forward compatibility checks.
    pub schema_version: u32,
}

impl JournalMeta {
    /// Current schema version used by this crate.
    pub const SCHEMA_VERSION: u32 = 1;

    /// Creates a new metadata record.
    pub fn new(hash_algorithm: HashAlgorithm, now_epoch_seconds: i64) -> Self {
        Self {
            agef_version: "0.1".to_owned(),
            hash_algorithm,
            created_at: now_epoch_seconds,
            schema_version: Self::SCHEMA_VERSION,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::HashAlgorithm;

    #[test]
    fn journal_meta_postcard_roundtrip() {
        let meta = JournalMeta::new(HashAlgorithm::Blake3, 1_711_111_999);
        let encoded = postcard::to_allocvec(&meta).unwrap_or_else(|_| unreachable!());
        let decoded: JournalMeta =
            postcard::from_bytes(&encoded).unwrap_or_else(|_| unreachable!());
        assert_eq!(decoded, meta);
    }
}
