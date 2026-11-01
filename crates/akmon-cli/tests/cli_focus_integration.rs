//! v3.0 focus release: archived commands are gone, deprecated ones are hidden, trust-path
//! commands stay quiet outside a git repository, and journal lock conflicts get their own
//! JSON error category.

use std::path::Path;
use std::process::{Command, Output};

use akmon_journal::RedbObjectStore;
use serde_json::Value;
use tempfile::tempdir;
use uuid::Uuid;

#[allow(dead_code)]
mod common;
use common::{akmon_bin_path, create_clean_session, journal_db_path};

fn akmon(cwd: &Path, args: &[&str]) -> Output {
    Command::new(akmon_bin_path())
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("run akmon")
}

fn stdout_json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

#[test]
fn archived_commands_and_flags_are_rejected() {
    let tmp = tempdir().expect("tmp");
    for args in [
        &["slo", "verify", "x.json"][..],
        &["spec", "feature"],
        &["scout", "--task", "t"],
        &["import"],
        &["export", "--all"],
        &["sign", "550e8400-e29b-41d4-a716-446655440000"],
        &["--index", "chat"],
        &["--dossier", "d.json", "chat"],
    ] {
        let out = akmon(tmp.path(), args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`akmon {}` must be a usage error, stderr: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn help_leads_with_evidence_positioning_and_hides_deprecated_commands() {
    let tmp = tempdir().expect("tmp");
    let out = akmon(tmp.path(), &["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("Seal AI agent sessions into signed, tamper-evident evidence"));
    assert!(help.contains("Agent options"));
    let commands: Vec<&str> = help
        .lines()
        .skip_while(|l| !l.starts_with("Commands:"))
        .skip(1)
        .take_while(|l| !l.trim().is_empty())
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    for hidden in [
        "audit", "evidence", "sign", "slo", "spec", "scout", "import", "export",
    ] {
        assert!(
            !commands.contains(&hidden),
            "`{hidden}` must not be listed in --help: {commands:?}"
        );
    }
    assert_eq!(commands.first(), Some(&"seal"), "trust commands come first");
    assert_eq!(
        commands.get(1),
        Some(&"bundle"),
        "trust commands come first"
    );
}

#[test]
fn deprecated_audit_and_evidence_verify_still_run_with_notice() {
    let tmp = tempdir().expect("tmp");
    for cmd in ["audit", "evidence"] {
        let out = akmon(tmp.path(), &[cmd, "verify", "missing-file"]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(&format!("`akmon {cmd} verify` is deprecated"))
                && stderr.contains("akmon bundle verify"),
            "{cmd}: missing deprecation notice: {stderr}"
        );
        assert_eq!(out.status.code(), Some(1), "{cmd}: legacy exit code kept");
    }
}

#[test]
fn trust_commands_print_no_git_sandbox_notice_outside_a_repository() {
    let cwd = tempdir().expect("cwd");
    let journal = tempdir().expect("journal");
    let sid = Uuid::new_v4();
    create_clean_session(journal.path(), sid);
    let journal_arg = journal.path().display().to_string();
    let sid_arg = sid.to_string();
    let out = akmon(cwd.path(), &["verify", &sid_arg, "--journal", &journal_arg]);
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("no git repository found"),
        "verify must not print the agent sandbox notice: {stderr}"
    );
}

#[test]
fn inspect_binary_without_resolve_is_json_usage_error_in_json_mode() {
    let tmp = tempdir().expect("tmp");
    let out = akmon(
        tmp.path(),
        &[
            "inspect",
            "550e8400-e29b-41d4-a716-446655440000",
            "--binary",
            "hex",
            "--format",
            "json",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
    let v = stdout_json(&out);
    assert_eq!(v["category"], "usage");
    let err = v["error"].as_str().unwrap_or_default();
    assert!(err.contains("--binary hex requires --resolve"), "{err}");

    let human = akmon(
        tmp.path(),
        &[
            "inspect",
            "550e8400-e29b-41d4-a716-446655440000",
            "--binary",
            "base64",
        ],
    );
    assert_eq!(human.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&human.stderr).contains("--binary base64 requires --resolve"));
}

#[test]
fn locked_journal_is_reported_as_journal_locked() {
    let journal = tempdir().expect("journal");
    let sid = Uuid::new_v4();
    create_clean_session(journal.path(), sid);
    // Hold redb's exclusive lock, as the TUI or a running agent would.
    let _held = RedbObjectStore::open(&journal_db_path(journal.path())).expect("hold journal");
    let cwd = tempdir().expect("cwd");
    let journal_arg = journal.path().display().to_string();
    let sid_arg = sid.to_string();
    let out_path = cwd.path().join("out.akmon").display().to_string();
    for args in [
        vec![
            "verify",
            &sid_arg,
            "--journal",
            &journal_arg,
            "--format",
            "json",
        ],
        vec![
            "inspect",
            &sid_arg,
            "--journal",
            &journal_arg,
            "--format",
            "json",
        ],
        vec![
            "redact",
            &sid_arg,
            "--journal",
            &journal_arg,
            "--output",
            &out_path,
            "--object",
            "00",
            "--reason",
            "test",
            "--format",
            "json",
        ],
        vec![
            "bundle",
            "export",
            &sid_arg,
            "--journal",
            &journal_arg,
            "--output",
            &out_path,
            "--format",
            "json",
        ],
    ] {
        let out = akmon(cwd.path(), &args);
        assert_eq!(out.status.code(), Some(3), "{args:?}");
        let v = stdout_json(&out);
        assert_eq!(v["category"], "journal_locked", "{args:?}: {v}");
    }
}

#[test]
fn policy_show_effective_conflicting_profiles_is_a_usage_error() {
    let tmp = tempdir().expect("tmp");
    let out = akmon(
        tmp.path(),
        &[
            "--policy-profile",
            "prod",
            "policy",
            "show-effective",
            "--profile",
            "dev",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--policy-profile prod and --profile dev"),
        "{stderr}"
    );

    let out = akmon(
        tmp.path(),
        &[
            "--output",
            "json",
            "--policy-profile",
            "prod",
            "policy",
            "show-effective",
            "--profile",
            "dev",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
    let v = stdout_json(&out);
    assert_eq!(v["ok"], false);
    assert_eq!(v["category"], "usage");
    assert!(
        v["error"]
            .as_str()
            .is_some_and(|e| e.contains("--policy-profile prod and --profile dev")),
        "{v}"
    );
}

#[test]
fn locked_journal_is_journal_locked_for_replay_diff_and_imports() {
    let journal = tempdir().expect("journal");
    let sid = Uuid::new_v4();
    create_clean_session(journal.path(), sid);
    let cwd = tempdir().expect("cwd");
    let journal_arg = journal.path().display().to_string();
    let sid_arg = sid.to_string();
    let other_sid = Uuid::new_v4().to_string();
    let bundle = cwd.path().join("b.akmon").display().to_string();
    let export = akmon(
        cwd.path(),
        &[
            "bundle",
            "export",
            &sid_arg,
            "--journal",
            &journal_arg,
            "--output",
            &bundle,
        ],
    );
    assert_eq!(
        export.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    let trace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/openai_v2_weather_legacy.otlp.json")
        .display()
        .to_string();
    let _held = RedbObjectStore::open(&journal_db_path(journal.path())).expect("hold journal");
    for args in [
        vec![
            "replay",
            &sid_arg,
            "--journal",
            &journal_arg,
            "--format",
            "json",
        ],
        vec![
            "diff",
            &sid_arg,
            &other_sid,
            "--journal",
            &journal_arg,
            "--format",
            "json",
        ],
        vec![
            "bundle",
            "import",
            &bundle,
            "--journal",
            &journal_arg,
            "--rename-to",
            &other_sid,
            "--format",
            "json",
        ],
        vec![
            "otel",
            "import",
            &trace,
            "--journal",
            &journal_arg,
            "--format",
            "json",
        ],
    ] {
        let out = akmon(cwd.path(), &args);
        assert_eq!(out.status.code(), Some(3), "{args:?}");
        let v = stdout_json(&out);
        assert_eq!(v["category"], "journal_locked", "{args:?}: {v}");
    }
}
