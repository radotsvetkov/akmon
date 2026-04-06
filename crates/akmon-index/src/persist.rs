//! Load / save [`RepoIndex`](crate::RepoIndex) without pulling in the embedding stack.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use crate::RepoIndex;
use crate::error::IndexError;

/// Writes `index` to `path` (creates parent directories).
pub fn save_index(index: &RepoIndex, path: &Path) -> Result<(), IndexError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = File::create(path)?;
    bincode::serialize_into(&mut f, index).map_err(|e| IndexError::Serialization(e.to_string()))?;
    f.flush()?;
    Ok(())
}

/// Reads a [`RepoIndex`] previously written by [`save_index`].
pub fn load_index(path: &Path) -> Result<RepoIndex, IndexError> {
    let f = File::open(path)?;
    bincode::deserialize_from(f).map_err(|e| IndexError::Serialization(e.to_string()))
}
