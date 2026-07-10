//! P6 e2e: multi-agent through the compiled binary. The child protocol is
//! the tested product: one JSON task in on stdin, exactly one JSON result
//! line out on stdout, fresh scoped context, and the caps (turns, wall
//! clock, concurrency, depth) enforced on both sides of the process
//! boundary.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use noob_testkit::{MockServer, RawStep, chat_stream_datas, sse_headers};
use serde_json::Value;

fn write_env(dir: &std::path::Path, base_url: &str) {
    std::fs::write(
        dir.join(".env"),
        format!("NOOB_BASE_URL={base_url}\nNOOB_MODEL=mockmodel\n"),
    )
    .unwrap();
}

fn noob(config_dir: &std::path::Path, workspace: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noob"));
    cmd.env("NOOB_CONFIG_DIR", config_dir)
        .current_dir(workspace)
        .env_remove("NOOB_BASE_URL")
        .env_remove("NOOB_MODEL")
        .env_remove("NOOB_API_STYLE")
        .env_remove("NOOB_CTX")
        .env_remove("NOOB_SANDBOX")
        .env_remove("NOOB_DEPTH")
        .env_remove("NOOB_TASK_CONCURRENCY")
        .env_remove("NOOB_TASK_MAX_TURNS")
        .env_remove("NOOB_TASK_WALL_CLOCK_S");
    cmd
}

struct Rig {
    server: MockServer,
    config: tempfile::TempDir,
    work: tempfile::TempDir,
}

fn rig() -> Rig {
    let server = MockServer::start();
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());
    Rig { server, config, work }
}

impl Rig {
    fn api_requests(&self) -> Vec<Value> {
        self.server
            .recorded()
            .iter()
            .filter(|r| r.path.ends_with("/chat/completions"))
            .map(|r| r.json().unwrap())
            .collect()
    }

    /// Run `noob child` with the given stdin payload; returns the output.
    fn run_child(&self, payload: &str, depth: Option<&str>) -> std::process::Output {
        let mut cmd = noob(self.config.path(), self.work.path());
        cmd.arg("child").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(d) = depth {
            cmd.env("NOOB_DEPTH", d);
        }
        let mut child = cmd.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    /// Enqueue a streamed completion that stalls `sleep_ms` before its body.
    fn enqueue_slow_completion(&self, text: &str, sleep_ms: u64) {
        let mut steps = vec![RawStep::Bytes(sse_headers()), RawStep::SleepMs(sleep_ms)];
        for d in chat_stream_datas(text) {
            steps.push(RawStep::Bytes(format!("data: {d}\n\n").into_bytes()));
        }
        self.server.enqueue_raw(steps);
    }
}

fn tool_names(req: &Value) -> Vec<String> {
    req["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap().to_string())
        .collect()
}

/// The child protocol: one JSON object in, exactly one JSON line out on
/// stdout, progress on stderr, a fresh 2-message context, and the
/// read-only default tool set.
#[test]
fn child_protocol_single_result_line() {
    let rig = rig();
    rig.server.enqueue_stream_completion("the child answer");

    let out = rig.run_child(r#"{"prompt": "inspect the workspace"}"#, None);
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));

    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "stdout must carry exactly one line: {stdout:?}");
    let result: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(result["status"], "ok");
    assert_eq!(result["result"], "the child answer");
    assert_eq!(result["turns"], 1);
    assert!(result["usage"]["prompt"].is_u64(), "usage missing: {result}");
    // The streamed text went to stderr as progress, never to stdout.
    assert!(String::from_utf8_lossy(&out.stderr).contains("the child answer"));

    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1);
    let msgs = reqs[0]["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2, "fresh context: system + prompt only");
    assert_eq!(msgs[1]["content"], "inspect the workspace");
    // Read-only default: exploration set only (no skills in this workspace).
    assert_eq!(tool_names(&reqs[0]), ["read", "grep", "glob", "ls"]);
    rig.server.assert_clean();
}

/// tools:"all" gives the child the full set minus nothing (depth 1 still
/// spawns); depth 2 loses the task tool structurally.
#[test]
fn child_tool_sets_by_mode_and_depth() {
    let rig = rig();
    rig.server.enqueue_stream_completion("one");
    let out = rig.run_child(r#"{"prompt": "p", "tools": "all"}"#, Some("1"));
    assert!(out.status.success());
    rig.server.expect_prefix_break(); // second child = a fresh transcript
    rig.server.expect_tools_change(); // ...with a different tool set
    rig.server.enqueue_stream_completion("two");
    let out = rig.run_child(r#"{"prompt": "p", "tools": "all"}"#, Some("2"));
    assert!(out.status.success());

    let reqs = rig.api_requests();
    let depth1 = tool_names(&reqs[0]);
    let depth2 = tool_names(&reqs[1]);
    assert!(depth1.contains(&"task".to_string()), "depth 1 may spawn: {depth1:?}");
    assert!(depth1.contains(&"bash".to_string()));
    assert!(!depth2.contains(&"task".to_string()), "depth 2 must not spawn: {depth2:?}");
    assert!(depth2.contains(&"bash".to_string()));
    rig.server.assert_clean();
}

/// A malformed payload never hangs or crashes: one error line, exit 1.
#[test]
fn child_rejects_bad_payloads_with_one_line() {
    let rig = rig();
    for payload in ["not json", "{}", r#"{"prompt": "  "}"#, r#"{"prompt":"x","tools":"root"}"#] {
        let out = rig.run_child(payload, None);
        assert!(!out.status.success(), "payload {payload:?} must fail");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let line = stdout.lines().find(|l| !l.trim().is_empty()).unwrap();
        let result: Value = serde_json::from_str(line).unwrap();
        assert_eq!(result["status"], "error", "payload {payload:?}");
    }
    rig.server.assert_clean();
}

/// The child enforces its turn cap and reports the miss as a structured
/// error (exit 1), never as a hang.
#[test]
fn child_errors_at_the_turn_cap() {
    let rig = rig();
    std::fs::write(rig.work.path().join("f"), "content\n").unwrap();
    rig.server
        .enqueue_stream_toolcalls(&[("t1", "read", r#"{"path":"f"}"#)], None);
    rig.server
        .enqueue_stream_toolcalls(&[("t2", "grep", r#"{"pattern":"content","path":"f"}"#)], None);

    let out = rig.run_child(r#"{"prompt": "keep reading", "max_turns": 2}"#, None);
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let result: Value =
        serde_json::from_str(stdout.lines().find(|l| !l.trim().is_empty()).unwrap()).unwrap();
    assert_eq!(result["status"], "error");
    assert!(
        result["result"].as_str().unwrap().contains("2-round cap"),
        "{result}"
    );
    assert_eq!(result["turns"], 2);
    rig.server.assert_clean();
}

/// child_fanout: three task calls in one batch come back as three results
/// in emission order; each child ran a fresh context. Concurrency 1 makes
/// the response-to-child mapping deterministic.
#[test]
fn child_fanout() {
    let rig = rig();
    rig.server.allow_interleaving();
    rig.server.enqueue_stream_toolcalls(
        &[
            ("f1", "task", r#"{"prompt":"helper alpha"}"#),
            ("f2", "task", r#"{"prompt":"helper beta"}"#),
            ("f3", "task", r#"{"prompt":"helper gamma"}"#),
        ],
        None,
    );
    rig.server.enqueue_stream_completion("alpha done");
    rig.server.enqueue_stream_completion("beta done");
    rig.server.enqueue_stream_completion("gamma done");
    rig.server.enqueue_stream_completion("collected all three");

    let out = noob(rig.config.path(), rig.work.path())
        .env("NOOB_TASK_CONCURRENCY", "1")
        .args(["exec", "-p", "spawn three helpers"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 5, "parent x2 + three children");
    // The parent's second request carries the three results in emission
    // order; only the result string entered the transcript.
    let parent2 = reqs.last().unwrap()["messages"].as_array().unwrap().clone();
    let results: Vec<&str> = parent2
        .iter()
        .filter(|m| m["role"] == "tool")
        .map(|m| m["content"].as_str().unwrap())
        .collect();
    assert_eq!(results, ["alpha done", "beta done", "gamma done"]);
    // Children saw the task prompt and nothing of the parent's history.
    let child_reqs: Vec<&Value> = reqs
        .iter()
        .filter(|r| {
            r["messages"][1]["content"]
                .as_str()
                .is_some_and(|c| c.starts_with("helper "))
        })
        .collect();
    assert_eq!(child_reqs.len(), 3);
    for r in child_reqs {
        assert_eq!(r["messages"].as_array().unwrap().len(), 2);
        assert!(!r["messages"][1]["content"].as_str().unwrap().contains("spawn three"));
    }
    rig.server.assert_clean();
}

/// Cap exhaustion: with concurrency 2, the first two children run together
/// (their requests arrive before either response lands) and the third
/// queues until a slot frees. All three still complete.
#[test]
fn fanout_respects_the_concurrency_cap() {
    let rig = rig();
    rig.server.allow_interleaving();
    rig.server.enqueue_stream_toolcalls(
        &[
            ("c1", "task", r#"{"prompt":"helper one"}"#),
            ("c2", "task", r#"{"prompt":"helper two"}"#),
            ("c3", "task", r#"{"prompt":"helper three"}"#),
        ],
        None,
    );
    // Each child's model response takes ~400 ms: the third child cannot
    // even START (and thus send its request) until a slot frees.
    for text in ["one done", "two done", "three done"] {
        rig.enqueue_slow_completion(text, 400);
    }
    rig.server.enqueue_stream_completion("all collected");

    let out = noob(rig.config.path(), rig.work.path())
        .env("NOOB_TASK_CONCURRENCY", "2")
        .args(["exec", "-p", "fan out"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));

    // Arrival times of the three child requests, in order.
    let mut child_arrivals: Vec<Instant> = rig
        .server
        .recorded()
        .iter()
        .filter(|r| {
            r.json()
                .and_then(|j| j["messages"][1]["content"].as_str().map(|c| c.starts_with("helper ")))
                .unwrap_or(false)
        })
        .map(|r| r.arrived)
        .collect();
    child_arrivals.sort();
    assert_eq!(child_arrivals.len(), 3);
    let first_two_gap = child_arrivals[1].duration_since(child_arrivals[0]);
    let third_gap = child_arrivals[2].duration_since(child_arrivals[0]);
    assert!(
        first_two_gap < Duration::from_millis(350),
        "the first two children must overlap; gap was {first_two_gap:?}"
    );
    assert!(
        third_gap >= Duration::from_millis(350),
        "the third child must wait for a slot; gap was {third_gap:?}"
    );
    // Queued is queued, not dropped: all three results returned.
    let parent2 = rig.api_requests().last().unwrap()["messages"].clone();
    let results: Vec<String> = parent2
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "tool")
        .map(|m| m["content"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|r| r.ends_with("done")), "{results:?}");
    rig.server.assert_clean();
}

/// The wall clock: a wedged child is killed (whole process group) and the
/// parent gets a teaching error instead of a hang; the loop continues.
#[test]
fn task_wall_clock_kills_a_wedged_child() {
    let rig = rig();
    rig.server.allow_interleaving();
    rig.server
        .enqueue_stream_toolcalls(&[("w1", "task", r#"{"prompt":"never finishes"}"#)], None);
    // The child's model response never arrives within its watchdog window;
    // the PARENT's 1s wall clock fires first and kills the child.
    rig.server.enqueue_raw(vec![
        RawStep::Bytes(sse_headers()),
        RawStep::SleepMs(20_000),
    ]);
    rig.server.enqueue_stream_completion("moving on");

    let started = Instant::now();
    let out = noob(rig.config.path(), rig.work.path())
        .env("NOOB_TASK_WALL_CLOCK_S", "1")
        .args(["exec", "-p", "spawn a doomed helper"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "the wall clock did not fire; took {:?}",
        started.elapsed()
    );

    let reqs = rig.api_requests();
    let result = reqs
        .last()
        .unwrap()["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(result.contains("exceeded the 1s wall clock"), "{result}");
    rig.server.assert_clean();
}

/// Depth cap through the parent surface: at NOOB_DEPTH=2 the parent itself
/// registers no task tool, so the schema is structurally absent.
#[test]
fn depth_cap_removes_the_task_schema() {
    let rig = rig();
    rig.server.enqueue_stream_completion("shallow");
    let out = noob(rig.config.path(), rig.work.path())
        .env("NOOB_DEPTH", "2")
        .args(["exec", "-p", "hello"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let names = tool_names(&rig.api_requests()[0]);
    assert!(!names.contains(&"task".to_string()), "{names:?}");
    assert_eq!(names.len(), 7, "the 7 core tools only");
    rig.server.assert_clean();
}
