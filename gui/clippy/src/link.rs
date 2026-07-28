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

pub struct Link {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Incoming>,
    /// Set once the child has exited, so a send does not try a closed pipe.
    ended: bool,
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
        notify: impl Fn() + Send + 'static,
    ) -> Result<Link, String> {
        let mut command = Command::new(program);
        command.arg("serve");
        if let Some(id) = session {
            command.args(["--resume", id]);
        }
        command
            .current_dir(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|e| {
            format!("cannot start {program:?}: {e}; is noob on PATH, or set NOOB_BIN")
        })?;

        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or("the agent has no stdout")?;
        let stderr = child.stderr.take();
        let (tx, rx) = channel();

        {
            let tx: Sender<Incoming> = tx.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    // A line that does not decode is skipped rather than fatal,
                    // which is the protocol's own rule: one bad frame must not
                    // end a session.
                    if let Some(frame) = noob_proto::decode::<Event>(&line)
                        && tx.send(Incoming::Frame(frame.body)).is_err()
                    {
                        return;
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
        })
    }

    /// Send one command. A closed pipe ends the link rather than erroring at
    /// every later call.
    pub fn send(&mut self, command: Cmd) {
        if self.ended {
            return;
        }
        let Some(stdin) = self.stdin.as_mut() else {
            return;
        };
        let line = noob_proto::encode(&command);
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
