//! High-level `tar.zst` bundle read/write helpers.

use crate::BundleError;
use crate::events::{EventsReader, EventsWriter};
use crate::manifest::Manifest;
use crate::objects::object_filename;
use akmon_journal::{AGEF_SPEC_VERSION, Event, Hash, HashAlgorithm};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;

/// Default cap on the total number of bytes a single bundle is allowed to decompress to.
///
/// Reading stops with [`BundleError::BundleTooLarge`] once a `tar.zst` would expand past this,
/// so a tiny malicious archive cannot exhaust memory on the machine reading it (the
/// decompression-bomb guard). 1 GiB is generous for real agent sessions while still bounding
/// the worst case.
pub const DEFAULT_MAX_BUNDLE_DECODED_BYTES: u64 = 1 << 30;

/// Fully-materialized bundle contents.
#[derive(Debug, Clone)]
pub struct BundleContents {
    /// Parsed manifest.
    pub manifest: Manifest,
    /// Decoded event sequence.
    pub events: Vec<Event>,
    /// Object bytes keyed by digest hash.
    pub objects: HashMap<Hash, Vec<u8>>,
}

/// Read options for bundle parsing.
#[derive(Debug, Clone)]
pub struct ReadBundleOptions {
    /// Whether unknown extra files in archive should be accepted.
    pub allow_extra_files: bool,
    /// Maximum accepted frame length in `events.bin`.
    pub max_event_frame_len: u32,
}

impl Default for ReadBundleOptions {
    fn default() -> Self {
        Self {
            allow_extra_files: false,
            max_event_frame_len: crate::DEFAULT_MAX_EVENT_FRAME_LEN,
        }
    }
}

/// Write options for bundle emission.
#[derive(Debug, Clone)]
pub struct WriteBundleOptions {
    /// zstd compression level used for output.
    pub zstd_level: i32,
}

impl Default for WriteBundleOptions {
    fn default() -> Self {
        Self { zstd_level: 19 }
    }
}

/// Writes one AGEF bundle as `tar.zst`.
pub fn write_bundle<W: Write>(
    writer: W,
    manifest: &Manifest,
    events: &[Event],
    objects: &HashMap<Hash, Vec<u8>>,
    options: &WriteBundleOptions,
) -> Result<(), BundleError> {
    let manifest_bytes = manifest.to_canonical_json_bytes()?;
    // Validate the declared algorithm up front so an unsupported manifest is never written.
    parse_algorithm(&manifest.hash_algorithm)?;

    let mut events_bytes = Vec::new();
    {
        let mut ew = EventsWriter::new(&mut events_bytes);
        for event in events {
            ew.write_event(event)?;
        }
        let _ = ew.finish()?;
    }

    let encoder = zstd::Encoder::new(writer, options.zstd_level).map_err(|err| {
        BundleError::InvalidCompression(format!("zstd encoder init failed: {err}"))
    })?;
    let mut builder = tar::Builder::new(encoder.auto_finish());
    append_bytes(&mut builder, "manifest.json", &manifest_bytes)?;
    append_bytes(&mut builder, "events.bin", &events_bytes)?;

    let mut sorted: BTreeMap<String, (&Hash, &Vec<u8>)> = BTreeMap::new();
    for (hash, bytes) in objects {
        sorted.insert(hash.to_hex(), (hash, bytes));
    }
    for (_, (hash, bytes)) in sorted {
        let path = format!("objects/{}", object_filename(hash));
        append_bytes(&mut builder, &path, bytes)?;
    }
    builder
        .finish()
        .map_err(|err| BundleError::InvalidArchive(format!("tar finalize failed: {err}")))?;
    Ok(())
}

/// Reads one AGEF bundle from `tar.zst`.
///
/// The archive must be unambiguous: a second `manifest.json`, `events.bin`, or `objects/<hex>`
/// entry is rejected (different tar tools would otherwise show an auditor different content), and
/// object file names must be exactly 64 lowercase hex characters. In strict mode
/// (`allow_extra_files == false`) every entry must be a regular file, except a single directory
/// entry named exactly `objects/`; symlinks, hard links, devices, and other entry types are
/// rejected. The total decompressed size is capped at [`DEFAULT_MAX_BUNDLE_DECODED_BYTES`]; use
/// [`read_bundle_limited`] to choose a different cap.
pub fn read_bundle<R: Read>(
    reader: R,
    options: &ReadBundleOptions,
) -> Result<BundleContents, BundleError> {
    read_bundle_limited(reader, options, DEFAULT_MAX_BUNDLE_DECODED_BYTES)
}

/// Like [`read_bundle`], but with an explicit cap on the total decompressed size in bytes (the
/// decompression-bomb guard). Exposed so a verifier can let an operator raise the cap deliberately
/// for a legitimately large session (`--max-decoded-bytes`).
pub fn read_bundle_limited<R: Read>(
    reader: R,
    options: &ReadBundleOptions,
    max_decoded_bytes: u64,
) -> Result<BundleContents, BundleError> {
    let decoder = zstd::Decoder::new(reader).map_err(|err| {
        BundleError::InvalidCompression(format!("zstd decoder init failed: {err}"))
    })?;
    let mut archive = tar::Archive::new(LimitedReader::new(decoder, max_decoded_bytes));

    let mut manifest_bytes: Option<Vec<u8>> = None;
    let mut events_bytes: Option<Vec<u8>> = None;
    let mut objects_raw: Vec<(String, Vec<u8>)> = Vec::new();
    let mut seen_object_names: HashSet<String> = HashSet::new();

    let entries = archive
        .entries()
        .map_err(|err| archive_read_error(err, max_decoded_bytes, "tar entries failed"))?;
    for entry in entries {
        let mut entry = entry
            .map_err(|err| archive_read_error(err, max_decoded_bytes, "tar entry read failed"))?;
        let entry_type = entry.header().entry_type();
        let path = entry
            .path()
            .map_err(|err| BundleError::InvalidArchive(format!("invalid tar path: {err}")))?
            .to_path_buf();
        let path_str = path_to_unix(path);
        if !entry_type.is_file() {
            if options.allow_extra_files || (entry_type.is_dir() && path_str == "objects/") {
                continue;
            }
            return Err(BundleError::UnknownBundleFile(format!(
                "{path_str} (non-regular tar entry type {entry_type:?})"
            )));
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|err| archive_read_error(err, max_decoded_bytes, "tar entry payload"))?;

        match path_str.as_str() {
            "manifest.json" => {
                if manifest_bytes.replace(bytes).is_some() {
                    return Err(BundleError::InvalidArchive(
                        "duplicate manifest.json entry".to_owned(),
                    ));
                }
            }
            "events.bin" => {
                if events_bytes.replace(bytes).is_some() {
                    return Err(BundleError::InvalidArchive(
                        "duplicate events.bin entry".to_owned(),
                    ));
                }
            }
            _ if path_str.starts_with("objects/") => {
                let name = path_str.trim_start_matches("objects/").to_owned();
                if name.contains('/') || name.is_empty() {
                    return Err(BundleError::InvalidArchive(format!(
                        "invalid object entry path: {path_str}"
                    )));
                }
                if !seen_object_names.insert(name.clone()) {
                    return Err(BundleError::InvalidArchive(format!(
                        "duplicate object entry: {path_str}"
                    )));
                }
                objects_raw.push((name, bytes));
            }
            _ => {
                if !options.allow_extra_files {
                    return Err(BundleError::UnknownBundleFile(path_str));
                }
            }
        }
    }

    let manifest_bytes = manifest_bytes
        .ok_or_else(|| BundleError::InvalidArchive("missing required manifest.json".to_owned()))?;
    let events_bytes = events_bytes
        .ok_or_else(|| BundleError::InvalidArchive("missing required events.bin".to_owned()))?;

    let manifest = Manifest::from_json_bytes(&manifest_bytes)?;
    manifest.validate_agef_version(AGEF_SPEC_VERSION)?;
    let algorithm = parse_algorithm(&manifest.hash_algorithm)?;

    let mut events = Vec::new();
    let mut er = EventsReader::with_hash_algorithm_and_max_frame_len(
        Cursor::new(events_bytes),
        algorithm,
        options.max_event_frame_len,
    );
    while let Some(event) = er.read_event()? {
        events.push(event);
    }

    let mut objects = HashMap::new();
    for (name, bytes) in objects_raw {
        if name.len() != 64
            || !name
                .as_bytes()
                .iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        {
            return Err(BundleError::InvalidArchive(format!(
                "invalid object filename (must be 64 lowercase hex characters): {name}"
            )));
        }
        let mut digest = [0_u8; 32];
        hex::decode_to_slice(name.as_bytes(), &mut digest)
            .map_err(|err| BundleError::InvalidArchive(format!("invalid object hex: {err}")))?;
        let hash = Hash::from_bytes(algorithm, digest);
        objects.insert(hash, bytes);
    }

    Ok(BundleContents {
        manifest,
        events,
        objects,
    })
}

/// Reads one AGEF bundle from `tar.zst` and verifies its integrity before returning.
///
/// This is the safe-by-default entry point: it calls [`read_bundle`] and then
/// [`crate::verify::verify_bundle_strict`], returning the first integrity violation
/// as a [`BundleError`]. Use [`read_bundle`] directly only when you intend to verify
/// separately (for example to collect a full fail-soft report via
/// [`crate::verify::verify_bundle`]).
pub fn read_verified_bundle<R: Read>(
    reader: R,
    options: &ReadBundleOptions,
) -> Result<BundleContents, BundleError> {
    let contents = read_bundle(reader, options)?;
    crate::verify::verify_bundle_strict(&contents)?;
    Ok(contents)
}

fn append_bytes<W: Write>(
    builder: &mut tar::Builder<W>,
    path: &str,
    bytes: &[u8],
) -> Result<(), BundleError> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, path, bytes)
        .map_err(|err| BundleError::InvalidArchive(format!("append {path} failed: {err}")))?;
    Ok(())
}

fn path_to_unix(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Maps a manifest `hash_algorithm` string to a [`HashAlgorithm`].
///
/// # Errors
///
/// Returns [`BundleError::UnsupportedHashAlgorithm`] for anything other than `sha256`/`blake3`.
pub fn parse_algorithm(value: &str) -> Result<HashAlgorithm, BundleError> {
    match value {
        "sha256" => Ok(HashAlgorithm::Sha256),
        "blake3" => Ok(HashAlgorithm::Blake3),
        other => Err(BundleError::UnsupportedHashAlgorithm(other.to_owned())),
    }
}

/// Sentinel carried inside an [`std::io::Error`] when the decoded-byte budget is spent.
#[derive(Debug)]
struct DecodedSizeLimitExceeded;

impl std::fmt::Display for DecodedSizeLimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("decompressed size limit exceeded")
    }
}

impl std::error::Error for DecodedSizeLimitExceeded {}

/// `Read` adapter that errors once more than `limit` total bytes have been pulled from the
/// inner stream. Wrapping the zstd decoder with this caps a bundle's total decompressed size,
/// so a small archive cannot expand without bound (the decompression-bomb guard).
struct LimitedReader<R> {
    inner: R,
    remaining: u64,
}

impl<R> LimitedReader<R> {
    fn new(inner: R, limit: u64) -> Self {
        Self {
            inner,
            remaining: limit,
        }
    }
}

impl<R: Read> Read for LimitedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            // Budget spent: a clean end of stream still returns 0, but any further byte means
            // the decompressed output exceeds the cap.
            let mut probe = [0_u8; 1];
            return match self.inner.read(&mut probe)? {
                0 => Ok(0),
                _ => Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    DecodedSizeLimitExceeded,
                )),
            };
        }
        let cap = std::cmp::min(buf.len() as u64, self.remaining) as usize;
        let read = self.inner.read(&mut buf[..cap])?;
        self.remaining -= read as u64;
        Ok(read)
    }
}

/// Maps an I/O error raised while reading the archive: the decoded-size sentinel becomes
/// [`BundleError::BundleTooLarge`]; anything else becomes a contextual archive error.
fn archive_read_error(err: std::io::Error, limit: u64, context: &str) -> BundleError {
    if err
        .get_ref()
        .is_some_and(|inner| inner.downcast_ref::<DecodedSizeLimitExceeded>().is_some())
    {
        BundleError::BundleTooLarge(limit)
    } else {
        BundleError::InvalidArchive(format!("{context}: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Producer, SessionMetadata};
    use akmon_journal::EventKind;

    fn sample_manifest() -> Manifest {
        Manifest {
            agef_version: AGEF_SPEC_VERSION.to_owned(),
            producer: Producer {
                name: "akmon".to_owned(),
                version: "2.0.0".to_owned(),
            },
            session: SessionMetadata {
                id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
                head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
                created_at: "2026-05-04T14:00:00Z".to_owned(),
                ended_at: "2026-05-04T14:01:00Z".to_owned(),
            },
            hash_algorithm: "sha256".to_owned(),
            object_count: 1,
            event_count: 2,
            signatures: None,
            operator_attestations: None,
            extra: BTreeMap::new(),
        }
    }

    fn sample_event(seq: u64) -> Event {
        Event {
            parents: vec![],
            kind: if seq == 0 {
                EventKind::SessionStart {
                    cwd_hash: Hash::from_bytes(HashAlgorithm::Sha256, [0x11; 32]),
                    config_hash: Hash::from_bytes(HashAlgorithm::Sha256, [0x12; 32]),
                }
            } else {
                EventKind::SessionEnd { summary_hash: None }
            },
            emitted_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_000 + seq as i64)
                .expect("ts"),
            sequence: seq,
        }
    }

    #[test]
    fn t_write_then_read_bundle_round_trip() {
        let manifest = sample_manifest();
        let events = vec![sample_event(0), sample_event(1)];
        let object_hash = Hash::from_bytes(HashAlgorithm::Sha256, [0xAB; 32]);
        let objects = HashMap::from([(object_hash, b"hello".to_vec())]);

        let mut out = Vec::new();
        write_bundle(
            &mut out,
            &manifest,
            &events,
            &objects,
            &WriteBundleOptions::default(),
        )
        .expect("write");

        let parsed = read_bundle(Cursor::new(out), &ReadBundleOptions::default()).expect("read");
        assert_eq!(parsed.manifest.agef_version, manifest.agef_version);
        assert_eq!(parsed.events.len(), events.len());
        assert_eq!(parsed.objects.len(), 1);
    }

    #[test]
    fn t_decompression_size_limit_rejected() {
        let manifest = sample_manifest();
        let events = vec![sample_event(0), sample_event(1)];
        let object_hash = Hash::from_bytes(HashAlgorithm::Sha256, [0xAB; 32]);
        let objects = HashMap::from([(object_hash, b"hello".to_vec())]);

        let mut out = Vec::new();
        write_bundle(
            &mut out,
            &manifest,
            &events,
            &objects,
            &WriteBundleOptions::default(),
        )
        .expect("write");

        // A tiny decoded-byte cap makes reading fail with BundleTooLarge rather than panic or
        // exhaust memory: this is the decompression-bomb guard firing.
        let err = read_bundle_limited(Cursor::new(out.clone()), &ReadBundleOptions::default(), 16)
            .expect_err("tiny cap must reject");
        assert!(
            matches!(err, BundleError::BundleTooLarge(16)),
            "expected BundleTooLarge(16), got {err:?}"
        );

        // The same bundle reads cleanly under the generous default cap.
        assert!(
            read_bundle(Cursor::new(out), &ReadBundleOptions::default()).is_ok(),
            "a normal bundle must read under the default cap"
        );
    }

    #[test]
    fn t_read_bundle_rejects_missing_manifest() {
        let mut archive_bytes = Vec::new();
        {
            let encoder = zstd::Encoder::new(&mut archive_bytes, 19).expect("zstd");
            let mut builder = tar::Builder::new(encoder.auto_finish());
            append_bytes(&mut builder, "events.bin", &[]).expect("append");
            builder.finish().expect("finish");
        }
        let err = read_bundle(Cursor::new(archive_bytes), &ReadBundleOptions::default())
            .expect_err("must fail");
        assert!(matches!(err, BundleError::InvalidArchive(_)));
    }

    #[test]
    fn t_read_bundle_rejects_missing_events_bin() {
        let mut archive_bytes = Vec::new();
        {
            let encoder = zstd::Encoder::new(&mut archive_bytes, 19).expect("zstd");
            let mut builder = tar::Builder::new(encoder.auto_finish());
            append_bytes(
                &mut builder,
                "manifest.json",
                &sample_manifest()
                    .to_canonical_json_bytes()
                    .expect("manifest"),
            )
            .expect("append");
            builder.finish().expect("finish");
        }
        let err = read_bundle(Cursor::new(archive_bytes), &ReadBundleOptions::default())
            .expect_err("must fail");
        assert!(matches!(err, BundleError::InvalidArchive(_)));
    }

    #[test]
    fn t_read_bundle_rejects_unknown_extra_file_strict() {
        let mut archive_bytes = Vec::new();
        {
            let encoder = zstd::Encoder::new(&mut archive_bytes, 19).expect("zstd");
            let mut builder = tar::Builder::new(encoder.auto_finish());
            append_bytes(
                &mut builder,
                "manifest.json",
                &sample_manifest()
                    .to_canonical_json_bytes()
                    .expect("manifest"),
            )
            .expect("manifest");
            append_bytes(&mut builder, "events.bin", &[]).expect("events");
            append_bytes(&mut builder, "weird.txt", b"x").expect("extra");
            builder.finish().expect("finish");
        }
        let err = read_bundle(Cursor::new(archive_bytes), &ReadBundleOptions::default())
            .expect_err("must fail");
        assert!(matches!(err, BundleError::UnknownBundleFile(_)));
    }

    #[test]
    fn t_read_bundle_accepts_unknown_extra_file_with_option() {
        let mut archive_bytes = Vec::new();
        {
            let encoder = zstd::Encoder::new(&mut archive_bytes, 19).expect("zstd");
            let mut builder = tar::Builder::new(encoder.auto_finish());
            append_bytes(
                &mut builder,
                "manifest.json",
                &sample_manifest()
                    .to_canonical_json_bytes()
                    .expect("manifest"),
            )
            .expect("manifest");
            append_bytes(&mut builder, "events.bin", &[]).expect("events");
            append_bytes(&mut builder, "weird.txt", b"x").expect("extra");
            builder.finish().expect("finish");
        }
        let parsed = read_bundle(
            Cursor::new(archive_bytes),
            &ReadBundleOptions {
                allow_extra_files: true,
                ..ReadBundleOptions::default()
            },
        )
        .expect("must parse");
        assert_eq!(parsed.events.len(), 0);
    }

    #[test]
    fn t_write_bundle_uses_zstd_level_19_default() {
        let opts = WriteBundleOptions::default();
        assert_eq!(opts.zstd_level, 19);
    }

    /// Builds a `tar.zst` from (path, bytes) regular-file entries, in order.
    fn tar_zst(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut archive_bytes = Vec::new();
        {
            let encoder = zstd::Encoder::new(&mut archive_bytes, 3).expect("zstd");
            let mut builder = tar::Builder::new(encoder.auto_finish());
            for (path, bytes) in entries {
                append_bytes(&mut builder, path, bytes).expect("append");
            }
            builder.finish().expect("finish");
        }
        archive_bytes
    }

    fn manifest_bytes() -> Vec<u8> {
        sample_manifest()
            .to_canonical_json_bytes()
            .expect("manifest")
    }

    #[test]
    fn t_read_bundle_rejects_duplicate_manifest() {
        let m = manifest_bytes();
        let bytes = tar_zst(&[
            ("manifest.json", &m),
            ("events.bin", &[]),
            ("manifest.json", &m),
        ]);
        let err = read_bundle(Cursor::new(bytes), &ReadBundleOptions::default())
            .expect_err("duplicate manifest must fail");
        assert!(
            matches!(&err, BundleError::InvalidArchive(msg) if msg.contains("duplicate manifest.json")),
            "got {err:?}"
        );
    }

    #[test]
    fn t_read_bundle_rejects_duplicate_events_bin() {
        let m = manifest_bytes();
        let bytes = tar_zst(&[
            ("manifest.json", &m),
            ("events.bin", &[]),
            ("events.bin", &[]),
        ]);
        let err = read_bundle(Cursor::new(bytes), &ReadBundleOptions::default())
            .expect_err("duplicate events.bin must fail");
        assert!(matches!(err, BundleError::InvalidArchive(_)), "got {err:?}");
    }

    #[test]
    fn t_read_bundle_rejects_duplicate_object_entry() {
        let m = manifest_bytes();
        let name = format!("objects/{}", "ab".repeat(32));
        let bytes = tar_zst(&[
            ("manifest.json", &m),
            ("events.bin", &[]),
            (&name, b"one"),
            (&name, b"two"),
        ]);
        let err = read_bundle(Cursor::new(bytes), &ReadBundleOptions::default())
            .expect_err("duplicate object must fail");
        assert!(
            matches!(&err, BundleError::InvalidArchive(msg) if msg.contains("duplicate object")),
            "got {err:?}"
        );
    }

    #[test]
    fn t_read_bundle_rejects_uppercase_object_name() {
        let m = manifest_bytes();
        let name = format!("objects/{}", "AB".repeat(32));
        let bytes = tar_zst(&[("manifest.json", &m), ("events.bin", &[]), (&name, b"x")]);
        let err = read_bundle(Cursor::new(bytes), &ReadBundleOptions::default())
            .expect_err("uppercase object name must fail");
        assert!(
            matches!(&err, BundleError::InvalidArchive(msg) if msg.contains("lowercase")),
            "got {err:?}"
        );
    }

    /// Builds a bundle whose archive also carries a symlink entry named `evil`.
    fn bundle_with_symlink() -> Vec<u8> {
        let m = manifest_bytes();
        let mut archive_bytes = Vec::new();
        {
            let encoder = zstd::Encoder::new(&mut archive_bytes, 3).expect("zstd");
            let mut builder = tar::Builder::new(encoder.auto_finish());
            append_bytes(&mut builder, "manifest.json", &m).expect("manifest");
            append_bytes(&mut builder, "events.bin", &[]).expect("events");
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_mode(0o777);
            builder
                .append_link(&mut header, "evil", "/etc/passwd")
                .expect("symlink");
            builder.finish().expect("finish");
        }
        archive_bytes
    }

    #[test]
    fn t_read_bundle_rejects_symlink_entry_in_strict_mode() {
        let err = read_bundle(
            Cursor::new(bundle_with_symlink()),
            &ReadBundleOptions::default(),
        )
        .expect_err("symlink must fail in strict mode");
        assert!(
            matches!(&err, BundleError::UnknownBundleFile(msg) if msg.contains("evil")),
            "got {err:?}"
        );
    }

    #[test]
    fn t_read_bundle_skips_symlink_entry_with_allow_extra_files() {
        let opts = ReadBundleOptions {
            allow_extra_files: true,
            ..ReadBundleOptions::default()
        };
        assert!(read_bundle(Cursor::new(bundle_with_symlink()), &opts).is_ok());
    }

    #[test]
    fn t_read_bundle_accepts_objects_directory_entry_in_strict_mode() {
        let m = manifest_bytes();
        let mut archive_bytes = Vec::new();
        {
            let encoder = zstd::Encoder::new(&mut archive_bytes, 3).expect("zstd");
            let mut builder = tar::Builder::new(encoder.auto_finish());
            append_bytes(&mut builder, "manifest.json", &m).expect("manifest");
            append_bytes(&mut builder, "events.bin", &[]).expect("events");
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Directory);
            header.set_size(0);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "objects/", std::io::empty())
                .expect("dir");
            builder.finish().expect("finish");
        }
        assert!(read_bundle(Cursor::new(archive_bytes), &ReadBundleOptions::default()).is_ok());
    }
}
