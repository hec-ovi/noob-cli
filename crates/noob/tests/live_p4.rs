//! Live P4 smoke (opt-in: NOOB_LIVE=1, `./dev.sh smoke`): a real call
//! through the websearch MCP server at :8000, driven by qwen through the
//! shipped binary. This is live smoke item 6: the prompt line routes the
//! model to mcp_connect, the catalog routes it to mcp_call, and the
//! search result shapes the answer.

use std::process::Command;

use serde_json::Value;

fn live_base_url() -> String {
    std::env::var("NOOB_LIVE_BASE_URL").unwrap_or_else(|_| "http://localhost:8090/v1".to_string())
}

fn websearch_url() -> String {
    std::env::var("NOOB_LIVE_MCP_URL").unwrap_or_else(|_| "http://localhost:8000/mcp".to_string())
}

#[test]
#[ignore = "live: needs qwen at :8090 and the websearch MCP at :8000 (NOOB_LIVE=1)"]
fn live_websearch_through_mcp() {
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(
        config.path().join(".env"),
        format!(
            "NOOB_BASE_URL={}\nNOOB_API_KEY=noauth\nNOOB_MODEL=qwen3.6-35b-a3b\n",
            live_base_url()
        ),
    )
    .unwrap();
    std::fs::write(
        config.path().join("mcp.json"),
        format!(
            r#"{{"servers": {{"websearch": {{"url": "{}", "timeout_s": 60}}}}}}"#,
            websearch_url()
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
            "Use the websearch MCP server to search the web for \"Rust programming \
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
    // The model went through the whole MCP chain...
    assert!(
        events
            .iter()
            .any(|e| e["t"] == "tool" && e["name"] == "mcp_connect"),
        "no mcp_connect call in: {stdout}"
    );
    let calls: Vec<&Value> = events
        .iter()
        .filter(|e| e["t"] == "tool" && e["name"] == "mcp_call")
        .collect();
    assert!(!calls.is_empty(), "no mcp_call in: {stdout}");
    assert!(
        calls.iter().any(|c| c["args"]["server"] == "websearch"),
        "mcp_call did not target websearch: {stdout}"
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
