//! Run one child process to completion and collect its merged output.
//!
//! Shared by every tool that shells out (`bash`, `websearch`). The care here
//! is all about not leaving anything behind: the child gets its own session so
//! a timeout or Ctrl-C kills the whole tree, stdout and stderr share one pipe
//! so interleaving matches a terminal, the reader runs on its own thread so a
//! fast producer cannot deadlock against the wait, and the leader is polled
//! with WNOWAIT so its pgid cannot be recycled under a later group kill.
//!
//! The caller owns the argv and the summary; this owns the process lifecycle.

use std::io::Read;
use std::os::fd::FromRawFd;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use noob_provider::http::INTERRUPTED;

use super::truncate::HeadTailBuffer;

/// What one finished command left behind. `body` is the merged, truncated
/// output, already carrying any note about escaped background processes.
pub(crate) struct Run {
    pub code: i32,
    pub body: String,
    pub elapsed: Duration,
}

/// Cancellation and timeout are errors, not exit codes: both mean the output
/// is partial and the caller must say so rather than report a verdict.
pub(crate) enum RunError {
    Canceled { body: String, elapsed: Duration },
    TimedOut { body: String, timeout_s: u64 },
    Spawn(String),
}

/// Run `command` with a deadline, returning its merged output.
///
/// `program` names the binary for the spawn-failure message; `head`/`tail` are
/// the truncation budget for the collected output. `progress` taps the same
/// stream for anything watching: the collector already has every byte as it
/// arrives, and nothing else in the process does, so a build scrolls live
/// instead of appearing all at once when it ends. None when nothing is
/// watching, which is the default and costs a branch per chunk.
pub(crate) fn run(
    mut command: Command,
    program: &str,
    timeout_s: u64,
    head: usize,
    tail: usize,
    progress: Option<crate::emit::Progress>,
) -> Result<Run, RunError> {
    // One pipe; the child gets its write end as BOTH stdout and stderr, so
    // interleaving matches what a terminal would show. O_CLOEXEC is load-
    // bearing: without it a concurrently spawned sibling process inherits
    // the write end and the reader never sees EOF.
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(RunError::Spawn(
            "cannot create a pipe for the command".to_string(),
        ));
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
        return Err(RunError::Spawn(
            "cannot configure the command output pipe".to_string(),
        ));
    }
    let (stdout, stderr) = unsafe {
        let dup = libc::fcntl(write_fd, libc::F_DUPFD_CLOEXEC, 0);
        if dup < 0 {
            libc::close(read_fd);
            libc::close(write_fd);
            return Err(RunError::Spawn(
                "cannot duplicate the pipe for stderr".to_string(),
            ));
        }
        (Stdio::from_raw_fd(write_fd), Stdio::from_raw_fd(dup))
    };

    command.stdin(Stdio::null()).stdout(stdout).stderr(stderr);
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
            return Err(RunError::Spawn(format!("cannot run {program}: {e}")));
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
    let collected = Arc::new(Mutex::new(HeadTailBuffer::new(head, tail)));
    let eof_seen = Arc::new(AtomicBool::new(false));
    // Set when the tool gives up on the pipe (a setsid escapee can hold it
    // open forever): the collector then discards instead of buffering, so
    // an abandoned reader can never grow memory without bound.
    let abandoned = Arc::new(AtomicBool::new(false));
    let (t_buf, t_eof, t_gone) = (collected.clone(), eof_seen.clone(), abandoned.clone());
    let collector = std::thread::spawn(move || {
        let mut progress = progress;
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
                        // After the buffer, never instead of it: the model's
                        // copy of the output is the one that must not depend
                        // on whether anybody is watching.
                        if let Some(progress) = progress.as_mut() {
                            progress.feed(&chunk[..n]);
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
        // Before the tap is drained, not after. Everything the caller decides
        // from here (whether the pipe reached EOF, and therefore whether the
        // command left something behind) must be settled while nothing can
        // block, or a watcher that reads slowly changes what the model is
        // told about a command that ran perfectly.
        t_eof.store(true, Ordering::SeqCst);
        // Now it can block. `run` joins this thread, so draining here is also
        // what keeps a call's live lines ahead of the frame that closes it.
        if let Some(progress) = progress.take() {
            progress.finish();
        }
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
        return Err(RunError::Canceled { body, elapsed });
    }
    if timed_out {
        return Err(RunError::TimedOut { body, timeout_s });
    }
    Ok(Run {
        code: status.and_then(|s| s.code()).unwrap_or(-1),
        body,
        elapsed,
    })
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
