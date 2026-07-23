//! P6 e2e: multi-agent through the compiled binary. The child protocol is
//! the tested product: one JSON task in on stdin, exactly one JSON result
//! line out on stdout, fresh scoped context, and the caps (turns, wall
//! clock, concurrency, depth) enforced on both sides of the process
//! boundary.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use noob_testkit::mcp::{McpHttpServer, echo_tools};
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
    Rig {
        server,
        config,
        work,
    }
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
        cmd.arg("child")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
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

/// Scripted stall before each child's model-response body in the two
/// concurrency tests below. The stall is the overlap witness: the server
/// cannot complete a child's response earlier than its arrival + STALL, so a
/// sibling request that ARRIVES within one stall of another provably
/// overlapped it, and a request that had to wait for a slot cannot arrive
/// before a full stall has elapsed. The assertions compare recorded arrival
/// gaps against this constant, not a hand-tuned margin; it is sized so even a
/// loaded host spawns a child process well inside one stall.
const CHILD_STALL: Duration = Duration::from_millis(2_000);

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
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        1,
        "stdout must carry exactly one line: {stdout:?}"
    );
    let result: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(result["status"], "ok");
    assert_eq!(result["result"], "the child answer");
    assert_eq!(result["turns"], 1);
    assert!(
        result["usage"]["prompt"].is_u64(),
        "usage missing: {result}"
    );
    // The streamed text went to stderr as progress, never to stdout.
    assert!(String::from_utf8_lossy(&out.stderr).contains("the child answer"));

    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1);
    let msgs = reqs[0]["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2, "fresh context: system + prompt only");
    assert_eq!(msgs[1]["content"], "inspect the workspace");
    // Read-only default: exploration set only (no skills in this workspace).
    assert_eq!(
        tool_names(&reqs[0]),
        ["read", "grep", "glob", "ls", "context"]
    );
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
    assert!(
        depth1.contains(&"subagent".to_string()),
        "depth 1 may spawn: {depth1:?}"
    );
    assert!(depth1.contains(&"bash".to_string()));
    assert!(
        !depth2.contains(&"subagent".to_string()),
        "depth 2 must not spawn: {depth2:?}"
    );
    assert!(depth2.contains(&"bash".to_string()));
    rig.server.assert_clean();
}

/// Defense in depth behind the filtered schemas: a read-only child whose
/// model hallucinates a mutating call gets a teaching refusal, and nothing
/// lands on disk.
#[test]
fn read_only_child_refuses_hallucinated_mutations() {
    let rig = rig();
    rig.server.enqueue_stream_toolcalls(
        &[("m1", "write", r#"{"path":"evil.txt","content":"boom"}"#)],
        None,
    );
    rig.server
        .enqueue_stream_completion("understood, reporting instead");

    let out = rig.run_child(r#"{"prompt": "survey the repo"}"#, None);
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !rig.work.path().join("evil.txt").exists(),
        "a read-only child executed a mutating call"
    );
    let reqs = rig.api_requests();
    let refusal = reqs[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(refusal.contains("this sub-agent is read-only"), "{refusal}");
    assert!(refusal.contains("report what you found"), "{refusal}");
    rig.server.assert_clean();
}

/// The web profile keeps real MCP research available while structurally
/// refusing the stray file write caught in the installed research skill.
#[test]
fn web_child_has_only_local_reads_and_one_web_mcp() {
    let rig = rig();
    let web = McpHttpServer::start(echo_tools());
    std::fs::write(
        rig.config.path().join("mcp.json"),
        format!(
            r#"{{"servers":{{"Web_Search":{{"url":"{}"}}}}}}"#,
            web.url()
        ),
    )
    .unwrap();
    rig.server.enqueue_stream_toolcalls(
        &[(
            "m1",
            "write",
            r#"{"path":"stray-findings.md","content":"must not land"}"#,
        )],
        None,
    );
    rig.server.enqueue_stream_toolcalls(
        &[("connect", "mcp_connect", r#"{"server":"Web_Search"}"#)],
        None,
    );
    rig.server.enqueue_stream_toolcalls(
        &[
            (
                "search",
                "mcp_call",
                r#"{"server":"Web_Search","tool":"echo","args":{"text":"search primary docs"}}"#,
            ),
            (
                "fetch",
                "mcp_call",
                r#"{"server":"Web_Search","tool":"echo","args":{"text":"fetch source"}}"#,
            ),
        ],
        None,
    );
    rig.server
        .enqueue_stream_completion("returning the sourced synthesis instead");

    let out = rig.run_child(
        r#"{"prompt":"research and return findings","tools":"web"}"#,
        None,
    );
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!rig.work.path().join("stray-findings.md").exists());

    let reqs = rig.api_requests();
    assert_eq!(
        tool_names(&reqs[0]),
        [
            "read",
            "grep",
            "glob",
            "ls",
            "context",
            "mcp_connect",
            "mcp_call"
        ]
    );
    assert!(
        reqs[0]["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("MCP servers (use mcp_connect): Web_Search")
    );
    let refusal = reqs[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "tool")
        .unwrap()["content"]
        .as_str()
        .unwrap();
    assert!(refusal.contains("this sub-agent is read-only"), "{refusal}");
    assert_eq!(web.calls().len(), 2);
    web.assert_clean();
    rig.server.assert_clean();
}

/// A memory-only first answer is not accepted. The child gets one internal
/// correction, uses the configured MCP server twice, and reports only the
/// evidence-backed completion while staying inside the original round cap.
#[test]
fn web_child_corrects_an_unsupported_completion_with_real_mcp_evidence() {
    let rig = rig();
    let web = McpHttpServer::start(echo_tools());
    std::fs::write(
        rig.config.path().join("mcp.json"),
        format!(
            r#"{{"servers":{{"Web_Search":{{"url":"{}"}}}}}}"#,
            web.url()
        ),
    )
    .unwrap();

    rig.server
        .enqueue_stream_completion("unsupported answer from memory");
    rig.server.enqueue_stream_toolcalls(
        &[("connect", "mcp_connect", r#"{"server":"Web_Search"}"#)],
        None,
    );
    rig.server.enqueue_stream_toolcalls(
        &[
            (
                "search",
                "mcp_call",
                r#"{"server":"Web_Search","tool":"echo","args":{"text":"search official docs"}}"#,
            ),
            (
                "fetch",
                "mcp_call",
                r#"{"server":"Web_Search","tool":"echo","args":{"text":"fetch primary source"}}"#,
            ),
        ],
        None,
    );
    rig.server
        .enqueue_stream_completion("evidence-backed answer");

    let out = rig.run_child(
        r#"{"prompt":"research current behavior","tools":"web","max_turns":4}"#,
        None,
    );
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["status"], "ok");
    assert_eq!(result["result"], "evidence-backed answer");
    assert_eq!(result["turns"], 4);
    assert_eq!(web.calls().len(), 2);

    let requests = rig.api_requests();
    let corrective = requests[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| {
            message["role"] == "user"
                && message["content"]
                    .as_str()
                    .is_some_and(|text| text.contains("web research evidence gate"))
        })
        .expect("internal evidence correction")["content"]
        .as_str()
        .unwrap();
    assert!(corrective.contains("Web_Search"), "{corrective}");
    assert!(
        corrective.contains("at least 2 successful mcp_call"),
        "{corrective}"
    );
    assert!(
        corrective.contains("Do not answer from memory"),
        "{corrective}"
    );
    web.assert_clean();
    rig.server.assert_clean();
}

/// One ignored corrective follow-up is terminal. A second unsupported
/// completion becomes a structured child error instead of false success.
#[test]
fn web_child_rejects_a_second_completion_without_mcp_evidence() {
    let rig = rig();
    let web = McpHttpServer::start(echo_tools());
    std::fs::write(
        rig.config.path().join("mcp.json"),
        format!(r#"{{"servers":{{"websearch":{{"url":"{}"}}}}}}"#, web.url()),
    )
    .unwrap();
    rig.server.enqueue_stream_toolcalls(
        &[
            (
                "unconnected-search",
                "mcp_call",
                r#"{"server":"websearch","tool":"echo","args":{"text":"search"}}"#,
            ),
            (
                "unconnected-fetch",
                "mcp_call",
                r#"{"server":"websearch","tool":"echo","args":{"text":"fetch"}}"#,
            ),
        ],
        None,
    );
    rig.server.enqueue_stream_completion("first memory answer");
    rig.server.enqueue_stream_completion("second memory answer");

    let out = rig.run_child(
        r#"{"prompt":"research this","tools":"web","max_turns":4}"#,
        None,
    );
    assert!(!out.status.success());
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["status"], "error");
    assert_eq!(result["turns"], 3);
    assert!(
        result["result"]
            .as_str()
            .unwrap()
            .contains("without the required 2 mcp_call evidence calls after one corrective"),
        "{result}"
    );
    assert!(web.calls().is_empty());
    web.assert_clean();
    rig.server.assert_clean();
}

/// The evidence correction cannot silently grant extra inference rounds. If
/// the first unsupported answer consumes the original budget, the child
/// fails immediately without another provider request.
#[test]
fn web_child_does_not_extend_an_exhausted_round_budget() {
    let rig = rig();
    let web = McpHttpServer::start(echo_tools());
    std::fs::write(
        rig.config.path().join("mcp.json"),
        format!(r#"{{"servers":{{"websearch":{{"url":"{}"}}}}}}"#, web.url()),
    )
    .unwrap();
    rig.server
        .enqueue_stream_completion("unsupported one-round answer");

    let out = rig.run_child(
        r#"{"prompt":"research this","tools":"web","max_turns":1}"#,
        None,
    );
    assert!(!out.status.success());
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["status"], "error");
    assert_eq!(result["turns"], 1);
    assert!(
        result["result"]
            .as_str()
            .unwrap()
            .contains("original 1-round budget is exhausted"),
        "{result}"
    );
    assert_eq!(rig.api_requests().len(), 1);
    assert!(web.calls().is_empty());
    web.assert_clean();
    rig.server.assert_clean();
}

/// A malformed payload never hangs or crashes: one error line, exit 1.
#[test]
fn child_rejects_bad_payloads_with_one_line() {
    let rig = rig();
    for payload in [
        "not json",
        "{}",
        r#"{"prompt": "  "}"#,
        r#"{"prompt":"x","tools":"root"}"#,
    ] {
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
    rig.server.enqueue_stream_toolcalls(
        &[("t2", "grep", r#"{"pattern":"content","path":"f"}"#)],
        None,
    );

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

/// Two rounds before the cap the child receives exactly one budget nudge,
/// so a model that follows it delivers a final report instead of running
/// into the cap abort mid-gathering (the live research-child failure).
#[test]
fn child_budget_nudge_lands_once_and_the_report_still_ships() {
    let rig = rig();
    std::fs::write(rig.work.path().join("f"), "content\n").unwrap();
    rig.server
        .enqueue_stream_toolcalls(&[("t1", "read", r#"{"path":"f"}"#)], None);
    rig.server.enqueue_stream_toolcalls(
        &[("t2", "grep", r#"{"pattern":"content","path":"f"}"#)],
        None,
    );
    rig.server
        .enqueue_stream_completion("FINAL-REPORT after the nudge");

    let out = rig.run_child(r#"{"prompt": "inspect then report", "max_turns": 4}"#, None);
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let result: Value =
        serde_json::from_str(stdout.lines().find(|l| !l.trim().is_empty()).unwrap()).unwrap();
    assert_eq!(result["status"], "ok");
    assert!(
        result["result"].as_str().unwrap().contains("FINAL-REPORT"),
        "{result}"
    );

    // Injected exactly once, exactly two rounds before the cap: rounds one
    // and two see no nudge, round three carries one.
    let requests = rig.api_requests();
    assert_eq!(requests.len(), 3);
    let nudges = |req: &Value| {
        req["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| {
                m["role"] == "user"
                    && m["content"]
                        .as_str()
                        .is_some_and(|c| c.starts_with("[budget]"))
            })
            .count()
    };
    assert_eq!(nudges(&requests[0]), 0);
    assert_eq!(nudges(&requests[1]), 0);
    assert_eq!(nudges(&requests[2]), 1);
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
            ("f1", "subagent", r#"{"prompt":"helper alpha"}"#),
            ("f2", "subagent", r#"{"prompt":"helper beta"}"#),
            ("f3", "subagent", r#"{"prompt":"helper gamma"}"#),
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
        assert!(
            !r["messages"][1]["content"]
                .as_str()
                .unwrap()
                .contains("spawn three")
        );
    }
    rig.server.assert_clean();
}

/// Provider and sandbox choices are part of the child protocol. CLI flags
/// outrank `.env` for the root and must keep that precedence after the
/// process boundary instead of silently sending detached work elsewhere.
#[test]
fn child_inherits_root_cli_provider_model_and_yolo() {
    let selected = MockServer::start();
    selected.allow_interleaving();
    let ignored = MockServer::start();
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &ignored.base_url());

    selected.enqueue_stream_toolcalls(
        &[(
            "spawn",
            "subagent",
            r#"{"prompt":"check inherited runtime"}"#,
        )],
        None,
    );
    selected.enqueue_stream_completion("child used root runtime");
    selected.enqueue_stream_completion("parent collected child");

    let out = noob(config.path(), work.path())
        .args([
            "exec",
            "-p",
            "delegate once",
            "--base-url",
            &selected.base_url(),
            "--model",
            "root-selected-model",
            "--yolo",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let requests: Vec<Value> = selected
        .recorded()
        .iter()
        .filter(|request| request.path.ends_with("/chat/completions"))
        .map(|request| request.json().unwrap())
        .collect();
    assert_eq!(requests.len(), 3, "root twice plus one child");
    assert!(
        requests
            .iter()
            .all(|request| request["model"] == "root-selected-model")
    );
    let child = requests
        .iter()
        .find(|request| request["messages"][1]["content"] == "check inherited runtime")
        .expect("child request");
    assert!(
        child["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("sandbox: off (--yolo)")
    );
    assert!(
        ignored.recorded().is_empty(),
        "child contacted the .env server"
    );
    selected.assert_clean();
}

/// A skill that delegates is parent orchestration context. Once loaded, its
/// name is excluded from the child resolver so an all-tools child cannot
/// rediscover and recursively execute the same workflow.
#[test]
fn loaded_orchestration_skill_is_not_rediscovered_by_child() {
    let rig = rig();
    rig.server.allow_interleaving();
    let skill = rig.work.path().join(".noob/skills/research");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: research\ndescription: delegate deep investigations\n---\n\
         Load this and delegate one investigation with run_in_background.\n",
    )
    .unwrap();

    rig.server
        .enqueue_stream_toolcalls(&[("load", "skill", r#"{"name":"research"}"#)], None);
    rig.server.enqueue_stream_toolcalls(
        &[(
            "spawn",
            "subagent",
            r#"{"prompt":"inspect without recursion","tools":"all"}"#,
        )],
        None,
    );
    rig.server
        .enqueue_stream_completion("child did not recurse");
    rig.server.enqueue_stream_completion("research collected");

    let out = noob(rig.config.path(), rig.work.path())
        .args(["exec", "-p", "use research"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let requests = rig.api_requests();
    let child = requests
        .iter()
        .find(|request| request["messages"][1]["content"] == "inspect without recursion")
        .expect("child request");
    let names = tool_names(child);
    assert!(!names.contains(&"skill".to_string()), "{names:?}");
    assert!(
        names.contains(&"subagent".to_string()),
        "all-tools child stays capable"
    );
    assert!(
        !child["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("- research:"),
        "the loaded orchestration skill leaked into the child resolver"
    );
    rig.server.assert_clean();
}

/// Runtime overrides and loaded-skill exclusions cross every process boundary,
/// not only the first one. The root loads `research`, its all-tools child loads
/// `domain`, and the grandchild must inherit the selected endpoint/model/yolo
/// while seeing neither ancestor orchestration skill.
#[test]
fn nested_children_inherit_runtime_and_transitive_skill_exclusions() {
    let selected = MockServer::start();
    selected.allow_interleaving();
    let ignored = MockServer::start();
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &ignored.base_url());

    for (name, description) in [
        ("research", "delegate research investigations"),
        ("domain", "delegate domain inspections"),
    ] {
        let dir = work.path().join(".noob/skills").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n{name} workflow\n"),
        )
        .unwrap();
    }

    // Root: load research, then delegate to an all-tools child.
    selected.enqueue_stream_toolcalls(&[("root-load", "skill", r#"{"name":"research"}"#)], None);
    selected.enqueue_stream_toolcalls(
        &[(
            "root-spawn",
            "subagent",
            r#"{"prompt":"child stage","tools":"all"}"#,
        )],
        None,
    );
    // Child: research is excluded, but domain remains loadable. Loading domain
    // adds it to the exclusion chain before the nested process is spawned.
    selected.enqueue_stream_toolcalls(&[("child-load", "skill", r#"{"name":"domain"}"#)], None);
    selected.enqueue_stream_toolcalls(
        &[(
            "child-spawn",
            "subagent",
            r#"{"prompt":"grandchild stage","tools":"all"}"#,
        )],
        None,
    );
    selected.enqueue_stream_completion("grandchild inherited runtime");
    selected.enqueue_stream_completion("child collected grandchild");
    selected.enqueue_stream_completion("root collected child");

    let out = noob(config.path(), work.path())
        .args([
            "exec",
            "-p",
            "exercise nested inheritance",
            "--base-url",
            &selected.base_url(),
            "--model",
            "root-selected-model",
            "--yolo",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let requests: Vec<Value> = selected
        .recorded()
        .iter()
        .filter(|request| request.path.ends_with("/chat/completions"))
        .map(|request| request.json().unwrap())
        .collect();
    assert_eq!(requests.len(), 7, "root x3, child x3, grandchild x1");
    assert!(
        requests
            .iter()
            .all(|request| request["model"] == "root-selected-model"),
        "a descendant lost the root model override: {requests:?}"
    );

    let initial = |prompt: &str| {
        requests
            .iter()
            .find(|request| {
                request["messages"]
                    .as_array()
                    .is_some_and(|messages| messages.len() == 2 && messages[1]["content"] == prompt)
            })
            .unwrap_or_else(|| panic!("missing initial request for {prompt:?}"))
    };
    let child = initial("child stage");
    let child_system = child["messages"][0]["content"].as_str().unwrap();
    assert!(child_system.contains("sandbox: off (--yolo)"));
    assert!(!child_system.contains("- research:"), "{child_system}");
    assert!(child_system.contains("- domain:"), "{child_system}");
    let child_tools = tool_names(child);
    assert!(
        child_tools.contains(&"skill".to_string()),
        "{child_tools:?}"
    );
    assert!(
        child_tools.contains(&"subagent".to_string()),
        "{child_tools:?}"
    );

    let grandchild = initial("grandchild stage");
    let grandchild_system = grandchild["messages"][0]["content"].as_str().unwrap();
    assert!(grandchild_system.contains("sandbox: off (--yolo)"));
    assert!(
        !grandchild_system.contains("- research:"),
        "{grandchild_system}"
    );
    assert!(
        !grandchild_system.contains("- domain:"),
        "{grandchild_system}"
    );
    let grandchild_tools = tool_names(grandchild);
    assert!(
        !grandchild_tools.contains(&"skill".to_string()),
        "{grandchild_tools:?}"
    );
    assert!(
        !grandchild_tools.contains(&"subagent".to_string()),
        "depth-2 grandchild retained delegation: {grandchild_tools:?}"
    );

    assert!(
        ignored.recorded().is_empty(),
        "a descendant contacted the .env endpoint"
    );
    selected.assert_clean();
}

/// Byte-identity guard for the agents fan-out panel: a headless surface (exec)
/// shows the exact per-task `* task ...` / `* task done` activity lines it
/// always did and NOTHING from the panel (no `agents (` header, no `agent N:`
/// rows). The panel is a themed-REPL-only affordance; non-tty surfaces must
/// stay byte-for-byte unchanged.
#[test]
fn fanout_panel_is_absent_on_the_exec_surface() {
    let rig = rig();
    rig.server.allow_interleaving();
    rig.server.enqueue_stream_toolcalls(
        &[
            ("f1", "subagent", r#"{"prompt":"helper alpha"}"#),
            ("f2", "subagent", r#"{"prompt":"helper beta"}"#),
            ("f3", "subagent", r#"{"prompt":"helper gamma"}"#),
        ],
        None,
    );
    rig.server.enqueue_stream_completion("alpha done");
    rig.server.enqueue_stream_completion("beta done");
    rig.server.enqueue_stream_completion("gamma done");
    rig.server.enqueue_stream_completion("collected");

    let out = noob(rig.config.path(), rig.work.path())
        .args(["exec", "-p", "spawn three helpers"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    // The classic per-task lines are intact for every agent.
    assert!(
        stderr.contains("* subagent helper alpha"),
        "missing start line:\n{stderr}"
    );
    assert!(
        stderr.contains("* subagent helper beta"),
        "missing start line:\n{stderr}"
    );
    assert!(
        stderr.contains("* subagent helper gamma"),
        "missing start line:\n{stderr}"
    );
    assert!(
        stderr.contains("* done (1 turns)"),
        "missing completion line:\n{stderr}"
    );
    // None of the panel bytes reach a headless surface.
    assert!(
        !stderr.contains("agents ("),
        "the panel header leaked into exec:\n{stderr}"
    );
    assert!(
        !stderr.contains("agent 1:"),
        "a panel row leaked into exec:\n{stderr}"
    );
    assert!(
        !stdout_has_panel(&out.stdout),
        "the panel leaked into exec stdout"
    );
    rig.server.assert_clean();
}

fn stdout_has_panel(stdout: &[u8]) -> bool {
    let s = String::from_utf8_lossy(stdout);
    s.contains("agents (") || s.contains("agent 1:")
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
            ("c1", "subagent", r#"{"prompt":"helper one"}"#),
            ("c2", "subagent", r#"{"prompt":"helper two"}"#),
            ("c3", "subagent", r#"{"prompt":"helper three"}"#),
        ],
        None,
    );
    // Each child's model response stalls for CHILD_STALL: the third child
    // cannot even START (and thus send its request) until a slot frees.
    for text in ["one done", "two done", "three done"] {
        rig.enqueue_slow_completion(text, CHILD_STALL.as_millis() as u64);
    }
    rig.server.enqueue_stream_completion("all collected");

    let out = noob(rig.config.path(), rig.work.path())
        .env("NOOB_TASK_CONCURRENCY", "2")
        .args(["exec", "-p", "fan out"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Arrival times of the three child requests, in order.
    let mut child_arrivals: Vec<Instant> = rig
        .server
        .recorded()
        .iter()
        .filter(|r| {
            r.json()
                .and_then(|j| {
                    j["messages"][1]["content"]
                        .as_str()
                        .map(|c| c.starts_with("helper "))
                })
                .unwrap_or(false)
        })
        .map(|r| r.arrived)
        .collect();
    child_arrivals.sort();
    assert_eq!(child_arrivals.len(), 3);
    let first_two_gap = child_arrivals[1].duration_since(child_arrivals[0]);
    let third_gap = child_arrivals[2].duration_since(child_arrivals[0]);
    // Structural overlap: the second request arrived while the first child's
    // response was still inside its stall, i.e. before the first child could
    // possibly have completed.
    assert!(
        first_two_gap < CHILD_STALL,
        "the first two children must overlap; gap was {first_two_gap:?}"
    );
    // Structural queueing: a slot frees no earlier than one full stall after
    // the first arrival, and the third child cannot send before that.
    assert!(
        third_gap >= CHILD_STALL,
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

/// A child at depth 1 keeps the subagent tool, but cannot turn the root's
/// configured fan-out into another full fan-out of its own. Its nested calls
/// run one at a time even when the configured root cap is four.
#[test]
fn nested_fanout_is_serial_without_disabling_nested_agents() {
    let rig = rig();
    rig.server.allow_interleaving();
    rig.server.enqueue_stream_toolcalls(
        &[
            ("n1", "subagent", r#"{"prompt":"nested one"}"#),
            ("n2", "subagent", r#"{"prompt":"nested two"}"#),
            ("n3", "subagent", r#"{"prompt":"nested three"}"#),
        ],
        None,
    );
    for text in ["nested one done", "nested two done", "nested three done"] {
        rig.enqueue_slow_completion(text, CHILD_STALL.as_millis() as u64);
    }
    rig.server
        .enqueue_stream_completion("nested results collected");

    let out = noob(rig.config.path(), rig.work.path())
        .env("NOOB_DEPTH", "1")
        .env("NOOB_TASK_CONCURRENCY", "4")
        .args(["exec", "-p", "delegate nested work"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let mut arrivals: Vec<Instant> = rig
        .server
        .recorded()
        .iter()
        .filter(|request| {
            request
                .json()
                .and_then(|body| {
                    body["messages"][1]["content"]
                        .as_str()
                        .map(|content| content.starts_with("nested "))
                })
                .unwrap_or(false)
        })
        .map(|request| request.arrived)
        .collect();
    arrivals.sort();
    assert_eq!(arrivals.len(), 3, "all nested calls must still execute");
    // Structural seriality: each next nested request can only be sent after
    // the previous child's stalled response completed, so consecutive
    // arrivals must be at least one full stall apart.
    for pair in arrivals.windows(2) {
        let gap = pair[1].duration_since(pair[0]);
        assert!(
            gap >= CHILD_STALL,
            "nested calls overlapped despite the depth-1 clamp; gap was {gap:?}"
        );
    }
    rig.server.assert_clean();
}

/// The wall clock: a wedged child is killed (whole process group) and the
/// parent gets a teaching error instead of a hang; the loop continues.
#[test]
fn task_wall_clock_kills_a_wedged_child() {
    let rig = rig();
    rig.server.allow_interleaving();
    rig.server.enqueue_stream_toolcalls(
        &[("w1", "subagent", r#"{"prompt":"never finishes"}"#)],
        None,
    );
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
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "the wall clock did not fire; took {:?}",
        started.elapsed()
    );

    let reqs = rig.api_requests();
    let result = reqs.last().unwrap()["messages"]
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

#[test]
fn parent_death_kills_bash_descendant_of_child_agent() {
    let rig = rig();
    rig.server.allow_interleaving();
    let marker = rig.work.path().join("orphan-bash.pid");
    let bash_args = serde_json::json!({
        "cmd": "echo $$ > \"$NOOB_ORPHAN_MARKER\"; trap '' TERM; while :; do sleep 1; done"
    })
    .to_string();
    rig.server.enqueue_stream_toolcalls_for(
        noob_testkit::RequestMatch::UserPrompt("delegate the command".to_string()),
        &[(
            "s1",
            "subagent",
            r#"{"prompt":"start the requested command","tools":"all"}"#,
        )],
        None,
    );
    rig.server.enqueue_stream_toolcalls_for(
        noob_testkit::RequestMatch::UserPrompt("start the requested command".to_string()),
        &[("b1", "bash", &bash_args)],
        None,
    );

    // The root noob process spawns `noob child` with PDEATHSIG. Killing the
    // root must also kill Bash after Bash escaped into its own session and
    // ignored SIGTERM.
    let mut parent = noob(rig.config.path(), rig.work.path());
    parent
        .env("NOOB_ORPHAN_MARKER", &marker)
        .args(["exec", "-p", "delegate the command"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut parent = parent.spawn().unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    while !marker.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        marker.is_file(),
        "the child never started its Bash descendant"
    );
    let bash_pid: libc::pid_t = std::fs::read_to_string(&marker)
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    parent.kill().unwrap();
    parent.wait().unwrap();
    let is_live = |pid: libc::pid_t| {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        stat.rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            != Some("Z")
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    while is_live(bash_pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !is_live(bash_pid),
        "Bash process {bash_pid} survived its noob parent"
    );
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
    assert!(!names.contains(&"subagent".to_string()), "{names:?}");
    assert_eq!(
        names.len(),
        9,
        "the 9 core tools only (no subagent at the depth cap)"
    );
    rig.server.assert_clean();
}
