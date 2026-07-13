//! Live P2 smoke (opt-in: NOOB_LIVE=1, `./dev.sh smoke`): the compiled
//! binary driving the whole agent loop against the real qwen endpoint.
//! This is the P2 slice of the all-terrain gauntlet: a real edit
//! round-trip, and prefix-cache reuse proven from the endpoint's own
//! cached-token counters across a resumed session.

use std::process::Command;

use serde_json::Value;

fn live_base_url() -> String {
    std::env::var("NOOB_LIVE_BASE_URL").unwrap_or_else(|_| "http://localhost:8090/v1".to_string())
}

/// Minimal std-only HTTP POST for the tokenizer check: the noob crate must
/// not grow an HTTP dependency (only noob-provider may own one), and the
/// dev image carries no curl.
fn post_json(root: &str, path: &str, body: &Value) -> Value {
    use std::io::{Read, Write};
    let host_port = root
        .strip_prefix("http://")
        .expect("live base url is plain http");
    let mut stream = std::net::TcpStream::connect(host_port).expect("connect llama-server");
    let payload = body.to_string();
    let req = format!(
        "POST {path} HTTP/1.1\r\nhost: {host_port}\r\ncontent-type: application/json\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream.write_all(req.as_bytes()).unwrap();
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).unwrap();
    let text = String::from_utf8_lossy(&resp);
    // Body = everything after the header break; tolerate chunked framing by
    // slicing from the first { to the last }.
    let after = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or(&text);
    let start = after.find('{').expect("JSON body");
    let end = after.rfind('}').expect("JSON body end");
    serde_json::from_str(&after[start..=end]).expect("tokenize response parses")
}

fn rig() -> (tempfile::TempDir, tempfile::TempDir) {
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
    (config, work)
}

fn noob(config: &std::path::Path, work: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_noob"))
        .env("NOOB_CONFIG_DIR", config)
        .current_dir(work)
        .env_remove("NOOB_BASE_URL")
        .args(args)
        .output()
        .unwrap()
}

/// Live smoke: qwen reads a file, edits it with the edit tool, and the
/// change lands on disk. The whole loop, through the shipped binary.
#[test]
#[ignore = "live: needs qwen at :8090 (NOOB_LIVE=1)"]
fn live_edit_round_trip() {
    let (config, work) = rig();
    std::fs::write(
        work.path().join("greeting.py"),
        "def greet():\n    return \"hello world\"\n",
    )
    .unwrap();

    let out = noob(
        config.path(),
        work.path(),
        &[
            "exec",
            "-p",
            "In greeting.py, change the returned string \"hello world\" to \
             \"hello noob\" using the edit tool. Read the file first.",
        ],
    );
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let content = std::fs::read_to_string(work.path().join("greeting.py")).unwrap();
    assert!(
        content.contains("hello noob"),
        "the edit did not land: {content}"
    );
    assert!(!content.contains("hello world"));
}

/// Live smoke: three turns in one resumed session; by turn 3 the endpoint
/// reports a high cached-prompt share, proving the append-only prefix
/// discipline reaches llama.cpp's KV cache end to end.
#[test]
#[ignore = "live: needs qwen at :8090 (NOOB_LIVE=1)"]
fn live_session_cache_reuse() {
    let (config, work) = rig();
    std::fs::write(work.path().join("notes.txt"), "the magic word is plover\n").unwrap();

    let session = "live-cache";
    let prompts = [
        "Read notes.txt and remember the magic word.",
        "What file did you just read? Answer in one line.",
        "What was the magic word? Answer in one line.",
    ];
    let mut last_done: Option<Value> = None;
    for p in prompts {
        let out = noob(
            config.path(),
            work.path(),
            &["exec", "--json", "--session", session, "-p", p],
        );
        assert!(
            out.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        let events: Vec<Value> = stdout
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .collect();
        last_done = events.iter().find(|e| e["t"] == "done").cloned();
        // Session recall sanity on the last turn; the answer arrives split
        // across text deltas, so reassemble before checking.
        if p.contains("magic word") {
            let text: String = events
                .iter()
                .filter(|e| e["t"] == "text")
                .filter_map(|e| e["d"].as_str())
                .collect();
            assert!(
                text.to_lowercase().contains("plover"),
                "no recall across the resumed session: {text}"
            );
        }
    }
    let done = last_done.expect("done event with usage");
    let prompt_tokens = done["usage"]["prompt"].as_u64().unwrap();
    let cached = done["usage"]["cached_prompt"].as_u64().unwrap();
    assert!(
        cached * 10 >= prompt_tokens * 7,
        "turn 3 cached share too low: {cached} of {prompt_tokens} prompt tokens \
         (prefix discipline broken?)"
    );
}

/// Live budget check: the assembled head and tools measured with the REAL
/// qwen tokenizer via llama-server /tokenize, against the same ceilings the
/// offline tiktoken test enforces.
#[test]
#[ignore = "live: needs qwen at :8090 (NOOB_LIVE=1)"]
fn live_tokenizer_budget() {
    let (config, work) = rig();
    let out = noob(config.path(), work.path(), &["debug", "prompt", "--json"]);
    assert!(out.status.success());
    let artifact: Value = serde_json::from_slice(&out.stdout).unwrap();

    let base = live_base_url();
    let root = base.trim_end_matches("/v1");
    let count = |text: &str| -> usize {
        let v = post_json(root, "/tokenize", &serde_json::json!({"content": text}));
        v["tokens"]
            .as_array()
            .expect("llama-server /tokenize")
            .len()
    };
    let head_tokens = count(artifact["head"].as_str().unwrap());
    let tools_tokens = count(&artifact["tools"].to_string());
    assert!(
        head_tokens <= 560,
        "head {head_tokens} tokens on the qwen tokenizer"
    );
    assert!(
        tools_tokens <= 940,
        "tools {tools_tokens} tokens on the qwen tokenizer"
    );
    assert!(head_tokens + tools_tokens <= 1500);
}

/// User-style requirements check: the real model creates and advances its own
/// visible plan, performs a file change, then a fresh resumed process can use
/// the context tool and accurately explain what the preceding turn did.
#[test]
#[ignore = "live: needs qwen at :8090 (NOOB_LIVE=1)"]
fn live_plan_context_and_followup_awareness() {
    let (config, work) = rig();
    let session = "live-requirements-awareness";
    let first = noob(
        config.path(),
        work.path(),
        &[
            "exec",
            "--json",
            "--session",
            session,
            "-p",
            "Use the plan tool before any file tool. Plan, create requirement-proof.txt \
             containing exactly AWARE-OF-MY-PLAN, verify it, and mark every plan item complete.",
        ],
    );
    assert!(
        first.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let first_events: Vec<Value> = String::from_utf8_lossy(&first.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    assert!(
        first_events
            .iter()
            .any(|event| event["t"] == "tool" && event["name"] == "plan"),
        "the model never created a visible plan: {}",
        String::from_utf8_lossy(&first.stdout)
    );
    assert_eq!(
        std::fs::read_to_string(work.path().join("requirement-proof.txt"))
            .unwrap()
            .trim(),
        "AWARE-OF-MY-PLAN"
    );

    let followup = noob(
        config.path(),
        work.path(),
        &[
            "exec",
            "--json",
            "--session",
            session,
            "-p",
            "Use the context tool once. Then explain what file you created in the previous turn \
             and whether that plan finished.",
        ],
    );
    assert!(
        followup.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&followup.stdout),
        String::from_utf8_lossy(&followup.stderr)
    );
    let followup_events: Vec<Value> = String::from_utf8_lossy(&followup.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    assert!(
        followup_events
            .iter()
            .any(|event| event["t"] == "tool" && event["name"] == "context"),
        "the model did not inspect its context budget: {}",
        String::from_utf8_lossy(&followup.stdout)
    );
    let answer: String = followup_events
        .iter()
        .filter(|event| event["t"] == "text")
        .filter_map(|event| event["d"].as_str())
        .collect();
    assert!(
        answer.contains("requirement-proof.txt"),
        "lost prior-turn awareness: {answer}"
    );
    assert!(
        answer.to_ascii_lowercase().contains("complet"),
        "did not understand the plan finished: {answer}"
    );
}
