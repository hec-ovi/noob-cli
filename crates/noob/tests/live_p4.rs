//! Live P4 smoke (opt-in: `./dev.sh smoke`): a real web lookup through the
//! `websearch` tool, driven by the local model through the shipped binary.
//! This is live smoke item 6: the registered tool routes the model to a real
//! search, and the result shapes the answer. Needs the websearch CLI on PATH
//! (`uv tool install websearch-skill`) and working egress.

use std::process::Command;

use serde_json::Value;

fn live_base_url() -> String {
    std::env::var("NOOB_LIVE_BASE_URL").unwrap_or_else(|_| "http://localhost:8080/v1".to_string())
}

/// llama-server serves whatever it loaded under its `--alias`; the default
/// here matches the local server, and any other endpoint sets its own.
fn live_model() -> String {
    std::env::var("NOOB_LIVE_MODEL").unwrap_or_else(|_| "llm".to_string())
}

#[test]
#[ignore = "live: needs a local endpoint at :8080 and the websearch CLI on PATH (./dev.sh smoke)"]
fn live_websearch_through_the_tool() {
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(
        config.path().join(".env"),
        format!(
            "NOOB_BASE_URL={}\nNOOB_API_KEY=noauth\nNOOB_MODEL={}\n",
            live_base_url(),
            live_model()
        ),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_noob"))
        .env("NOOB_CONFIG_DIR", config.path())
        .current_dir(work.path())
        .env_remove("NOOB_BASE_URL")
        .args([
            "exec",
            "--json",
            "-p",
            "Use the websearch tool to search the web for \"Rust programming \
             language\" and answer in one line: what year did Rust 1.0 come out?",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let events: Vec<Value> = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    // The model reached the web through the tool...
    let calls: Vec<&Value> = events
        .iter()
        .filter(|e| e["t"] == "tool" && e["name"] == "websearch")
        .collect();
    assert!(!calls.is_empty(), "no websearch call in: {stdout}");
    assert!(
        calls.iter().any(|c| c["args"]["action"] == "search"),
        "no websearch search action in: {stdout}"
    );
    // ...and produced a grounded answer (Rust 1.0 shipped in 2015).
    let text: String = events
        .iter()
        .filter(|e| e["t"] == "text")
        .filter_map(|e| e["d"].as_str())
        .collect();
    assert!(
        text.contains("2015"),
        "answer not grounded by the search: {text}"
    );
}
