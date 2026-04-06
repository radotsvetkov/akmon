//! Integration: `akmon config … --json` prints parseable JSON.

use std::process::Command;

#[test]
fn config_path_json_is_valid() {
    let tmp = tempfile::tempdir().expect("tmp");
    let out = Command::new(env!("CARGO_BIN_EXE_akmon"))
        .env("HOME", tmp.path())
        .args(["config", "path", "--json"])
        .output()
        .expect("spawn akmon");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be JSON");
    assert!(v.get("path").and_then(|p| p.as_str()).is_some());
}
