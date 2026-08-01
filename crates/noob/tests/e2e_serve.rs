//! e2e: `noob serve`, the agent as a protocol endpoint. `Command` frames in on
//! stdin, `Event` frames out on stdout, and nothing else on either.
//!
//! This is the surface a front end drives, so what is asserted here is what a
//! front end is entitled to assume: every line of stdout is a frame it can
//! decode, prompts run in the order they were sent, and a command it does not
//! understand costs it that command rather than its session.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use noob_testkit::MockServer;
use serde_json::Value;

fn write_env(dir: &Path, base_url: &str) {
    std::fs::write(
        dir.join(".env"),
        format!("NOOB_BASE_URL={base_url}\nNOOB_API_KEY=k\nNOOB_MODEL=mockmodel\n"),
    )
    .unwrap();
}

/// Start a server, feed it the given command lines, and collect every frame it
/// wrote. Stdin closes after the last line, which is how a front end says it is
/// done and how the session ends.
fn serve(config: &Path, work: &Path, commands: &[Value]) -> (Vec<Value>, String) {
    serve_with(config, work, &[], commands)
}

fn serve_with(
    config: &Path,
    work: &Path,
    extra: &[&str],
    commands: &[Value],
) -> (Vec<Value>, String) {
    let mut child: Child = {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_noob"));
        noob_testkit::scrub_noob_env(&mut cmd);
        cmd.env("NOOB_CONFIG_DIR", config);
        cmd.current_dir(work);
        cmd.arg("serve");
        cmd.args(extra);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.spawn().unwrap()
    };
    {
        let mut stdin = child.stdin.take().unwrap();
        for command in commands {
            writeln!(stdin, "{command}").unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let frames = stdout
        .lines()
        .map(|line| {
            let value: Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("stdout must be frames only, got {line:?}: {e}"));
            assert_eq!(value["v"], 2, "{line}");
            value
        })
        .collect();
    (frames, String::from_utf8_lossy(&out.stderr).into_owned())
}

fn kinds(frames: &[Value]) -> Vec<&str> {
    frames.iter().map(|f| f["t"].as_str().unwrap()).collect()
}

fn text(frames: &[Value]) -> String {
    frames
        .iter()
        .filter(|f| f["t"] == "text.delta")
        .map(|f| f["d"].as_str().unwrap())
        .collect()
}

#[test]
fn a_prompt_in_produces_a_whole_turn_out() {
    let server = MockServer::start();
    server.enqueue_stream_toolcalls(
        &[("c1", "write", r#"{"path":"out.txt","content":"hi\n"}"#)],
        None,
    );
    server.enqueue_completion("wrote out.txt");
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());

    let (frames, stderr) = serve(
        config.path(),
        work.path(),
        &[serde_json::json!({"v": 1, "t": "prompt.submit", "text": "write out.txt"})],
    );
    assert_eq!(stderr, "", "the protocol is the only output");
    server.assert_clean();

    let kinds = kinds(&frames);
    assert_eq!(kinds.first(), Some(&"session.start"), "{kinds:?}");
    assert_eq!(kinds.last(), Some(&"session.end"), "{kinds:?}");
    for expected in ["turn.start", "tool.start", "file.edit", "tool.end", "turn.end"] {
        assert!(kinds.contains(&expected), "no {expected} in {kinds:?}");
    }
    assert_eq!(text(&frames), "wrote out.txt");
    assert_eq!(
        std::fs::read_to_string(work.path().join("out.txt")).unwrap(),
        "hi\n",
        "the turn really ran; the frames are not a simulation of one"
    );

    // A served session persists, so a front end can reopen the conversation.
    let id = frames[0]["id"].as_str().unwrap();
    assert!(!id.is_empty(), "serve names its session");
    assert_eq!(frames.last().unwrap()["id"], id);
}

#[test]
fn prompts_run_in_the_order_they_arrive_and_share_one_session() {
    let server = MockServer::start();
    server.enqueue_completion("first");
    server.enqueue_completion("second");
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());

    let (frames, _) = serve(
        config.path(),
        work.path(),
        &[
            serde_json::json!({"v": 1, "t": "prompt.submit", "text": "one"}),
            serde_json::json!({"v": 1, "t": "prompt.queue", "text": "two"}),
        ],
    );
    server.assert_clean();

    assert_eq!(text(&frames), "firstsecond");
    let turns: Vec<u64> = frames
        .iter()
        .filter(|f| f["t"] == "turn.start")
        .map(|f| f["turn"].as_u64().unwrap())
        .collect();
    assert_eq!(turns, vec![1, 2], "turns are numbered and ordered");
    assert_eq!(
        frames.iter().filter(|f| f["t"] == "session.start").count(),
        1,
        "one session, two turns"
    );

    // The second request carried the first exchange: this is one conversation,
    // not two independent runs that happen to share a process.
    let recorded = server.recorded();
    assert_eq!(recorded.len(), 2);
    let second = recorded[1].json().unwrap();
    let roles: Vec<&str> = second["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["system", "user", "assistant", "user"]);
}

/// The degradation rule, applied to the inbound half. A front end built
/// against a newer agent loses the command it sent, not the session.
#[test]
fn a_command_the_agent_does_not_understand_is_ignored() {
    let server = MockServer::start();
    server.enqueue_completion("still here");
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());

    let (frames, stderr) = serve(
        config.path(),
        work.path(),
        &[
            serde_json::json!({"v": 1, "t": "something.invented.later", "payload": 1}),
            serde_json::json!({"v": 99, "t": "prompt.submit", "text": "from the future"}),
            serde_json::json!({"v": 1, "t": "prompt.submit", "text": "hello"}),
        ],
    );
    assert_eq!(stderr, "");
    server.assert_clean();
    assert_eq!(text(&frames), "still here");
    assert_eq!(
        frames.iter().filter(|f| f["t"] == "turn.start").count(),
        1,
        "only the one command it understood ran"
    );
}

/// Malformed input must not take the session with it, which is the same
/// promise the outbound half makes.
#[test]
fn a_malformed_line_does_not_end_the_session() {
    let server = MockServer::start();
    server.enqueue_completion("fine");
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noob"));
    noob_testkit::scrub_noob_env(&mut cmd);
    let mut child = cmd
        .env("NOOB_CONFIG_DIR", config.path())
        .current_dir(work.path())
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin, "not json at all").unwrap();
        writeln!(stdin).unwrap();
        writeln!(stdin, "{{\"v\":1,\"t\":\"prompt.submit\",\"text\":\"hi\"}}").unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    server.assert_clean();
    assert!(stdout.contains(r#""t":"text.delta""#), "{stdout}");
    assert!(stdout.contains(r#""t":"session.end""#), "{stdout}");
}

/// A front end that goes away closes stdin. The agent finishes what it has and
/// shuts down rather than waiting on a pipe nobody will write to.
#[test]
fn closing_stdin_ends_the_session() {
    let server = MockServer::start();
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noob"));
    noob_testkit::scrub_noob_env(&mut cmd);
    let mut child = cmd
        .env("NOOB_CONFIG_DIR", config.path())
        .current_dir(work.path())
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    // session.start lands before any command, so a front end knows it is up.
    let mut lines = BufReader::new(stdout).lines();
    let first: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(first["t"], "session.start");

    drop(child.stdin.take());
    let status = child.wait().unwrap();
    assert!(status.success(), "a closed front end is not a failure");
    server.assert_clean();
}

/// A resumed session replays its whole picture before anything new: every
/// recorded frame streams back first - the prompt included, as `user.echo` -
/// so a front end rebuilds all of its panes from what already happened, and
/// only then does the live conversation continue.
#[test]
fn a_resumed_session_replays_its_record_before_the_live_stream() {
    let server = MockServer::start();
    server.enqueue_completion("first answer");
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());
    let (frames, _) = serve(
        config.path(),
        work.path(),
        &[serde_json::json!({"v": 1, "t": "prompt.submit", "text": "hello there"})],
    );
    let id = frames[0]["id"].as_str().unwrap().to_string();
    server.assert_clean();

    server.enqueue_completion("second answer");
    let (frames, stderr) = serve_with(
        config.path(),
        work.path(),
        &["--resume", &id],
        &[serde_json::json!({"v": 1, "t": "prompt.submit", "text": "and again"})],
    );
    assert_eq!(stderr, "", "the protocol is the only output");
    server.assert_clean();

    assert_eq!(frames[0]["t"], "session.start");
    assert_eq!(frames[0]["resumed"], true);
    // The record comes back whole and in order: the old turn, its prompt as
    // user.echo, its text; then the live turn.
    let full = text(&frames);
    let old = full.find("first answer").expect("the recorded answer replays");
    let live = full.find("second answer").expect("the live answer arrives");
    assert!(old < live, "{full}");
    let echoed: Vec<&str> = frames
        .iter()
        .filter(|f| f["t"] == "user.echo")
        .map(|f| f["text"].as_str().unwrap())
        .collect();
    assert_eq!(echoed, ["hello there"], "the prompt replays exactly once");
    // And a third life replays both turns, so the record really grew.
    let (frames, _) = serve_with(config.path(), work.path(), &["--resume", &id], &[]);
    let echoed: Vec<&str> = frames
        .iter()
        .filter(|f| f["t"] == "user.echo")
        .map(|f| f["text"].as_str().unwrap())
        .collect();
    assert_eq!(echoed, ["hello there", "and again"]);
}
