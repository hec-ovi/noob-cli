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

use super::truncate::{BASH_HEAD, BASH_TAIL, HeadTailBuffer};
use super::{ToolCtx, ToolOutcome, need_str, opt_u64};

const DEFAULT_TIMEOUT_S: u64 = 120;
const MAX_TIMEOUT_S: u64 = 600;

pub fn run(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    match run_inner(ctx, args) {
        Ok(out) => out,
        Err(msg) if msg.starts_with("command canceled by user") => {
            ToolOutcome::canceled_with(msg)
        }
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
    let collected = Arc::new(Mutex::new(HeadTailBuffer::new(BASH_HEAD, BASH_TAIL)));
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
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => {
                unsafe { libc::kill(-pid, libc::SIGKILL) };
                let _ = child.wait();
                break None;
            }
        }
        if INTERRUPTED.load(Ordering::SeqCst) {
            interrupted = true;
            unsafe { libc::kill(-pid, libc::SIGKILL) };
            let _ = child.wait();
            break None;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            unsafe { libc::kill(-pid, libc::SIGKILL) };
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
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
        stragglers_killed = true;
    }
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
    let summary = format!("bash {} ({:.1}s, exit {code})", brief(cmd), elapsed.as_secs_f32());
    let mut out = if code == 0 {
        if body.is_empty() {
            body = "(no output)".to_string();
        }
        ToolOutcome::ok(body, summary)
    } else {
        let mut out = ToolOutcome::err(format!("exit code {code}\n{body}"));
        out.summary = summary;
        out
    };
    out.warning = warning;
    Ok(out)
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
        let out = run(
            &ctx,
            &json!({"cmd": "echo one; echo two >&2; echo three"}),
        );
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(out.content, "one\ntwo\nthree\n");
        assert!(out.summary.starts_with("bash echo one; echo two >&2; echo three ("));
        assert!(out.summary.ends_with("exit 0)"));
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
        let out = run(&ctx, &json!({"cmd": "echo early; sleep 30", "timeout_s": 1}));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(out.is_error);
        assert!(out.content.contains("timed out after 1s and was killed"));
        assert!(out.content.contains("early"), "partial output kept: {}", out.content);
    }

    #[test]
    fn big_output_is_head_tail_truncated() {
        let (_t, ctx) = test_ctx();
        let out = run(&ctx, &json!({"cmd": "seq 1 20000"}));
        assert!(!out.is_error);
        assert!(out.content.len() <= BASH_HEAD + BASH_TAIL + 200);
        assert!(out.content.starts_with("1\n2\n"));
        assert!(out.content.trim_end().ends_with("20000"));
        assert!(out.content.contains("[output truncated:"));
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
            out.content.contains("[background processes left by the command were killed"),
            "{}",
            out.content
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
        let out = run(&ctx, &json!({"cmd": "head -c 300000 /dev/zero | tr '\\0' 'x'"}));
        assert!(!out.is_error);
        assert!(out.content.contains("xxx"));
    }
}
