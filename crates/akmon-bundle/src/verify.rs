//! Self-contained, store-independent AGEF bundle integrity verification.
//!
//! [`read_bundle`](crate::read_bundle) only *parses* a bundle: it trusts each
//! `objects/<hex>` filename as the object key and trusts the manifest. This
//! module re-verifies a parsed [`BundleContents`] against the AGEF integrity
//! rules (AGEF §A.10) using only the bundle's own bytes — no
//! [`ObjectStore`](akmon_journal::ObjectStore) and no journal handle required:
//!
//! - every object's bytes re-hash to its declared key (`objects/<hex>`),
//! - the event sequence is `0..n-1`,
//! - the first event is `SessionStart` with no parents,
//! - each later event links to exactly the prior event's content hash,
//! - every object referenced by an event is present in the bundle,
//! - there is exactly one terminal `SessionEnd`,
//! - the manifest `session.head` equals the terminal event's content hash, and
//! - the manifest `object_count` / `event_count` match the bundle contents.
//!
//! This is an intentionally independent second implementation of the checks in
//! [`akmon_journal::verify_linear_history_against_store`]. For a tamper-evidence
//! product, verifying the portable artifact through a path that does not depend
//! on the redb-backed store is defense in depth: a defect in one verifier does
//! not silently pass tampered evidence through the other. It also lets a
//! third-party AGEF reader verify a bundle without taking a journal/store
//! dependency.
//!
//! Violation category strings are kept aligned with the CLI
//! `BundleVerifyReportV1` categories so the command surface can delegate here
//! without changing its machine-readable contract.

use crate::archive::{BundleContents, parse_algorithm};
use crate::error::BundleError;
use akmon_journal::{EventKind, Hash, digest_bytes, referenced_object_hashes_for_kind};

/// A single integrity violation found while verifying a parsed bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleViolation {
    /// `manifest.hash_algorithm` is not supported by this reader.
    UnsupportedHashAlgorithm(String),
    /// `manifest.object_count` does not match the number of `objects/` entries.
    ObjectCountMismatch {
        /// Count declared by the manifest.
        declared: u64,
        /// Count actually present in the bundle.
        actual: u64,
    },
    /// `manifest.event_count` does not match the number of decoded events.
    EventCountMismatch {
        /// Count declared by the manifest.
        declared: u64,
        /// Count actually present in the bundle.
        actual: u64,
    },
    /// An object's bytes do not hash to its `objects/<hex>` key.
    ObjectKeyHashMismatch {
        /// The object key (declared digest) whose bytes did not match.
        object_hash: Hash,
    },
    /// The bundle declares no events (no `SessionStart`).
    NoEvents,
    /// An event's `sequence` does not equal its position.
    SequenceMismatch {
        /// Zero-based position in the event stream.
        position: usize,
        /// The (incorrect) declared sequence value.
        declared: u64,
    },
    /// An event's content could not be canonically hashed.
    EventContentHash {
        /// Zero-based position in the event stream.
        position: usize,
        /// Underlying hashing error description.
        error: String,
    },
    /// An event's parent linkage is broken.
    BrokenParentChain {
        /// Zero-based position in the event stream.
        position: usize,
        /// Human-readable explanation.
        detail: String,
    },
    /// An object referenced by an event is absent from the bundle.
    MissingObject {
        /// The missing object's declared hash.
        object_hash: Hash,
        /// Content hash of the event that referenced it.
        referenced_by: Hash,
    },
    /// No `SessionEnd` event is present.
    SessionEndMissing,
    /// More than one `SessionEnd` event is present.
    SessionEndDuplicate {
        /// Number of `SessionEnd` events found.
        count: usize,
    },
    /// A single `SessionEnd` exists but is not the terminal event.
    SessionEndNotTerminal,
    /// `manifest.session.head` is not a valid digest for the active algorithm.
    InvalidManifestHead {
        /// Underlying parse error description.
        detail: String,
    },
    /// `manifest.session.head` does not match the terminal event hash.
    HeadMismatch {
        /// Head declared by the manifest.
        declared: Hash,
        /// Content hash of the terminal event.
        terminal: Hash,
    },
}

impl BundleViolation {
    /// Returns the stable category identifier for this violation.
    ///
    /// These strings match the CLI `BundleVerifyReportV1` categories.
    #[must_use]
    pub fn category(&self) -> &'static str {
        match self {
            Self::UnsupportedHashAlgorithm(_) => "unsupported_hash_algorithm",
            Self::ObjectCountMismatch { .. } => "object_count_mismatch",
            Self::EventCountMismatch { .. } => "event_count_mismatch",
            Self::ObjectKeyHashMismatch { .. } => "object_key_hash_mismatch",
            Self::NoEvents => "no_events",
            Self::SequenceMismatch { .. } => "sequence",
            Self::EventContentHash { .. } => "event_content_hash",
            Self::BrokenParentChain { .. } => "broken_parent_chain",
            Self::MissingObject { .. } => "missing_object",
            Self::SessionEndMissing => "session_end_missing",
            Self::SessionEndDuplicate { .. } => "session_end_duplicate",
            Self::SessionEndNotTerminal => "session_end_not_terminal",
            Self::InvalidManifestHead { .. } => "invalid_manifest_head",
            Self::HeadMismatch { .. } => "head_mismatch",
        }
    }

    /// Returns a human-readable description of this violation.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::UnsupportedHashAlgorithm(name) => {
                format!("manifest hash_algorithm is not supported: {name}")
            }
            Self::ObjectCountMismatch { declared, actual } => format!(
                "manifest object_count {declared} does not match objects/ entry count {actual}"
            ),
            Self::EventCountMismatch { declared, actual } => format!(
                "manifest event_count {declared} does not match events.bin event count {actual}"
            ),
            Self::ObjectKeyHashMismatch { object_hash } => format!(
                "object bytes do not match object path digest {}",
                object_hash.to_hex()
            ),
            Self::NoEvents => "bundle contains no events".to_owned(),
            Self::SequenceMismatch { position, declared } => {
                format!("event at position {position} declares sequence {declared}")
            }
            Self::EventContentHash { position, error } => {
                format!("event at position {position} failed content hashing: {error}")
            }
            Self::BrokenParentChain { position, detail } => {
                format!("event at position {position}: {detail}")
            }
            Self::MissingObject {
                object_hash,
                referenced_by,
            } => format!(
                "object {} referenced by event {} is not present in bundle",
                object_hash.to_hex(),
                referenced_by.to_hex()
            ),
            Self::SessionEndMissing => "SessionEnd event is missing".to_owned(),
            Self::SessionEndDuplicate { count } => {
                format!("SessionEnd appears multiple times (count={count})")
            }
            Self::SessionEndNotTerminal => "SessionEnd is not the terminal event".to_owned(),
            Self::InvalidManifestHead { detail } => {
                format!("manifest session.head is not a valid digest: {detail}")
            }
            Self::HeadMismatch { declared, terminal } => format!(
                "declared head does not match terminal event hash (declared {}, terminal {})",
                declared.to_hex(),
                terminal.to_hex()
            ),
        }
    }
}

/// Fail-soft result of verifying a parsed bundle's integrity.
///
/// All detectable violations are accumulated rather than failing on the first,
/// so an auditor sees every problem in one pass. Use [`BundleVerificationReport::is_clean`]
/// for the pass/fail decision.
#[derive(Debug, Clone, Default)]
pub struct BundleVerificationReport {
    /// Number of events walked.
    pub events_checked: u64,
    /// Number of object references resolved against the bundle object set.
    pub object_references_checked: u64,
    /// All violations found (empty when the bundle passes).
    pub violations: Vec<BundleViolation>,
}

impl BundleVerificationReport {
    /// Returns `true` when no violations were recorded.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Verifies a parsed bundle's integrity and returns a fail-soft report.
///
/// This performs no I/O and requires no object store; it operates solely on the
/// already-decoded [`BundleContents`].
#[must_use]
pub fn verify_bundle(contents: &BundleContents) -> BundleVerificationReport {
    let mut report = BundleVerificationReport::default();

    let algorithm = match parse_algorithm(&contents.manifest.hash_algorithm) {
        Ok(algorithm) => algorithm,
        Err(_) => {
            report
                .violations
                .push(BundleViolation::UnsupportedHashAlgorithm(
                    contents.manifest.hash_algorithm.clone(),
                ));
            return report;
        }
    };

    // Declared counts must match what the archive actually contains.
    if contents.manifest.object_count != contents.objects.len() as u64 {
        report
            .violations
            .push(BundleViolation::ObjectCountMismatch {
                declared: contents.manifest.object_count,
                actual: contents.objects.len() as u64,
            });
    }
    if contents.manifest.event_count != contents.events.len() as u64 {
        report.violations.push(BundleViolation::EventCountMismatch {
            declared: contents.manifest.event_count,
            actual: contents.events.len() as u64,
        });
    }

    // Every stored object's bytes must re-hash to its declared key.
    for (hash, bytes) in &contents.objects {
        if digest_bytes(algorithm, bytes.as_slice()) != *hash {
            report
                .violations
                .push(BundleViolation::ObjectKeyHashMismatch {
                    object_hash: hash.clone(),
                });
        }
    }

    if contents.events.is_empty() {
        report.violations.push(BundleViolation::NoEvents);
        return report;
    }

    let mut session_end_count = 0_usize;
    let mut last_session_end_position: Option<usize> = None;
    // Content hash of the previous event; `None` after a non-hashable event so a
    // single hashing failure does not cascade into spurious parent-link errors.
    let mut prev_hash: Option<Hash> = None;

    for (idx, event) in contents.events.iter().enumerate() {
        report.events_checked += 1;

        if matches!(event.kind, EventKind::SessionEnd { .. }) {
            session_end_count += 1;
            last_session_end_position = Some(idx);
        }

        if event.sequence != idx as u64 {
            report.violations.push(BundleViolation::SequenceMismatch {
                position: idx,
                declared: event.sequence,
            });
        }

        let content_hash = match event.content_hash(algorithm) {
            Ok(hash) => hash,
            Err(err) => {
                report.violations.push(BundleViolation::EventContentHash {
                    position: idx,
                    error: err.to_string(),
                });
                prev_hash = None;
                continue;
            }
        };

        if idx == 0 {
            if !matches!(event.kind, EventKind::SessionStart { .. }) || !event.parents.is_empty() {
                report.violations.push(BundleViolation::BrokenParentChain {
                    position: idx,
                    detail: "first event must be SessionStart with no parents".to_owned(),
                });
            }
        } else if let Some(prev) = prev_hash.as_ref()
            && (event.parents.len() != 1 || event.parents.first() != Some(prev))
        {
            report.violations.push(BundleViolation::BrokenParentChain {
                position: idx,
                detail: format!(
                    "parent does not match prior event hash (expected {})",
                    prev.to_hex()
                ),
            });
        }

        for object_hash in referenced_object_hashes_for_kind(&event.kind) {
            report.object_references_checked += 1;
            if !contents.objects.contains_key(&object_hash) {
                report.violations.push(BundleViolation::MissingObject {
                    object_hash,
                    referenced_by: content_hash.clone(),
                });
            }
        }

        prev_hash = Some(content_hash);
    }

    match session_end_count {
        0 => report.violations.push(BundleViolation::SessionEndMissing),
        1 => {
            if last_session_end_position != Some(contents.events.len() - 1) {
                report
                    .violations
                    .push(BundleViolation::SessionEndNotTerminal);
            }
        }
        count => report
            .violations
            .push(BundleViolation::SessionEndDuplicate { count }),
    }

    // `prev_hash` is the terminal event's content hash iff the final event hashed
    // successfully; skip the head check otherwise (the hashing failure is already reported).
    if let Some(terminal) = prev_hash {
        match Hash::parse_hex(algorithm, &contents.manifest.session.head) {
            Ok(declared) => {
                if declared != terminal {
                    report
                        .violations
                        .push(BundleViolation::HeadMismatch { declared, terminal });
                }
            }
            Err(err) => report
                .violations
                .push(BundleViolation::InvalidManifestHead {
                    detail: err.to_string(),
                }),
        }
    }

    report
}

/// Verifies a parsed bundle and returns an error on the first violation.
///
/// Integrity violations map onto the dedicated [`BundleError`] variants
/// ([`BundleError::ObjectHashMismatch`], [`BundleError::MissingObject`],
/// [`BundleError::HeadMismatch`]); structural and manifest violations map onto
/// [`BundleError::InvalidArchive`] / [`BundleError::InvalidManifest`].
pub fn verify_bundle_strict(contents: &BundleContents) -> Result<(), BundleError> {
    let report = verify_bundle(contents);
    match report.violations.first() {
        None => Ok(()),
        Some(violation) => Err(violation_to_error(violation)),
    }
}

fn violation_to_error(violation: &BundleViolation) -> BundleError {
    match violation {
        BundleViolation::ObjectKeyHashMismatch { object_hash } => {
            BundleError::ObjectHashMismatch(object_hash.to_hex())
        }
        BundleViolation::MissingObject { object_hash, .. } => {
            BundleError::MissingObject(object_hash.to_hex())
        }
        BundleViolation::HeadMismatch { declared, terminal } => BundleError::HeadMismatch {
            expected: declared.to_hex(),
            found: terminal.to_hex(),
        },
        BundleViolation::UnsupportedHashAlgorithm(name) => {
            BundleError::UnsupportedHashAlgorithm(name.clone())
        }
        BundleViolation::ObjectCountMismatch { .. }
        | BundleViolation::EventCountMismatch { .. }
        | BundleViolation::InvalidManifestHead { .. } => BundleError::InvalidManifest(format!(
            "{}: {}",
            violation.category(),
            violation.message()
        )),
        other => BundleError::InvalidArchive(format!("{}: {}", other.category(), other.message())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::{
        ReadBundleOptions, WriteBundleOptions, read_verified_bundle, write_bundle,
    };
    use crate::manifest::{Manifest, Producer, SessionMetadata};
    use akmon_journal::{AGEF_SPEC_VERSION, Event, EventKind, HashAlgorithm};
    use std::collections::{BTreeMap, HashMap};
    use std::io::Cursor;

    fn algo() -> HashAlgorithm {
        HashAlgorithm::Sha256
    }

    fn object(byte: u8) -> (Hash, Vec<u8>) {
        let bytes = vec![byte; 8];
        (digest_bytes(algo(), &bytes), bytes)
    }

    fn ts(seconds: i64) -> time::OffsetDateTime {
        time::OffsetDateTime::from_unix_timestamp(seconds).expect("ts")
    }

    /// Builds a valid two-event session (`SessionStart` -> `SessionEnd`) plus a
    /// well-formed manifest whose head matches the terminal event hash.
    fn valid_bundle() -> BundleContents {
        let (cwd_hash, cwd_bytes) = object(0x11);
        let (config_hash, config_bytes) = object(0x12);

        let start = Event {
            parents: vec![],
            kind: EventKind::SessionStart {
                cwd_hash: cwd_hash.clone(),
                config_hash: config_hash.clone(),
            },
            emitted_at: ts(1_700_000_000),
            sequence: 0,
        };
        let start_hash = start.content_hash(algo()).expect("start hash");

        let end = Event {
            parents: vec![start_hash.clone()],
            kind: EventKind::SessionEnd { summary_hash: None },
            emitted_at: ts(1_700_000_001),
            sequence: 1,
        };
        let end_hash = end.content_hash(algo()).expect("end hash");

        let objects = HashMap::from([(cwd_hash, cwd_bytes), (config_hash, config_bytes)]);
        let manifest = Manifest {
            agef_version: AGEF_SPEC_VERSION.to_owned(),
            producer: Producer {
                name: "akmon".to_owned(),
                version: "test".to_owned(),
            },
            session: SessionMetadata {
                id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
                head: end_hash.to_hex(),
                created_at: "2026-05-04T14:00:00Z".to_owned(),
                ended_at: "2026-05-04T14:01:00Z".to_owned(),
            },
            hash_algorithm: "sha256".to_owned(),
            object_count: 2,
            event_count: 2,
            extra: BTreeMap::new(),
        };

        BundleContents {
            manifest,
            events: vec![start, end],
            objects,
        }
    }

    fn categories(report: &BundleVerificationReport) -> Vec<&'static str> {
        report
            .violations
            .iter()
            .map(BundleViolation::category)
            .collect()
    }

    #[test]
    fn t_valid_bundle_is_clean() {
        let contents = valid_bundle();
        let report = verify_bundle(&contents);
        assert!(report.is_clean(), "violations: {:?}", report.violations);
        assert_eq!(report.events_checked, 2);
        assert!(verify_bundle_strict(&contents).is_ok());
    }

    #[test]
    fn t_corrupt_object_bytes_detected() {
        let mut contents = valid_bundle();
        // Replace one object's bytes so they no longer hash to the stored key.
        let key = contents.objects.keys().next().expect("object").clone();
        contents.objects.insert(key, b"tampered".to_vec());

        let report = verify_bundle(&contents);
        assert!(categories(&report).contains(&"object_key_hash_mismatch"));
        assert!(matches!(
            verify_bundle_strict(&contents),
            Err(BundleError::ObjectHashMismatch(_))
        ));
    }

    #[test]
    fn t_head_mismatch_detected() {
        let mut contents = valid_bundle();
        // Point head at the (non-terminal) SessionStart event hash.
        let start_hash = contents.events[0].content_hash(algo()).expect("hash");
        contents.manifest.session.head = start_hash.to_hex();

        let report = verify_bundle(&contents);
        assert!(categories(&report).contains(&"head_mismatch"));
        assert!(matches!(
            verify_bundle_strict(&contents),
            Err(BundleError::HeadMismatch { .. })
        ));
    }

    #[test]
    fn t_missing_referenced_object_detected() {
        let mut contents = valid_bundle();
        let referenced = match &contents.events[0].kind {
            EventKind::SessionStart { cwd_hash, .. } => cwd_hash.clone(),
            _ => unreachable!("first event is SessionStart"),
        };
        contents.objects.remove(&referenced);
        contents.manifest.object_count = contents.objects.len() as u64;

        let report = verify_bundle(&contents);
        assert!(categories(&report).contains(&"missing_object"));
        assert!(matches!(
            verify_bundle_strict(&contents),
            Err(BundleError::MissingObject(_))
        ));
    }

    #[test]
    fn t_object_count_mismatch_detected() {
        let mut contents = valid_bundle();
        contents.manifest.object_count = 99;
        let report = verify_bundle(&contents);
        assert!(categories(&report).contains(&"object_count_mismatch"));
    }

    #[test]
    fn t_event_count_mismatch_detected() {
        let mut contents = valid_bundle();
        contents.manifest.event_count = 1;
        let report = verify_bundle(&contents);
        assert!(categories(&report).contains(&"event_count_mismatch"));
    }

    #[test]
    fn t_broken_parent_chain_detected() {
        let mut contents = valid_bundle();
        contents.events[1].parents.clear();
        let report = verify_bundle(&contents);
        assert!(categories(&report).contains(&"broken_parent_chain"));
    }

    #[test]
    fn t_sequence_mismatch_detected() {
        let mut contents = valid_bundle();
        contents.events[1].sequence = 7;
        let report = verify_bundle(&contents);
        assert!(categories(&report).contains(&"sequence"));
    }

    #[test]
    fn t_session_end_missing_detected() {
        let mut contents = valid_bundle();
        // Drop the terminal SessionEnd, leaving only SessionStart.
        contents.events.truncate(1);
        contents.manifest.event_count = 1;
        // Head now must match the remaining terminal event (SessionStart).
        let start_hash = contents.events[0].content_hash(algo()).expect("hash");
        contents.manifest.session.head = start_hash.to_hex();

        let report = verify_bundle(&contents);
        assert!(categories(&report).contains(&"session_end_missing"));
    }

    #[test]
    fn t_no_events_detected() {
        let mut contents = valid_bundle();
        contents.events.clear();
        contents.manifest.event_count = 0;
        let report = verify_bundle(&contents);
        assert!(categories(&report).contains(&"no_events"));
    }

    #[test]
    fn t_read_verified_bundle_round_trip_ok() {
        let contents = valid_bundle();
        let mut out = Vec::new();
        write_bundle(
            &mut out,
            &contents.manifest,
            &contents.events,
            &contents.objects,
            &WriteBundleOptions::default(),
        )
        .expect("write");

        let parsed = read_verified_bundle(Cursor::new(out), &ReadBundleOptions::default())
            .expect("verified read");
        assert_eq!(parsed.events.len(), 2);
    }
}
