//! Object load and preview helpers for `--resolve` diff mode.

use akmon_journal::{Hash, ObjectStore};

use crate::{DiffDivergence, DiffError, ResolvedContent};

/// Maximum bytes loaded per object in resolve mode (diff-specific cap).
pub const RESOLVE_READ_CAP_BYTES: usize = 10 * 1024 * 1024;

// Mirror akmon-cli/src/main.rs RESOLVE_* constants for preview consistency between `akmon inspect --resolve`
// and `akmon diff --resolve`. Promote to shared module in Phase 7 cleanup; akmon-tools/akmon-journal don't
// currently host CLI preview policy.
const RESOLVE_TEXT_MAX_BYTES: usize = 10 * 1024;
const RESOLVE_TEXT_PREVIEW_MAX_LINES: usize = 5;
const RESOLVE_TEXT_PREVIEW_MAX_LINE_BYTES: usize = 1024;
const RESOLVE_BINARY_HEX_MAX_BYTES: usize = 64;
/// Stable v2.0.0 skip reason when an object exceeds [`RESOLVE_READ_CAP_BYTES`].
pub const RESOLVE_SKIP_EXCEEDS_CAP: &str = "exceeds 10 MiB cap";
/// Stable v2.0.0 skip reason when the store has no bytes for a hash.
pub const RESOLVE_SKIP_OBJECT_MISSING: &str = "object missing from store";
/// Stable v2.0.0 skip reason when the divergence field is not backed by object bytes.
pub const RESOLVE_SKIP_NOT_DEREFERENCABLE: &str = "field not dereferenceable";

/// Pair of object stores used to load bytes for resolve mode.
#[derive(Clone, Copy)]
pub struct ResolveContext<'a> {
    /// Session A object store.
    pub store_a: &'a dyn ObjectStore,
    /// Session B object store.
    pub store_b: &'a dyn ObjectStore,
}

/// Outcome of loading one object under the resolve read cap.
#[derive(Debug)]
pub enum ResolveOutcome {
    /// Bytes loaded successfully (length ≤ cap).
    Loaded(Vec<u8>),
    /// Object exists but is larger than the read cap.
    ExceedsCap {
        /// Size reported by the store payload (diagnostics; skip reason stays a stable string).
        #[allow(dead_code)]
        actual_size: usize,
    },
    /// No object at this hash.
    ObjectMissing,
}

/// Loads object bytes, applying [`RESOLVE_READ_CAP_BYTES`].
pub fn resolve_object_capped(
    store: &dyn ObjectStore,
    hash: &Hash,
) -> Result<ResolveOutcome, DiffError> {
    match store.get(hash) {
        Ok(None) => Ok(ResolveOutcome::ObjectMissing),
        Ok(Some(bytes)) => {
            let len = bytes.len();
            if len > RESOLVE_READ_CAP_BYTES {
                Ok(ResolveOutcome::ExceedsCap { actual_size: len })
            } else {
                Ok(ResolveOutcome::Loaded(bytes.to_vec()))
            }
        }
        Err(err) => Err(DiffError::StoreAccessFailed {
            source: Box::new(std::io::Error::other(err.to_string())),
        }),
    }
}

/// Builds a preview string for in-cap bytes (inspect-style text rules or hex for binary).
#[must_use]
pub fn render_preview(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return Some("<empty>".to_owned());
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(render_text_preview(text)),
        Err(_) => Some(format_hex_preview(bytes, RESOLVE_BINARY_HEX_MAX_BYTES)),
    }
}

/// Longest UTF-8 safe prefix of `line` with byte length ≤ `max_bytes`, and omitted tail length.
fn truncate_line_to_max_bytes(line: &str, max_bytes: usize) -> (&str, usize) {
    if line.len() <= max_bytes {
        return (line, 0);
    }
    let mut end = 0;
    for (idx, ch) in line.char_indices() {
        let next = idx + ch.len_utf8();
        if next <= max_bytes {
            end = next;
        } else {
            break;
        }
    }
    let omitted = line.len() - end;
    (&line[..end], omitted)
}

fn format_line_maybe_truncated(line: &str) -> String {
    if line.len() > RESOLVE_TEXT_PREVIEW_MAX_LINE_BYTES {
        let (prefix, omitted) =
            truncate_line_to_max_bytes(line, RESOLVE_TEXT_PREVIEW_MAX_LINE_BYTES);
        format!("{prefix}... (truncated line, {omitted} more bytes)")
    } else {
        line.to_owned()
    }
}

fn render_text_preview(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return "<empty>".to_owned();
    }
    if text.len() > RESOLVE_TEXT_MAX_BYTES || lines.len() > RESOLVE_TEXT_PREVIEW_MAX_LINES {
        let shown = lines
            .iter()
            .take(RESOLVE_TEXT_PREVIEW_MAX_LINES)
            .map(|line| format_line_maybe_truncated(line))
            .collect::<Vec<_>>()
            .join("\n");
        let more = lines.len().saturating_sub(RESOLVE_TEXT_PREVIEW_MAX_LINES);
        format!("{shown}\n... ({more} more lines)")
    } else {
        lines
            .iter()
            .map(|line| format_line_maybe_truncated(line))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn format_hex_preview(bytes: &[u8], max_bytes: usize) -> String {
    let preview_len = bytes.len().min(max_bytes);
    let preview = bytes[..preview_len]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    if bytes.len() > max_bytes {
        format!(
            "{preview}... (truncated, {} more bytes)",
            bytes.len() - max_bytes
        )
    } else {
        preview
    }
}

/// Populates [`DiffDivergence::resolved`] or [`DiffDivergence::resolved_skip_reason`] for two hashes.
pub fn attach_resolved_content_pair(
    div: &mut DiffDivergence,
    ctx: ResolveContext<'_>,
    hash_a: &Hash,
    hash_b: &Hash,
) -> Result<(), DiffError> {
    let oa = resolve_object_capped(ctx.store_a, hash_a)?;
    let ob = resolve_object_capped(ctx.store_b, hash_b)?;
    match (oa, ob) {
        (ResolveOutcome::Loaded(a), ResolveOutcome::Loaded(b)) => {
            let bytes_match = a == b;
            div.resolved = Some(ResolvedContent {
                a_size_bytes: a.len(),
                b_size_bytes: b.len(),
                a_preview: render_preview(&a),
                b_preview: render_preview(&b),
                bytes_match,
            });
            div.resolved_skip_reason = None;
        }
        (ResolveOutcome::ExceedsCap { .. }, _) | (_, ResolveOutcome::ExceedsCap { .. }) => {
            div.resolved = None;
            div.resolved_skip_reason = Some(RESOLVE_SKIP_EXCEEDS_CAP.to_owned());
        }
        (ResolveOutcome::ObjectMissing, _) | (_, ResolveOutcome::ObjectMissing) => {
            div.resolved = None;
            div.resolved_skip_reason = Some(RESOLVE_SKIP_OBJECT_MISSING.to_owned());
        }
    }
    Ok(())
}

/// Sets [`RESOLVE_SKIP_NOT_DEREFERENCABLE`] for non-hash divergences.
pub fn mark_not_dereferenceable(div: &mut DiffDivergence) {
    div.resolved = None;
    div.resolved_skip_reason = Some(RESOLVE_SKIP_NOT_DEREFERENCABLE.to_owned());
}

#[cfg(test)]
mod tests {
    use akmon_journal::{HashAlgorithm, MemoryObjectStore};

    use super::*;

    #[test]
    fn t_resolve_object_capped_loaded_under_cap() {
        let store = MemoryObjectStore::new(HashAlgorithm::Sha256);
        let h = store.put(b"hello").expect("put");
        match resolve_object_capped(&store, &h).expect("ok") {
            ResolveOutcome::Loaded(v) => assert_eq!(v, b"hello"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn t_resolve_object_capped_at_cap() {
        let store = MemoryObjectStore::new(HashAlgorithm::Sha256);
        let bytes = vec![0u8; RESOLVE_READ_CAP_BYTES];
        let h = store.put(&bytes).expect("put");
        match resolve_object_capped(&store, &h).expect("ok") {
            ResolveOutcome::Loaded(v) => assert_eq!(v.len(), RESOLVE_READ_CAP_BYTES),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn t_resolve_object_capped_over_cap() {
        let store = MemoryObjectStore::new(HashAlgorithm::Sha256);
        let bytes = vec![0u8; RESOLVE_READ_CAP_BYTES + 1];
        let h = store.put(&bytes).expect("put");
        match resolve_object_capped(&store, &h).expect("ok") {
            ResolveOutcome::ExceedsCap { actual_size } => {
                assert_eq!(actual_size, RESOLVE_READ_CAP_BYTES + 1);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn t_resolve_object_capped_missing() {
        let store = MemoryObjectStore::new(HashAlgorithm::Sha256);
        let h = Hash::from_bytes(HashAlgorithm::Sha256, [7u8; 32]);
        match resolve_object_capped(&store, &h).expect("ok") {
            ResolveOutcome::ObjectMissing => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn t_render_preview_short_text() {
        let s = render_preview(b"hi").expect("preview");
        assert_eq!(s, "hi");
    }

    #[test]
    fn t_render_preview_long_text_truncates_lines() {
        let mut text = String::new();
        for i in 0..20 {
            text.push_str(&format!("line {i}\n"));
        }
        let s = render_preview(text.as_bytes()).expect("preview");
        assert!(s.contains("line 0"));
        assert!(s.contains("more lines"));
        assert!(!s.contains("line 19"));
    }

    #[test]
    fn t_render_preview_binary_hex() {
        let s = render_preview(&[0x00, 0xff, 0xab]).expect("preview");
        assert!(s.contains("00"));
        assert!(s.contains("ff"));
    }

    #[test]
    fn t_render_preview_long_line_with_multibyte_chars() {
        let mut line = String::new();
        while line.len() < 1100 {
            line.push_str("你好");
        }
        assert!(
            !line.is_char_boundary(RESOLVE_TEXT_PREVIEW_MAX_LINE_BYTES),
            "byte {} must fall inside a multi-byte character (你好 is 6 bytes; 1020 + 4 = 1024)",
            RESOLVE_TEXT_PREVIEW_MAX_LINE_BYTES
        );
        assert!(
            line.len() > RESOLVE_TEXT_PREVIEW_MAX_LINE_BYTES,
            "need a line longer than preview cap"
        );
        let text = format!("{line}\nrest");
        let s = render_preview(text.as_bytes()).expect("preview");
        assert!(!s.is_empty());
        assert!(s.contains("truncated line"));
    }
}
