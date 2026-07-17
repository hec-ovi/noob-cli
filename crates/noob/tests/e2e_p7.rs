//! P7 e2e: `noob doctor` (every problem gets one line with its fix; exit 1
//! when anything FAILs) and the zero-friction exit words in the REPL.

use std::io::Write;
use std::process::{Command, Stdio};

use noob_testkit::MockServer;
use serde_json::json;

fn noob(config_dir: &std::path::Path, workspace: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noob"));
    cmd.env("NOOB_CONFIG_DIR", config_dir)
        .current_dir(workspace)
        .env_remove("NOOB_BASE_URL")
        .env_remove("NOOB_MODEL")
        .env_remove("NOOB_API_STYLE")
        .env_remove("NOOB_CTX")
        .env_remove("NOOB_TASK_CONCURRENCY")
        .env_remove("NOOB_SANDBOX")
        .env_remove("NOOB_DEPTH");
    cmd
}

fn llama_props(slots: u64, padding: usize) -> serde_json::Value {
    json!({
        "default_generation_settings": {"n_ctx": 131_072},
        "total_slots": slots,
        "chat_template": "x".repeat(padding),
        "chat_template_caps": {
            "supports_tools": true,
            "supports_tool_calls": true,
            "supports_parallel_tool_calls": true
        },
        "build_info": "b10015-test"
    })
}

#[test]
fn doctor_healthy_setup_exits_zero() {
    let server = MockServer::start();
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(
        config.path().join(".env"),
        format!(
            "NOOB_BASE_URL={}\nNOOB_MODEL=mockmodel\n",
            server.base_url()
        ),
    )
    .unwrap();
    std::fs::write(
        config.path().join("mcp.json"),
        r#"{"servers": {"websearch": {"url": "http://localhost:8000"}}}"#,
    )
    .unwrap();
    // The reachability GET on {base}/models.
    server.enqueue_json(
        200,
        json!({"object": "list", "data": [{"id": "mockmodel"}]}),
    );
    // Real llama.cpp properties include the chat template and exceed the old
    // 4 KiB doctor body limit. Keep this fixture large enough to guard that
    // regression while remaining tiny by test-suite standards.
    server.enqueue_json(200, llama_props(5, 16 * 1024));

    let out = noob(config.path(), work.path())
        .arg("doctor")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    for needle in [
        "ok    config dir",
        ".env parsed (2 keys)",
        "answers /models (HTTP 200)",
        "style chat",
        "llama.cpp slots: 5 available; enough for the parent + 4 detached sub-agents",
        "mcp.json: 1 server(s) configured (websearch)",
        "ok    workspace",
        "ok    sandbox:",
    ] {
        assert!(stdout.contains(needle), "missing {needle:?} in:\n{stdout}");
    }
    assert!(!stdout.contains("FAIL"), "{stdout}");
}

#[test]
fn doctor_warns_when_llamacpp_slots_cannot_cover_configured_fanout() {
    let server = MockServer::start();
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(
        config.path().join(".env"),
        format!(
            "NOOB_BASE_URL={}\nNOOB_MODEL=org/model:Q4_K\nNOOB_TASK_CONCURRENCY=4\n",
            server.base_url()
        ),
    )
    .unwrap();
    server.enqueue_json(200, json!({"object": "list", "data": []}));
    server.enqueue_json(200, llama_props(2, 16 * 1024));

    let out = noob(config.path(), work.path())
        .arg("doctor")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(
        stdout.contains(
            "warn  llama.cpp has 2 slot(s), but the parent + 4 configured detached sub-agents need 5"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains("restart llama-server with --parallel 5"),
        "{stdout}"
    );
    assert!(stdout.contains("set NOOB_TASK_CONCURRENCY=1"), "{stdout}");

    let paths: Vec<String> = server.recorded().into_iter().map(|r| r.path).collect();
    assert_eq!(
        paths,
        [
            "/v1/models".to_string(),
            "/props?model=org%2Fmodel%3AQ4_K&autoload=false".to_string()
        ]
    );
}

#[test]
fn doctor_warns_about_context_and_tool_template_limits() {
    let server = MockServer::start();
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(
        config.path().join(".env"),
        format!(
            "NOOB_BASE_URL={}\nNOOB_MODEL=mockmodel\nNOOB_CTX=131072\nNOOB_TASK_CONCURRENCY=3\n",
            server.base_url()
        ),
    )
    .unwrap();
    server.enqueue_json(200, json!({"object": "list", "data": []}));
    let mut props = llama_props(4, 0);
    props["default_generation_settings"]["n_ctx"] = json!(32_768);
    props["chat_template_caps"]["supports_parallel_tool_calls"] = json!(false);
    server.enqueue_json(200, props);

    let out = noob(config.path(), work.path())
        .arg("doctor")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(
        stdout.contains("n_ctx=32768, below NOOB_CTX=131072"),
        "{stdout}"
    );
    assert!(
        stdout.contains("parallel tool calls disabled")
            && stdout.contains("supports_parallel_tool_calls"),
        "{stdout}"
    );
}

#[test]
fn doctor_ignores_non_llama_props_with_a_similar_field() {
    let server = MockServer::start();
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(
        config.path().join(".env"),
        format!(
            "NOOB_BASE_URL={}\nNOOB_MODEL=mockmodel\n",
            server.base_url()
        ),
    )
    .unwrap();
    server.enqueue_json(200, json!({"object": "list", "data": []}));
    server.enqueue_json(200, json!({"total_slots": 1, "provider": "not-llama"}));

    let out = noob(config.path(), work.path())
        .arg("doctor")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(!stdout.contains("llama.cpp"), "{stdout}");
    assert!(!stdout.contains("NOOB_TASK_CONCURRENCY"), "{stdout}");
}

#[test]
fn doctor_unreachable_endpoint_fails_with_a_fix() {
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    // Discard port: connection refused immediately.
    std::fs::write(
        config.path().join(".env"),
        "NOOB_BASE_URL=http://127.0.0.1:9/v1\n",
    )
    .unwrap();

    let out = noob(config.path(), work.path())
        .arg("doctor")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "{stdout}");
    assert!(
        stdout.contains("FAIL  endpoint http://127.0.0.1:9/v1 is unreachable"),
        "{stdout}"
    );
    assert!(stdout.contains("fix:"), "{stdout}");
    assert!(stdout.contains("fix the FAIL lines above"), "{stdout}");
}

#[test]
fn doctor_broken_env_and_mcp_json_fail_with_fixes() {
    let server = MockServer::start();
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(
        config.path().join(".env"),
        format!(
            "NOOB_BASE_URL={}\nthis line is not a pair\n",
            server.base_url()
        ),
    )
    .unwrap();
    std::fs::write(config.path().join("mcp.json"), "{ definitely not json").unwrap();

    let out = noob(config.path(), work.path())
        .arg("doctor")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "{stdout}");
    assert!(
        stdout.contains("FAIL") && stdout.contains(".env"),
        "{stdout}"
    );
    assert!(stdout.contains("expected KEY=VALUE"), "{stdout}");
    // The broken .env must NOT be papered over by localhost autodetect.
    assert!(stdout.contains("FAIL  endpoint config:"), "{stdout}");
    assert!(
        stdout.contains("mcp.json") && stdout.contains("not valid JSON"),
        "{stdout}"
    );
}

#[test]
fn doctor_missing_config_dir_fails() {
    let work = tempfile::tempdir().unwrap();
    let ghost = work.path().join("no-such-config-dir");
    let out = noob(&ghost, work.path()).arg("doctor").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "{stdout}");
    assert!(
        stdout.contains("FAIL  config dir") && stdout.contains("does not exist"),
        "{stdout}"
    );
}

// ---------------------------------------------------------------------------
// Compaction hardening (design record: .research/context-compaction-survival)
// ---------------------------------------------------------------------------

fn compaction_rig() -> (MockServer, tempfile::TempDir, tempfile::TempDir) {
    let server = MockServer::start();
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(
        config.path().join(".env"),
        format!(
            "NOOB_BASE_URL={}\nNOOB_MODEL=mockmodel\n",
            server.base_url()
        ),
    )
    .unwrap();
    (server, config, work)
}

fn messages_text(req: &serde_json::Value) -> String {
    req["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap_or("").to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Prune-first: when replacing old fat tool results alone frees enough
/// context, compaction never calls the summarizer at all (no LLM call, no
/// hallucination risk), and the conversational skeleton survives as
/// placeholders.
#[test]
fn compaction_prune_path_skips_the_summarizer() {
    let (server, config, work) = compaction_rig();
    // A ~30 KiB file: its read result is prunable (over the 2 KiB floor).
    let big: String = (0..300)
        .map(|i| format!("line {i:03} {}\n", "x".repeat(90)))
        .collect();
    std::fs::write(work.path().join("big.txt"), &big).unwrap();
    server.enqueue_stream_toolcalls(
        &[("p1", "read", r#"{"path":"big.txt"}"#)],
        Some((2000, 50)), // pushes the estimate over 75% of 4096 next round
    );
    server.enqueue_stream_completion("noted the file");

    let out = noob(config.path(), work.path())
        .env("NOOB_CTX", "4096")
        .args(["exec", "-p", "look at big.txt"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("pruned 1 old tool results"),
        "stderr: {stderr}"
    );

    let reqs: Vec<serde_json::Value> = server
        .recorded()
        .iter()
        .filter(|r| r.path.ends_with("/chat/completions"))
        .map(|r| r.json().unwrap())
        .collect();
    assert_eq!(reqs.len(), 2, "prune must not spend a summarize request");
    let round2 = messages_text(&reqs[1]);
    assert!(
        round2.contains("[an old read result (") && round2.contains("re-run the tool"),
        "placeholder missing:\n{round2}"
    );
    assert!(!round2.contains("line 250"), "the fat body must be gone");
    server.assert_clean();
}

/// Plant `n` files of ~1.9 KiB each (under the 2 KiB prune floor, so their
/// read results are NOT prunable and the summarize path must run).
fn plant_medium_files(work: &std::path::Path, names: &[&str]) {
    for name in names {
        let marker = format!("{name} content padding\n");
        let body = marker.repeat(1900 / marker.len());
        std::fs::write(work.join(name), body).unwrap();
    }
}

/// The summarize path splices a schema'd summary plus the deterministic
/// pinned block (task, files touched), and a second cycle in a NEW process
/// carries the pins forward even though the in-memory file registry is
/// empty after resume.
#[test]
fn compaction_pins_survive_two_cycles_across_resume() {
    let (server, config, work) = compaction_rig();
    plant_medium_files(work.path(), &["f1.txt", "f2.txt", "f3.txt", "f4.txt"]);

    // Cycle 1: three medium read turns; usage on the third forces the
    // trigger, and only the earliest turn falls out of the protected tail.
    server.enqueue_stream_toolcalls(&[("c1", "read", r#"{"path":"f1.txt"}"#)], None);
    server.enqueue_stream_toolcalls(&[("c2", "read", r#"{"path":"f2.txt"}"#)], None);
    server.enqueue_stream_toolcalls(&[("c3", "read", r#"{"path":"f3.txt"}"#)], Some((3400, 50)));
    server.enqueue_stream_completion("## Goal\nread the three files\n## Next steps\ncontinue");
    server.enqueue_stream_completion("ok");
    server.expect_prefix_break(); // the summarize request
    server.expect_prefix_break(); // the continuation after the splice
    let out = noob(config.path(), work.path())
        .env("NOOB_CTX", "4096")
        .args(["exec", "--session", "pins-s1", "-p", "read the three files"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Cycle 2: resume in a FRESH process (empty seen-files registry), one
    // more medium read whose usage re-crosses the trigger.
    server.enqueue_stream_toolcalls(&[("c4", "read", r#"{"path":"f4.txt"}"#)], Some((3400, 50)));
    server.enqueue_stream_completion("## Goal\nread the three files\n## Next steps\nfinish");
    server.enqueue_stream_completion("done");
    server.expect_prefix_break(); // summarize request in the fresh process
    server.expect_prefix_break(); // the continuation after the second splice
    let out = noob(config.path(), work.path())
        .env("NOOB_CTX", "4096")
        .args(["exec", "--session", "pins-s1", "-p", "continue"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let reqs: Vec<serde_json::Value> = server
        .recorded()
        .iter()
        .filter(|r| r.path.ends_with("/chat/completions"))
        .map(|r| r.json().unwrap())
        .collect();
    assert_eq!(
        reqs.len(),
        8,
        "cycle 1: r1-r3 + summarize + r4; cycle 2: r5 + summarize + r6"
    );
    // Cycle 1 splice: summary + pins, assembled by the harness.
    let spliced1 = messages_text(&reqs[4]);
    assert!(spliced1.contains("[conversation summary]"), "{spliced1}");
    assert!(spliced1.contains("## Goal"), "{spliced1}");
    assert!(
        spliced1.contains("[task: read the three files]"),
        "{spliced1}"
    );
    assert!(
        spliced1.contains("[files touched: f1.txt, f2.txt, f3.txt]"),
        "{spliced1}"
    );
    // Cycle 2's summarizer INPUT carries the previous summary and pins
    // (merge, never summary-of-summary alone)...
    let sum2_input = messages_text(&reqs[6]);
    assert!(
        sum2_input.contains("[task: read the three files]"),
        "{sum2_input}"
    );
    assert!(
        sum2_input.contains("[files touched: f1.txt"),
        "{sum2_input}"
    );
    // ...and the second splice re-pins everything, merging this process's
    // own reads (f4) with the carried list, even though this process never
    // touched f1-f3 itself.
    let spliced2 = messages_text(&reqs[7]);
    assert!(
        spliced2.contains("[task: read the three files]"),
        "{spliced2}"
    );
    assert!(
        spliced2.contains("[files touched: f1.txt, f2.txt, f3.txt, f4.txt]"),
        "{spliced2}"
    );
    server.assert_clean();
}

/// A summarizer that returns nothing gets one retry; when that also fails
/// and nothing is prunable, the deterministic hard drop runs, and the
/// pinned block still carries the ground truth into the fresh context.
#[test]
fn failed_summary_hard_drops_with_pins() {
    let (server, config, work) = compaction_rig();
    plant_medium_files(work.path(), &["f1.txt", "f2.txt", "f3.txt"]);
    server.enqueue_stream_toolcalls(&[("h1", "read", r#"{"path":"f1.txt"}"#)], None);
    server.enqueue_stream_toolcalls(&[("h2", "read", r#"{"path":"f2.txt"}"#)], None);
    server.enqueue_stream_toolcalls(&[("h3", "read", r#"{"path":"f3.txt"}"#)], Some((3400, 50)));
    server.enqueue_stream_completion(""); // empty summary
    server.enqueue_stream_completion(""); // empty again on the retry
    server.enqueue_stream_completion("recovered");
    server.expect_prefix_break(); // the first summarize request
    server.expect_prefix_break(); // the continuation after the drop

    let out = noob(config.path(), work.path())
        .env("NOOB_CTX", "4096")
        .args(["exec", "-p", "check the files"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let reqs: Vec<serde_json::Value> = server
        .recorded()
        .iter()
        .filter(|r| r.path.ends_with("/chat/completions"))
        .map(|r| r.json().unwrap())
        .collect();
    assert_eq!(reqs.len(), 6, "r1-r3, summarize, retry, continuation");
    let spliced = messages_text(&reqs[5]);
    assert!(
        spliced.contains("items removed because the summarizer returned nothing]"),
        "{spliced}"
    );
    assert!(spliced.contains("[task: check the files]"), "{spliced}");
    assert!(
        spliced.contains("[files touched: f1.txt, f2.txt, f3.txt]"),
        "{spliced}"
    );
    server.assert_clean();
}

/// Bare `exit` and `quit` leave the REPL like /quit does: nobody should
/// have to learn slash commands to get out. No API request is made.
#[test]
fn repl_bare_exit_words_leave_cleanly() {
    let server = MockServer::start();
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(
        config.path().join(".env"),
        format!(
            "NOOB_BASE_URL={}\nNOOB_MODEL=mockmodel\n",
            server.base_url()
        ),
    )
    .unwrap();
    for word in ["exit", "quit"] {
        let mut child = noob(config.path(), work.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("{word}\n").as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{word}: {:?}", out.status);
    }
    assert!(
        server.recorded().is_empty(),
        "bare exit words must not trigger any model request"
    );
    server.assert_clean();
}
