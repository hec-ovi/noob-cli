//! e2e: the protocol side-channel, against the compiled binary and the mock
//! server. What a front end sees is a file of newline-delimited frames written
//! beside a session that behaves exactly as it did without them.
//!
//! Two properties matter more than any individual frame, and both are asserted
//! here rather than argued for: the stream is off unless `NOOB_EMIT` names a
//! file, and turning it on changes no byte of what the human sees.

use std::path::Path;
use std::process::Command;

use noob_testkit::MockServer;
use serde_json::Value;

fn write_env(dir: &Path, base_url: &str) {
    std::fs::write(
        dir.join(".env"),
        format!("NOOB_BASE_URL={base_url}\nNOOB_API_KEY=k\nNOOB_MODEL=mockmodel\n"),
    )
    .unwrap();
}

fn noob(config_dir: &Path, work: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noob"));
    noob_testkit::scrub_noob_env(&mut cmd);
    cmd.env("NOOB_CONFIG_DIR", config_dir);
    cmd.current_dir(work);
    cmd
}

/// Every frame in the file, decoded. A line that does not decode fails the
/// test rather than being skipped: a consumer that cannot read the stream is
/// the whole failure mode this protocol exists to avoid.
fn frames(path: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .map(|line| {
            let value: Value =
                serde_json::from_str(line).unwrap_or_else(|e| panic!("bad frame {line:?}: {e}"));
            assert_eq!(value["v"], 1, "every frame carries its version: {line}");
            assert!(value["t"].is_string(), "every frame names its type: {line}");
            value
        })
        .collect()
}

fn kinds(frames: &[Value]) -> Vec<&str> {
    frames.iter().map(|f| f["t"].as_str().unwrap()).collect()
}

fn first<'a>(frames: &'a [Value], kind: &str) -> &'a Value {
    frames
        .iter()
        .find(|f| f["t"] == kind)
        .unwrap_or_else(|| panic!("no {kind} frame in {:?}", kinds(frames)))
}

/// The default. Nothing is written, nothing is opened, and the surface the
/// human sees is the one it has always been.
#[test]
fn no_frames_are_written_unless_the_sink_is_named() {
    let server = MockServer::start();
    server.enqueue_completion("hello");
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());

    let out = noob(config.path(), work.path())
        .args(["exec", "-p", "say hi"])
        .output()
        .unwrap();

    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");
    assert!(
        !work.path().join("frames.ndjson").exists(),
        "a sink nobody asked for must not appear"
    );
    server.assert_clean();
}

/// The guarantee the whole design rests on: a watcher runs beside the human
/// surface, never in place of it. Byte-for-byte on stdout and stderr.
#[test]
fn emitting_changes_no_byte_of_what_the_human_sees() {
    let run = |emit: Option<&Path>| {
        let server = MockServer::start();
        server.enqueue_stream_toolcalls(
            &[("call-1", "write", r#"{"path":"a.txt","content":"one\ntwo\n"}"#)],
            None,
        );
        server.enqueue_completion("wrote it");
        let config = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        write_env(config.path(), &server.base_url());
        let mut cmd = noob(config.path(), work.path());
        if let Some(path) = emit {
            cmd.env("NOOB_EMIT", path);
        }
        let out = cmd.args(["exec", "-p", "write a.txt"]).output().unwrap();
        server.assert_clean();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };

    let sink = tempfile::tempdir().unwrap();
    let quiet = run(None);
    let watched = run(Some(&sink.path().join("frames.ndjson")));
    assert_eq!(quiet.0, watched.0, "stdout must not move");
    assert_eq!(quiet.1, watched.1, "stderr must not move");
    assert!(!quiet.0.is_empty(), "the run produced nothing to compare");
}

/// One whole turn, from the session opening to the model's last word, with a
/// tool call and a file write in the middle. This is the stream a code view is
/// driven from, so it is asserted as a sequence rather than as a set.
#[test]
fn a_turn_with_a_tool_call_produces_a_complete_stream() {
    let server = MockServer::start();
    server.enqueue_stream_toolcalls(
        &[(
            "call-write",
            "write",
            r#"{"path":"notes.md","content":"alpha\nbeta\n"}"#,
        )],
        Some((1200, 7)),
    );
    server.enqueue_completion("done");
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());
    let sink = work.path().join("frames.ndjson");

    let out = noob(config.path(), work.path())
        .env("NOOB_EMIT", &sink)
        .args(["exec", "-p", "write notes.md"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    server.assert_clean();

    let frames = frames(&sink);
    // Measurements are a series that runs alongside the conversation and can
    // land between any two frames, so the shape of the conversation is
    // asserted without them.
    let shape: Vec<&str> = kinds(&frames)
        .into_iter()
        .filter(|kind| *kind != "metrics")
        .collect();
    assert!(
        shape.starts_with(&["session.start", "turn.start"]),
        "{shape:?}"
    );
    assert_eq!(shape.last(), Some(&"turn.end"), "{shape:?}");

    let start = first(&frames, "session.start");
    assert_eq!(start["model"], "mockmodel");
    assert_eq!(start["resumed"], false);
    assert!(
        start["workspace"].as_str().unwrap().ends_with(
            work.path()
                .canonicalize()
                .unwrap()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
        ),
        "the workspace names the tree the paths below are relative to: {start}"
    );

    // The call opens and closes under one id, which is what a consumer pairs
    // on. Counting positions is what the stream this replaces forced, and it
    // is wrong the moment anything runs concurrently.
    let tool_start = first(&frames, "tool.start");
    assert_eq!(tool_start["call_id"], "call-write");
    assert_eq!(tool_start["name"], "write");
    assert_eq!(tool_start["args"]["path"], "notes.md");
    assert!(tool_start["brief"].as_str().unwrap().contains("notes.md"));
    let tool_end = first(&frames, "tool.end");
    assert_eq!(tool_end["call_id"], "call-write");
    assert!(tool_end.get("error").is_none(), "the write succeeded");

    // The write is a diff a consumer can draw without reading the file.
    let edit = first(&frames, "file.edit");
    assert_eq!(edit["path"], "notes.md");
    assert_eq!(edit["call_id"], "call-write", "attributed to its call");
    assert_eq!(edit["before"], "");
    assert_eq!(edit["after"], "alpha\nbeta");
    assert_eq!(edit["span"]["start"], 1);
    assert_eq!(edit["span"]["end"], 2);

    // Prefill is what the endpoint computed, not the whole prompt.
    let usage = first(&frames, "usage");
    assert_eq!(usage["usage"]["prompt"], 1200);
    assert_eq!(usage["usage"]["cached_prompt"], 0);
    assert_eq!(usage["usage"]["completion"], 7);
    assert!(usage["usage"]["context_total"].as_u64().unwrap() > 0);

    let text: String = frames
        .iter()
        .filter(|f| f["t"] == "text.delta")
        .map(|f| f["d"].as_str().unwrap())
        .collect();
    assert_eq!(text, "done");

    // Both turns are numbered and both are closed.
    let turn_starts: Vec<u64> = frames
        .iter()
        .filter(|f| f["t"] == "turn.start")
        .map(|f| f["turn"].as_u64().unwrap())
        .collect();
    let turn_ends: Vec<u64> = frames
        .iter()
        .filter(|f| f["t"] == "turn.end")
        .map(|f| f["turn"].as_u64().unwrap())
        .collect();
    assert_eq!(turn_starts, vec![1]);
    assert_eq!(turn_ends, vec![1]);

    // Context fill is a series: a value, the window it is measured against,
    // and a stamp on the stream's own clock, so a consumer can graph it
    // without a clock of its own.
    let context: Vec<&serde_json::Value> = frames
        .iter()
        .filter(|f| f["t"] == "metrics" && f["group"] == "context")
        .collect();
    assert!(!context.is_empty(), "{:?}", kinds(&frames));
    let sample = &context[0]["samples"][0];
    assert_eq!(sample["key"], "used");
    assert_eq!(sample["unit"], "tokens");
    assert!(sample["max"].as_f64().unwrap() > 0.0, "{sample}");
    for frame in &context {
        assert!(frame["at_ms"].is_u64(), "{frame}");
    }
    // Only where something moved: the agent refreshes at several boundaries
    // that can land back to back on identical numbers.
    let used: Vec<f64> = context
        .iter()
        .map(|f| f["samples"][0]["value"].as_f64().unwrap())
        .collect();
    assert!(
        used.windows(2).all(|pair| pair[0] != pair[1]),
        "a series repeated a point: {used:?}"
    );
}

/// Reads run eight wide, so a file frame that cannot name its call is
/// unattributable. Two reads in one batch must each carry their own id.
#[test]
fn concurrent_reads_attribute_their_file_frames_to_the_right_call() {
    let server = MockServer::start();
    server.enqueue_stream_toolcalls(
        &[
            ("call-a", "read", r#"{"path":"a.txt"}"#),
            ("call-b", "read", r#"{"path":"b.txt"}"#),
        ],
        None,
    );
    server.enqueue_completion("read both");
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());
    std::fs::write(work.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(work.path().join("b.txt"), "only\n").unwrap();
    let sink = work.path().join("frames.ndjson");

    let out = noob(config.path(), work.path())
        .env("NOOB_EMIT", &sink)
        .args(["exec", "-p", "read both"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    server.assert_clean();

    let frames = frames(&sink);
    let opens: Vec<(&str, &str, u64)> = frames
        .iter()
        .filter(|f| f["t"] == "file.open")
        .map(|f| {
            (
                f["path"].as_str().unwrap(),
                f["call_id"].as_str().unwrap(),
                f["lines"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(opens.len(), 2, "{opens:?}");
    // Completion order is not emission order, so match by identity.
    assert!(opens.contains(&("a.txt", "call-a", 3)), "{opens:?}");
    assert!(opens.contains(&("b.txt", "call-b", 1)), "{opens:?}");

    let spans: Vec<(&str, &str, u64, u64)> = frames
        .iter()
        .filter(|f| f["t"] == "file.span")
        .map(|f| {
            (
                f["path"].as_str().unwrap(),
                f["call_id"].as_str().unwrap(),
                f["span"]["start"].as_u64().unwrap(),
                f["span"]["end"].as_u64().unwrap(),
            )
        })
        .collect();
    assert!(spans.contains(&("a.txt", "call-a", 1, 3)), "{spans:?}");
    assert!(spans.contains(&("b.txt", "call-b", 1, 1)), "{spans:?}");

    // Every call that opened also closed, whatever order they finished in.
    let started: Vec<&str> = frames
        .iter()
        .filter(|f| f["t"] == "tool.start")
        .map(|f| f["call_id"].as_str().unwrap())
        .collect();
    let ended: Vec<&str> = frames
        .iter()
        .filter(|f| f["t"] == "tool.end")
        .map(|f| f["call_id"].as_str().unwrap())
        .collect();
    assert_eq!(started, vec!["call-a", "call-b"]);
    assert_eq!(ended.len(), 2);
    for id in &started {
        assert!(ended.contains(id), "{id} opened and never closed");
    }
}

/// A failed call is fields, not prose. This is the same failure that used to
/// render as a bare `exit code 1`, now carrying its parts separately: the
/// class, the message, the rest, and what to do next.
#[test]
fn a_failed_call_carries_its_error_as_fields() {
    let server = MockServer::start();
    server.enqueue_stream_toolcalls(&[("call-x", "read", r#"{"path":"missing.txt"}"#)], None);
    server.enqueue_completion("it is gone");
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());
    let sink = work.path().join("frames.ndjson");

    noob(config.path(), work.path())
        .env("NOOB_EMIT", &sink)
        .args(["exec", "-p", "read missing.txt"])
        .output()
        .unwrap();
    server.assert_clean();

    let frames = frames(&sink);
    let end = first(&frames, "tool.end");
    assert_eq!(end["call_id"], "call-x");
    let error = &end["error"];
    // Classed where it was minted, not guessed from the message here.
    assert_eq!(error["kind"], "not_found");
    assert!(
        error["message"].as_str().unwrap().contains("missing.txt"),
        "{error}"
    );
    assert!(error["detail"].is_string(), "the rest is carried whole");
    // A refused read opened no file.
    assert!(
        !frames.iter().any(|f| f["t"] == "file.open"),
        "nothing was opened"
    );
}

/// A sink that cannot be opened turns emission off. A broken watcher must
/// never be able to stop the agent working.
#[test]
fn an_unusable_sink_is_not_a_session_failure() {
    let server = MockServer::start();
    server.enqueue_completion("still fine");
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());

    let out = noob(config.path(), work.path())
        .env("NOOB_EMIT", work.path().join("no/such/dir/frames.ndjson"))
        .args(["exec", "-p", "say hi"])
        .output()
        .unwrap();

    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "still fine");
    server.assert_clean();
}

/// A build scrolls. The whole reason this frame exists is that a command's
/// summary arrives when it is already over, and what somebody watching wants
/// is the line it is on right now.
#[test]
fn a_shell_command_sends_its_output_line_by_line_as_it_runs() {
    let server = MockServer::start();
    server.enqueue_stream_toolcalls(
        &[(
            "call-sh",
            "bash",
            r#"{"cmd":"printf 'compiling one\\ncompiling two\\n'; echo 'error: it broke' >&2; exit 3"}"#,
        )],
        None,
    );
    server.enqueue_completion("it failed");
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());
    let sink = work.path().join("frames.ndjson");

    let out = noob(config.path(), work.path())
        .env("NOOB_EMIT", &sink)
        .args(["exec", "-p", "build it"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    server.assert_clean();

    let frames = frames(&sink);
    let progress: Vec<(&str, &str)> = frames
        .iter()
        .filter(|f| f["t"] == "tool.progress")
        .map(|f| (f["call_id"].as_str().unwrap(), f["line"].as_str().unwrap()))
        .collect();
    // Every line, attributed to the call that produced it, so eight concurrent
    // calls cannot mix their output.
    assert_eq!(
        progress,
        [
            ("call-sh", "compiling one"),
            ("call-sh", "compiling two"),
            ("call-sh", "error: it broke"),
        ],
        "{progress:?}"
    );

    // Live output arrives before the call closes, or it is not live.
    let kinds = kinds(&frames);
    let last_progress = kinds.iter().rposition(|k| *k == "tool.progress").unwrap();
    let end = kinds.iter().position(|k| *k == "tool.end").unwrap();
    assert!(last_progress < end, "{kinds:?}");

    // The same call also carries its failure as fields: the class, the exit
    // status as a number, and what to do about it where that is knowable.
    let error = &first(&frames, "tool.end")["error"];
    assert_eq!(error["kind"], "exit_status");
    assert_eq!(error["code"], 3);
    assert!(error["message"].as_str().unwrap().contains("it broke"), "{error}");
}

/// Compaction is the one honest end of a file's life: content the model saw
/// is no longer in its context. A code view that never hears this keeps
/// showing pages the agent has forgotten.
#[test]
fn compaction_closes_every_file_the_model_had_open() {
    let server = MockServer::start();
    let big: String = (0..300)
        .map(|i| format!("line {i:03} {}\n", "x".repeat(90)))
        .collect();
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());
    std::fs::write(work.path().join("big.txt"), &big).unwrap();
    let sink = work.path().join("frames.ndjson");

    // Usage on the first round pushes the estimate past the threshold, so the
    // second round compacts before it asks anything.
    server.enqueue_stream_toolcalls(&[("p1", "read", r#"{"path":"big.txt"}"#)], Some((2000, 50)));
    server.enqueue_stream_completion("noted the file");

    let out = noob(config.path(), work.path())
        .env("NOOB_EMIT", &sink)
        .env("NOOB_CTX", "4096")
        .args(["exec", "-p", "look at big.txt"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    server.assert_clean();

    let frames = frames(&sink);
    let opened: Vec<&str> = frames
        .iter()
        .filter(|f| f["t"] == "file.open")
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(opened, ["big.txt"]);
    let closed: Vec<&str> = frames
        .iter()
        .filter(|f| f["t"] == "file.close")
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(closed, ["big.txt"], "{:?}", kinds(&frames));

    // The same names on both sides, or a consumer cannot pair them.
    let kinds = kinds(&frames);
    let open_at = kinds.iter().position(|k| *k == "file.open").unwrap();
    let close_at = kinds.iter().position(|k| *k == "file.close").unwrap();
    assert!(open_at < close_at, "{kinds:?}");
    // Nothing asked for it: compaction is not a tool call.
    let close = first(&frames, "file.close");
    assert!(close.get("call_id").is_none(), "{close}");
}

/// The file registry is append-only, so a second compaction must not announce
/// the end of a file that ended at the first one. Only what is still in the
/// model's context can leave it.
#[test]
fn a_second_compaction_only_closes_what_is_still_open() {
    let server = MockServer::start();
    let big = |name: &str| -> String {
        (0..300)
            .map(|i| format!("{name} line {i:03} {}\n", "x".repeat(80)))
            .collect()
    };
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());
    std::fs::write(work.path().join("one.txt"), big("one")).unwrap();
    std::fs::write(work.path().join("two.txt"), big("two")).unwrap();
    let sink = work.path().join("frames.ndjson");

    // Two rounds, each reading one file and each reporting usage past the
    // threshold, so each is followed by a compaction.
    server.enqueue_stream_toolcalls(&[("r1", "read", r#"{"path":"one.txt"}"#)], Some((2000, 50)));
    server.enqueue_stream_toolcalls(&[("r2", "read", r#"{"path":"two.txt"}"#)], Some((2000, 50)));
    server.enqueue_stream_completion("read both");

    let out = noob(config.path(), work.path())
        .env("NOOB_EMIT", &sink)
        .env("NOOB_CTX", "4096")
        .args(["exec", "-p", "read both files"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    server.assert_clean();

    let frames = frames(&sink);
    let closed: Vec<&str> = frames
        .iter()
        .filter(|f| f["t"] == "file.close")
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    // Each file closes exactly once, at the compaction that dropped it.
    assert_eq!(closed, ["one.txt", "two.txt"], "{closed:?}");
}
