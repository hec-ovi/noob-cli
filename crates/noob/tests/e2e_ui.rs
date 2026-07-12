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

// The test-only screen emulator (tests/vt.rs), included as a module so the
// dock repro can render noob's captured bytes into a fixed rows x cols screen.
#[path = "vt.rs"]
mod vt;

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
    spawn_pty_sized(rig, envs, None, &[])
}

/// Spawn with a specific terminal size. `size = Some((rows, cols))` sets the
/// pty winsize so scrolling behavior on a small screen is reproducible; noob
/// reads only `cols` (via TIOCGWINSZ) and is otherwise row-agnostic, so the
/// row count matters only to the emulator that replays the captured bytes.
fn spawn_pty_sized(
    rig: &Rig,
    envs: &[(&str, &str)],
    size: Option<(u16, u16)>,
    args: &[&str],
) -> Pty {
    let (master, slave) = unsafe {
        let mut m: libc::c_int = 0;
        let mut s: libc::c_int = 0;
        let ws = size.map(|(rows, cols)| libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        });
        let ws_ptr = ws
            .as_ref()
            .map(|w| w as *const libc::winsize)
            .unwrap_or(std::ptr::null());
        assert_eq!(
            libc::openpty(&mut m, &mut s, std::ptr::null_mut(), std::ptr::null(), ws_ptr),
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
    cmd.args(args);
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
    Pty {
        master,
        child: Some(child),
        done,
        watchdog: Some(watchdog),
        seen: String::new(),
        raw: Vec::new(),
        cursor: 0,
    }
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
    /// The exact bytes read from the master, undecoded. `seen` is a lossy
    /// UTF-8 view for substring waits; the screen emulator needs the real
    /// bytes (a box-drawing glyph split across a read boundary would otherwise
    /// become a replacement char).
    raw: Vec<u8>,
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
                Ok(n) => {
                    self.raw.extend_from_slice(&buf[..n]);
                    self.seen.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
                Err(e) => panic!("pty read error: {e}; saw:\n{}", self.seen),
            }
        }
    }

    /// Pull whatever the child emits over `budget`, into `raw`/`seen`, without
    /// blocking on a marker. Used to capture the trailing dock repaints (the
    /// frame is redrawn after the last output, and the liveness comet repaints
    /// it on a 120 ms cadence) before snapshotting the screen.
    fn drain(&mut self, budget: std::time::Duration) {
        use std::os::fd::AsRawFd;
        let fd = self.master.as_raw_fd();
        let deadline = std::time::Instant::now() + budget;
        let mut buf = [0u8; 4096];
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let ms = (remaining.as_millis() as i32).min(40);
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            let ready = unsafe { libc::poll(&mut pfd, 1, ms) };
            if ready <= 0 {
                continue; // timeout or EINTR: keep polling until the budget ends
            }
            match self.master.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    self.raw.extend_from_slice(&buf[..n]);
                    self.seen.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
                Err(_) => break,
            }
        }
    }

    /// Replay everything captured so far into a fresh rows x cols screen.
    fn screen(&self, rows: u16, cols: u16) -> vt::Vt {
        let mut vt = vt::Vt::new(rows as usize, cols as usize);
        vt.feed(&self.raw);
        vt
    }

    /// Resize the pty (TIOCSWINSZ updates the winsize the child reads) and raise
    /// SIGWINCH in the child. The child here is not a controlling-tty session
    /// leader, so TIOCSWINSZ alone does not auto-deliver the signal the way a
    /// real terminal does; sending it explicitly exercises noob's reflow path
    /// against the freshly updated width. Used to prove the dock reflows without
    /// a keystroke.
    fn resize(&mut self, rows: u16, cols: u16) {
        use std::os::fd::AsRawFd;
        let ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
        unsafe {
            libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &ws);
            if let Some(child) = &self.child {
                libc::kill(child.id() as i32, libc::SIGWINCH);
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
    pty.wait_for("resume with"); // the exit hint tells you how to reopen
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
    pty.wait_for("resume with");
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

/// Write a session transcript file so a resume can replay it. `items` are the
/// per-item JSON objects (the user/assistant/tool shapes the session log uses);
/// each is wrapped as one `{"t":"item","item":...}` line under a meta header.
fn write_session(config: &std::path::Path, id: &str, items: &[Value]) {
    let dir = config.join("sessions");
    std::fs::create_dir_all(&dir).unwrap();
    let mut out = format!("{}\n", serde_json::json!({"t":"meta","v":1,"id":id,"created_ms":0}));
    for item in items {
        out.push_str(&format!("{}\n", serde_json::json!({"t":"item","item":item})));
    }
    std::fs::write(dir.join(format!("{id}.jsonl")), out).unwrap();
}

/// Drop every SGR escape so an assertion can key on the plain text a human sees.
fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Resuming a saved session redraws the prior conversation on screen: the
/// earlier human line and the model's reply both appear (as plain, strip-ANSI
/// tokens) before the first new prompt, while a synthetic `[skills updated]`
/// item is filtered out. Display-only: no model request is made on resume.
#[test]
fn resume_redisplays_the_prior_conversation() {
    let rig = rig();
    write_session(
        rig.config.path(),
        "replayme",
        &[
            serde_json::json!({"role": "user", "text": "PRIORUSERLINE remember this"}),
            serde_json::json!({"role": "assistant", "text": "PRIORASSISTANTLINE understood.", "calls": [], "raw": []}),
            // Synthetic plumbing that must NOT be redisplayed.
            serde_json::json!({"role": "user", "text": "[skills updated] now available: HIDDENSKILL: nope."}),
            serde_json::json!({"role": "user", "text": "SECONDUSERLINE and this"}),
            serde_json::json!({"role": "assistant", "text": "SECONDASSISTANTLINE noted.", "calls": [], "raw": []}),
        ],
    );

    // Classic per-prompt editor so the replay lands before a plain RAW_READY.
    let mut pty = spawn_pty_sized(&rig, &[("NOOB_DOCK", "0")], None, &["--resume", "replayme"]);
    pty.wait_for("type a task");
    // The replay renders before the first prompt; wait for the last replayed
    // assistant line to be sure the whole transcript was drawn.
    pty.wait_for("SECONDASSISTANTLINE");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]); // Ctrl-D at the fresh prompt exits
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let plain = strip_ansi(&pty.seen);
    assert!(plain.contains("PRIORUSERLINE"), "prior user line not replayed:\n{plain}");
    assert!(plain.contains("PRIORASSISTANTLINE"), "prior assistant line not replayed:\n{plain}");
    assert!(plain.contains("SECONDUSERLINE"), "later user line not replayed:\n{plain}");
    assert!(
        !plain.contains("HIDDENSKILL"),
        "a synthetic [skills updated] item leaked into the replay:\n{plain}"
    );
    // Replay is display-only: resuming fires no model request.
    assert!(rig.api_requests().is_empty(), "replay must not make a model request");
    rig.server.assert_clean();
}

/// `--resume <bogus>` with no matching saved session prints a not-found notice
/// and still reaches a working prompt (it starts a fresh session).
#[test]
fn resume_of_a_missing_session_notes_and_starts_fresh() {
    let rig = rig();
    let mut pty = spawn_pty_sized(&rig, &[("NOOB_DOCK", "0")], None, &["--resume", "nosuchid"]);
    pty.wait_for("type a task");
    pty.wait_for("no saved session"); // the not-found notice
    pty.wait_for(RAW_READY); // still reaches a working prompt
    pty.send(&[0x04]); // Ctrl-D exits
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    assert!(pty.seen.contains("no saved session"), "the not-found notice never printed:\n{}", pty.seen);
    rig.server.assert_clean();
}

/// `--resume` is a canonical alias for `--session`/`--restore`: a session
/// created with `--session` resumes and extends under `--resume`.
#[test]
fn resume_alias_extends_a_session_created_with_session() {
    let rig = rig();
    rig.server.enqueue_stream_completion("noted");
    let out1 = run_repl(&rig, &["--session", "aliastest"], b"remember gamma\n/quit\n");
    assert!(out1.status.success(), "run 1 failed: {out1:?}");

    rig.server.enqueue_stream_completion("recalled");
    let out2 = run_repl(&rig, &["--resume", "aliastest"], b"what did i say\n/quit\n");
    assert!(out2.status.success(), "run 2 failed: {out2:?}");

    // Run 2 (under --resume) replayed run 1's user message into the request:
    // the alias resumed the same transcript --session created.
    let reqs = rig.api_requests();
    let last = reqs.last().unwrap();
    let msgs = last["messages"].as_array().unwrap();
    assert!(
        msgs.iter().any(|m| m["role"] == "user" && m["content"] == "remember gamma"),
        "--resume did not resume the --session transcript: {msgs:?}"
    );
    rig.server.assert_clean();
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
    pty.wait_for("resume with");
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
    pty.wait_for("resume with");
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
    pty.wait_for("resume with");
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
    pty.wait_for("resume with");
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
    pty.wait_for("resume with");
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
    pty.wait_for("resume with");
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
    pty.wait_for("resume with");
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
    pty.wait_for("resume with");
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
    pty.wait_for("resume with");
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
    pty.wait_for("resume with");
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
    pty.wait_for("resume with");
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
    pty.wait_for("resume with");
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
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1, "the queued message must NOT have been dispatched after the cancel");
    rig.server.assert_clean();
}

/// The multi-agent fan-out panel on the themed dock surface: a batch of three
/// distinct `task` calls renders one live checklist of agents (header with the
/// concurrency cap, one row per agent, running then done) with a one-line
/// result digest per finished agent, instead of three identical truncated
/// lines. The three prompts share a prefix and differ only in their tails, so
/// the panel must keep the rows visibly distinct.
#[test]
fn dock_renders_a_multi_agent_fanout_panel() {
    let rig = rig();
    rig.server.allow_interleaving();
    rig.server.enqueue_stream_toolcalls(
        &[
            ("f1", "subagent", r#"{"prompt":"Read the article at http://x/ALPHATAIL"}"#),
            ("f2", "subagent", r#"{"prompt":"Read the article at http://x/BETATAIL"}"#),
            ("f3", "subagent", r#"{"prompt":"Read the article at http://x/GAMMATAIL"}"#),
        ],
        None,
    );
    // Distinct child results; the panel digest is each child's first line. The
    // parent's collect turn (below) is the 4th queued completion.
    rig.server.enqueue_stream_completion("ALPHA-RESULT one");
    rig.server.enqueue_stream_completion("BETA-RESULT two");
    rig.server.enqueue_stream_completion("GAMMA-RESULT three");
    rig.server.enqueue_stream_completion("COLLECTED-END");

    // Force the cap so the header text is deterministic and all three overlap.
    let mut pty = spawn_pty_with(&rig, &[("NOOB_TASK_CONCURRENCY", "4")]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"fan out\r");
    pty.wait_for("Working");
    pty.wait_for("agents ("); // the panel opened as the agents started
    pty.wait_for("COLLECTED-END"); // the whole turn (fan-out + collect) finished
    settle();
    pty.drain(std::time::Duration::from_millis(300));
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let seen = &pty.seen;
    // Three distinct rows: the shared prefix did not collapse them.
    for tail in ["ALPHATAIL", "BETATAIL", "GAMMATAIL"] {
        assert!(seen.contains(tail), "distinct row for {tail} missing:\n{seen}");
    }
    // The header surfaces the concurrency cap and the fan-out reached all done.
    assert!(seen.contains("up to 4 at once"), "concurrency cap not in the header:\n{seen}");
    assert!(seen.contains("3/3 done"), "the panel never reached all-done:\n{seen}");
    // A finished agent carries the done glyph and a one-line result digest (a
    // child's result text can only reach the pty through the panel digest).
    assert!(seen.contains("[x] agent"), "no agent row reached the done glyph:\n{seen}");
    assert!(
        ["ALPHA-RESULT", "BETA-RESULT", "GAMMA-RESULT"].iter().any(|d| seen.contains(d)),
        "no result digest reached the panel:\n{seen}"
    );
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
    pty.wait_for("resume with");
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
    pty.wait_for("resume with");
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
    pty.wait_for("resume with");
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

// ---------------------------------------------------------------------------
// Screen-level dock reproduction. The byte-only PTY tests above cannot see a
// scroll-at-bottom cursor-math desync: they assert on the raw output bytes and
// have no screen model. This one replays noob's exact captured bytes into a
// small rows x cols emulator (tests/vt.rs) and inspects the dock the way a
// human would, both mid-turn (frame live) and at idle (frame torn down).
// ---------------------------------------------------------------------------

/// The U+203A input marker the dock's input row always leads with.
const MARKER: &str = "\u{203a}";

/// Find the dock's three rows in a rendered screen: the "Working" top rule, the
/// "Esc Esc to cancel" bottom rule, and the input row between them. Returns the
/// row indices if the top and bottom rules are both present.
fn dock_rows(screen: &[String]) -> Option<(usize, usize)> {
    let top = screen.iter().rposition(|r| r.contains("Working"))?;
    let bottom = screen.iter().rposition(|r| r.contains("Esc Esc to cancel"))?;
    Some((top, bottom))
}

/// The live input row in a rendered screen: the one leading with the U+203A
/// marker. The greeting banner carries the command names too but never the
/// marker, so this isolates the editable row from the banner.
fn input_row(screen: &[String]) -> Option<&String> {
    screen.iter().find(|r| r.contains(MARKER))
}

#[test]
fn dock_input_row_survives_a_scrolling_stream_at_the_screen_level() {
    // A small screen so the stream scrolls it several times over, and a width
    // wide enough that no single short line wraps.
    const ROWS: u16 = 12;
    const COLS: u16 = 64;

    let rig = rig();
    // Twenty-four short, unique lines (one per stream delta, since
    // `chat_stream_datas` cuts on whitespace and each line ends in `\n`), then a
    // final ZZEND marker. Stream the first fourteen, stall long enough to snap a
    // mid-turn screen, then stream the rest and finish.
    let mut text = String::new();
    for i in 1..=24 {
        text.push_str(&format!("row-{i:02}-xyz\n"));
    }
    text.push_str("ZZEND");
    // datas: [role, row-01..row-24, ZZEND, finish, usage, DONE]. Head = role +
    // rows 1..14 => 15 deltas.
    rig.server.enqueue_raw(stalled_stream(&text, 15, 1200, true));

    let mut pty = spawn_pty_sized(&rig, &[], Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working"); // the dock is up and the stream is flowing
    pty.wait_for("row-14-xyz"); // the last line before the stall has landed

    // MID-TURN: drain the trailing frame repaints during the stall, then snap.
    pty.drain(std::time::Duration::from_millis(500));
    let mid = pty.screen(ROWS, COLS);
    let mid_rows = mid.render();
    println!("\n{}", mid.dump("MID-TURN (frame live, mid-stall)"));

    // Let the stall lapse, the rest stream, and the turn finish.
    pty.wait_for("ZZEND");
    settle();
    pty.drain(std::time::Duration::from_millis(300));
    let end = pty.screen(ROWS, COLS);
    println!("\n{}", end.dump("END-OF-TURN (idle prompt)"));

    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    rig.server.assert_clean();

    // ---- MID-TURN assertions: the dock must be intact and live. ----
    let (top, bottom) = dock_rows(&mid_rows)
        .unwrap_or_else(|| panic!("mid-turn dock rules missing entirely:\n{}", mid.dump("mid")));
    assert_eq!(
        bottom,
        top + 2,
        "the dock is not three contiguous rows (top {top}, bottom {bottom}):\n{}",
        mid.dump("mid")
    );
    let input = &mid_rows[top + 1];
    assert!(
        input.contains(MARKER),
        "MID-TURN the input row lost its `{MARKER}` marker (input disappeared during \
         activity); input row = {input:?}\n{}",
        mid.dump("mid")
    );
    // The input row must be the dock's own row, not a line of streamed output
    // that scrolled into the marker's position.
    assert!(
        !input.contains("row-") && !input.contains("ZZEND"),
        "MID-TURN streamed output bled into the input row: {input:?}\n{}",
        mid.dump("mid")
    );

    // ---- END-OF-TURN assertions: the live turn frame (Working/cancel) is gone,
    //      replaced by the persistent idle input box so the input never collapses
    //      to a lone marker between turns. ----
    let end_rows = end.render();
    assert!(
        dock_rows(&end_rows).is_none(),
        "END-OF-TURN the live turn frame (Working/cancel) was not torn down:\n{}",
        end.dump("end")
    );
    let marker = end_rows
        .iter()
        .rposition(|r| r.contains(MARKER))
        .unwrap_or_else(|| panic!("END-OF-TURN no idle input box:\n{}", end.dump("end")));
    // The empty idle box reads as a live input (dim hint), never a bare marker,
    // and no streamed output bled into the input row.
    assert!(
        end_rows[marker].contains("type a message"),
        "END-OF-TURN the idle input lost its hint (collapsed to a bare marker): {:?}\n{}",
        end_rows[marker],
        end.dump("end")
    );
    assert!(
        !end_rows[marker].contains("row-") && !end_rows[marker].contains("ZZEND"),
        "END-OF-TURN streamed output bled into the idle input row: {:?}\n{}",
        end_rows[marker],
        end.dump("end")
    );
    // The box is framed: a rule directly below the input, and nothing past it.
    assert!(
        end_rows.get(marker + 1).is_some_and(|r| r.contains("──")),
        "END-OF-TURN the idle box has no bottom rule under the input:\n{}",
        end.dump("end")
    );
    for (i, r) in end_rows.iter().enumerate().skip(marker + 2) {
        assert!(
            r.is_empty(),
            "END-OF-TURN row {i} below the idle box is not blank: {r:?}\n{}",
            end.dump("end")
        );
    }
}

/// The input row is a visible affordance during a turn: while the draft is
/// empty the dock shows a dim "type to queue a message" placeholder (so the row
/// never reads as absent, the reported "input disappears during activity"), and
/// the first keystroke replaces it with the draft rather than sitting beside it.
#[test]
fn dock_input_row_shows_a_placeholder_when_empty_and_replaces_it_on_typing() {
    const ROWS: u16 = 12;
    const COLS: u16 = 64;

    let rig = rig();
    // Newline-terminated lines so each flushes mid-stream (the markdown renderer
    // holds an un-terminated line until turn end). Stream role + two lines, then
    // stall long enough to snap twice, then finish.
    let text = "aa-line\nbb-line\ncc-line\ndd-line\nZZEND";
    rig.server.enqueue_raw(stalled_stream(text, 3, 4000, true));
    rig.server.enqueue_stream_completion("second turn ran");

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.wait_for("bb-line"); // last head line flushed; inside the 4000 ms stall

    // EMPTY DRAFT: the placeholder is the visible input affordance.
    pty.drain(std::time::Duration::from_millis(500));
    let empty = pty.screen(ROWS, COLS);
    let empty_rows = empty.render();
    let (top, _bottom) = dock_rows(&empty_rows)
        .unwrap_or_else(|| panic!("dock rules missing:\n{}", empty.dump("empty")));
    assert!(
        empty_rows[top + 1].contains("type to queue a message"),
        "the empty input row shows no placeholder affordance: {:?}\n{}",
        empty_rows[top + 1],
        empty.dump("empty")
    );

    // TYPED: the placeholder is replaced by the draft, never shown alongside it.
    pty.send(b"my note");
    pty.drain(std::time::Duration::from_millis(400));
    let typed = pty.screen(ROWS, COLS);
    let typed_rows = typed.render();
    let (ttop, _) = dock_rows(&typed_rows)
        .unwrap_or_else(|| panic!("dock rules missing after typing:\n{}", typed.dump("typed")));
    let tinput = &typed_rows[ttop + 1];
    assert!(
        tinput.contains("my note") && !tinput.contains("type to queue a message"),
        "typing did not replace the placeholder: {tinput:?}\n{}",
        typed.dump("typed")
    );

    // The typed draft carries to the next prompt and submits whole (proving it
    // is a real draft, not the display-only placeholder).
    pty.wait_for("ZZEND");
    settle();
    pty.send(b"\r");
    pty.wait_for("second turn ran");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let reqs = rig.api_requests();
    assert_eq!(last_user(reqs.last().unwrap()), "my note");
    rig.server.assert_clean();
}

/// The plan is a single pinned region that updates in place, never a fresh block
/// stacked on every `todo` call (the reported console redundancy). Two `todo`
/// calls advance the same plan; mid-turn the live screen shows the LATEST state
/// exactly once, the superseded state is gone (overwritten in place, not scrolled
/// into history), and the plan sits inside the dock between the "Working" status
/// and the input row. Asserted on the screen, not the raw byte log: the old
/// state's bytes were emitted and then erased, so only a screen model can prove
/// it is no longer visible.
#[test]
fn dock_pins_the_plan_as_one_in_place_region() {
    const ROWS: u16 = 14;
    const COLS: u16 = 64;

    let rig = rig();
    let a = r#"{"todos":[{"content":"alpha","status":"pending"},{"content":"beta","status":"pending"}]}"#;
    let b = r#"{"todos":[{"content":"alpha","status":"completed"},{"content":"beta","status":"pending"}]}"#;
    rig.server.enqueue_stream_toolcalls(&[("p1", "todo", a)], None);
    rig.server.enqueue_stream_toolcalls(&[("p2", "todo", b)], None);
    // A stalled final turn so the screen can be snapped while the frame is live
    // (turn end tears the frame, regions and all, down).
    rig.server.enqueue_raw(stalled_stream("all planned ZZEND", 1, 3000, true));

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"plan it\r");
    pty.wait_for("Working");
    pty.wait_for("plan (1/2 done):"); // the second todo call pinned the new state

    pty.drain(std::time::Duration::from_millis(500));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();
    println!("\n{}", screen.dump("PLAN PINNED (mid-turn)"));

    // Release the stall and finish so the child exits cleanly.
    pty.wait_for("ZZEND");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    rig.server.assert_clean();

    // Exactly one plan block on the live screen: the pinned region, latest state.
    // The scrolled tool summary is "plan: N/2 done" (no paren), so "plan (" keys
    // on the block header alone.
    let headers = rows.iter().filter(|r| r.contains("plan (")).count();
    assert_eq!(headers, 1, "the plan must be one pinned block, not stacked:\n{}", screen.dump("plan"));
    let joined = rows.join("\n");
    assert!(joined.contains("[x] alpha"), "the advanced item is not shown:\n{}", screen.dump("plan"));
    assert!(joined.contains("[ ] beta"), "the pending item is not shown:\n{}", screen.dump("plan"));
    // The superseded state was overwritten in place, not left in the transcript.
    assert!(
        !joined.contains("[ ] alpha"),
        "the old plan state was stacked, not replaced in place:\n{}",
        screen.dump("plan")
    );
    assert!(
        !joined.contains("plan (0/2 done):"),
        "the old plan header was stacked, not replaced in place:\n{}",
        screen.dump("plan")
    );

    // The region sits inside the dock: below "Working", above the input row.
    let working = rows.iter().rposition(|r| r.contains("Working")).expect("Working status row");
    let header = rows.iter().position(|r| r.contains("plan (1/2 done):")).expect("plan header row");
    let input = rows.iter().rposition(|r| r.contains(MARKER)).expect("input row");
    assert!(
        working < header && header < input,
        "plan not pinned between status and input (working {working}, header {header}, input {input}):\n{}",
        screen.dump("plan")
    );
}

/// A pinned region row longer than the terminal is clamped to exactly one
/// physical row ending in an ellipsis. The in-place refresh (comet cadence,
/// keystrokes) must not erase that trailing glyph: a full-width row parks the
/// terminal's deferred-wrap latch in the last column, so a clear-to-end there
/// would blank the ellipsis. Snap the screen after several refresh ticks and
/// confirm the ellipsis is still on the row.
#[test]
fn dock_region_row_keeps_its_ellipsis_across_an_in_place_refresh() {
    const ROWS: u16 = 12;
    const COLS: u16 = 40;

    let rig = rig();
    let long = "this is a very long plan item that certainly exceeds the terminal width";
    let todo = format!(r#"{{"todos":[{{"content":"{long}","status":"completed"}}]}}"#);
    rig.server.enqueue_stream_toolcalls(&[("p1", "todo", todo.as_str())], None);
    rig.server.enqueue_raw(stalled_stream("done ZZEND", 1, 3000, true));

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.wait_for("plan (1/1 done):");
    // Span several 120ms comet refreshes: the in-place repaint is where a
    // full-width region row could lose its trailing ellipsis.
    pty.drain(std::time::Duration::from_millis(500));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();
    println!("\n{}", screen.dump("FULL-WIDTH REGION ROW"));

    pty.wait_for("ZZEND");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    rig.server.assert_clean();

    let item = rows.iter().find(|r| r.contains("this is a very")).expect("clamped plan item row");
    assert!(
        item.ends_with('…'),
        "the clamped region row lost its ellipsis on an in-place refresh: {item:?}\n{}",
        screen.dump("row")
    );
}

/// The pinned regions are bounded by the screen height, so a long plan can never
/// grow the live frame past the terminal (where the relative cursor moves would
/// clamp at the top edge and desync). On a short screen the overflow collapses
/// into one summary row and the frame stays intact and in order.
#[test]
fn dock_caps_pinned_regions_to_the_screen_height() {
    const ROWS: u16 = 10;
    const COLS: u16 = 50;

    let rig = rig();
    // Twelve items plus the header would be 13 region rows; the cap on a 10-row
    // screen is term_height - 4 = 6, so most collapse into a "+N more" row.
    let mut items = String::new();
    for i in 1..=12 {
        if i > 1 {
            items.push(',');
        }
        items.push_str(&format!(r#"{{"content":"item number {i:02}","status":"pending"}}"#));
    }
    let todo = format!(r#"{{"todos":[{items}]}}"#);
    rig.server.enqueue_stream_toolcalls(&[("p1", "todo", todo.as_str())], None);
    rig.server.enqueue_raw(stalled_stream("done ZZEND", 1, 3000, true));

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.wait_for("plan (0/12 done):");
    pty.drain(std::time::Duration::from_millis(500));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();
    println!("\n{}", screen.dump("CAPPED REGION (short screen)"));

    pty.wait_for("ZZEND");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    rig.server.assert_clean();

    // The frame is intact and on-screen: status, input row, and bottom rule all
    // present and in order within the ten rows (no top-edge clamp corruption).
    let working = rows.iter().rposition(|r| r.contains("Working")).expect("Working row on screen");
    let input = rows.iter().rposition(|r| r.contains(MARKER)).expect("input row on screen");
    let bottom =
        rows.iter().rposition(|r| r.contains("Esc Esc to cancel")).expect("bottom rule on screen");
    assert!(
        working < input && input < bottom,
        "frame rows out of order (working {working}, input {input}, bottom {bottom}):\n{}",
        screen.dump("cap")
    );
    // The overflow collapsed into a single summary row rather than overrunning.
    assert!(
        rows.iter().any(|r| r.contains("… +") && r.contains("more")),
        "no overflow summary row; the region was not capped to the screen:\n{}",
        screen.dump("cap")
    );
    let header = rows.iter().position(|r| r.contains("plan (0/12 done):")).expect("plan header");
    assert!(
        working < header && header < input,
        "plan not pinned inside the frame:\n{}",
        screen.dump("cap")
    );
}

/// The idle input is a persistent framed box from the very first prompt: a plain
/// rule above and below a `› type a message` line, present before any keystroke,
/// so the input never reads as a lone marker (the reported "input disappears when
/// inference finishes"). This is the dock default; the classic NOOB_DOCK=0 editor
/// keeps its bare-marker-expands behavior.
#[test]
fn dock_idle_input_is_a_persistent_framed_box() {
    const ROWS: u16 = 10;
    const COLS: u16 = 50;

    let rig = rig();
    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    // No keystroke: the framed idle box must already be on screen.
    pty.drain(std::time::Duration::from_millis(300));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();
    println!("\n{}", screen.dump("FRESH IDLE BOX (no keystroke)"));

    pty.send(&[0x04]); // Ctrl-D exits from the empty box
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    rig.server.assert_clean();

    let marker = rows
        .iter()
        .rposition(|r| r.contains(MARKER))
        .unwrap_or_else(|| panic!("no idle input box before typing:\n{}", screen.dump("idle")));
    assert!(
        rows[marker].contains("type a message"),
        "the fresh idle box is missing its hint (bare marker): {:?}\n{}",
        rows[marker],
        screen.dump("idle")
    );
    assert!(
        marker >= 1 && rows[marker - 1].contains("──"),
        "no top rule above the idle input:\n{}",
        screen.dump("idle")
    );
    assert!(
        rows.get(marker + 1).is_some_and(|r| r.contains("──")),
        "no bottom rule below the idle input:\n{}",
        screen.dump("idle")
    );
}

/// A terminal resize (SIGWINCH) reflows the idle box to the new width WITHOUT a
/// keystroke. The dock reads the width once and then blocks on input, so without
/// the signal the box would keep its startup width (the "first appearance width
/// is wrong" report, seen when a Docker pty is sized a beat after noob starts)
/// until the user typed. The box rules span the full terminal width, so their
/// dash count tracks the resize.
#[test]
fn dock_idle_box_reflows_on_resize_without_a_keystroke() {
    const ROWS: u16 = 12;

    let rig = rig();
    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, 50)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.drain(std::time::Duration::from_millis(300));

    let rule_dashes = |pty: &Pty, cols: u16| -> usize {
        let rows = pty.screen(ROWS, cols).render();
        let marker = rows.iter().rposition(|r| r.contains(MARKER)).expect("idle box marker");
        // The rule directly under the input row is the box bottom.
        rows.get(marker + 1).map(|r| r.chars().filter(|&c| c == '─').count()).unwrap_or(0)
    };

    let narrow = rule_dashes(&pty, 50);
    assert_eq!(narrow, 50, "the initial idle box rule should span the 50-col terminal");

    // Resize wider with NO keystroke: SIGWINCH must reflow the box.
    pty.resize(ROWS, 100);
    pty.drain(std::time::Duration::from_millis(500));
    let wide = rule_dashes(&pty, 100);
    assert_eq!(wide, 100, "the idle box did not reflow to 100 cols on resize (SIGWINCH ignored)");

    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    rig.server.assert_clean();
}

/// The same guarantee for logical lines LONGER than the terminal width, which
/// the terminal wraps into several physical rows. noob emits the whole line and
/// relies on the terminal to wrap and scroll; its dock erase/redraw only knows
/// three frame rows, so this is where a row-agnostic desync would surface.
#[test]
fn dock_input_row_survives_wrapping_lines_at_the_screen_level() {
    const ROWS: u16 = 12;
    const COLS: u16 = 64;

    let rig = rig();
    // Twelve lines of ~150 chars each: every one wraps to three physical rows at
    // width 64. Interior spaces mean each wraps across many word deltas.
    let mut text = String::new();
    for i in 1..=12 {
        text.push_str(&format!("para-{i:02} ").repeat(17));
        text.push('\n');
    }
    text.push_str("ZZEND");
    let datas = noob_testkit::chat_stream_datas(&text);
    rig.server.enqueue_raw(stalled_stream(&text, datas.len() / 2, 1200, true));

    let mut pty = spawn_pty_sized(&rig, &[], Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.wait_for("para-05"); // several wrapped lines have scrolled past
    pty.drain(std::time::Duration::from_millis(500));
    let mid = pty.screen(ROWS, COLS);
    let mid_rows = mid.render();
    println!("\n{}", mid.dump("WRAP MID-TURN (frame live, mid-stall)"));

    pty.wait_for("ZZEND");
    settle();
    pty.drain(std::time::Duration::from_millis(300));
    let end = pty.screen(ROWS, COLS);
    println!("\n{}", end.dump("WRAP END-OF-TURN (idle prompt)"));

    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    rig.server.assert_clean();

    // MID-TURN: the dock is three contiguous rows and the input row is present.
    let (top, bottom) = dock_rows(&mid_rows)
        .unwrap_or_else(|| panic!("mid-turn dock rules missing:\n{}", mid.dump("mid")));
    assert_eq!(bottom, top + 2, "the dock is not three contiguous rows:\n{}", mid.dump("mid"));
    let input = &mid_rows[top + 1];
    assert!(
        input.contains(MARKER),
        "MID-TURN the input row lost its `{MARKER}` marker: {input:?}\n{}",
        mid.dump("mid")
    );
    assert!(
        !input.contains("para-"),
        "MID-TURN wrapped output bled into the input row: {input:?}\n{}",
        mid.dump("mid")
    );

    // END-OF-TURN: the live frame is gone and a bare idle marker remains.
    let end_rows = end.render();
    assert!(
        dock_rows(&end_rows).is_none(),
        "END-OF-TURN the live frame was not torn down:\n{}",
        end.dump("end")
    );
    assert!(
        end_rows.iter().any(|r| r.trim_start().starts_with(MARKER)),
        "END-OF-TURN no idle `{MARKER}` prompt:\n{}",
        end.dump("end")
    );
}

// ---------------------------------------------------------------------------
// P6: slash-command completion in the raw input editor. Tab completes a
// `/`-prefixed command, an ambiguous prefix shows a candidate hint and stops at
// the common stem, and a non-slash line (or the argument region of a command)
// is never touched. Asserted through the compiled binary at a real pty; colors
// are never asserted.
// ---------------------------------------------------------------------------

/// Tab on a unique slash-command prefix completes it: `/pl` + Tab submits as
/// `/plan`, which dispatches (the plan-mode note prints). Without completion the
/// line would submit as `/pl` and be rejected as an unknown command. The classic
/// per-prompt editor gives a RAW_READY sync point and exercises the read_raw Tab
/// path.
#[test]
fn tab_completes_a_unique_slash_command_prefix() {
    let rig = rig();

    let mut pty = spawn_pty(&rig); // NOOB_DOCK=0: the read_raw path
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"/pl"); // an unambiguous prefix of exactly one command
    pty.send(&[0x09]); // Tab: complete the token to /plan
    pty.send(b"\r"); // submit the completed command
    pty.wait_for("cache prefix reset"); // enter_plan's note: /plan actually ran
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]); // Ctrl-D exits
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    // The tell-tale of a missed completion: `/pl` would dispatch as unknown.
    assert!(
        !pty.seen.contains("unknown command"),
        "the prefix did not complete; it dispatched as an unknown command:\n{}",
        pty.seen
    );
    assert!(rig.api_requests().is_empty(), "/plan makes no model request");
    rig.server.assert_clean();
}

/// An ambiguous prefix shows a dim candidate hint on the input row (both
/// commands listed), and Tab advances only to the common stem: it must never
/// pick one of them. `/s` matches `/status` and `/skills`, whose common stem is
/// `s` (already typed), so the hint stays and the token stays `/s`. Uses the
/// default dock driver and the screen emulator (colors stripped for the
/// assertion).
#[test]
fn ambiguous_prefix_shows_a_candidate_hint_and_tab_never_guesses() {
    const ROWS: u16 = 12;
    const COLS: u16 = 64;

    let rig = rig();
    let mut pty = spawn_pty_sized(&rig, &[], Some((ROWS, COLS)), &[]); // default dock
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);

    // Typing the ambiguous prefix: the input row lists both candidates.
    pty.send(b"/s");
    pty.drain(std::time::Duration::from_millis(400));
    let typed = pty.screen(ROWS, COLS);
    let typed_rows = typed.render();
    let row = input_row(&typed_rows)
        .unwrap_or_else(|| panic!("no input row after typing:\n{}", typed.dump("typed /s")));
    let plain = strip_ansi(row);
    assert!(
        plain.contains("/skills") && plain.contains("/status"),
        "the candidate hint did not list both commands: {plain:?}\n{}",
        typed.dump("typed /s")
    );

    // Tab advances only to the common stem `s` (already typed), so it neither
    // collapses to one command nor loses the hint.
    pty.send(&[0x09]);
    pty.drain(std::time::Duration::from_millis(400));
    let after = pty.screen(ROWS, COLS);
    let after_rows = after.render();
    let row = input_row(&after_rows)
        .unwrap_or_else(|| panic!("no input row after Tab:\n{}", after.dump("after tab")));
    let plain = strip_ansi(row);
    assert!(
        plain.contains("/skills") && plain.contains("/status"),
        "Tab wrongly collapsed the ambiguous prefix to one command: {plain:?}\n{}",
        after.dump("after tab")
    );

    pty.send(&[0x15]); // Ctrl-U clears the `/s` draft so Ctrl-D can exit
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    rig.server.assert_clean();
}

/// Regression guard: Tab on a non-slash line is inert. It inserts no literal
/// tab and completes nothing, so the exact typed line reaches the agent.
#[test]
fn tab_on_a_non_slash_line_is_inert() {
    let rig = rig();
    rig.server.enqueue_stream_completion("answered");

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"say");
    pty.send(&[0x09]); // Tab mid-line: must not insert a tab or complete
    pty.send(b" hi\r");
    pty.wait_for("answered");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(last_user(&reqs[0]), "say hi", "Tab altered a non-slash line");
    assert!(!last_user(&reqs[0]).contains('\t'), "a literal tab leaked into the line");
    rig.server.assert_clean();
}

/// Completion applies only to the command token, never its arguments. Once a
/// space is present, Tab is inert: `/skills st` + Tab submits verbatim (the
/// `/skills` subcommand handler then rejects `st`), rather than completing `st`
/// to `/status`.
#[test]
fn tab_does_not_complete_in_the_argument_region() {
    let rig = rig();

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"/skills st"); // a space has started the arguments
    pty.send(&[0x09]); // Tab in the argument region: inert
    pty.send(b"\r");
    // The line submitted as `/skills st`: the subcommand handler rejects `st`.
    // Had Tab completed the argument to `/status`, this notice would be absent.
    pty.wait_for("unknown /skills subcommand");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen);
    assert!(rig.api_requests().is_empty(), "no command here makes a model request");
    rig.server.assert_clean();
}
