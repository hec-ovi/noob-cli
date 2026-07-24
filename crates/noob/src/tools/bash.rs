//! bash: `bash -c <cmd>` in the workspace, stdout and stderr merged at the
//! file-descriptor level (real `2>&1` semantics), own process group so a
//! timeout or Ctrl-C kills the whole tree, tail-heavy truncation because
//! compilers and test runners put the verdict last.

use std::io::Read;
use std::os::fd::FromRawFd;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use noob_provider::http::INTERRUPTED;

use super::truncate::HeadTailBuffer;
use super::{ToolCtx, ToolOutcome, need_str, opt_u64};

const DEFAULT_TIMEOUT_S: u64 = 120;
const MAX_TIMEOUT_S: u64 = 600;

/// Interpreters and toolchains worth naming when a command is not found.
/// Deliberately short: this is a hint after a failure, not a catalog.
const PROBED: &[&str] = &[
    "python3", "node", "deno", "bun", "ruby", "perl", "go", "cargo", "gcc", "make", "jq",
];

/// What of `PROBED` is actually on PATH, resolved once per process.
///
/// A sandbox the model cannot see gets probed instead: in the local bake-off
/// 9 of 50 shell rounds were spent discovering the image (`which node`,
/// `ls /usr/bin/node*`) against a runtime that ships bash, git, python3 and
/// uv. Naming the set costs nothing until a command actually fails, which is
/// why it lives here and not in the environment block, where it would be paid
/// on every request forever.
fn available() -> &'static str {
    static AVAILABLE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    AVAILABLE.get_or_init(|| {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let dirs: Vec<_> = std::env::split_paths(&path).collect();
        PROBED
            .iter()
            .filter(|name| {
                dirs.iter().any(|dir| {
                    let candidate = dir.join(name);
                    std::fs::metadata(&candidate).is_ok_and(|m| m.is_file())
                })
            })
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    })
}

/// Exit 127 is "command not found". Answer the question the model is about to
/// spend a round asking.
fn not_found_hint(code: i32, body: &str) -> Option<String> {
    if code != 127 || !body.contains("not found") {
        return None;
    }
    Some(match available() {
        "" => "\nnone of the usual interpreters are on PATH here".to_string(),
        found => format!("\navailable here: {found}"),
    })
}

pub fn run(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    match run_inner(ctx, args) {
        Ok(out) => out,
        Err(msg) if msg.starts_with("command canceled by user") => ToolOutcome::canceled_with(msg),
        Err(msg) => ToolOutcome::err(msg),
    }
}

fn run_inner(ctx: &ToolCtx, args: &Value) -> Result<ToolOutcome, String> {
    let cmd = need_str(args, "cmd")?;
    if cmd.trim().is_empty() {
        return Err("cmd is empty; send the shell command to run".to_string());
    }
    let timeout_s = opt_u64(args, "timeout_s")?
        .unwrap_or(DEFAULT_TIMEOUT_S)
        .clamp(1, MAX_TIMEOUT_S);

    // One pipe; the child gets its write end as BOTH stdout and stderr, so
    // interleaving matches what a terminal would show. O_CLOEXEC is load-
    // bearing: without it a concurrently spawned sibling process inherits
    // the write end and the reader never sees EOF.
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err("cannot create a pipe for the command".to_string());
    }
    let (read_fd, write_fd) = (fds[0], fds[1]);
    let read_flags = unsafe { libc::fcntl(read_fd, libc::F_GETFL) };
    if read_flags < 0
        || unsafe { libc::fcntl(read_fd, libc::F_SETFL, read_flags | libc::O_NONBLOCK) } < 0
    {
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
        return Err("cannot configure the command output pipe".to_string());
    }
    let (stdout, stderr) = unsafe {
        let dup = libc::fcntl(write_fd, libc::F_DUPFD_CLOEXEC, 0);
        if dup < 0 {
            libc::close(read_fd);
            libc::close(write_fd);
            return Err("cannot duplicate the pipe for stderr".to_string());
        }
        (Stdio::from_raw_fd(write_fd), Stdio::from_raw_fd(dup))
    };

    let mut command = Command::new("bash");
    command
        .arg("-c")
        .arg(cmd)
        .current_dir(&ctx.workspace)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    unsafe {
        use std::os::unix::process::CommandExt;
        // New session = new process group; kill(-pgid) reaches every child.
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let started = Instant::now();
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            unsafe { libc::close(read_fd) };
            return Err(format!("cannot run bash: {e}"));
        }
    };
    // Command keeps the Stdio fds so it could be re-spawned; drop it NOW or
    // the parent's copies of the write ends stay open and the reader never
    // sees EOF.
    drop(command);
    // Read until EOF on a thread so a fast producer can never fill the pipe
    // and deadlock against try_wait. The buffer is shared: when a background
    // survivor holds the pipe open past the grace window, the partial output
    // is still recoverable without joining.
    let mut reader = unsafe { std::fs::File::from_raw_fd(read_fd) };
    let collected = Arc::new(Mutex::new(HeadTailBuffer::new(
        ctx.caps.bash_head,
        ctx.caps.bash_tail,
    )));
    let eof_seen = Arc::new(AtomicBool::new(false));
    // Set when the tool gives up on the pipe (a setsid escapee can hold it
    // open forever): the collector then discards instead of buffering, so
    // an abandoned reader can never grow memory without bound.
    let abandoned = Arc::new(AtomicBool::new(false));
    let (t_buf, t_eof, t_gone) = (collected.clone(), eof_seen.clone(), abandoned.clone());
    let collector = std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            if t_gone.load(Ordering::SeqCst) {
                break;
            }
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    if !t_gone.load(Ordering::SeqCst) {
                        t_buf.lock().unwrap().extend(&chunk[..n]);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
        t_eof.store(true, Ordering::SeqCst);
    });

    let pid = child.id() as i32;
    let deadline = started + Duration::from_secs(timeout_s);
    let mut timed_out = false;
    let mut interrupted = false;
    let mut group_killed = false;
    // Exit is detected with waitid(WNOWAIT), which leaves the leader a
    // zombie: the zombie pins the pgid, so every group SIGKILL below
    // (including the post-exit straggler kill) fires before the leader is
    // reaped and can never hit a recycled process group. The real reap
    // (child.wait) happens once, after the last possible group kill.
    loop {
        match leader_exited(pid) {
            Ok(true) => break,
            Ok(false) => {}
            Err(_) => {
                unsafe { libc::kill(-pid, libc::SIGKILL) };
                group_killed = true;
                break;
            }
        }
        if INTERRUPTED.load(Ordering::SeqCst) {
            interrupted = true;
            unsafe { libc::kill(-pid, libc::SIGKILL) };
            group_killed = true;
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            unsafe { libc::kill(-pid, libc::SIGKILL) };
            group_killed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let elapsed = started.elapsed();

    // EOF comes when every write end closes. A backgrounded survivor
    // ("server &") would hold the pipe forever: background bash is out of
    // scope for v0.1, so after a short grace the whole group is killed to
    // keep the tool call synchronous. If something escaped the group
    // (setsid), abandon the reader and keep the partial output.
    let wait_eof = |window: Duration| {
        let deadline = Instant::now() + window;
        while !eof_seen.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        eof_seen.load(Ordering::SeqCst)
    };
    let mut stragglers_killed = false;
    if !timed_out && !interrupted && !wait_eof(Duration::from_millis(200)) {
        unsafe { libc::kill(-pid, libc::SIGKILL) };
        group_killed = true;
        stragglers_killed = true;
    }
    // Group kills are done: reap the leader (SIGKILL on the zombie above
    // was a no-op, so a real exit code survives the straggler kill), then
    // collect group members that reparented to this process.
    let status = child.wait().ok();
    reap_group_zombies(
        pid,
        if group_killed {
            Duration::from_millis(500)
        } else {
            Duration::ZERO
        },
    );
    let eof = wait_eof(Duration::from_millis(500));
    if eof {
        let _ = collector.join();
    } else {
        // Something escaped the process group (setsid) and still holds the
        // pipe. Stop and join the non-blocking collector; the escapee was
        // NOT killed and the result must say so honestly.
        abandoned.store(true, Ordering::SeqCst);
        let _ = collector.join();
    }

    let mut body = collected.lock().unwrap().render();
    if !eof {
        body.push_str(
            "\n[a background process started by the command is still running and holding \
             its output open; the extra output is discarded; keep commands foreground-only]",
        );
    } else if stragglers_killed {
        body.push_str(
            "\n[background processes left by the command were killed when it finished; \
             keep commands foreground-only]",
        );
    }

    if interrupted {
        return Err(format!(
            "command canceled by user after {:.1}s; partial output:\n{body}",
            elapsed.as_secs_f32()
        ));
    }
    if timed_out {
        return Err(format!(
            "command timed out after {timeout_s}s and was killed; raise timeout_s \
             (max {MAX_TIMEOUT_S}) or run something faster; partial output:\n{body}"
        ));
    }
    // One-time workspace-mode warning, UI-only (never in the transcript).
    // Attached only when a command actually ran, so an early parameter
    // error cannot consume the one-shot silently.
    let warning = (ctx.sandbox == super::guard::Sandbox::Workspace
        && !ctx.bash_warned.swap(true, Ordering::SeqCst))
    .then(|| "no sandbox: commands run directly on your host".to_string());
    let code = status.and_then(|s| s.code()).unwrap_or(-1);
    let summary = format!(
        "bash {} ({:.1}s, exit {code})",
        brief(cmd),
        elapsed.as_secs_f32()
    );
    let mut out = if code == 0 {
        if body.is_empty() {
            body = "(no output)".to_string();
        }
        ToolOutcome::ok(body, summary)
    } else {
        let hint = not_found_hint(code, &body).unwrap_or_default();
        let mut out = ToolOutcome::err(format!("exit code {code}\n{body}{hint}"));
        out.summary = summary;
        out
    };
    out.warning = warning;
    Ok(out)
}

/// Has the leader exited? Polled with waitid + WNOWAIT so the process stays
/// an unreaped zombie: reaping it would free its pgid for reuse and a later
/// kill(-pid) could reach an unrelated, freshly spawned group.
fn leader_exited(pid: i32) -> Result<bool, ()> {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if rc != 0 {
        return Err(());
    }
    // WNOHANG with no state change returns 0 and leaves si_pid zeroed.
    Ok(unsafe { info.si_pid() } == pid)
}

/// Collect every already-exited member of the command's process group that
/// reparented to this process. When noob is pid 1 (the container case)
/// orphaned grandchildren land here as zombies and nothing else ever waits
/// on them. waitpid on the NEGATIVE pgid reaps exactly those, without ever
/// touching children other threads own (MCP servers and sub-agents run in
/// their own groups), and errors with ECHILD on a host run where orphans
/// reparent to the real init instead. Must run AFTER the leader was reaped:
/// -pgid matches the leader too, and stealing its status would break the
/// exit code. Best-effort within `window`: a member that survives (setsid
/// escapee, an unkilled background process) stays unreaped until it exits
/// after a later call or the process ends; that residue is a known limit.
fn reap_group_zombies(pgid: i32, window: Duration) {
    let deadline = Instant::now() + window;
    loop {
        match unsafe { libc::waitpid(-pgid, std::ptr::null_mut(), libc::WNOHANG) } {
            0 => {
                // Members remain but none is waitable yet.
                if Instant::now() >= deadline {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            rc if rc > 0 => {} // reaped one member; keep draining
            _ => return,       // ECHILD: nothing of ours left in the group
        }
    }
}

/// First few words of the command for the one-line UI summary.
fn brief(cmd: &str) -> String {
    let one_line = cmd.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > 40 {
        let cut: String = one_line.chars().take(40).collect();
        format!("{cut}…")
    } else {
        one_line
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_ctx;
    use super::*;
    use serde_json::json;

    #[test]
    fn merged_output_in_order_and_exit_zero() {
        let (_t, ctx) = test_ctx();
        let out = run(&ctx, &json!({"cmd": "echo one; echo two >&2; echo three"}));
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(out.content, "one\ntwo\nthree\n");
        assert!(
            out.summary
                .starts_with("bash echo one; echo two >&2; echo three (")
        );
        assert!(out.summary.ends_with("exit 0)"));
    }

    /// A missing command answers the follow-up question in the same round,
    /// instead of costing a `which`/`ls /usr/bin` discovery round.
    #[test]
    fn a_missing_command_names_what_is_available() {
        let (_t, ctx) = test_ctx();
        let out = run(&ctx, &json!({"cmd": "definitely-not-a-real-binary-xyz"}));
        assert!(out.is_error, "{}", out.content);
        assert!(
            out.content.starts_with("exit code 127\n"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("available here:")
                || out.content.contains("none of the usual interpreters"),
            "127 must name the sandbox contents: {}",
            out.content
        );
    }

    /// The hint is scoped to 127. Every other failure keeps its body clean.
    #[test]
    fn other_exit_codes_carry_no_inventory() {
        let (_t, ctx) = test_ctx();
        let out = run(&ctx, &json!({"cmd": "echo 'file not found' >&2; exit 2"}));
        assert!(out.is_error);
        assert!(!out.content.contains("available here:"), "{}", out.content);
    }

    #[test]
    fn nonzero_exit_is_an_error_with_the_code_first() {
        let (_t, ctx) = test_ctx();
        let out = run(&ctx, &json!({"cmd": "echo boom >&2; exit 3"}));
        assert!(out.is_error);
        assert!(out.content.starts_with("exit code 3\n"));
        assert!(out.content.contains("boom"));
    }

    #[test]
    fn runs_in_the_workspace() {
        let (_t, ctx) = test_ctx();
        std::fs::write(ctx.workspace.join("marker.txt"), "").unwrap();
        let out = run(&ctx, &json!({"cmd": "ls"}));
        assert!(out.content.contains("marker.txt"));
    }

    #[test]
    fn timeout_kills_the_whole_process_group() {
        let (_t, ctx) = test_ctx();
        let started = std::time::Instant::now();
        // The sleep is a CHILD of bash; only a group kill reaches it.
        let out = run(
            &ctx,
            &json!({"cmd": "echo early; sleep 30", "timeout_s": 1}),
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(out.is_error);
        assert!(out.content.contains("timed out after 1s and was killed"));
        assert!(
            out.content.contains("early"),
            "partial output kept: {}",
            out.content
        );
    }

    #[test]
    fn big_output_is_head_tail_truncated() {
        use super::super::truncate::{BASH_HEAD, BASH_TAIL};

        let (_t, ctx) = test_ctx();
        let out = run(&ctx, &json!({"cmd": "seq 1 20000"}));
        assert!(!out.is_error);
        assert!(out.content.len() <= BASH_HEAD + BASH_TAIL + 200);
        assert!(out.content.starts_with("1\n2\n"));
        assert!(out.content.trim_end().ends_with("20000"));
        assert!(out.content.contains("[output truncated:"));
    }

    #[test]
    fn uncapped_ctx_keeps_big_output_whole() {
        let (_t, mut ctx) = test_ctx();
        ctx.caps = super::super::truncate::Caps::uncapped();
        let out = run(&ctx, &json!({"cmd": "seq 1 20000"}));
        assert!(!out.is_error);
        assert!(!out.content.contains("[output truncated:"));
        // seq 1..20000 is ~108 KiB; the whole stream survives.
        assert!(out.content.starts_with("1\n2\n"));
        assert!(out.content.contains("\n10000\n"));
        assert!(out.content.trim_end().ends_with("20000"));
    }

    #[test]
    fn empty_output_is_stated() {
        let (_t, ctx) = test_ctx();
        let out = run(&ctx, &json!({"cmd": "true"}));
        assert_eq!(out.content, "(no output)");
    }

    #[test]
    fn background_survivor_does_not_hang_the_tool() {
        let (_t, ctx) = test_ctx();
        let started = std::time::Instant::now();
        // The backgrounded sleep inherits the pipe; without the grace+kill
        // the collector would wait 30s for EOF.
        let out = run(&ctx, &json!({"cmd": "sleep 30 & echo started"}));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "hung on the background survivor"
        );
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("started"));
        assert!(
            out.content
                .contains("[background processes left by the command were killed"),
            "{}",
            out.content
        );
    }

    #[test]
    fn straggler_kill_preserves_the_leader_exit_code() {
        let (_t, ctx) = test_ctx();
        // The leader exits 7 before the straggler group kill; reaping is
        // deferred until after that kill (the zombie pins the pgid) and
        // the real exit code must survive the deferral.
        let out = run(&ctx, &json!({"cmd": "sleep 30 & echo started; exit 7"}));
        assert!(out.is_error);
        assert!(out.content.starts_with("exit code 7\n"), "{}", out.content);
        assert!(out.content.contains("were killed"), "{}", out.content);
    }

    #[test]
    fn group_zombies_reparented_to_this_process_are_reaped() {
        let (_t, ctx) = test_ctx();
        // Mimic the container, where noob runs as pid 1 and orphaned
        // grandchildren reparent to it: a subreaper receives them the same
        // way without needing to be pid 1.
        unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1u64) };
        let out = run(&ctx, &json!({"cmd": "echo $$; sleep 30 &"}));
        assert!(!out.is_error, "{}", out.content);
        let leader: i32 = out.content.lines().next().unwrap().trim().parse().unwrap();
        // The backgrounded sleep was group-killed and reparented here; the
        // post-kill drain must have reaped it, leaving nothing waitable in
        // the command's group.
        let rc = unsafe { libc::waitpid(-leader, std::ptr::null_mut(), libc::WNOHANG) };
        assert_eq!(
            rc, -1,
            "an unreaped zombie from the command's group remains"
        );
    }

    #[test]
    fn setsid_escapee_reports_honestly_and_returns_promptly() {
        let (_t, ctx) = test_ctx();
        let started = std::time::Instant::now();
        // The escapee leaves the process group, survives the straggler
        // kill, and holds the pipe; the tool must return anyway and must
        // NOT claim it was killed.
        let out = run(&ctx, &json!({"cmd": "setsid sleep 2 & echo hi"}));
        assert!(started.elapsed() < Duration::from_secs(2), "did not detach");
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("hi"));
        assert!(
            out.content.contains("is still running and holding"),
            "{}",
            out.content
        );
        assert!(!out.content.contains("were killed"), "{}", out.content);
    }

    #[test]
    fn early_param_error_does_not_consume_the_sandbox_warning() {
        let (_t, mut ctx) = test_ctx();
        ctx.sandbox = super::super::guard::Sandbox::Workspace;
        let bad = run(&ctx, &json!({"cmd": "true", "timeout_s": "potato"}));
        assert!(bad.is_error);
        assert!(bad.warning.is_none());
        let ok = run(&ctx, &json!({"cmd": "true"}));
        assert_eq!(
            ok.warning.as_deref(),
            Some("no sandbox: commands run directly on your host")
        );
        // One-time: the second successful run stays quiet.
        let again = run(&ctx, &json!({"cmd": "true"}));
        assert!(again.warning.is_none());
    }

    #[test]
    fn no_output_capture_deadlock_on_fast_huge_writers() {
        let (_t, ctx) = test_ctx();
        // >64 KiB (default pipe capacity) written at once: hangs if the
        // parent waits before draining.
        let out = run(
            &ctx,
            &json!({"cmd": "head -c 300000 /dev/zero | tr '\\0' 'x'"}),
        );
        assert!(!out.is_error);
        assert!(out.content.contains("xxx"));
    }
}
