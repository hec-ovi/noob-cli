//! Multi-agent (P6): the `subagent` tool spawns the binary itself
//! (`current_exe() child`). The process boundary is the context boundary:
//! the payload goes to the child's stdin as one JSON object, exactly one
//! JSON result line comes back on stdout, progress flows on stderr, and
//! only the result string enters the parent transcript. argv + stdin +
//! stdout is the whole IPC surface.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use noob_provider::http::INTERRUPTED;
use noob_provider::types::ToolSpec;

use crate::tools::{ToolCtx, ToolOutcome, need_str, opt_str, opt_u64};

mod background;
#[cfg(test)]
pub use background::JobProgressSnapshot;
pub use background::{BackgroundHub, JobsSnapshot, ReadyResult};

/// Recursion ceiling: depth 0 (the user's agent) and depth 1 children may
/// spawn; at depth 2 the subagent tool is simply not registered.
pub const MAX_DEPTH: u32 = 2;
pub const DEFAULT_CONCURRENCY: usize = 4;
pub const DEFAULT_MAX_TURNS: u32 = 25;
/// Per-child wall clock; the parent kills the whole process group on expiry.
pub const DEFAULT_WALL_CLOCK_S: u64 = 300;

/// Child progress is UI-only and can be noisy. Keep a bounded head while
/// draining the rest so parallel children cannot grow parent memory.
const STDERR_CAP: usize = 64 * 1024;
static PARENT_DIED: AtomicBool = AtomicBool::new(false);

/// Session-scoped sub-agent settings, resolved once at bootstrap.
#[derive(Clone, Debug)]
pub struct TaskCfg {
    /// This process's depth (NOOB_DEPTH, 0 for the user's agent).
    pub depth: u32,
    pub concurrency: usize,
    pub max_turns: u32,
    pub wall_clock: Duration,
    /// Surface bounded child stderr as `[subagent] ...` after the child exits.
    pub verbose: bool,
    /// Present only for the default interactive dock. Other surfaces keep the
    /// original inline child contract.
    pub background: Option<BackgroundHub>,
}

#[derive(Clone)]
struct TaskRequest {
    prompt: String,
    tools_mode: String,
    max_turns: u32,
}

#[derive(Clone)]
struct RunCfg {
    depth: u32,
    wall_clock: Duration,
    verbose: bool,
    workspace: std::path::PathBuf,
    progress: Option<background::ProgressLog>,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "subagent".to_string(),
        description: "Spawn an independent sub-agent with its own context and tools that returns \
                      one result; call several times to fan out."
            .to_string(),
        parameters: json!({"type": "object", "properties": {
            "prompt": {"type": "string", "description": "complete standalone instructions"},
            "tools": {"type": "string", "enum": ["read-only", "all"],
                      "description": "default read-only; use all for Bash, MCP/web, or file changes"},
            "max_turns": {"type": "integer"}
        }, "required": ["prompt"]}),
    }
}

pub fn run(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    let Some(cfg) = &ctx.task else {
        return ToolOutcome::err(
            "sub-agents are not available here; do the work yourself with the other tools",
        );
    };
    let prompt = match need_str(args, "prompt") {
        Ok(p) if !p.trim().is_empty() => p.to_string(),
        Ok(_) => return ToolOutcome::err("parameter \"prompt\" is empty; resend the call"),
        Err(e) => return ToolOutcome::err(e),
    };
    let tools_mode = match opt_str(args, "tools") {
        Ok(None) => "read-only".to_string(),
        Ok(Some(m @ ("read-only" | "all"))) => m.to_string(),
        Ok(Some(other)) => {
            return ToolOutcome::err(format!(
                "parameter \"tools\" must be \"read-only\" or \"all\", got {other:?}; \
                 resend the call"
            ));
        }
        Err(e) => return ToolOutcome::err(e),
    };
    // Both sides enforce the turn cap: the parent clamps the request here,
    // the child clamps again against its own environment.
    let max_turns = match opt_u64(args, "max_turns") {
        Ok(Some(n)) => clamp_max_turns(n, cfg.max_turns),
        Ok(None) => cfg.max_turns,
        Err(e) => return ToolOutcome::err(e),
    };

    let request = TaskRequest {
        prompt,
        tools_mode,
        max_turns,
    };
    let run_cfg = RunCfg {
        depth: cfg.depth,
        wall_clock: cfg.wall_clock,
        verbose: cfg.verbose,
        workspace: ctx.workspace.clone(),
        progress: None,
    };
    // Every dock child detaches. Full-tool children take the cross-process
    // workspace lease around each write/edit/bash call, so inference and
    // read-only work remain parallel without permitting concurrent mutations.
    if let Some(hub) = &cfg.background {
        return hub.submit(run_cfg, request);
    }
    run_task(&run_cfg, &request, || INTERRUPTED.load(Ordering::SeqCst))
}

fn run_task(
    cfg: &RunCfg,
    request: &TaskRequest,
    interrupted: impl Fn() -> bool + Copy,
) -> ToolOutcome {
    let deadline = Instant::now() + cfg.wall_clock;
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => return ToolOutcome::err(format!("cannot locate the noob binary: {e}")),
    };
    let parent_pid = unsafe { libc::getpid() };
    let mut command = Command::new(exe);
    command
        .arg("child")
        .env("NOOB_DEPTH", (cfg.depth + 1).to_string())
        .current_dir(&cfg.workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    // If the parent dies, SIGTERM lets the child's watchdog kill Bash, MCP,
    // and nested-agent descendants before the child exits. The parent-pid
    // recheck closes the fork-to-prctl race.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent_pid {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "the parent exited while spawning the sub-agent",
                ));
            }
            Ok(())
        });
    }
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => return ToolOutcome::err(format!("cannot spawn the sub-agent: {e}")),
    };
    // One JSON object in, then EOF: the child reads stdin to end.
    let payload = json!({
        "prompt": request.prompt,
        "tools": request.tools_mode,
        "max_turns": request.max_turns,
    });
    {
        let mut stdin = child.stdin.take().expect("piped stdin");
        let bytes = format!("{payload}\n");
        if let Err(error) =
            write_child_input_with(&mut stdin, bytes.as_bytes(), deadline, interrupted)
        {
            kill_group(&mut child);
            return match error {
                ChildInputError::Canceled => ToolOutcome::canceled(),
                ChildInputError::Timeout => ToolOutcome::err(format!(
                    "the sub-agent did not accept its task within {}s and was killed; \
                     retry with a smaller prompt or raise NOOB_TASK_WALL_CLOCK_S",
                    cfg.wall_clock.as_secs()
                )),
                ChildInputError::Closed(error) => ToolOutcome::err(format!(
                    "the sub-agent exited before reading its task ({error}); try again"
                )),
            };
        }
    } // drop closes the pipe

    // Readers on threads so a chatty child never deadlocks on a full pipe.
    let stdout = child.stdout.take().expect("piped stdout");
    let stdout_reader = std::thread::spawn(move || read_all(stdout));
    let stderr = child.stderr.take().expect("piped stderr");
    let verbose = cfg.verbose;
    let live_progress = cfg.progress.clone();
    let stderr_reader = std::thread::spawn(move || {
        drain_stderr_with(stderr, verbose, |bytes| {
            if let Some(progress) = &live_progress {
                progress.push(bytes);
            }
        })
    });

    // The wait loop owns the three exits: completion, wall clock, Ctrl-C.
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => {
                kill_group(&mut child);
                break None;
            }
        }
        if interrupted() {
            kill_group(&mut child);
            let _ = stdout_reader.join();
            let progress = stderr_reader.join().unwrap_or_default();
            return with_progress(ToolOutcome::canceled(), verbose, progress);
        }
        if Instant::now() >= deadline {
            kill_group(&mut child);
            let _ = stdout_reader.join();
            let progress = stderr_reader.join().unwrap_or_default();
            return with_progress(
                ToolOutcome::err(format!(
                    "the sub-agent exceeded the {}s wall clock and was killed; give it a \
                     smaller task or raise NOOB_TASK_WALL_CLOCK_S",
                    cfg.wall_clock.as_secs()
                )),
                verbose,
                progress,
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let stdout_text = stdout_reader.join().unwrap_or_default();
    let progress = stderr_reader.join().unwrap_or_default();

    // The child's contract: exactly one JSON line on stdout. Parse the last
    // non-empty line so an accidental stray print does not break the parent.
    let result_line = stdout_text.lines().rev().find(|l| !l.trim().is_empty());
    let parsed = result_line.and_then(|line| serde_json::from_str::<Value>(line).ok());
    let Some(parsed) = parsed else {
        let code = status
            .map(|s| s.code().map_or("signal".to_string(), |c| c.to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        return with_progress(
            ToolOutcome::err(format!(
                "the sub-agent produced no result (exit {code}); its task may have \
                 crashed; retry with a simpler prompt"
            )),
            verbose,
            progress,
        );
    };
    let result = parsed.get("result").and_then(Value::as_str).unwrap_or("");
    let turns = parsed.get("turns").and_then(Value::as_u64).unwrap_or(0);
    let outcome = if parsed.get("status").and_then(Value::as_str) == Some("ok") {
        ToolOutcome::ok(result, format!("done ({turns} turns)"))
    } else {
        ToolOutcome::err(format!("sub-agent error: {result}"))
    };
    with_progress(outcome, verbose, progress)
}

fn clamp_max_turns(requested: u64, configured: u32) -> u32 {
    requested.clamp(1, u64::from(configured.max(1))) as u32
}

enum ChildInputError {
    Canceled,
    Timeout,
    Closed(String),
}

/// Send an arbitrarily large child prompt without letting a full pipe hide
/// cancellation or the task wall clock. No output-length policy is imposed;
/// every byte is written while the child continues reading.
fn write_child_input_with(
    stdin: &mut std::process::ChildStdin,
    mut bytes: &[u8],
    deadline: Instant,
    interrupted: impl Fn() -> bool,
) -> Result<(), ChildInputError> {
    use std::os::fd::AsRawFd;
    let fd = stdin.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(ChildInputError::Closed(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    while !bytes.is_empty() {
        if interrupted() {
            return Err(ChildInputError::Canceled);
        }
        if Instant::now() >= deadline {
            return Err(ChildInputError::Timeout);
        }
        match stdin.write(bytes) {
            Ok(0) => return Err(ChildInputError::Closed("the input pipe closed".into())),
            Ok(n) => bytes = &bytes[n..],
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(ChildInputError::Closed(error.to_string())),
        }
    }
    Ok(())
}

/// Ask the child to clean up its whole descendant tree, then force-kill and
/// reap if it does not exit promptly.
fn kill_group(child: &mut Child) {
    let pid = child.id() as libc::pid_t;
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break,
        }
    }
    kill_descendants(pid);
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Install only in `noob child`. PDEATHSIG sets the atomic from a minimal
/// signal handler; a normal watchdog thread then performs `/proc` traversal,
/// kills every descendant, and exits. No allocation or locking occurs in the
/// signal handler itself.
pub(crate) fn install_parent_death_cleanup() {
    extern "C" fn on_parent_death(_: libc::c_int) {
        PARENT_DIED.store(true, Ordering::SeqCst);
        INTERRUPTED.store(true, Ordering::SeqCst);
    }
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_parent_death as *const () as usize;
        action.sa_flags = 0;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut());
    }
    std::thread::Builder::new()
        .name("noob-parent-watchdog".to_string())
        .spawn(|| {
            loop {
                if PARENT_DIED.load(Ordering::SeqCst) {
                    let pid = unsafe { libc::getpid() };
                    for _ in 0..3 {
                        kill_descendants(pid);
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    unsafe { libc::_exit(143) };
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })
        .expect("spawn parent-death watchdog");
}

/// Kill the current snapshot of every Linux descendant, including processes
/// that created their own session. Children are kept alive while this scan
/// runs, so their descendants cannot be reparented out from under it first.
fn kill_descendants(root: libc::pid_t) {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    let mut parents = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some(rest) = stat.rsplit_once(')').map(|(_, rest)| rest) else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let _state = fields.next();
        let Some(parent) = fields.next().and_then(|field| field.parse().ok()) else {
            continue;
        };
        parents.push((pid, parent));
    }
    let mut tree = std::collections::HashSet::from([root]);
    loop {
        let before = tree.len();
        for &(pid, parent) in &parents {
            if tree.contains(&parent) {
                tree.insert(pid);
            }
        }
        if tree.len() == before {
            break;
        }
    }
    tree.remove(&root);
    for pid in tree {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

/// Read the child's single result line without an output-length cap.
fn read_all(mut stream: impl Read) -> String {
    let mut kept = Vec::new();
    let mut buf = [0u8; 8 * 1024];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => kept.extend_from_slice(&buf[..n]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    String::from_utf8(kept)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}

/// Capture child progress when verbose, else discard. This function only
/// reads memory and never touches the terminal; the completed ToolOutcome
/// is surfaced later by the parent's ordered UI path.
#[cfg(test)]
fn drain_stderr(mut stream: impl Read, verbose: bool) -> String {
    drain_stderr_with(&mut stream, verbose, |_| {})
}

fn drain_stderr_with(
    mut stream: impl Read,
    verbose: bool,
    mut on_progress: impl FnMut(&[u8]),
) -> String {
    if !verbose {
        let mut buf = [0u8; 8 * 1024];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => on_progress(&buf[..n]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        return String::new();
    }
    let mut kept = Vec::with_capacity(STDERR_CAP);
    let mut omitted = 0usize;
    let mut buf = [0u8; 8 * 1024];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                on_progress(&buf[..n]);
                let take = n.min(STDERR_CAP.saturating_sub(kept.len()));
                kept.extend_from_slice(&buf[..take]);
                omitted = omitted.saturating_add(n - take);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    let mut text = String::from_utf8_lossy(&kept).into_owned();
    if omitted > 0 {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!(
            "[task progress truncated: {omitted} bytes omitted]"
        ));
    }
    text
}

fn with_progress(mut outcome: ToolOutcome, verbose: bool, progress: String) -> ToolOutcome {
    if verbose && !progress.is_empty() {
        outcome.warning = Some(
            progress
                .lines()
                .map(|line| format!("[subagent] {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_ctx;

    fn cfg() -> TaskCfg {
        TaskCfg {
            depth: 0,
            concurrency: DEFAULT_CONCURRENCY,
            max_turns: DEFAULT_MAX_TURNS,
            wall_clock: Duration::from_secs(DEFAULT_WALL_CLOCK_S),
            verbose: false,
            background: None,
        }
    }

    #[test]
    fn without_task_cfg_the_tool_refuses() {
        let (_tmp, ctx) = test_ctx();
        let out = run(&ctx, &json!({"prompt": "do things"}));
        assert!(out.is_error);
        assert!(out.content.contains("not available here"));
    }

    #[test]
    fn argument_validation_teaches() {
        let (_tmp, mut ctx) = test_ctx();
        ctx.task = Some(cfg());
        let out = run(&ctx, &json!({}));
        assert!(
            out.content
                .contains("missing required parameter \"prompt\"")
        );
        let out = run(&ctx, &json!({"prompt": "  "}));
        assert!(out.content.contains("is empty"));
        let out = run(&ctx, &json!({"prompt": "x", "tools": "everything"}));
        assert!(out.content.contains("\"read-only\" or \"all\""));
    }

    #[test]
    fn oversized_turn_request_clamps_before_narrowing() {
        assert_eq!(clamp_max_turns(u64::MAX, 20), 20);
        assert_eq!(clamp_max_turns(0, 20), 1);
    }

    #[test]
    fn child_result_stdout_is_not_length_capped() {
        let big = vec![b'x'; 5 * 1024 * 1024];
        let got = read_all(std::io::Cursor::new(big));
        assert_eq!(got.len(), 5 * 1024 * 1024);
    }

    #[test]
    fn verbose_progress_is_captured_bounded_and_attached_after_completion() {
        let input = vec![b'x'; STDERR_CAP + 123];
        let progress = drain_stderr(std::io::Cursor::new(input), true);
        assert!(progress.len() < STDERR_CAP + 100);
        assert!(progress.contains("[task progress truncated: 123 bytes omitted]"));

        let out = with_progress(ToolOutcome::ok("done", "done"), true, progress);
        let warning = out.warning.unwrap();
        assert!(warning.starts_with("[subagent] "));
        assert!(warning.contains("123 bytes omitted"));
    }

    #[test]
    fn non_verbose_progress_is_drained_and_discarded() {
        let mut live = Vec::new();
        let progress =
            drain_stderr_with(std::io::Cursor::new(vec![b'x'; 100_000]), false, |bytes| {
                live.extend_from_slice(bytes)
            });
        assert!(progress.is_empty());
        assert_eq!(
            live.len(),
            100_000,
            "live UI receives progress even when retention is off"
        );
    }

    #[test]
    fn blocked_child_input_remains_cancelable() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        let mut child = Command::new("sh")
            .arg("-c")
            .arg("exec sleep 5")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let canceled = Arc::new(AtomicBool::new(false));
        let trigger = canceled.clone();
        let setter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            trigger.store(true, Ordering::SeqCst);
        });
        let input = vec![b'x'; 4 * 1024 * 1024];
        let started = Instant::now();
        let result = write_child_input_with(
            &mut stdin,
            &input,
            Instant::now() + Duration::from_secs(2),
            || canceled.load(Ordering::SeqCst),
        );
        setter.join().unwrap();
        let _ = child.kill();
        let _ = child.wait();

        assert!(matches!(result, Err(ChildInputError::Canceled)));
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    // Spawning real children is exercised end to end in tests/e2e_p6.rs;
    // unit tests here cannot use current_exe (it is the test harness, not
    // the noob binary).
}
