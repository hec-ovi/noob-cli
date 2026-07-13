//! The egress invariant, mechanically enforced: only noob-provider may
//! depend on ureq (the sole path to the network stack), checked against
//! cargo metadata; and a binary run against the mock touches no URL other
//! than the configured base.

use std::process::Command;

use noob_testkit::MockServer;
use serde_json::Value;

#[test]
fn only_the_provider_depends_on_ureq() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR"); // crates/noob
    // --no-deps: the declared dependencies of the workspace members are the
    // whole check, and skipping resolution keeps the test offline-clean
    // (locked-but-never-built optional deps have no cached manifests).
    let out = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .current_dir(manifest_dir)
        .output()
        .expect("cargo metadata runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let meta: Value = serde_json::from_slice(&out.stdout).unwrap();

    let members: Vec<&str> = meta["workspace_members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect();
    let mut ureq_users = Vec::new();
    for pkg in meta["packages"].as_array().unwrap() {
        if !members.iter().any(|m| *m == pkg["id"].as_str().unwrap()) {
            continue;
        }
        let name = pkg["name"].as_str().unwrap();
        let depends_on_ureq = pkg["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["name"] == "ureq");
        if depends_on_ureq {
            ureq_users.push(name.to_string());
        }
    }
    assert_eq!(
        ureq_users,
        vec!["noob-provider"],
        "ureq (the network stack) leaked outside noob-provider"
    );
}

/// Every request the binary makes lands on the configured base; nothing
/// phones anywhere else (no telemetry, no update checks, no title calls).
#[test]
fn zero_foreign_requests() {
    let server = MockServer::start();
    server.enqueue_stream_completion("quiet");
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(
        config.path().join(".env"),
        format!("NOOB_BASE_URL={}\n", server.base_url()),
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_noob"))
        .env("NOOB_CONFIG_DIR", config.path())
        .current_dir(work.path())
        .args(["exec", "-p", "hi"])
        .output()
        .unwrap();
    assert!(out.status.success());

    for req in server.recorded() {
        assert!(
            req.path.starts_with("/v1/"),
            "unexpected request path {}",
            req.path
        );
    }
    server.assert_clean();
}
