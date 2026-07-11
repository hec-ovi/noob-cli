//! v0.2.x interface e2e: the raw-mode line editor through the compiled binary.
//! A real pty makes the REPL see a terminal, so the termios editor engages;
//! these drive it byte-for-byte the way a keyboard would and assert on the
//! EDITED result that reaches the agent (the recorded request), never on how
//! it looks. A piped run must take the cooked path with no box and no
//! bracketed-paste toggles, byte-identical to before the editor existed.

use std::io::{Read, Write};
use std::os::fd::FromRawFd;
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
    fn api_requests(&self) -> Vec<Value> {
        self.server
            .recorded()
            .iter()
            .filter(|r| r.path.ends_with("/chat/completions"))
            .map(|r| r.json().unwrap())
            .collect()
    }
}

/// The last user message in a recorded chat request: the line the editor
/// actually submitted.
fn last_user(req: &Value) -> String {
    req["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Spawn the REPL on a fresh pty and return (child, master fd file, a cancel
/// flag + watchdog handle). The child's stdin/stdout/stderr are the slave, so
/// `is_terminal()` is true and the raw editor engages.
fn spawn_pty(rig: &Rig) -> Pty {
    // These tests exercise the classic per-prompt editor explicitly. The dock
    // is now the product default and has its own whole-turn tests below.
    spawn_pty_with(rig, &[("NOOB_DOCK", "0")])
}

/// Spawn with exactly the requested UI environment. An empty slice exercises
/// the default dock; `NOOB_DOCK=0` is the classic escape hatch.
fn spawn_pty_with(rig: &Rig, envs: &[(&str, &str)]) -> Pty {
    let (master, slave) = unsafe {
        let mut m: libc::c_int = 0;
        let mut s: libc::c_int = 0;
        assert_eq!(
            libc::openpty(&mut m, &mut s, std::ptr::null_mut(), std::ptr::null(), std::ptr::null()),
            0,
            "openpty failed"
        );
        (std::fs::File::from_raw_fd(m), s)
    };
    let stdio = |fd: i32| unsafe { std::process::Stdio::from_raw_fd(libc::dup(fd)) };
    // Force the themed color surface on regardless of the host's TERM, so the
    // pty tests exercise the real interactive path (a color terminal) and the
    // thinking scanner engages deterministically.
    let mut cmd = noob(rig.config.path(), rig.work.path());
    cmd.env("COLORTERM", "truecolor").env_remove("NO_COLOR");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let child = cmd
        .stdin(stdio(slave))
        .stdout(stdio(slave))
        .stderr(stdio(slave))
        .spawn()
        .unwrap();
    unsafe { libc::close(slave) };

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let child_pid = child.id() as i32;
    let done = Arc::new(AtomicBool::new(false));
    let wd_done = done.clone();
    let watchdog = std::thread::spawn(move || {
        for _ in 0..200 {
            if wd_done.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        unsafe { libc::kill(child_pid, libc::SIGKILL) };
    });
    Pty { master, child: Some(child), done, watchdog: Some(watchdog), seen: String::new(), cursor: 0 }
}

/// The sequence the editor writes right after `tcsetattr(raw)` succeeds.
/// Waiting for it proves the terminal is raw, so editing keys sent afterward
/// are handled by the editor and not the cooked line discipline (which would
/// treat Ctrl-U/Ctrl-C/Ctrl-D as VKILL/VINTR/VEOF).
const RAW_READY: &str = "\x1b[?2004h";

struct Pty {
    master: std::fs::File,
    child: Option<std::process::Child>,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    watchdog: Option<std::thread::JoinHandle<()>>,
    seen: String,
    /// How far `wait_for` has consumed, so successive calls match successive
    /// occurrences (each prompt re-emits the same markers).
    cursor: usize,
}

impl Pty {
    fn send(&mut self, bytes: &[u8]) {
        self.master.write_all(bytes).unwrap();
    }

    /// Read the master until `marker` appears at or after the last match, then
    /// advance past it. Consuming, so it syncs to one prompt at a time.
    fn wait_for(&mut self, marker: &str) {
        let mut buf = [0u8; 4096];
        loop {
            if let Some(pos) = self.seen[self.cursor..].find(marker) {
                self.cursor += pos + marker.len();
                return;
            }
            match self.master.read(&mut buf) {
                Ok(0) => panic!("pty closed before {marker:?}; saw:\n{}", self.seen),
                Ok(n) => self.seen.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(e) => panic!("pty read error: {e}; saw:\n{}", self.seen),
            }
        }
    }

    /// Wait for the child to exit and return its status, stopping the watchdog.
    fn finish(&mut self) -> std::process::ExitStatus {
        let status = self.child.take().unwrap().wait().unwrap();
        self.done.store(true, std::sync::atomic::Ordering::SeqCst);
        self.watchdog.take().unwrap().join().ok();
        status
    }
}

/// The editor's line editing reaches the agent: text typed, then killed with
/// Ctrl-U, then the real line typed and submitted with a carriage return. The
/// agent must receive only the edited line. Ctrl-D on the next empty prompt
/// exits cleanly (distinct from a reprompt).
#[test]
fn raw_editor_edits_then_submits_the_clean_line() {
    let rig = rig();
    rig.server.enqueue_stream_completion("done one");

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY); // prompt 1 is now in raw mode
    pty.send(b"garbage draft");
    pty.send(&[0x15]); // Ctrl-U: kill the whole line
    pty.send(b"say hi\r"); // the real line, submitted with CR
    pty.wait_for("done one");
    pty.wait_for(RAW_READY); // prompt 2 is now in raw mode
    pty.send(&[0x04]); // Ctrl-D at the empty prompt: exit
    pty.wait_for("resume with --session"); // the exit hint tells you how to reopen
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1, "only the edited line should have run");
    assert_eq!(last_user(&reqs[0]), "say hi", "the killed draft leaked into the message");
    rig.server.assert_clean();
}

/// The idle prompt is a bare marker; the first keystroke expands it into a
/// framed box, so a horizontal rule (the frame's top/bottom line) only appears
/// once the human starts typing. The assertion is behavioral, not cosmetic: the
/// rule glyph is present after typing (the frame drew) and the edited line still
/// reaches the agent. Colors are never asserted.
#[test]
fn raw_editor_expands_a_framed_box_when_typing_starts() {
    let rig = rig();
    rig.server.enqueue_stream_completion("framed reply");

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY); // raw mode: the bare marker is drawn, no frame yet
    let before_typing = pty.seen.len();
    pty.send(b"hello frame");
    // Typing expands the box, so the frame's rule (a run of the horizontal line
    // glyph) is emitted; the banner's own rule is already behind the cursor.
    pty.wait_for("──");
    // The rule must appear after the point where typing began (the banner's own
    // rule is earlier, already behind the cursor when raw mode started).
    assert!(
        pty.seen[before_typing..].contains("──"),
        "the frame rule must be drawn only after typing:\n{}",
        pty.seen
    );
    pty.send(b"\r"); // submit
    pty.wait_for("framed reply");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]); // Ctrl-D exits
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1, "only the submitted line should have run");
    assert_eq!(last_user(&reqs[0]), "hello frame", "the framed line must reach the agent intact");
    rig.server.assert_clean();
}

/// The REPL persists its session and `--session <id>` resumes it: a second run
/// against the same id byte-extends the first run's transcript.
#[test]
fn repl_session_resume_extends_the_transcript() {
    let rig = rig();
    rig.server.enqueue_stream_completion("noted");
    let out1 = run_repl(&rig, &["--session", "reptest"], b"remember alpha\n/quit\n");
    assert!(out1.status.success(), "run 1 failed: {out1:?}");

    rig.server.enqueue_stream_completion("recalled");
    let out2 = run_repl(&rig, &["--restore", "reptest"], b"what did i say\n/quit\n");
    assert!(out2.status.success(), "run 2 failed: {out2:?}");

    // Run 2's request replays run 1's user message: the transcript resumed and
    // extended append-only (the mock's prefix assertion also saw no break).
    let reqs = rig.api_requests();
    let last = reqs.last().unwrap();
    let msgs = last["messages"].as_array().unwrap();
    assert!(
        msgs.iter().any(|m| m["role"] == "user" && m["content"] == "remember alpha"),
        "resumed transcript missing the first turn: {msgs:?}"
    );
    rig.server.assert_clean();
}

/// Run the REPL with args and piped stdin; return its output.
fn run_repl(rig: &Rig, args: &[&str], input: &[u8]) -> std::process::Output {
    let mut child = noob(rig.config.path(), rig.work.path())
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

/// Ctrl-C at the prompt cancels the current line and reprompts; it never
/// submits. The line typed before it must not reach the agent, and the line
/// typed after it must.
#[test]
fn raw_ctrl_c_cancels_the_line_without_submitting() {
    let rig = rig();
    rig.server.enqueue_stream_completion("answered");

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY); // in raw mode: Ctrl-C is a byte, not VINTR
    pty.send(b"abandon this");
    pty.send(&[0x03]); // Ctrl-C: cancel, reprompt
    pty.wait_for("interrupted");
    pty.wait_for(RAW_READY); // the reprompt is in raw mode
    pty.send(b"real one\r");
    pty.wait_for("answered");
    pty.wait_for(RAW_READY); // the next prompt is in raw mode
    pty.send(b"/quit\r");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1, "the canceled draft must not have run");
    assert_eq!(last_user(&reqs[0]), "real one");
    rig.server.assert_clean();
}

/// A multi-line submission delivered in one raw read (as a terminal that
/// ignores bracketed paste would deliver a multi-line paste) runs one turn per
/// line: the tail after the first Enter is carried to the next prompt instead
/// of being dropped.
#[test]
fn raw_multiline_input_runs_one_turn_per_line() {
    let rig = rig();
    rig.server.enqueue_stream_completion("first done");
    rig.server.enqueue_stream_completion("second done");

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY); // raw: the tty does not canonicalize the newlines
    pty.send(b"line one\nline two\n"); // two lines in a single write
    pty.wait_for("first done");
    pty.wait_for("second done");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]); // Ctrl-D exits
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2, "each line should be its own turn");
    assert_eq!(last_user(&reqs[0]), "line one");
    assert_eq!(last_user(&reqs[1]), "line two");
    rig.server.assert_clean();
}

/// The thinking scanner sweeps during the request-to-first-token gap: after a
/// prompt is submitted, at least one comet frame reaches the terminal before the
/// reply arrives. The assertion is that it rendered at all (a lifecycle fact),
/// not how it looks; the piped test below is the byte-identity counterpart that
/// proves a non-tty surface shows none of it.
#[test]
fn raw_repl_shows_a_thinking_scanner_while_the_model_works() {
    let rig = rig();
    rig.server.enqueue_stream_completion("scanned reply");

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"work on it\r");
    pty.wait_for("scanned reply");
    pty.wait_for(RAW_READY); // back at a fresh prompt
    pty.send(&[0x04]); // Ctrl-D exits
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    // The comet glyph appears before the reply and is then cleared; its bytes
    // remain in the stream even though the line was wiped.
    let last_comet = pty
        .seen
        .rfind('▪')
        .unwrap_or_else(|| panic!("the thinking scanner never rendered a comet frame:\n{}", pty.seen));
    // ...and it is torn down before the reply: no frame lands after the reply
    // text begins, so the model's words never interleave with the animation
    // (the first output byte joins the animation thread before it is written).
    let reply_at = pty.seen.find("scanned reply").expect("reply never arrived");
    assert!(
        last_comet < reply_at,
        "a comet frame rendered after the reply began (scanner not torn down):\n{}",
        pty.seen
    );
    rig.server.assert_clean();
}

/// A piped REPL (stdin not a terminal) takes the cooked reader: the plain `> `
/// marker prints, and neither the box frame, the bracketed-paste toggles, nor
/// the thinking scanner ever reach the output. This is the byte-identity guard
/// for the non-tty surface.
#[test]
fn piped_repl_uses_cooked_reader_with_no_box() {
    let rig = rig();
    rig.server.enqueue_stream_completion("piped answer");

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
        .write_all(b"hello there\n/quit\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("piped answer"), "turn did not run: {stdout}");
    assert!(stdout.contains("> "), "cooked prompt marker missing: {stdout}");
    assert!(!stdout.contains('›'), "the box marker leaked into a piped repl: {stdout}");
    assert!(
        !stdout.contains("\x1b[?2004h") && !stdout.contains("\x1b[?2004l"),
        "bracketed paste toggled on a piped repl: {stdout}"
    );
    assert!(!stdout.contains('▪'), "the thinking scanner leaked into a piped repl: {stdout}");
    rig.server.assert_clean();
}

// ---------------------------------------------------------------------------
// The dock driver (default, with NOOB_DOCK=0 as the opt-out): the persistent-input REPL where the
// input frame stays live during a turn (fable.md v0.3.0). These prove the
// driver against the same bar as the classic editor: what reaches the agent,
// never how it looks.
// ---------------------------------------------------------------------------

const DOCK: &[(&str, &str)] = &[("NOOB_DOCK", "1")];

#[test]
fn dock_is_default_and_liveness_survives_first_output() {
    let rig = rig();
    rig.server.enqueue_stream_completion("default dock reply");

    // No NOOB_DOCK variable: the persistent driver is the default.
    let mut pty = spawn_pty_with(&rig, &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.wait_for("default dock reply");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let reply = pty.seen.find("default dock reply").unwrap();
    let last_working = pty.seen.rfind("Working").unwrap();
    assert!(
        last_working > reply,
        "whole-turn liveness disappeared after the first output:\n{}",
        pty.seen
    );
    rig.server.assert_clean();
}

#[test]
fn interactive_model_markdown_renders_headings_code_json_and_tables() {
    let rig = rig();
    rig.server.enqueue_stream_completion(
        "### Status\n**ready** with `inline`\n```json\n{\"ok\": true, \"n\": 2}\n```\n\
         | name | state |\n| :--- | ---: |\n| noob | ready |\nRENDER-END",
    );

    let mut pty = spawn_pty_with(&rig, &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"show formatting\r");
    pty.wait_for("RENDER-END");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    assert!(!pty.seen.contains("### Status"), "heading markdown leaked as source");
    assert!(!pty.seen.contains("**ready**"), "bold markdown leaked as source");
    assert!(!pty.seen.contains("```json"), "fence markdown leaked as source");
    assert!(
        pty.seen.contains("┌─ ") && pty.seen.contains("json"),
        "JSON fence lost its labelled gutter"
    );
    assert!(pty.seen.contains('┬'), "the table was not laid out as a grid");
    rig.server.assert_clean();
}

/// Cross a boundary that has no byte marker (turn teardown to the next
/// prompt's reader). Generous next to the epilogue's sub-millisecond cost.
fn settle() {
    std::thread::sleep(std::time::Duration::from_millis(400));
}

/// Chunked-transfer frames for a run of SSE `data:` payloads (one frame per
/// event, no terminator), for scripting a stream that stalls mid-reply.
fn sse_frames(datas: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for d in datas {
        let event = format!("data: {d}\n\n");
        out.extend_from_slice(format!("{:x}\r\n", event.len()).as_bytes());
        out.extend_from_slice(event.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Dock parity with the classic editor: editing keys shape the line, only
/// the edited line reaches the agent, Ctrl-D exits with the session hint.
#[test]
fn dock_edits_and_submits_like_the_classic_editor() {
    let rig = rig();
    rig.server.enqueue_stream_completion("docked reply");

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY); // the session guard went raw (once per session)
    pty.send(b"garbage draft");
    pty.send(&[0x15]); // Ctrl-U kills the line
    pty.send(b"say hi\r");
    // Streamed words arrive as separate deltas with dock repaints between
    // them, so multi-word markers would never match contiguously.
    pty.wait_for("docked");
    pty.wait_for("reply");
    settle(); // the next prompt has no raw-toggle marker: raw spans the session
    pty.send(&[0x04]); // Ctrl-D at the empty prompt exits
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1, "only the edited line should have run");
    assert_eq!(last_user(&reqs[0]), "say hi", "the killed draft leaked into the message");
    rig.server.assert_clean();
}

/// The root corruption the dock exists to fix: keystrokes during a streaming
/// turn are captured into the live draft (nothing echoes into the model's
/// output), survive the turn, and submit as the NEXT message. The reply text
/// itself arrives intact around the stall.
#[test]
fn dock_captures_typing_during_a_slow_stream() {
    let rig = rig();
    let datas = noob_testkit::chat_stream_datas("Alpha waits then finishes cleanly.");
    // Stall the stream after the first content word, long enough to type.
    let mut steps = vec![noob_testkit::RawStep::Bytes({
        let mut b = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n".to_vec();
        b.extend_from_slice(&sse_frames(&datas[..2])); // role delta + "Alpha "
        b
    })];
    steps.push(noob_testkit::RawStep::SleepMs(900));
    steps.push(noob_testkit::RawStep::Bytes({
        let mut b = sse_frames(&datas[2..]);
        b.extend_from_slice(b"0\r\n\r\n");
        b
    }));
    rig.server.enqueue_raw(steps);
    rig.server.enqueue_stream_completion("second turn ran");

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start\r");
    pty.wait_for("Alpha"); // the stream is up, now inside the 900 ms stall
    pty.send(b"queued while busy"); // typed mid-turn: must land in the draft
    pty.wait_for("finishes");
    pty.wait_for("cleanly.");
    settle(); // back at the prompt, the draft already in the input row
    pty.send(b"\r"); // submit the captured draft as the next message
    pty.wait_for("second");
    pty.wait_for("ran");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2, "the mid-turn typing must not fire its own request");
    assert_eq!(last_user(&reqs[0]), "start");
    assert_eq!(
        last_user(&reqs[1]),
        "queued while busy",
        "the mid-turn draft must submit whole as the next message"
    );
    rig.server.assert_clean();
}

/// A confirmation raised by agent code mid-turn (the skills-dir write gate)
/// is answered from the keyboard through the dock's modal: the reader thread
/// owns stdin, so the ask must travel the event channel and back.
#[test]
fn dock_answers_a_mid_turn_confirmation() {
    let rig = rig();
    rig.server.enqueue_stream_toolcalls(
        &[(
            "call_1",
            "write",
            r#"{"path": ".claude/skills/made/SKILL.md", "content": "---\nname: made\ndescription: test\n---\nbody\n"}"#,
        )],
        None,
    );
    rig.server.enqueue_stream_completion("skill written");

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"make a skill\r");
    pty.wait_for("[y/N]"); // the gate's question, rendered by the dock modal
    pty.send(b"y\r");
    pty.wait_for("skill");
    pty.wait_for("written");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let written = rig.work.path().join(".claude/skills/made/SKILL.md");
    assert!(written.is_file(), "the granted write must have executed");
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2, "toolcall turn + result turn");
    rig.server.assert_clean();
}

#[test]
fn dock_double_esc_cancels_an_open_confirmation_and_the_tool_batch() {
    let rig = rig();
    rig.server.enqueue_stream_toolcalls(
        &[(
            "call_1",
            "write",
            r#"{"path": ".claude/skills/nope/SKILL.md", "content": "never"}"#,
        )],
        None,
    );

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"try the write\r");
    pty.wait_for("[y/N]");
    pty.send(b"\x1b\x1b");
    pty.wait_for("[interrupted]");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    assert!(!rig.work.path().join(".claude/skills/nope/SKILL.md").exists());
    assert_eq!(rig.api_requests().len(), 1, "the canceled batch must not continue");
    rig.server.assert_clean();
}

#[test]
fn dock_typeahead_before_an_ask_cannot_confirm_it() {
    let rig = rig();
    let datas = noob_testkit::chat_stream_toolcalls_datas(
        &[(
            "call_1",
            "write",
            r#"{"path": ".claude/skills/nope/SKILL.md", "content": "never"}"#,
        )],
        None,
    );
    let mut tail = sse_frames(&datas);
    tail.extend_from_slice(b"0\r\n\r\n");
    rig.server.enqueue_raw(vec![
        noob_testkit::RawStep::Bytes(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n"
                .to_vec(),
        ),
        noob_testkit::RawStep::SleepMs(500),
        noob_testkit::RawStep::Bytes(tail),
    ]);

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"try the write\r");
    pty.wait_for("Working");
    pty.send(b"y\r"); // submitted before the question exists: queue, never consent
    pty.wait_for("[queued]");
    pty.wait_for("[y/N]"); // still waiting for a fresh answer
    pty.send(b"\x1b\x1b");
    pty.wait_for("[interrupted]");
    pty.wait_for("y"); // canceled queue returned to the editable draft
    pty.send(&[0x15]);
    pty.send(&[0x04]);
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    assert!(!rig.work.path().join(".claude/skills/nope/SKILL.md").exists());
    assert_eq!(rig.api_requests().len(), 1);
    rig.server.assert_clean();
}

/// Review fix (high): in dock mode /compact runs its summarizer request
/// through the render loop, so a keyboard Ctrl-C (a raw byte, not SIGINT)
/// still cancels it. Without the fix the byte is captured by the reader and
/// never sets INTERRUPTED, so the request is uninterruptible for up to 300s.
/// Here the summarizer stalls; Ctrl-C must cancel within ~1 watchdog tick.
#[test]
fn dock_compact_is_cancelable_with_ctrl_c() {
    let rig = rig();
    // One bulky text reply (no tool result, so pruning saves nothing) gives
    // compaction a middle and forces the summarizer LLM call. The END marker
    // lets the test wait for the whole reply to stream so it is back at an idle
    // prompt before /compact; the mock reports tiny usage, so auto-compaction
    // never fires on its own.
    // The reply must exceed the tail budget (NOOB_CTX/4 = 1024 tokens ≈ 4 KiB)
    // on its own so it does not all fit in the retained tail, leaving a middle
    // of >= 2 items for the summarizer.
    rig.server.enqueue_stream_completion(&format!("reply {} END-ONE", "x".repeat(6000)));
    // The summarizer request: 200 headers, then a long silence. The watchdog
    // first-byte budget is 300s, so only INTERRUPTED can end this early.
    rig.server.enqueue_raw(vec![
        noob_testkit::RawStep::Bytes(noob_testkit::sse_headers()),
        noob_testkit::RawStep::SleepMs(8000),
    ]);
    // The summarizer request is the sanctioned compaction prefix break.
    rig.server.expect_prefix_break();

    // NOOB_CTX floors at 4096; a smaller value silently reverts to the default.
    let mut pty = spawn_pty_with(&rig, &[("NOOB_DOCK", "1"), ("NOOB_CTX", "4096")]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start\r");
    pty.wait_for("END-ONE"); // the whole reply has streamed; the turn is ending
    settle(); // back at the idle prompt (a mid-turn Enter is inert pre-queue)
    pty.send(b"/compact\r");
    pty.wait_for("compacting"); // the summarizer request is now in flight, stalled
    pty.send(b"keep this draft\r");
    pty.wait_for("[queued]");
    pty.send(&[0x03]); // Ctrl-C: a raw byte in dock mode, must still cancel
    pty.wait_for("compaction canceled"); // the watchdog tripped via INTERRUPTED
    pty.wait_for("keep this draft"); // canceled auxiliary turns restore queued input
    pty.send(&[0x15]); // clear the restored draft
    pty.send(&[0x04]); // Ctrl-D exits
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2, "the driving turn + the canceled summarizer");
    // The 2nd request is the summarizer (compact.md system prompt), proving
    // the cancel hit the compaction request, not a normal turn.
    let sys = reqs[1]["messages"][0]["content"].as_str().unwrap_or("");
    assert!(sys.contains("summarize an agent session"), "2nd req not the summarizer: {sys}");
    rig.server.assert_clean();
}

#[test]
fn dock_second_ctrl_c_hard_exits_with_terminal_restore() {
    let rig = rig();
    rig.server.enqueue_raw(stalled_stream("Working END-NEVER", 2, 8000, false));

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.send(&[0x03, 0x03]);
    pty.wait_for("\x1b[?2004l");
    let status = pty.finish();

    assert_eq!(status.code(), Some(130), "hard cancel: {status:?};\n{}", pty.seen);
    assert!(
        pty.seen.contains("\x1b[?2004l"),
        "hard exit did not restore terminal modes:\n{}",
        pty.seen
    );
}

/// Review fix (medium): Ctrl-D at a mid-turn y/N confirmation denies (the
/// contract: anything but an explicit yes is No) instead of being swallowed.
/// The same Key::Eof path also unblocks the worker if the reader dies while a
/// modal is open, which would otherwise hang the render loop forever.
#[test]
fn dock_ctrl_d_at_a_confirmation_denies_and_continues() {
    let rig = rig();
    rig.server.enqueue_stream_toolcalls(
        &[(
            "call_1",
            "write",
            r#"{"path": ".claude/skills/nope/SKILL.md", "content": "---\nname: nope\ndescription: t\n---\nb\n"}"#,
        )],
        None,
    );
    // After the denial the tool result is a refusal; the agent continues and
    // the mock answers the follow-up turn.
    rig.server.enqueue_stream_completion("left it alone");

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"make a skill\r");
    pty.wait_for("[y/N]");
    pty.send(&[0x04]); // Ctrl-D at the confirmation: deny
    pty.wait_for("left");
    pty.wait_for("alone");
    settle();
    pty.send(&[0x04]); // Ctrl-D at the empty prompt: exit
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let denied = rig.work.path().join(".claude/skills/nope/SKILL.md");
    assert!(!denied.is_file(), "the write must have been denied, not executed");
    rig.server.assert_clean();
}

/// A run whose stream sends `head_words` deltas, then holds `stall_ms`, then
/// (optionally) sends the rest and closes. `chat_stream_datas` splits on
/// whitespace, so head_words counts role delta + that many words.
fn stalled_stream(text: &str, head_deltas: usize, stall_ms: u64, resume: bool) -> Vec<noob_testkit::RawStep> {
    let datas = noob_testkit::chat_stream_datas(text);
    let mut head = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n".to_vec();
    head.extend_from_slice(&sse_frames(&datas[..head_deltas]));
    let mut steps = vec![
        noob_testkit::RawStep::Bytes(head),
        noob_testkit::RawStep::SleepMs(stall_ms),
    ];
    if resume {
        let mut tail = sse_frames(&datas[head_deltas..]);
        tail.extend_from_slice(b"0\r\n\r\n");
        steps.push(noob_testkit::RawStep::Bytes(tail));
    }
    steps
}

/// M5 (double-ESC cancel): a first ESC during a turn arms a red hint; a second
/// ESC inside the window commits, setting INTERRUPTED so the watchdog trips the
/// in-flight read and the agent finalizes the turn with `[interrupted]`.
#[test]
fn dock_double_esc_cancels_a_running_turn() {
    let rig = rig();
    // Stream one word then stall indefinitely; only a cancel ends it.
    rig.server.enqueue_raw(stalled_stream("Working END-NEVER", 2, 8000, false));

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working"); // the turn is streaming, now stalled
    pty.send(&[0x1b]); // first ESC: arm
    pty.wait_for("press ESC again to cancel"); // the red hint appears
    pty.send(&[0x1b]); // second ESC: commit the cancel
    pty.wait_for("[interrupted]"); // the agent finalized the canceled turn
    settle();
    pty.send(&[0x04]); // Ctrl-D exits
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    assert!(!pty.seen.contains("END-NEVER"), "the stalled tail must never have streamed");
    rig.server.assert_clean();
}

/// M5: a single ESC only arms; if no second ESC lands the turn runs to
/// completion. Here the stream resumes after the arm and the reply finishes
/// normally, with no interrupt.
#[test]
fn dock_single_esc_does_not_cancel() {
    let rig = rig();
    // One word, a short stall, then the rest of the reply and a clean close.
    rig.server.enqueue_raw(stalled_stream("Working through it END-OK", 2, 1500, true));

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.send(&[0x1b]); // a lone ESC: arms only
    pty.wait_for("press ESC again to cancel");
    // No second ESC. The stall lapses, the rest streams, the turn completes.
    pty.wait_for("END-OK");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    assert!(!pty.seen.contains("[interrupted]"), "one ESC must not cancel the turn");
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1, "exactly the one turn ran to completion");
    rig.server.assert_clean();
}

/// M6 (queue): typing a message and pressing Enter WHILE a turn runs queues it
/// (the "N queued" indicator shows) instead of firing it; when the turn ends it
/// dispatches as the next turn, in order.
#[test]
fn dock_queues_a_message_and_dispatches_after_the_turn() {
    let rig = rig();
    // Turn 1 streams a word, holds long enough to type ahead, then finishes.
    rig.server.enqueue_raw(stalled_stream("Working END-ONE", 2, 3000, true));
    // Turn 2 is the dispatched queued message.
    rig.server.enqueue_stream_completion("second done");

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working"); // turn 1 is streaming, now stalled
    pty.send(b"queued msg\r"); // typed + Enter mid-turn: queues, does not fire
    pty.wait_for("[queued]"); // accepted messages are echoed immediately
    pty.wait_for("1 queued"); // the queue indicator confirms it landed
    pty.wait_for("END-ONE"); // turn 1 finishes
    pty.wait_for("second"); // turn 2 = the dispatched queued message's reply
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2, "the driving turn + the dispatched queued message");
    assert_eq!(last_user(&reqs[0]), "go");
    assert_eq!(last_user(&reqs[1]), "queued msg", "the queued message must run as the next turn");
    rig.server.assert_clean();
}

/// M6: interrupting a turn with a queued message drains it back into the editor
/// (an interrupt means "stop, I will steer") rather than firing it. The message
/// reappears at the prompt as an editable draft and no second request is made.
#[test]
fn dock_interrupt_drains_the_queue_to_the_draft() {
    let rig = rig();
    rig.server.enqueue_raw(stalled_stream("Working END-NEVER", 2, 8000, false));

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.send(b"hold me\r"); // queue a message mid-turn
    pty.wait_for("1 queued");
    pty.send(b"\x1b\x1b"); // both taps in one kernel read must still cancel
    pty.wait_for("[interrupted]");
    // The queued message is restored to the editor, not dispatched.
    pty.wait_for("hold me");
    pty.send(&[0x15]); // Ctrl-U clears the restored draft
    pty.send(&[0x04]); // Ctrl-D on the now-empty line exits
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1, "the queued message must NOT have been dispatched after the cancel");
    rig.server.assert_clean();
}

/// Write a SKILL.md (name + description + body) at `dir`.
fn write_skill_md(dir: &std::path::Path, name: &str, desc: &str, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {desc}\n---\n{body}\n"),
    )
    .unwrap();
}

/// Every message's content across a recorded request, joined, for substring
/// assertions on what the model was actually sent.
fn all_content(req: &Value) -> String {
    req["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap_or("").to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// M8 (skills on the fly): a session that started with NO skills installs one
/// with `/skills add` and immediately uses it. The `skill` tool must be
/// registered mid-session (absent at bootstrap), and the skill body must load.
#[test]
fn skills_add_registers_the_tool_and_the_skill_loads() {
    let rig = rig();
    // A source skill outside every discovery path, so it is not present until
    // it is installed.
    write_skill_md(
        &rig.work.path().join("src-demo"),
        "demo",
        "demo skill for the test",
        "STEP-ONE: do the demo thing.",
    );
    // The "use demo" turn: the model loads the skill, then answers.
    rig.server.enqueue_stream_toolcalls(&[("c1", "skill", r#"{"name":"demo"}"#)], None);
    rig.server.enqueue_stream_completion("used the demo skill");

    let mut pty = spawn_pty(&rig); // classic REPL: per-prompt RAW_READY sync
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"/skills add src-demo\r");
    pty.wait_for("installed skill demo");
    pty.wait_for(RAW_READY); // back at the prompt, skill now registered
    pty.send(b"use demo\r");
    pty.wait_for("used the demo skill");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]);
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2, "the tool-call round and the completion round");
    // The skill tool was registered mid-session: the first request already
    // carries it, though the session booted with no skills.
    let tools = reqs[0]["tools"].as_array().expect("tools array");
    assert!(
        tools.iter().any(|t| t["function"]["name"] == "skill"),
        "the skill tool must be registered after /skills add"
    );
    // The in-band announcement reached the model, and the skill body loaded.
    assert!(all_content(&reqs[0]).contains("[skills updated]"), "missing the in-band note");
    assert!(all_content(&reqs[1]).contains("STEP-ONE"), "the skill body did not load");
    rig.server.assert_clean();
}

#[test]
fn dock_canceled_skill_clone_restores_queued_input() {
    use std::os::unix::fs::PermissionsExt;

    let rig = rig();
    let bin = rig.work.path().join("fake-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let git = bin.join("git");
    std::fs::write(&git, "#!/bin/sh\nexec /bin/sleep 30\n").unwrap();
    std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = bin.to_string_lossy().into_owned();
    let envs = [("NOOB_DOCK", "1"), ("PATH", path.as_str())];

    let mut pty = spawn_pty_with(&rig, &envs);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"/skills add https://example.invalid/demo.git\r");
    pty.wait_for("Working");
    pty.send(b"keep skill draft\r");
    pty.wait_for("[queued]");
    pty.send(&[0x03]);
    pty.wait_for("skill installation canceled by user");
    pty.wait_for("keep skill draft");
    pty.send(&[0x15]);
    pty.send(&[0x04]);
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    assert!(rig.api_requests().is_empty(), "the restored draft must not auto-run");
}

/// M8: removing a skill mid-session announces it and the `skill` tool then
/// rejects loading it (the staleness backstop: the frozen prompt-head index
/// still lists it, but the in-band note and the tool's own check correct that).
#[test]
fn skills_remove_announces_and_the_tool_rejects_the_gone_skill() {
    let rig = rig();
    // Boot WITH the skill installed (a discovery path), so the tool exists.
    write_skill_md(
        &rig.work.path().join(".noob/skills/demo"),
        "demo",
        "demo skill for the test",
        "STEP-ONE: do the demo thing.",
    );
    // After removal the model still tries to load it (the head is stale); the
    // tool must reject, and the model then answers.
    rig.server.enqueue_stream_toolcalls(&[("c1", "skill", r#"{"name":"demo"}"#)], None);
    rig.server.enqueue_stream_completion("the skill is gone");

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"/skills remove demo\r");
    pty.wait_for("removed demo");
    pty.wait_for(RAW_READY);
    pty.send(b"use demo\r");
    pty.wait_for("the skill is gone");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]);
    pty.wait_for("resume with --session");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2);
    // The removal was announced in-band so the model's working set is corrected.
    assert!(
        all_content(&reqs[0]).contains("no longer available"),
        "the removal must be announced to the model"
    );
    // The tool structurally rejected the gone skill (the hard backstop).
    assert!(
        all_content(&reqs[1]).contains("unknown skill"),
        "the skill tool must reject a removed skill"
    );
    assert!(!rig.work.path().join(".noob/skills/demo").exists(), "the skill dir must be gone");
    rig.server.assert_clean();
}
