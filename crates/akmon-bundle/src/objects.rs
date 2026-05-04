//! Object path and file I/O helpers for AGEF bundles.

use crate::BundleError;
use akmon_journal::Hash;
use std::path::{Path, PathBuf};

/// Returns bundle object filename (`<hex>`).
pub fn object_filename(hash: &Hash) -> String {
    hash.to_hex()
}

/// Returns relative bundle object path (`objects/<hex>`).
pub fn object_path(hash: &Hash) -> PathBuf {
    PathBuf::from("objects").join(object_filename(hash))
}

/// Writes one object file to `root/objects/<hex>`.
pub fn write_object_file(root: &Path, hash: &Hash, bytes: &[u8]) -> Result<(), BundleError> {
    let path = root.join(object_path(hash));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Reads one object file from `root/objects/<hex>`.
pub fn read_object_file(root: &Path, hash: &Hash) -> Result<Vec<u8>, BundleError> {
    let path = root.join(object_path(hash));
    std::fs::read(path).map_err(BundleError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use akmon_journal::HashAlgorithm;

    fn hash() -> Hash {
        Hash::from_bytes(HashAlgorithm::Sha256, [0xAB; 32])
    }

    #[test]
    fn t_object_path_lowercase_hex() {
        let path = object_path(&hash());
        let path_str = path.to_string_lossy();
        assert!(path_str.starts_with("objects/"));
        let name = object_filename(&hash());
        assert_eq!(name, name.to_lowercase());
    }

    #[test]
    fn t_object_filename_no_separator() {
        let name = object_filename(&hash());
        assert!(!name.contains('/'));
        assert!(!name.contains('\\'));
    }

    #[test]
    fn t_write_then_read_object_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let h = hash();
        let bytes = b"object-bytes";
        write_object_file(dir.path(), &h, bytes).expect("write");
        let got = read_object_file(dir.path(), &h).expect("read");
        assert_eq!(got, bytes);
    }

    #[test]
    fn t_read_object_missing_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = read_object_file(dir.path(), &hash()).expect_err("missing");
        assert!(matches!(err, BundleError::Io(_)));
    }
}
