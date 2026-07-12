//! P2 e2e: the compiled binary against the mock server, through the full
//! agent loop (tool execution, scheduling, breakers, sessions, compaction).
//! Named tests locked by ARCHITECTURE.md land here: cache_prefix,
//! parallel_calls, mutate_barrier, doom_loop, compaction, exec_json,
//! session_resume.

use std::process::Command;

use noob_testkit::MockServer;
use serde_json::Value;

fn write_env(dir: &std::path::Path, base_url: &str) {
    std::fs::write(
        dir.join(".env"),
        format!("NOOB_BASE_URL={base_url}\nNOOB_MODEL=mockmodel\n"),
    )
    .unwrap();
}

/// Binary command with isolated config + workspace dirs.
fn noob(config_dir: &std::path::Path, workspace: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noob"));
    cmd.env("NOOB_CONFIG_DIR", config_dir)
        .current_dir(workspace)
        .env_remove("NOOB_BASE_URL")
        .env_remove("NOOB_MODEL")
        .env_remove("NOOB_API_STYLE")
        .env_remove("NOOB_CTX")
        .env_remove("NOOB_SANDBOX");
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
    fn run(&self, args: &[&str]) -> std::process::Output {
        noob(self.config.path(), self.work.path())
            .args(args)
            .output()
            .unwrap()
    }

    fn api_requests(&self) -> Vec<Value> {
        self.server
            .recorded()
            .iter()
            .filter(|r| r.path.ends_with("/chat/completions"))
            .map(|r| r.json().unwrap())
            .collect()
    }
}

fn ok(out: &std::process::Output) -> String {
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Three requests in one exec run; every request is an exact prefix
/// extension of the previous one, checked at the JSON item level here on
/// top of the mock's automatic assertion.
#[test]
fn cache_prefix() {
    let rig = rig();
    std::fs::write(rig.work.path().join("a.txt"), "alpha\n").unwrap();
    std::fs::write(rig.work.path().join("b.txt"), "beta\n").unwrap();
    rig.server
        .enqueue_stream_toolcalls(&[("c1", "read", r#"{"path":"a.txt"}"#)], None);
    rig.server
        .enqueue_stream_toolcalls(&[("c2", "read", r#"{"path":"b.txt"}"#)], None);
    rig.server.enqueue_stream_completion("both read");

    let stdout = ok(&rig.run(&["exec", "-p", "read both files"]));
    assert!(stdout.contains("both read"));

    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 3);
    for w in reqs.windows(2) {
        let prev = w[0]["messages"].as_array().unwrap();
        let next = w[1]["messages"].as_array().unwrap();
        assert!(next.len() > prev.len());
        assert_eq!(&next[..prev.len()], &prev[..], "byte-level prefix broke");
    }
    // The tool results landed, in order, with the file contents.
    let last = reqs[2]["messages"].as_array().unwrap();
    let tools: Vec<&Value> = last.iter().filter(|m| m["role"] == "tool").collect();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0]["tool_call_id"], "c1");
    assert!(tools[0]["content"].as_str().unwrap().contains("alpha"));
    assert_eq!(tools[1]["tool_call_id"], "c2");
    rig.server.assert_clean();
}

/// One assistant turn with three read calls: all three results come back
/// in emission order, one tool message per call id.
#[test]
fn parallel_calls() {
    let rig = rig();
    for (name, content) in [("x.txt", "XX"), ("y.txt", "YY"), ("z.txt", "ZZ")] {
        std::fs::write(rig.work.path().join(name), content).unwrap();
    }
    rig.server.enqueue_stream_toolcalls(
        &[
            ("p1", "read", r#"{"path":"x.txt"}"#),
            ("p2", "read", r#"{"path":"y.txt"}"#),
            ("p3", "read", r#"{"path":"z.txt"}"#),
        ],
        None,
    );
    rig.server.enqueue_stream_completion("done");

    ok(&rig.run(&["exec", "-p", "read all three"]));

    let reqs = rig.api_requests();
    let msgs = reqs[1]["messages"].as_array().unwrap();
    let tools: Vec<&Value> = msgs.iter().filter(|m| m["role"] == "tool").collect();
    let ids: Vec<&str> = tools.iter().map(|t| t["tool_call_id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["p1", "p2", "p3"], "results must keep emission order");
    for (t, marker) in tools.iter().zip(["XX", "YY", "ZZ"]) {
        assert!(t["content"].as_str().unwrap().contains(marker));
    }
    rig.server.assert_clean();
}

/// Two bash calls in one batch are strict sequential barriers: the first
/// (with a deliberate delay) fully lands before the second starts.
#[test]
fn mutate_barrier() {
    let rig = rig();
    rig.server.enqueue_stream_toolcalls(
        &[
            ("m1", "bash", r#"{"cmd":"sleep 0.1; echo A >> log.txt"}"#),
            ("m2", "bash", r#"{"cmd":"echo B >> log.txt"}"#),
        ],
        None,
    );
    rig.server.enqueue_stream_completion("done");

    ok(&rig.run(&["exec", "-p", "run both"]));

    let log = std::fs::read_to_string(rig.work.path().join("log.txt")).unwrap();
    assert_eq!(log, "A\nB\n", "mutating calls must serialize in emission order");
    rig.server.assert_clean();
}

/// The same (tool, args) call three times: the third is intercepted with
/// the doom-loop message instead of executing.
#[test]
fn doom_loop() {
    let rig = rig();
    std::fs::write(rig.work.path().join("f.txt"), "content\n").unwrap();
    for _ in 0..3 {
        rig.server
            .enqueue_stream_toolcalls(&[("d", "read", r#"{"path":"f.txt"}"#)], None);
    }
    rig.server.enqueue_stream_completion("gave up");

    ok(&rig.run(&["exec", "-p", "loop forever"]));

    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 4);
    let msgs = reqs[3]["messages"].as_array().unwrap();
    let tools: Vec<&str> = msgs
        .iter()
        .filter(|m| m["role"] == "tool")
        .map(|m| m["content"].as_str().unwrap())
        .collect();
    assert_eq!(tools.len(), 3);
    assert!(tools[0].contains("content"), "first executes");
    assert!(tools[1].contains("content"), "second executes");
    assert!(
        tools[2].contains("repeated identical call"),
        "third intercepted: {}",
        tools[2]
    );
    rig.server.assert_clean();
}

/// exec --json: one JSONL event per loop step, machine-readable end to end.
#[test]
fn exec_json() {
    let rig = rig();
    std::fs::write(rig.work.path().join("f.txt"), "data\n").unwrap();
    rig.server
        .enqueue_stream_toolcalls(&[("j1", "read", r#"{"path":"f.txt"}"#)], None);
    rig.server.enqueue_stream_completion("all done here");

    let stdout = ok(&rig.run(&["exec", "--json", "-p", "read it"]));

    let events: Vec<Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad JSONL {l:?}: {e}")))
        .collect();
    let kinds: Vec<&str> = events.iter().map(|e| e["t"].as_str().unwrap()).collect();
    assert_eq!(kinds.iter().filter(|k| **k == "tool").count(), 1);
    assert_eq!(kinds.iter().filter(|k| **k == "result").count(), 1);
    assert!(kinds.contains(&"text"));
    assert_eq!(*kinds.last().unwrap(), "done");

    let tool = events.iter().find(|e| e["t"] == "tool").unwrap();
    assert_eq!(tool["name"], "read");
    assert_eq!(tool["args"]["path"], "f.txt");
    let result = events.iter().find(|e| e["t"] == "result").unwrap();
    assert_eq!(result["id"], "j1");
    assert_eq!(result["err"], false);
    let done = events.iter().find(|e| e["t"] == "done").unwrap();
    assert!(done["usage"]["prompt"].as_u64().is_some());
    // The streamed text reassembles to the final answer.
    let text: String = events
        .iter()
        .filter(|e| e["t"] == "text")
        .map(|e| e["d"].as_str().unwrap())
        .collect();
    assert_eq!(text, "all done here");
    rig.server.assert_clean();
}

/// Two processes, one session id: the second run replays the first run's
/// transcript byte-identically and extends it.
#[test]
fn session_resume() {
    let rig = rig();
    rig.server.enqueue_stream_completion("first answer");
    rig.server.enqueue_stream_completion("second answer");

    ok(&rig.run(&["exec", "--session", "s-resume", "-p", "first question"]));
    let out2 = ok(&rig.run(&["exec", "--session", "s-resume", "-p", "second question"]));
    assert!(out2.contains("second answer"));

    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2);
    let first = reqs[0]["messages"].as_array().unwrap();
    let second = reqs[1]["messages"].as_array().unwrap();
    // Same day + same config = identical system head; the resumed request
    // is an exact prefix extension ACROSS processes.
    assert_eq!(&second[..first.len()], &first[..]);
    let texts: Vec<&str> = second
        .iter()
        .map(|m| m["content"].as_str().unwrap_or(""))
        .collect();
    assert!(texts.contains(&"first question"));
    assert!(texts.contains(&"first answer"));
    assert!(texts.contains(&"second question"));

    let session_file = rig.config.path().join("sessions/s-resume.jsonl");
    assert!(session_file.is_file());
    rig.server.assert_clean();
}

/// exec never redisplays a resumed transcript (that is a REPL-only affordance):
/// a second `--session` run prints only the new answer, not the prior turns nor
/// the replay marker, keeping the exec surface byte-identical on resume.
#[test]
fn exec_resume_does_not_replay_prior_turns() {
    let rig = rig();
    rig.server.enqueue_stream_completion("first answer here");
    rig.server.enqueue_stream_completion("second answer here");

    ok(&rig.run(&["exec", "--session", "s-noreplay", "-p", "first question"]));
    let out2 = ok(&rig.run(&["exec", "--session", "s-noreplay", "-p", "second question"]));

    assert!(out2.contains("second answer here"));
    // The prior turns are loaded into context but never echoed to stdout.
    assert!(!out2.contains("first question"), "exec replayed a prior user turn to stdout");
    assert!(!out2.contains("first answer here"), "exec replayed a prior assistant turn to stdout");
    assert!(!out2.contains('\u{203a}'), "the replay user marker leaked into exec stdout");
    rig.server.assert_clean();
}

/// Forced compaction: a small NOOB_CTX plus inflated reported usage makes
/// the loop summarize the middle before the next request. Asserts the
/// summarize request shape, the spliced summary, and the session reset.
#[test]
fn compaction() {
    let rig = rig();
    // ~8 KiB tool result so the transcript has real bulk.
    let big: String = (0..400).map(|i| format!("filler line {i}\n")).collect();
    std::fs::write(rig.work.path().join("big.txt"), &big).unwrap();
    // Round 1 reports usage near the 4096 ceiling -> round 2 compacts first.
    rig.server.enqueue_stream_toolcalls(
        &[("c1", "read", r#"{"path":"big.txt"}"#)],
        Some((3500, 100)),
    );
    // The summarize request's canned answer.
    rig.server.enqueue_stream_completion("SUMMARY-OF-EVERYTHING");
    // The post-compaction continuation.
    rig.server.enqueue_stream_completion("continuing after compaction");
    // Two sanctioned breaks: the summarize request, then the rebuilt prefix.
    rig.server.expect_prefix_break();
    rig.server.expect_prefix_break();

    let out = noob(rig.config.path(), rig.work.path())
        .env("NOOB_CTX", "4096")
        .args(["exec", "--session", "s-compact", "-p", "read the big file"])
        .output()
        .unwrap();
    let stdout = ok(&out);
    assert!(stdout.contains("continuing after compaction"));

    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 3);
    // Request 2 is the summarizer: compact.md system + the middle + the ask.
    let sum_msgs = reqs[1]["messages"].as_array().unwrap();
    assert!(
        sum_msgs[0]["content"]
            .as_str()
            .unwrap()
            .contains("summarize an agent session")
    );
    assert!(reqs[1]["tools"].is_null(), "the summarizer gets no tools");
    let last_sum = sum_msgs.last().unwrap();
    assert!(last_sum["content"].as_str().unwrap().contains("Output only the summary"));
    // Request 3 carries the spliced summary instead of the old middle.
    let cont_msgs = reqs[2]["messages"].as_array().unwrap();
    let joined: String = cont_msgs
        .iter()
        .map(|m| m["content"].as_str().unwrap_or("").to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("[conversation summary]"));
    assert!(joined.contains("SUMMARY-OF-EVERYTHING"));
    assert!(!joined.contains("filler line 250"), "the middle must be gone");

    // The session log recorded the reset; a resume sees the compacted state.
    let log = std::fs::read_to_string(rig.config.path().join("sessions/s-compact.jsonl"))
        .unwrap();
    assert!(log.lines().any(|l| l.contains("\"t\":\"reset\"")));
    rig.server.assert_clean();
}

/// Workspace sandbox mode: a write outside the workspace is refused and
/// the refusal (with its remedy) goes back to the model as the result.
#[test]
fn workspace_mode_refuses_outside_writes() {
    let rig = rig();
    rig.server.enqueue_stream_toolcalls(
        &[("w1", "write", r#"{"path":"/tmp/evil.txt","content":"x"}"#)],
        None,
    );
    rig.server.enqueue_stream_completion("understood");

    let out = noob(rig.config.path(), rig.work.path())
        .env("NOOB_SANDBOX", "workspace")
        .args(["exec", "-p", "write outside"])
        .output()
        .unwrap();
    ok(&out);

    assert!(!std::path::Path::new("/tmp/evil.txt").exists());
    let reqs = rig.api_requests();
    let msgs = reqs[1]["messages"].as_array().unwrap();
    let result = msgs.iter().find(|m| m["role"] == "tool").unwrap();
    assert!(
        result["content"].as_str().unwrap().contains("outside the workspace"),
        "{}",
        result["content"]
    );
    rig.server.assert_clean();
}

/// The REPL end to end through piped stdin: greeting, one answered turn,
/// /status, /quit.
#[test]
fn repl_smoke() {
    use std::io::Write;
    let rig = rig();
    rig.server.enqueue_stream_completion("repl says hi");

    let mut child = noob(rig.config.path(), rig.work.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"say hi\n/status\n/quit\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("repl says hi"), "stdout: {stdout}");
    assert!(stdout.contains("endpoint:"), "/status output missing: {stdout}");
    assert!(stdout.contains("mockmodel"));
    rig.server.assert_clean();
}

/// Ctrl-C while the first call of a batch runs: the remaining calls are
/// canceled with synthetic results and never execute (a mutation must not
/// land after the user canceled).
#[test]
fn sigint_mid_batch_cancels_remaining_calls() {
    let rig = rig();
    rig.server.enqueue_stream_toolcalls(
        &[
            ("s1", "bash", r#"{"cmd":"sleep 10"}"#),
            ("s2", "bash", r#"{"cmd":"echo landed > canary.txt"}"#),
        ],
        None,
    );

    let mut child = noob(rig.config.path(), rig.work.path())
        .args(["exec", "-p", "run both"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    // Let it reach the sleeping bash, then interrupt once.
    std::thread::sleep(std::time::Duration::from_millis(1500));
    unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(6),
            "did not exit promptly after SIGINT"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(130));
    assert!(
        !rig.work.path().join("canary.txt").exists(),
        "the second mutation ran after the user canceled"
    );
    rig.server.assert_clean();
}

/// A prompt that begins with a dash is arbitrary user text, not a flag.
#[test]
fn dash_leading_prompts_are_accepted() {
    let rig = rig();
    rig.server.enqueue_stream_completion("dashes are fine");
    let out = rig.run(&["exec", "-p", "--verbose is broken, help"]);
    let stdout = ok(&out);
    assert!(stdout.contains("dashes are fine"));
    let reqs = rig.api_requests();
    assert_eq!(reqs[0]["messages"][1]["content"], "--verbose is broken, help");
    rig.server.assert_clean();
}

/// Four consecutive tool errors inject the course-correct nudge; eight
/// abort a headless run with a structured error.
#[test]
fn consecutive_error_breakers() {
    let rig = rig();
    // Two batches of 4 failing reads each (8 consecutive errors total).
    for batch in 0..2 {
        let calls: Vec<(String, String, String)> = (0..4)
            .map(|i| {
                (
                    format!("e{batch}{i}"),
                    "read".to_string(),
                    format!(r#"{{"path":"missing-{batch}-{i}.txt"}}"#),
                )
            })
            .collect();
        let refs: Vec<(&str, &str, &str)> = calls
            .iter()
            .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str()))
            .collect();
        rig.server.enqueue_stream_toolcalls(&refs, None);
    }

    let out = rig.run(&["exec", "-p", "read the missing files"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("consecutive tool errors"),
        "stderr: {stderr}"
    );
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2);
    // The nudge landed after the first batch of 4 errors.
    let msgs = reqs[1]["messages"].as_array().unwrap();
    let nudge = msgs
        .iter()
        .filter(|m| m["role"] == "user")
        .any(|m| m["content"].as_str().unwrap_or("").contains("[note]"));
    assert!(nudge, "course-correct nudge missing");
    rig.server.assert_clean();
}
