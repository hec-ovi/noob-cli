//! The agent, as a supervised child speaking the protocol.
//!
//! `noob serve` is started with piped stdio, commands go down its stdin and
//! frames come back up its stdout. Nothing here knows what a frame means; that
//! is [`crate::state`]'s job. This layer's whole responsibility is that the two
//! pipes never block the interface.
//!
//! Reading happens on its own thread and hands frames over a channel, then
//! nudges the event loop. That is what keeps the redraw-on-change rule
//! intact: no polling anywhere, and no frame arriving unnoticed.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use noob_proto::{Command as Cmd, Event};

/// What the reader thread reports.
pub enum Incoming {
    Frame(Event),
    /// The agent's stderr said something. `serve` writes nothing there when it
    /// is healthy, so anything at all is worth showing.
    Diagnostic(String),
    /// The child is gone. Nothing further will arrive.
    Ended(String),
}

/// Exactly the command the agent is started with, built without starting it.
///
/// Its own function so the launch can be asserted on in a test: what a window
/// hands the agent decides what the agent does, and spawning a process to find
/// that out is not a test anybody runs.
///
/// `serve` takes no flag for anything in the CLI's config file and exits 2 on
/// one it does not know, so nothing configurable is passed as an argument. What
/// this does instead is take the named settings out of the child's environment.
/// The CLI reads its settings as "the process environment, then the `.env`", so
/// a value exported in whatever shell started this window would win over the
/// file forever and the settings panel would be writing lines nothing reads.
/// Cleared, the file is what the agent reads, which is the file the panel
/// writes, and it is re-read on every request rather than at launch.
pub fn command_for(
    program: &str,
    workspace: &std::path::Path,
    session: Option<&str>,
    clear: &[&str],
) -> Command {
    let mut command = Command::new(program);
    command.arg("serve");
    if let Some(id) = session {
        command.args(["--resume", id]);
    }
    for key in clear {
        command.env_remove(key);
    }
    command
        .current_dir(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// How many lines of the environment tail the panel keeps.
///
/// The tail carries the project's own `AGENTS.md`, the skills index and the
/// MCP line, and a machine with a shelf of skills can push that a long way.
/// The block says where it stopped rather than holding a megabyte of text
/// nobody scrolled to.
pub const ENV_LINES: usize = 2000;

/// What the thread that reads the environment tail hands back: the folder it
/// ran in, and either the lines or why there are none.
pub type Asked = (String, Result<Vec<String>, String>);

/// Exactly the command the environment tail is read with, built without
/// running it.
///
/// `noob debug env` prints the tail a session's prompt ends in, so it has to
/// be run the way the session is started or it prints another project's: the
/// same working directory, since the project's own `AGENTS.md`, its skills
/// and its `mcp.json` are all found relative to it, and the same keys taken
/// out of the environment, since the CLI prefers the environment over the
/// file the panel writes. Its own function for the same reason
/// [`command_for`] is one: what the window asks for is worth asserting
/// without starting a process.
pub fn env_command(program: &str, workspace: &std::path::Path, clear: &[&str]) -> Command {
    let mut command = Command::new(program);
    command.args(["debug", "env"]);
    for key in clear {
        command.env_remove(key);
    }
    command.current_dir(workspace);
    command
}

/// What the panel makes of what that command answered.
///
/// The other half of the seam: the window runs the process and hands the
/// three things a process answers with to this, so what the block shows can
/// be checked without a model, a config directory or a child at all.
pub fn env_from(ok: bool, stdout: &[u8], stderr: &[u8]) -> Result<Vec<String>, String> {
    if !ok {
        // The last thing it said, since a CLI that refuses says why on its
        // last line and prints its usage above it.
        let why = String::from_utf8_lossy(stderr);
        let last = why.lines().map(str::trim).rfind(|line| !line.is_empty());
        return Err(match last {
            Some(line) => format!("noob debug env failed: {line}"),
            None => String::from("noob debug env failed and said nothing"),
        });
    }
    let said = String::from_utf8_lossy(stdout);
    let mut lines: Vec<String> = said.lines().map(str::to_string).collect();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        return Err(String::from("noob debug env printed nothing"));
    }
    if lines.len() > ENV_LINES {
        lines.truncate(ENV_LINES);
        lines.push(format!("[the panel stops reading at {ENV_LINES} lines]"));
    }
    Ok(lines)
}

/// The version an agent that has not said anything yet is assumed to speak.
///
/// Nothing: its first frame says which, and a command written before that is
/// held rather than guessed at.
const UNKNOWN: u16 = 0;

/// One decoded frame's version, off the line it arrived on.
///
/// `v` leads every line by the protocol's own rule, so this reads the one field
/// it needs without paying for the rest. Used on the two lines that matter: the
/// first frame of a session, and a frame this window had to skip.
fn version_of(line: &str) -> Option<u16> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    u16::try_from(value.get("v")?.as_u64()?).ok()
}

pub struct Link {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Incoming>,
    /// Set once the child has exited, so a send does not try a closed pipe.
    ended: bool,
    /// The protocol version the agent stamps its frames with, learned from the
    /// first one it sends. [`UNKNOWN`] until then.
    ///
    /// Written by the reader thread, read here: the channel between them makes
    /// the write visible before the frame that carried it is handed over.
    speaks: Arc<AtomicU16>,
    /// Commands typed before the agent said which version it speaks. Sent in
    /// order the moment it does. A prompt at a starting window would otherwise
    /// be written at this window's version and dropped by an older agent, which
    /// is a prompt that vanishes with nothing anywhere saying why.
    waiting: Vec<Cmd>,
}

impl Link {
    /// Start `noob serve` in `workspace`, waking `notify` whenever something
    /// arrives.
    ///
    /// `notify` is called after the frame is queued, never with it: the event
    /// loop's job is to come and drain, and passing a payload through a wakeup
    /// couples the two lifetimes for nothing.
    pub fn spawn(
        program: &str,
        workspace: &std::path::Path,
        session: Option<&str>,
        clear: &[&str],
        notify: impl Fn() + Send + 'static,
    ) -> Result<Link, String> {
        let mut command = command_for(program, workspace, session, clear);
        let mut child = command.spawn().map_err(|e| {
            format!("cannot start {program:?}: {e}; is noob on PATH, or set NOOB_BIN")
        })?;

        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or("the agent has no stdout")?;
        let stderr = child.stderr.take();
        let (tx, rx) = channel();

        let speaks = Arc::new(AtomicU16::new(UNKNOWN));
        {
            let tx: Sender<Incoming> = tx.clone();
            let speaks = Arc::clone(&speaks);
            std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                let mut told = false;
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    // A line that does not decode is skipped rather than fatal,
                    // which is the protocol's own rule: one bad frame must not
                    // end a session.
                    match noob_proto::decode::<Event>(&line) {
                        Some(frame) => {
                            // Before the frame goes over: what the agent stamps
                            // is what this window has to write back to it.
                            speaks
                                .compare_exchange(
                                    UNKNOWN,
                                    frame.v,
                                    Ordering::Release,
                                    Ordering::Relaxed,
                                )
                                .ok();
                            if tx.send(Incoming::Frame(frame.body)).is_err() {
                                return;
                            }
                        }
                        // The one skip worth a word: an agent newer than this
                        // window, whose frames are being dropped whole. Said
                        // once, since it is true of every frame after it.
                        None => {
                            if let Some(v) = version_of(&line)
                                && v > noob_proto::VERSION
                            {
                                speaks
                                    .compare_exchange(
                                        UNKNOWN,
                                        v,
                                        Ordering::Release,
                                        Ordering::Relaxed,
                                    )
                                    .ok();
                                if !told {
                                    told = true;
                                    let say = format!(
                                        "the agent speaks protocol {v} and this window {}: \
                                         its frames are being skipped. update no0b.",
                                        noob_proto::VERSION
                                    );
                                    if tx.send(Incoming::Diagnostic(say)).is_err() {
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    notify();
                }
                let _ = tx.send(Incoming::Ended(String::from("the agent exited")));
                notify();
            });
        }
        if let Some(stderr) = stderr {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if tx.send(Incoming::Diagnostic(line)).is_err() {
                        return;
                    }
                }
            });
        }

        Ok(Link {
            child,
            stdin,
            rx,
            ended: false,
            speaks,
            waiting: Vec::new(),
        })
    }

    /// Which protocol version the agent speaks, or `None` before its first
    /// frame. The window says so when it is not this window's own.
    pub fn speaks(&self) -> Option<u16> {
        match self.speaks.load(Ordering::Acquire) {
            UNKNOWN => None,
            v => Some(v),
        }
    }

    /// Send one command, written at the version the agent reads. A closed pipe
    /// ends the link rather than erroring at every later call.
    ///
    /// Held until the agent's first frame says which version that is: a reader
    /// refuses anything stamped above its own and says nothing about it, so a
    /// command written a version too high is a command that never happened.
    pub fn send(&mut self, command: Cmd) {
        if self.ended {
            return;
        }
        let Some(v) = self.speaks() else {
            self.waiting.push(command);
            return;
        };
        self.write(v, &command);
    }

    fn write(&mut self, v: u16, command: &Cmd) {
        let Some(stdin) = self.stdin.as_mut() else {
            return;
        };
        let line = noob_proto::encode_at(v, command);
        if stdin.write_all(line.as_bytes()).is_err() || stdin.flush().is_err() {
            self.ended = true;
            self.stdin = None;
        }
    }

    /// Everything that has arrived since the last drain.
    pub fn drain(&mut self) -> Vec<Incoming> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(item) => {
                    if matches!(item, Incoming::Ended(_)) {
                        self.ended = true;
                    }
                    out.push(item);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.ended = true;
                    break;
                }
            }
        }
        // Whatever was typed before the agent's first frame goes out now, in
        // the order it was typed, at the version that frame announced.
        if !self.waiting.is_empty()
            && !self.ended
            && let Some(v) = self.speaks()
        {
            for command in std::mem::take(&mut self.waiting) {
                self.write(v, &command);
            }
        }
        out
    }

    pub fn is_alive(&self) -> bool {
        !self.ended
    }

    /// Close stdin and let the agent finish. `serve` ends its session when its
    /// input closes, so this is a request to stop, not a kill.
    pub fn shutdown(&mut self) {
        self.stdin = None;
        self.ended = true;
        // A wait would block the window's close on the agent's last turn, so
        // the child is reaped without waiting for it. Its own SIGHUP handling
        // and the closed pipe end it.
        let _ = self.child.try_wait();
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    /// A stand-in for the agent: a script that writes down how it was called and
    /// where it was called from, then exits.
    ///
    /// Starting a real `noob serve` here would put a model behind a unit test.
    /// What is being checked is the one thing this module decides, which is the
    /// command line the child is given.
    fn stub(dir: &Path, name: &str, log: &Path) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nfor arg in \"$@\"; do echo \"$arg\" >> {log}; done\npwd >> {log}\n",
                log = log.display()
            ),
        )
        .expect("a stub agent");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("a runnable stub");
        path
    }

    /// The `lines` the stub wrote, once it has run. The child is started and
    /// reaped by the operating system in its own time, so this waits for the
    /// whole of what it was going to say rather than reading it half written.
    fn written(log: &Path, lines: usize) -> Vec<String> {
        // Ten seconds of deadline, returning the moment the lines are there:
        // a run that passes stays instant, and a machine busy under the full
        // parallel gate no longer misses the stub's scheduling window, which
        // flaked this test twice in one day at two seconds.
        for _ in 0..1000 {
            if let Ok(text) = std::fs::read_to_string(log)
                && text.lines().count() >= lines
            {
                return text.lines().map(str::to_string).collect();
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the stub agent never ran: {}", log.display());
    }

    /// Start a stub, riding out ETXTBSY: the write's descriptor is
    /// close-on-exec, but a test forking in parallel holds it for the
    /// instant between our write and its own exec, and exec of a script
    /// held open for writing is refused (os error 26). The window is
    /// microseconds wide; retrying is the whole fix, and it flaked the
    /// suite three times in one day before it was understood.
    fn spawn_stub(program: &Path, workspace: &Path, session: Option<&str>) -> Link {
        for _ in 0..200 {
            match Link::spawn(
                program.to_str().expect("a path"),
                workspace,
                session,
                &[],
                || {},
            ) {
                Ok(link) => return link,
                Err(why) if why.contains("Text file busy") => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(why) => panic!("the stub did not start: {why}"),
            }
        }
        panic!("ETXTBSY never cleared for {}", program.display());
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("no0b-link-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp dir");
        dir
    }

    /// What the settings panel writes has to be what the agent runs with.
    ///
    /// `serve` refuses a flag it does not know, so neither of the two settings
    /// the panel owns can be passed as an argument; they reach the agent through
    /// the CLI's own file, which it re-reads on every request. What the launch
    /// does about them is get out of the way. The CLI prefers the process
    /// environment over that file, so an exported value in whatever shell
    /// started this window would outrank every line the panel writes, and the
    /// rows would be a control that does nothing.
    #[test]
    fn the_agent_s_own_settings_are_left_to_its_file() {
        let workspace = std::env::temp_dir();
        let fresh = command_for("noob", &workspace, None, &crate::agent::OWNED);
        let args: Vec<&str> = fresh
            .get_args()
            .map(|arg| arg.to_str().expect("a flag is text"))
            .collect();
        assert_eq!(args, ["serve"], "serve takes no flag for a config setting");
        assert_eq!(fresh.get_current_dir(), Some(workspace.as_path()));

        let mut cleared: Vec<&str> = fresh
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.to_str().expect("a name is text"))
            .collect();
        cleared.sort_unstable();
        assert_eq!(
            cleared,
            {
                let mut owned = crate::agent::OWNED.to_vec();
                owned.sort_unstable();
                owned
            },
            "the settings the panel writes are not left to the file"
        );
        assert!(
            fresh.get_envs().all(|(_, value)| value.is_none()),
            "the launch sets a value the file can no longer change"
        );

        // And a resumed session still arrives as its flag, cleared the same way.
        let resumed = command_for(
            "noob",
            &workspace,
            Some("19fb08fb0cf-55ace-0-ee6569bb"),
            &crate::agent::OWNED,
        );
        let args: Vec<&str> = resumed
            .get_args()
            .map(|arg| arg.to_str().expect("a flag is text"))
            .collect();
        assert_eq!(args, ["serve", "--resume", "19fb08fb0cf-55ace-0-ee6569bb"]);
        assert_eq!(resumed.get_envs().count(), crate::agent::OWNED.len());
    }

    /// The session chosen in the picker has to reach the agent, or resuming one
    /// silently starts a fresh conversation in the right folder, which looks
    /// exactly like a session whose transcript has been lost.
    #[test]
    fn a_chosen_session_reaches_the_agent_as_resume() {
        let dir = temp("resume");
        let workspace = dir.join("workspace");
        std::fs::create_dir_all(&workspace).expect("a workspace");
        let log = dir.join("called-with");
        let program = stub(&dir, "resuming-noob", &log);

        let link = spawn_stub(&program, &workspace, Some("19fb08fb0cf-55ace-0-ee6569bb"));
        assert_eq!(
            written(&log, 4),
            [
                "serve",
                "--resume",
                "19fb08fb0cf-55ace-0-ee6569bb",
                workspace.to_str().expect("a path"),
            ],
            "the id and the folder both have to arrive"
        );
        drop(link);

        // And with no session chosen the flag is not there at all, rather than
        // being passed empty.
        let plain_log = dir.join("called-plain");
        let plain = stub(&dir, "fresh-noob", &plain_log);
        let link = spawn_stub(&plain, &workspace, None);
        assert_eq!(
            written(&plain_log, 2),
            ["serve", workspace.to_str().expect("a path")]
        );
        drop(link);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The environment tail the panel shows has to be the tail the session's
    /// prompt ends in, and the only thing that decides that is where the
    /// command runs and what it runs with. Asserted on the command rather
    /// than by starting one: a real `noob debug env` reads a config
    /// directory, a workspace and every skill on the machine.
    #[test]
    fn the_env_is_read_the_way_the_agent_is_started() {
        let workspace = std::env::temp_dir();
        let asking = env_command("noob", &workspace, &crate::agent::OWNED);
        let args: Vec<&str> = asking
            .get_args()
            .map(|arg| arg.to_str().expect("a flag is text"))
            .collect();
        assert_eq!(args, ["debug", "env"]);
        assert_eq!(
            asking.get_current_dir(),
            Some(workspace.as_path()),
            "a tail read somewhere else is another project's tail"
        );
        let mut cleared: Vec<&str> = asking
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.to_str().expect("a name is text"))
            .collect();
        cleared.sort_unstable();
        assert_eq!(
            cleared,
            {
                let mut owned = crate::agent::OWNED.to_vec();
                owned.sort_unstable();
                owned
            },
            "the serve child and the env read different settings"
        );
        // The same treatment `serve` gets, which is what makes the two the
        // same prompt.
        let serving = command_for("noob", &workspace, None, &crate::agent::OWNED);
        assert_eq!(asking.get_current_dir(), serving.get_current_dir());
        assert_eq!(asking.get_envs().count(), serving.get_envs().count());
    }

    /// What the panel shows for each of the three things that command can do:
    /// print the tail, print nothing, and fail.
    #[test]
    fn what_the_env_command_answered_is_what_the_block_shows() {
        let tail = env_from(true, b"<env>\ncwd: /work\n</env>\n\n\n", b"").expect("a tail");
        assert_eq!(
            tail,
            ["<env>", "cwd: /work", "</env>"],
            "the blank lines on the end are not part of the tail"
        );

        // A failure says why, off the last line the CLI wrote, rather than
        // leaving the block empty with nothing anywhere saying what happened.
        let why = env_from(false, b"", b"usage: noob debug <what>\nno such subcommand: env\n")
            .expect_err("it failed");
        assert!(why.contains("no such subcommand: env"), "{why}");
        let quiet = env_from(false, b"", b"").expect_err("it failed");
        assert!(quiet.contains("said nothing"), "{quiet}");
        let empty = env_from(true, b"\n \n", b"").expect_err("nothing was printed");
        assert!(empty.contains("printed nothing"), "{empty}");

        // And a tail longer than the block keeps says where it stopped.
        let long = "line\n".repeat(ENV_LINES + 40);
        let cut = env_from(true, long.as_bytes(), b"").expect("a tail");
        assert_eq!(cut.len(), ENV_LINES + 1);
        assert!(cut[ENV_LINES].contains("stops reading"), "{:?}", cut[ENV_LINES]);
    }

    /// An agent older than this window still gets what is typed at it.
    ///
    /// The protocol's reader refuses a frame stamped above its own version and
    /// says nothing about the refusal, so a window that writes at its own
    /// version has every prompt swallowed by an agent one release behind: the
    /// orb turns, no turn ever starts, and nothing anywhere says why. This is
    /// the shape of that install (a 0.11.2 window over a 0.11.1 agent), and
    /// what it must produce is a command the agent can read.
    #[test]
    fn a_command_is_written_at_the_version_the_agent_speaks() {
        let dir = temp("older-agent");
        let log = dir.join("heard");
        let path = dir.join("older-noob");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\n\
                 echo '{{\"v\":1,\"t\":\"session.start\",\"id\":\"i\",\"workspace\":\"/w\",\
                 \"model\":\"m\",\"resumed\":false}}'\n\
                 while IFS= read -r line; do echo \"$line\" >> {log}; done\n",
                log = log.display()
            ),
        )
        .expect("a stub agent");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("a runnable stub");

        let mut link = spawn_stub(&path, &dir, None);
        // Typed before the agent has said anything, which is the case that
        // used to go out at this window's version and vanish.
        link.send(Cmd::PromptSubmit {
            text: String::from("say hi"),
        });
        let mut saw_start = false;
        for _ in 0..1000 {
            for item in link.drain() {
                saw_start |= matches!(item, Incoming::Frame(Event::SessionStart { .. }));
            }
            if saw_start && log.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(saw_start, "the stub agent never announced its session");
        assert_eq!(link.speaks(), Some(1), "the agent's version was not learned");
        let heard = written(&log, 1);
        assert_eq!(
            heard[0],
            r#"{"v":1,"t":"prompt.submit","text":"say hi"}"#,
            "the prompt has to arrive stamped at the version the agent reads"
        );
        drop(link);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A program that is not there is a message in the window, not a panic and
    /// not a window that never opens.
    #[test]
    fn an_agent_that_cannot_be_started_says_so() {
        let dir = temp("missing");
        let trouble = Link::spawn("no0b-nothing-of-this-name", &dir, None, &[], || {})
            .err()
        .expect("there is no such program");
        assert!(trouble.contains("NOOB_BIN"), "{trouble}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
