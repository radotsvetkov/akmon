//! High-level `tar.zst` bundle read/write helpers.

use crate::BundleError;
use crate::events::{EventsReader, EventsWriter};
use crate::manifest::Manifest;
use crate::objects::object_filename;
use akmon_journal::{AGEF_SPEC_VERSION, Event, Hash, HashAlgorithm};
use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;

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
    let algorithm = parse_algorithm(&manifest.hash_algorithm)?;

    let mut events_bytes = Vec::new();
    {
        let mut ew = EventsWriter::with_hash_algorithm(&mut events_bytes, algorithm);
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
        // TODO(byte-identical-bundles): Normalize mtime to a fixed value
        // (for example UNIX_EPOCH) to make tar bytes deterministic across
        // producers. v2.0.0 requires semantic round-trip equivalence only
        // (F6); byte-identical bundles would require this plus pinned zstd
        // settings.
        append_bytes(&mut builder, &path, bytes)?;
    }
    builder
        .finish()
        .map_err(|err| BundleError::InvalidArchive(format!("tar finalize failed: {err}")))?;
    Ok(())
}

/// Reads one AGEF bundle from `tar.zst`.
pub fn read_bundle<R: Read>(
    reader: R,
    options: &ReadBundleOptions,
) -> Result<BundleContents, BundleError> {
    let decoder = zstd::Decoder::new(reader).map_err(|err| {
        BundleError::InvalidCompression(format!("zstd decoder init failed: {err}"))
    })?;
    let mut archive = tar::Archive::new(decoder);

    let mut manifest_bytes: Option<Vec<u8>> = None;
    let mut events_bytes: Option<Vec<u8>> = None;
    let mut objects_raw: Vec<(String, Vec<u8>)> = Vec::new();

    let entries = archive
        .entries()
        .map_err(|err| BundleError::InvalidArchive(format!("tar entries failed: {err}")))?;
    for entry in entries {
        let mut entry = entry
            .map_err(|err| BundleError::InvalidArchive(format!("tar entry read failed: {err}")))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|err| BundleError::InvalidArchive(format!("invalid tar path: {err}")))?
            .to_path_buf();
        let path_str = path_to_unix(path);
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;

        match path_str.as_str() {
            "manifest.json" => manifest_bytes = Some(bytes),
            "events.bin" => events_bytes = Some(bytes),
            _ if path_str.starts_with("objects/") => {
                let name = path_str.trim_start_matches("objects/").to_owned();
                if name.contains('/') || name.is_empty() {
                    return Err(BundleError::InvalidArchive(format!(
                        "invalid object entry path: {path_str}"
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
        if name.len() != 64 || !name.as_bytes().iter().all(u8::is_ascii_hexdigit) {
            return Err(BundleError::InvalidArchive(format!(
                "invalid object filename: {name}"
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

pub(crate) fn parse_algorithm(value: &str) -> Result<HashAlgorithm, BundleError> {
    match value {
        "sha256" => Ok(HashAlgorithm::Sha256),
        "blake3" => Ok(HashAlgorithm::Blake3),
        other => Err(BundleError::UnsupportedHashAlgorithm(other.to_owned())),
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
}
