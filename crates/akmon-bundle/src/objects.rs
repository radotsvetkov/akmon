//! Object naming helpers for AGEF bundles.

use akmon_journal::Hash;

/// Returns bundle object filename (`<hex>`, lowercase): the entry is stored at `objects/<hex>`.
#[must_use]
pub fn object_filename(hash: &Hash) -> String {
    hash.to_hex()
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_journal::HashAlgorithm;

    fn hash() -> Hash {
        Hash::from_bytes(HashAlgorithm::Sha256, [0xAB; 32])
    }

    #[test]
    fn t_object_filename_lowercase_hex() {
        let name = object_filename(&hash());
        assert_eq!(name.len(), 64);
        assert_eq!(name, name.to_lowercase());
    }

    #[test]
    fn t_object_filename_no_separator() {
        let name = object_filename(&hash());
        assert!(!name.contains('/'));
        assert!(!name.contains('\\'));
    }
}
