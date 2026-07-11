//! MCP stdio transport: a child process speaking newline-delimited JSON-RPC.
//! One reader thread feeds an mpsc channel; calls block on `recv_timeout`
//! against the per-server deadline. A timeout kills the child's whole
//! process group (a wedged server can never block the loop) and the next
//! call respawns and re-handshakes transparently. Server-to-client requests
//! get a polite method-not-found reply so a waiting server cannot wedge us.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use noob_provider::http::INTERRUPTED;
use serde_json::Value;

use super::proto::{self, Inbound};

/// Bound on one inbound line: a runaway server must exhaust its own memory,
/// not ours. Larger than any sane tool result (which we cap at 20 KiB
/// anyway) with room for big tool catalogs.
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
/// Bound parsed messages waiting for the request loop. When a server floods
/// stdout, its pipe supplies the backpressure instead of an unbounded heap.
const INBOUND_QUEUE: usize = 16;
/// A silent MCP server must not hide Ctrl-C until its full request timeout.
const INTERRUPT_POLL: Duration = Duration::from_millis(50);

pub struct StdioTransport {
    command: String,
    args: Vec<String>,
    timeout: Duration,
    state: Mutex<State>,
    #[cfg(test)]
    test_interrupted: std::sync::atomic::AtomicBool,
}

struct State {
    proc: Option<Proc>,
    next_id: u64,
    protocol: String,
}

struct Proc {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<Value>,
    live: bool,
}

impl Proc {
    /// SIGKILL the whole process group and reap. The group exists because
    /// the child was spawned with `process_group(0)`, so a server that
    /// forked helpers cannot leave them behind.
    fn kill_group(&mut self) {
        if !self.live {
            return;
        }
        self.live = false;
        let pid = self.child.id() as libc::pid_t;
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        self.kill_group();
    }
}

impl StdioTransport {
    pub fn new(command: &str, args: &[String], timeout: Duration) -> StdioTransport {
        StdioTransport {
            command: command.to_string(),
            args: args.to_vec(),
            timeout,
            state: Mutex::new(State {
                proc: None,
                next_id: 1,
                protocol: proto::PROTOCOL_VERSION.to_string(),
            }),
            #[cfg(test)]
            test_interrupted: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn interrupted(&self) -> bool {
        if INTERRUPTED.load(Ordering::SeqCst) {
            return true;
        }
        #[cfg(test)]
        {
            self.test_interrupted.load(Ordering::SeqCst)
        }
        #[cfg(not(test))]
        {
            false
        }
    }

    /// Spawn + handshake if the process is not alive. Returns the
    /// negotiated protocol version.
    pub fn ensure_ready(&self) -> Result<String, String> {
        let mut state = self.state.lock().unwrap();
        self.ensure_locked(&mut state)?;
        Ok(state.protocol.clone())
    }

    /// One JSON-RPC request; spawns and re-handshakes first when needed.
    pub fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let mut state = self.state.lock().unwrap();
        self.ensure_locked(&mut state)?;
        self.rpc_locked(&mut state, method, params)
    }

    fn ensure_locked(&self, state: &mut State) -> Result<(), String> {
        let alive = match &mut state.proc {
            Some(proc) => proc.child.try_wait().map(|s| s.is_none()).unwrap_or(false),
            None => false,
        };
        if alive {
            return Ok(());
        }
        state.proc = None; // drops (and reaps) any exited child
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Server logging goes to the void, not into the UI stream; a
            // failing server surfaces through typed call errors instead.
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|e| {
                format!(
                    "cannot start MCP server command {:?}: {e}; check the \"command\" \
                     in mcp.json and that it is installed in the container",
                    self.command
                )
            })?;
        let stdin = child.stdin.take().expect("piped stdin");
        // Non-blocking stdin: a server that stops READING must not be able
        // to block the loop either (write_all on a full pipe never returns
        // and never sees the interrupt flag). Writes go through
        // write_deadline below.
        if let Err(error) = set_nonblocking(&stdin) {
            let pid = child.id() as libc::pid_t;
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "cannot configure the MCP server input pipe ({error}); restart noob and try again"
            ));
        }
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = mpsc::sync_channel(INBOUND_QUEUE);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_line_bounded(&mut reader, MAX_LINE_BYTES) {
                    Ok(Some(line)) => {
                        if let Ok(v) = serde_json::from_slice::<Value>(&line)
                            && tx.send(v).is_err()
                        {
                            return; // transport dropped
                        }
                        // Non-JSON lines (stray prints) are dropped.
                    }
                    Ok(None) | Err(_) => return, // EOF or an over-cap line
                }
            }
        });
        state.proc = Some(Proc { child, stdin, rx, live: true });

        // Handshake: initialize -> capture negotiated version -> initialized.
        let init = match self.rpc_locked(state, "initialize", proto::initialize_params()) {
            Ok(init) => init,
            Err(error) => {
                if let Some(proc) = &mut state.proc {
                    proc.kill_group();
                }
                state.proc = None;
                return Err(error);
            }
        };
        state.protocol = init
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(proto::PROTOCOL_VERSION)
            .to_string();
        let note = proto::notification("notifications/initialized");
        self.send_locked(state, &note)?;
        Ok(())
    }

    fn send_locked(&self, state: &mut State, msg: &Value) -> Result<(), String> {
        let deadline = Instant::now() + self.timeout;
        self.send_locked_until(state, msg, deadline)
    }

    fn send_locked_until(
        &self,
        state: &mut State,
        msg: &Value,
        deadline: Instant,
    ) -> Result<(), String> {
        let proc = state.proc.as_mut().expect("ensured");
        let write = write_deadline(
            &mut proc.stdin,
            format!("{msg}\n").as_bytes(),
            deadline,
            || self.interrupted(),
        );
        if let Err(e) = write {
            let canceled = e.kind() == std::io::ErrorKind::Interrupted && self.interrupted();
            if let Some(proc) = &mut state.proc {
                proc.kill_group();
            }
            state.proc = None;
            if canceled {
                return Err("MCP call canceled by user; the server process was killed and will be restarted on the next call".to_string());
            }
            return Err(format!(
                "the MCP server process is not accepting input ({e}); it was killed \
                 and will be restarted on the next call"
            ));
        }
        Ok(())
    }

    fn rpc_locked(&self, state: &mut State, method: &str, params: Value) -> Result<Value, String> {
        let id = state.next_id;
        state.next_id += 1;
        let msg = proto::request(id, method, params);
        self.send_locked(state, &msg)?;
        let deadline = Instant::now() + self.timeout;
        loop {
            if self.interrupted() {
                if let Some(proc) = &mut state.proc {
                    proc.kill_group();
                }
                state.proc = None;
                return Err(
                    "MCP call canceled by user; the server process was killed and will be restarted on the next call"
                        .to_string(),
                );
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                if let Some(proc) = &mut state.proc {
                    proc.kill_group();
                }
                state.proc = None;
                return Err(format!(
                    "MCP call timed out after {}s; the server process was killed and \
                     will be restarted on the next call",
                    self.timeout.as_secs()
                ));
            }
            let received = state
                .proc
                .as_mut()
                .expect("ensured")
                .rx
                .recv_timeout(remaining.min(INTERRUPT_POLL));
            match received {
                Ok(msg) => match proto::classify(&msg) {
                    Inbound::Response { id: got, outcome } if got == id => return outcome,
                    // A response to an id we gave up on earlier: stale, skip.
                    Inbound::Response { .. } => {}
                    Inbound::ServerRequest { id } => {
                        let reply = proto::method_not_found(&id);
                        self.send_locked_until(state, &reply, deadline)?;
                    }
                    Inbound::Other => {}
                },
                Err(mpsc::RecvTimeoutError::Timeout) => {} // loop re-checks deadline
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    state.proc = None;
                    return Err(
                        "the MCP server process exited unexpectedly; it will be \
                         restarted on the next call"
                            .to_string(),
                    );
                }
            }
        }
    }
}

fn set_nonblocking(stdin: &ChildStdin) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    let fd = stdin.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Write the whole buffer to a non-blocking pipe, bounded by `deadline`.
/// A full pipe (the server stopped reading) surfaces as a timeout error
/// instead of an unbounded block the interrupt flag can never reach.
fn write_deadline(
    stdin: &mut ChildStdin,
    mut buf: &[u8],
    deadline: Instant,
    interrupted: impl Fn() -> bool,
) -> std::io::Result<()> {
    while !buf.is_empty() {
        if interrupted() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "MCP call canceled by user",
            ));
        }
        match stdin.write(buf) {
            Ok(0) => return Err(std::io::Error::other("the pipe closed mid-write")),
            Ok(n) => buf = &buf[n..],
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "the server stopped reading its input",
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
    }
    if interrupted() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "MCP call canceled by user",
        ));
    }
    stdin.flush()
}

/// Read one `\n`-terminated line without unbounded growth. `Ok(None)` = EOF.
fn read_line_bounded(
    reader: &mut impl BufRead,
    cap: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }
        if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            line.extend_from_slice(&buf[..pos]);
            reader.consume(pos + 1);
            return Ok(Some(line));
        }
        line.extend_from_slice(buf);
        let n = buf.len();
        reader.consume(n);
        if line.len() > cap {
            return Err(std::io::Error::other("line exceeds the inbound cap"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Write a tiny POSIX-shell MCP server into `dir` and return its path.
    /// It answers initialize / tools/list / tools/call by substring match,
    /// with `body` spliced in for custom behaviors.
    pub(crate) fn shell_server(dir: &std::path::Path, name: &str, call_case: &str) -> String {
        let script = format!(
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"protocolVersion":"2025-11-25","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"mock","version":"0"}}}}}}\n' "$id" ;;
    *'"method":"tools/list"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"tools":[{{"name":"echo","description":"echoes text back","inputSchema":{{"type":"object","properties":{{"text":{{"type":"string"}}}},"required":["text"]}}}}]}}}}\n' "$id" ;;
    *'"method":"tools/call"'*)
      {call_case} ;;
    *) : ;;
  esac
done
"#,
        );
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        path.to_str().unwrap().to_string()
    }

    pub(crate) const ECHO_CALL: &str = r#"text=$(printf '%s' "$line" | sed -n 's/.*"text":"\([^"]*\)".*/\1/p')
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"echo: %s"}],"isError":false}}\n' "$id" "$text""#;

    #[test]
    fn round_trip_handshake_list_and_call() {
        let tmp = tempfile::tempdir().unwrap();
        let cmd = shell_server(tmp.path(), "server.sh", ECHO_CALL);
        let t = StdioTransport::new("sh", &[cmd], Duration::from_secs(5));
        assert_eq!(t.ensure_ready().unwrap(), "2025-11-25");
        let listed = t.request("tools/list", json!({})).unwrap();
        assert_eq!(listed["tools"][0]["name"], "echo");
        let result = t
            .request("tools/call", json!({"name": "echo", "arguments": {"text": "hi"}}))
            .unwrap();
        assert_eq!(result["content"][0]["text"], "echo: hi");
    }

    #[test]
    fn failed_initialize_is_killed_and_retried_from_a_fresh_process() {
        let tmp = tempfile::tempdir().unwrap();
        let flag = tmp.path().join("failed-once");
        let script = format!(
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      if [ ! -f {flag} ]; then
        touch {flag}
        printf '{{"jsonrpc":"2.0","id":%s,"error":{{"code":-32000,"message":"first init failed"}}}}\n' "$id"
      else
        printf '{{"jsonrpc":"2.0","id":%s,"result":{{"protocolVersion":"2025-11-25"}}}}\n' "$id"
      fi ;;
    *) : ;;
  esac
done
"#,
            flag = flag.display()
        );
        let path = tmp.path().join("init-fails-once.sh");
        std::fs::write(&path, script).unwrap();
        let t = StdioTransport::new(
            "sh",
            &[path.to_string_lossy().into_owned()],
            Duration::from_secs(2),
        );
        assert!(t.ensure_ready().unwrap_err().contains("first init failed"));
        assert_eq!(t.ensure_ready().unwrap(), "2025-11-25");
    }

    #[test]
    fn parsed_inbound_queue_is_bounded() {
        let (tx, _rx) = mpsc::sync_channel(INBOUND_QUEUE);
        for n in 0..INBOUND_QUEUE {
            tx.try_send(json!(n)).unwrap();
        }
        assert!(matches!(
            tx.try_send(json!("overflow")),
            Err(mpsc::TrySendError::Full(_))
        ));
    }

    #[test]
    fn timeout_kills_the_group_and_next_call_respawns() {
        let tmp = tempfile::tempdir().unwrap();
        // First call wedges (flag file marks the first attempt); later calls echo.
        let call_case = format!(
            r#"if [ ! -f {flag} ]; then touch {flag}; sleep 600; fi
      {ECHO_CALL}"#,
            flag = tmp.path().join("wedged.flag").display()
        );
        let cmd = shell_server(tmp.path(), "server.sh", &call_case);
        let t = StdioTransport::new("sh", &[cmd], Duration::from_secs(1));
        t.ensure_ready().unwrap();
        let started = Instant::now();
        let err = t
            .request("tools/call", json!({"name": "echo", "arguments": {"text": "x"}}))
            .unwrap_err();
        assert!(err.contains("timed out after 1s"), "{err}");
        assert!(err.contains("restarted on the next call"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5), "kill was not prompt");
        // Transparent respawn: the next call handshakes again and succeeds.
        let result = t
            .request("tools/call", json!({"name": "echo", "arguments": {"text": "back"}}))
            .unwrap();
        assert_eq!(result["content"][0]["text"], "echo: back");
    }

    #[test]
    fn interrupt_cancels_a_silent_rpc_and_next_call_respawns() {
        let tmp = tempfile::tempdir().unwrap();
        let call_case = format!(
            r#"if [ ! -f {flag} ]; then touch {flag}; sleep 600; fi
      {ECHO_CALL}"#,
            flag = tmp.path().join("interrupted.flag").display()
        );
        let cmd = shell_server(tmp.path(), "server.sh", &call_case);
        let t = std::sync::Arc::new(StdioTransport::new(
            "sh",
            &[cmd],
            Duration::from_secs(30),
        ));
        t.ensure_ready().unwrap();

        let signal = t.clone();
        let interrupter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            signal.test_interrupted.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let err = t
            .request("tools/call", json!({"name": "echo", "arguments": {"text": "x"}}))
            .unwrap_err();
        interrupter.join().unwrap();
        assert!(err.contains("canceled by user"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(2), "cancel was not prompt");

        t.test_interrupted.store(false, Ordering::SeqCst);
        let result = t
            .request("tools/call", json!({"name": "echo", "arguments": {"text": "back"}}))
            .unwrap();
        assert_eq!(result["content"][0]["text"], "echo: back");
    }

    #[test]
    fn dead_server_is_reported_and_respawned() {
        let tmp = tempfile::tempdir().unwrap();
        let call_case = format!(
            r#"if [ ! -f {flag} ]; then touch {flag}; exit 7; fi
      {ECHO_CALL}"#,
            flag = tmp.path().join("died.flag").display()
        );
        let cmd = shell_server(tmp.path(), "server.sh", &call_case);
        let t = StdioTransport::new("sh", &[cmd], Duration::from_secs(5));
        let err = t
            .request("tools/call", json!({"name": "echo", "arguments": {"text": "x"}}))
            .unwrap_err();
        assert!(err.contains("exited unexpectedly"), "{err}");
        let result = t
            .request("tools/call", json!({"name": "echo", "arguments": {"text": "alive"}}))
            .unwrap();
        assert_eq!(result["content"][0]["text"], "echo: alive");
    }

    #[test]
    fn missing_command_is_a_typed_error_with_remedy() {
        let t = StdioTransport::new("/does/not/exist-mcp", &[], Duration::from_secs(1));
        let err = t.ensure_ready().unwrap_err();
        assert!(err.contains("cannot start MCP server command"), "{err}");
        assert!(err.contains("mcp.json"), "{err}");
    }

    #[test]
    fn server_requests_get_method_not_found_and_do_not_wedge_us() {
        let tmp = tempfile::tempdir().unwrap();
        // On tools/call the server first asks US something, then answers.
        let call_case = format!(
            r#"printf '{{"jsonrpc":"2.0","id":9001,"method":"sampling/createMessage","params":{{}}}}\n'
      {ECHO_CALL}"#
        );
        let cmd = shell_server(tmp.path(), "server.sh", &call_case);
        let t = StdioTransport::new("sh", &[cmd], Duration::from_secs(5));
        let result = t
            .request("tools/call", json!({"name": "echo", "arguments": {"text": "ok"}}))
            .unwrap();
        assert_eq!(result["content"][0]["text"], "echo: ok");
    }

    #[test]
    fn full_pipe_write_times_out_instead_of_blocking_forever() {
        let tmp = tempfile::tempdir().unwrap();
        // Handshake normally, then stop reading stdin entirely: a huge
        // request fills the ~64 KiB pipe buffer and the old blocking
        // write_all would never return.
        let script = r#"#!/bin/sh
IFS= read -r line
id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"mock","version":"0"}}}\n' "$id"
IFS= read -r line
exec sleep 600
"#;
        let path = tmp.path().join("deaf.sh");
        std::fs::write(&path, script).unwrap();
        let script = path.to_str().unwrap().to_string();
        let t = StdioTransport::new("sh", &[script], Duration::from_secs(1));
        t.ensure_ready().unwrap();
        let big = "x".repeat(512 * 1024); // far past any pipe buffer
        let started = Instant::now();
        let err = t
            .request("tools/call", json!({"name": "echo", "arguments": {"text": big}}))
            .unwrap_err();
        assert!(err.contains("not accepting input"), "{err}");
        assert!(err.contains("restarted on the next call"), "{err}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "write blocked for {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn interrupt_cancels_a_blocked_pipe_write() {
        let tmp = tempfile::tempdir().unwrap();
        let script = r#"#!/bin/sh
IFS= read -r line
id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"mock","version":"0"}}}\n' "$id"
IFS= read -r line
exec sleep 600
"#;
        let path = tmp.path().join("deaf-interrupt.sh");
        std::fs::write(&path, script).unwrap();
        let script = path.to_str().unwrap().to_string();
        let t = std::sync::Arc::new(StdioTransport::new(
            "sh",
            &[script],
            Duration::from_secs(30),
        ));
        t.ensure_ready().unwrap();
        let signal = t.clone();
        let interrupter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            signal.test_interrupted.store(true, Ordering::SeqCst);
        });
        let big = "x".repeat(512 * 1024);
        let started = Instant::now();
        let err = t
            .request("tools/call", json!({"name": "echo", "arguments": {"text": big}}))
            .unwrap_err();
        interrupter.join().unwrap();
        assert!(err.contains("canceled by user"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(2), "cancel was not prompt");
    }

    #[test]
    fn bounded_line_reader_rejects_runaway_lines() {
        let mut small = std::io::Cursor::new(b"abc\ndef".to_vec());
        assert_eq!(read_line_bounded(&mut small, 10).unwrap(), Some(b"abc".to_vec()));
        assert_eq!(read_line_bounded(&mut small, 10).unwrap(), Some(b"def".to_vec()));
        assert_eq!(read_line_bounded(&mut small, 10).unwrap(), None);
        let mut big = std::io::Cursor::new(vec![b'x'; 100]);
        assert!(read_line_bounded(&mut big, 10).is_err());
    }
}
