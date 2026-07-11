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
    let child = noob(rig.config.path(), rig.work.path())
        .env("COLORTERM", "truecolor")
        .env_remove("NO_COLOR")
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

/// The REPL persists its session and `--session <id>` resumes it: a second run
/// against the same id byte-extends the first run's transcript.
#[test]
fn repl_session_resume_extends_the_transcript() {
    let rig = rig();
    rig.server.enqueue_stream_completion("noted");
    let out1 = run_repl(&rig, &["--session", "reptest"], b"remember alpha\n/quit\n");
    assert!(out1.status.success(), "run 1 failed: {out1:?}");

    rig.server.enqueue_stream_completion("recalled");
    let out2 = run_repl(&rig, &["--session", "reptest"], b"what did i say\n/quit\n");
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
    assert!(!stdout.contains('╭'), "the box frame leaked into a piped repl: {stdout}");
    assert!(
        !stdout.contains("\x1b[?2004h") && !stdout.contains("\x1b[?2004l"),
        "bracketed paste toggled on a piped repl: {stdout}"
    );
    assert!(!stdout.contains('▪'), "the thinking scanner leaked into a piped repl: {stdout}");
    rig.server.assert_clean();
}
