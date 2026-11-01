//! Human-readable and Markdown renderings of a [`BundleVerifyReportV1`].
//!
//! Rendered ONCE here so `akmon bundle verify` and `agef-verify` print identical wording. Every
//! manifest-sourced string (signature `scheme`/`key_id`, operator identity, `created_at`, producer)
//! is attacker-controlled and goes through [`sanitize_display`] (plus Markdown escaping for the
//! Markdown report) before it reaches a terminal or a document.
//!
//! The headline states only what was actually established: integrity alone, a signature verified
//! by a named trusted key, a signature present but not checked, or no signature at all.

use crate::report::{BundleVerifyReportV1, CaptureField, OperatorReport};
use crate::sanitize::sanitize_display;

/// Key ids of the head signatures that verified against a trusted key, sanitized.
fn verified_signer_key_ids(report: &BundleVerifyReportV1) -> Vec<String> {
    report
        .signatures
        .iter()
        .filter(|s| s.outcome == "verified")
        .map(|s| sanitize_display(&s.key_id))
        .collect()
}

/// One-line verdict built from what was actually established (TC-03).
#[must_use]
pub fn headline(report: &BundleVerifyReportV1) -> String {
    let capture_suffix = capture_human_suffix(report.capture.as_ref());
    if !report.passed {
        return format!("bundle verification FAILED{capture_suffix}");
    }
    let signature_part = if report.signature_verified {
        format!(
            "signature verified by key_id={}",
            verified_signer_key_ids(report).join(", key_id=")
        )
    } else if report.signatures.is_empty() {
        "UNSIGNED (integrity only)".to_owned()
    } else {
        "signature present but NOT verified (no trusted --verify-key matched; integrity only)"
            .to_owned()
    };
    format!("verified bundle: integrity OK; {signature_part}{capture_suffix}")
}

/// Short suffix appended to the headline so a structural import never reads as a bare
/// "verified bundle". Native bundles (`None`) and full OTEL imports add nothing.
#[must_use]
pub fn capture_human_suffix(capture: Option<&CaptureField>) -> String {
    match capture {
        Some(c) if c.level == "structural" => " — capture: STRUCTURAL (metadata only)".to_owned(),
        _ => String::new(),
    }
}

/// Prominent capture line for the human report body, or `None` for native (non-OTEL) bundles.
#[must_use]
pub fn capture_human_line(capture: Option<&CaptureField>) -> Option<String> {
    let c = capture?;
    let semconv = sanitize_display(&c.source_semconv);
    match c.level.as_str() {
        "structural" => Some(format!(
            "capture: STRUCTURAL (metadata only — message content was NOT captured by the source telemetry; source semconv {semconv})"
        )),
        "full" => Some("capture: FULL (OTEL import; message content captured)".to_owned()),
        other => Some(format!(
            "capture: {} (OTEL import; source semconv {semconv})",
            sanitize_display(&other.to_uppercase())
        )),
    }
}

/// Emits the human-readable operator-attestation block (decision D-20, honesty contract O8).
///
/// "verified" attaches to the KEY, never the self-asserted identity string. Every manifest-sourced
/// field is sanitized so it cannot spoof the surrounding report. Each line is written via `emit`
/// so both binaries share the same wording and route it to their own output stream.
pub fn print_operator_human_block(operators: &[OperatorReport], mut emit: impl FnMut(String)) {
    if operators.is_empty() {
        emit("operator: none (unattributed)".to_owned());
        return;
    }
    emit("operators:".to_owned());
    for op in operators {
        let oid = sanitize_display(&op.operator_id);
        let role = sanitize_display(&op.role);
        let org = sanitize_display(&op.org);
        let key_id = sanitize_display(&op.key_id);
        let scheme = sanitize_display(&op.scheme);
        let line = match op.outcome.as_str() {
            "verified" => format!(
                "  - verified by operator key key_id={key_id} -- self-asserted operator_id=\"{oid}\" role=\"{role}\" org=\"{org}\""
            ),
            "unverified_no_key" => format!(
                "  - present but UNVERIFIED (no trusted operator key) -- self-asserted operator_id=\"{oid}\" (do not trust this name)"
            ),
            "invalid" => format!(
                "  - INVALID signature for trusted operator key key_id={key_id} (tampered identity/head or corrupt signature; HARD FAIL)"
            ),
            other => format!(
                "  - {other} [{scheme}] key_id={key_id} -- self-asserted operator_id=\"{oid}\" (do not trust this name)"
            ),
        };
        emit(line);
    }
}

/// The full human report as lines: the headline first, then indented detail lines.
#[must_use]
pub fn human_lines(report: &BundleVerifyReportV1) -> Vec<String> {
    let mut lines = vec![format!(
        "{}: {}",
        headline(report),
        sanitize_display(&report.bundle_path)
    )];
    let mut push = |line: String| lines.push(format!("  {line}"));
    push(format!(
        "session_id: {}",
        sanitize_display(&report.session_id)
    ));
    push(format!(
        "head ({}): {}",
        sanitize_display(&report.hash_algorithm),
        sanitize_display(&report.head)
    ));
    push(format!("events: {}", report.events_in_bundle));
    if let Some(window) = &report.session_window {
        push(format!(
            "session window (event timestamps, producer clock): {} .. {}",
            window.first_event_at, window.last_event_at
        ));
    }
    push(format!("objects: {}", report.objects_in_bundle));
    if let Some(line) = capture_human_line(report.capture.as_ref()) {
        push(line);
    }
    push(format!(
        "integrity: {}",
        if report.integrity_ok { "OK" } else { "FAILED" }
    ));
    if !report.violations.is_empty() {
        push("integrity violations:".to_owned());
        for v in &report.violations {
            push(format!(
                "  - [{}] {}",
                v.category,
                sanitize_display(&v.message)
            ));
        }
    }
    if report.signatures.is_empty() {
        push("signatures: none".to_owned());
    } else {
        push("signatures:".to_owned());
        for s in &report.signatures {
            push(format!(
                "  - {} [{}] key_id={}",
                s.outcome,
                sanitize_display(&s.scheme),
                sanitize_display(&s.key_id)
            ));
        }
    }
    print_operator_human_block(&report.operators, &mut push);
    let meta = &report.unsigned_metadata;
    push(format!(
        "unsigned metadata (not covered by any signature): producer={} {} created_at={} ended_at={}",
        sanitize_display(&meta.producer_name),
        sanitize_display(&meta.producer_version),
        sanitize_display(&meta.created_at),
        sanitize_display(&meta.ended_at)
    ));
    if !report.warnings.is_empty() {
        push("warnings:".to_owned());
        for w in &report.warnings {
            push(format!(
                "  - [{}] {}",
                w.category,
                sanitize_display(&w.message)
            ));
        }
    }
    lines
}

/// Escapes an untrusted value for inline Markdown: sanitizes control/bidi characters, then
/// backslash-escapes the Markdown metacharacters that matter inline (emphasis, code, links, HTML,
/// strikethrough, and `|` so table cells stay intact). Values are always rendered after a fixed
/// prefix, never at the start of a line, so block-level markers need no escaping.
#[must_use]
pub fn markdown_escape(value: &str) -> String {
    let sanitized = sanitize_display(value);
    let mut out = String::with_capacity(sanitized.len());
    for ch in sanitized.chars() {
        if matches!(
            ch,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '|' | '~' | '&'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Head-signature cell for the Markdown checks table.
fn markdown_signature_cell(report: &BundleVerifyReportV1) -> (String, String) {
    if report.signatures.is_empty() {
        return (
            "NONE".to_owned(),
            "The bundle is unsigned. Nothing ties it to a key holder.".to_owned(),
        );
    }
    if report.signature_verified {
        let kids: Vec<String> = report
            .signatures
            .iter()
            .filter(|s| s.outcome == "verified")
            .map(|s| markdown_escape(&s.key_id))
            .collect();
        return (
            format!("VERIFIED by key_id {}", kids.join(", ")),
            "The holder of this trusted key signed this exact session head.".to_owned(),
        );
    }
    if report.signatures.iter().any(|s| s.outcome == "invalid") {
        return (
            "INVALID".to_owned(),
            "A signature names a trusted key but does not validate (tampering or corruption)."
                .to_owned(),
        );
    }
    (
        "PRESENT, NOT VERIFIED".to_owned(),
        "No trusted public key was supplied (or none matched), so the signature was not checked."
            .to_owned(),
    )
}

/// Operator-attestation cell for the Markdown checks table.
fn markdown_operator_cell(report: &BundleVerifyReportV1) -> (String, String) {
    if report.operators.is_empty() {
        return (
            "NONE".to_owned(),
            "No operator attestation is recorded.".to_owned(),
        );
    }
    let claims: Vec<String> = report
        .operators
        .iter()
        .map(|op| {
            format!(
                "claimed operator_id \"{}\" (role \"{}\") — key verified: {}",
                markdown_escape(&op.operator_id),
                markdown_escape(&op.role),
                if op.operator_key_verified {
                    format!("YES (key_id {})", markdown_escape(&op.key_id))
                } else {
                    format!("NO ({})", markdown_escape(&op.outcome))
                }
            )
        })
        .collect();
    let meaning = if report.operator_key_verified {
        "The identity text is self-asserted; what is verified is that a trusted operator key signed it."
    } else {
        "Self-asserted only: no trusted operator key verified this claim. Do not rely on the name."
    };
    (claims.join("<br>"), meaning.to_owned())
}

/// Capture-level cell for the Markdown checks table.
fn markdown_capture_cell(report: &BundleVerifyReportV1) -> (String, String) {
    match report.capture.as_ref() {
        None => (
            "FULL (native Akmon capture)".to_owned(),
            "Recorded by the reference producer with full message content.".to_owned(),
        ),
        Some(c) if c.level == "full" => (
            "FULL (OTEL import)".to_owned(),
            "The source telemetry included message content.".to_owned(),
        ),
        Some(c) if c.level == "structural" => (
            "STRUCTURAL (metadata only)".to_owned(),
            "The source telemetry did not capture message content; only structure and metadata are evidenced. This is not a full replay.".to_owned(),
        ),
        Some(c) => (
            markdown_escape(&c.level.to_uppercase()),
            "Unrecognized capture level recorded by the importer.".to_owned(),
        ),
    }
}

/// Renders a plain-language auditor report in Markdown.
#[must_use]
pub fn markdown(report: &BundleVerifyReportV1) -> String {
    let mut md = String::new();
    let mut line = |s: &str| {
        md.push_str(s);
        md.push('\n');
    };
    line("# AGEF evidence bundle verification report");
    line("");
    let verdict = if report.passed {
        if report.signature_verified {
            "PASSED — integrity verified and signature verified"
        } else {
            "PASSED — integrity verified (no signature verified)"
        }
    } else {
        "FAILED — do not rely on this bundle"
    };
    line(&format!("## VERDICT: {verdict}"));
    line("");
    line(&markdown_escape(&headline(report)));
    line("");
    line("| Check | Result | What it means |");
    line("|---|---|---|");
    line(&format!(
        "| Integrity (hash chain, objects, head, counts) | {} | {} |",
        if report.integrity_ok { "OK" } else { "FAILED" },
        if report.integrity_ok {
            "Every event and object is consistent with the session head; nothing was altered, added, or removed since the head was computed."
        } else {
            "The bundle is internally inconsistent: content was altered, added, removed, or corrupted."
        }
    ));
    let (sig_result, sig_meaning) = markdown_signature_cell(report);
    line(&format!(
        "| Head signature | {sig_result} | {sig_meaning} |"
    ));
    let (op_result, op_meaning) = markdown_operator_cell(report);
    line(&format!(
        "| Operator attestation | {op_result} | {op_meaning} |"
    ));
    let (cap_result, cap_meaning) = markdown_capture_cell(report);
    line(&format!("| Capture level | {cap_result} | {cap_meaning} |"));
    line("");
    line("## Session");
    line("");
    line(&format!(
        "- Session ID: {}",
        markdown_escape(&report.session_id)
    ));
    line(&format!(
        "- Session head ({}): {}",
        markdown_escape(&report.hash_algorithm),
        markdown_escape(&report.head)
    ));
    match &report.session_window {
        Some(w) => line(&format!(
            "- Session window (first to last event timestamp, producer clock): {} to {}",
            markdown_escape(&w.first_event_at),
            markdown_escape(&w.last_event_at)
        )),
        None => line("- Session window: no events"),
    }
    line(&format!("- Events: {}", report.events_in_bundle));
    line(&format!("- Objects: {}", report.objects_in_bundle));
    line(&format!(
        "- AGEF version: {}",
        markdown_escape(&report.agef_version)
    ));
    let meta = &report.unsigned_metadata;
    line(&format!(
        "- Producer (unsigned metadata): {} {}",
        markdown_escape(&meta.producer_name),
        markdown_escape(&meta.producer_version)
    ));
    line("");
    line("## Unsigned metadata");
    line("");
    line(
        "The following manifest fields are not covered by any signature; anyone holding the file can edit them. The event timestamps above are the ones covered by integrity.",
    );
    line("");
    line(&format!(
        "- Declared start (created_at): {}",
        markdown_escape(&meta.created_at)
    ));
    line(&format!(
        "- Declared end (ended_at): {}",
        markdown_escape(&meta.ended_at)
    ));
    if !report.violations.is_empty() {
        line("");
        line("## Violations");
        line("");
        for v in &report.violations {
            line(&format!(
                "- {}: {}",
                markdown_escape(&v.category),
                markdown_escape(&v.message)
            ));
        }
    }
    if !report.warnings.is_empty() {
        line("");
        line("## Warnings");
        line("");
        for w in &report.warnings {
            line(&format!(
                "- {}: {}",
                markdown_escape(&w.category),
                markdown_escape(&w.message)
            ));
        }
    }
    line("");
    line("## What this proves");
    line("");
    if report.integrity_ok {
        line(
            "- The events and objects in this bundle are exactly the ones the session head commits to: nothing was changed, added, or removed after the head was computed.",
        );
    } else {
        line(
            "- Nothing: integrity verification failed, so the contents cannot be tied to the session head.",
        );
    }
    if report.signature_verified {
        line(
            "- The holder of the trusted signing key named above signed this session head (and therefore these exact contents).",
        );
    }
    if report.operator_key_verified {
        line(
            "- A trusted operator key signed the operator identity claim shown above for this session head.",
        );
    }
    line("");
    line("## What this does not prove");
    line("");
    line(
        "- When the session happened. Event timestamps come from the producer's own clock; no trusted external timestamp (for example RFC 3161) is attached or checked by this report.",
    );
    if !report.signature_verified {
        line(
            "- Who produced the bundle. No signature was verified against a trusted key, so anyone could have created or re-created it.",
        );
    }
    line(
        "- Who a key belongs to. Binding a public key to a person or organization is outside this report.",
    );
    line(
        "- Operator identity. The operator_id, name, role, and organization are self-asserted text; only the key that signed them can be verified.",
    );
    if report.capture.as_ref().is_some_and(|c| c.level != "full") {
        line(
            "- What was said. Structural capture records metadata only, not message content; it is not a full replay of the session.",
        );
    }
    line(
        "- That the agent behaved correctly or that any obligation was met. This report covers evidence integrity and provenance only.",
    );
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{SignatureReport, UnsignedMetadata};

    fn report(passed: bool, signatures: Vec<SignatureReport>) -> BundleVerifyReportV1 {
        let signature_verified = signatures.iter().any(|s| s.outcome == "verified");
        BundleVerifyReportV1 {
            akmon_version: "t".to_owned(),
            agef_version: "0.1.3".to_owned(),
            bundle_path: "b.akmon".to_owned(),
            session_id: "s".to_owned(),
            events_in_bundle: 2,
            objects_in_bundle: 2,
            passed,
            violations: Vec::new(),
            signatures,
            operators: Vec::new(),
            capture: None,
            integrity_ok: passed,
            signature_verified,
            operator_key_verified: false,
            hash_algorithm: "sha256".to_owned(),
            head: "ab".repeat(32),
            session_window: None,
            unsigned_metadata: UnsignedMetadata {
                created_at: "c".to_owned(),
                ended_at: "e".to_owned(),
                producer_name: "akmon".to_owned(),
                producer_version: "v".to_owned(),
            },
            warnings: Vec::new(),
        }
    }

    fn sig(outcome: &str, key_id: &str) -> SignatureReport {
        SignatureReport {
            key_id: key_id.to_owned(),
            scheme: "ed25519".to_owned(),
            outcome: outcome.to_owned(),
        }
    }

    #[test]
    fn t_headline_variants() {
        assert_eq!(
            headline(&report(true, vec![])),
            "verified bundle: integrity OK; UNSIGNED (integrity only)"
        );
        assert!(
            headline(&report(true, vec![sig("unverified_no_key", "k")])).contains("NOT verified")
        );
        assert_eq!(
            headline(&report(true, vec![sig("verified", "kid1")])),
            "verified bundle: integrity OK; signature verified by key_id=kid1"
        );
        assert_eq!(
            headline(&report(false, vec![])),
            "bundle verification FAILED"
        );
    }

    #[test]
    fn t_headline_sanitizes_key_id() {
        let h = headline(&report(true, vec![sig("verified", "k\n- verified")]));
        assert!(!h.contains('\n'));
    }

    #[test]
    fn t_markdown_escape_neutralizes_tables_and_html() {
        assert_eq!(markdown_escape("a|b"), "a\\|b");
        assert_eq!(markdown_escape("<img>"), "\\<img\\>");
        assert_eq!(markdown_escape("*x*_y_`z`"), "\\*x\\*\\_y\\_\\`z\\`");
        assert_eq!(markdown_escape("line\nbreak"), "line\\\\x0abreak");
    }

    #[test]
    fn t_markdown_mentions_limits() {
        let md = markdown(&report(true, vec![]));
        assert!(md.contains("## VERDICT: PASSED"));
        assert!(md.contains("What this does not prove"));
        assert!(md.contains("Who produced the bundle"));
    }
}
