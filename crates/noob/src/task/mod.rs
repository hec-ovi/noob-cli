//! Multi-agent (P6): the `task` tool spawns the binary itself
//! (`current_exe() child`). The process boundary is the context boundary:
//! the payload goes to the child's stdin as one JSON object, exactly one
//! JSON result line comes back on stdout, progress flows on stderr, and
//! only the result string enters the parent transcript. argv + stdin +
//! stdout is the whole IPC surface.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use noob_provider::http::INTERRUPTED;
use noob_provider::types::ToolSpec;

use crate::tools::{ToolCtx, ToolOutcome, need_str, opt_str, opt_u64};

/// Recursion ceiling: depth 0 (the user's agent) and depth 1 children may
/// spawn; at depth 2 the task tool is simply not registered.
pub const MAX_DEPTH: u32 = 2;
pub const DEFAULT_CONCURRENCY: usize = 4;
pub const DEFAULT_MAX_TURNS: u32 = 25;
/// Per-child wall clock; the parent kills the whole process group on expiry.
pub const DEFAULT_WALL_CLOCK_S: u64 = 300;

/// Bound on the child's stdout (the single result line lives there; a child
/// that floods stdout is broken and gets cut off, not buffered forever).
const STDOUT_CAP: usize = 4 * 1024 * 1024;

/// Session-scoped sub-agent settings, resolved once at bootstrap.
#[derive(Clone, Debug)]
pub struct TaskCfg {
    /// This process's depth (NOOB_DEPTH, 0 for the user's agent).
    pub depth: u32,
    pub concurrency: usize,
    pub max_turns: u32,
    pub wall_clock: Duration,
    /// Relay child stderr as `[task] ...` lines instead of discarding it.
    pub verbose: bool,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "task".to_string(),
        description: "Spawn a sub-agent with a fresh context; it works alone and returns \
                      one result message."
            .to_string(),
        parameters: json!({"type": "object", "properties": {
            "prompt": {"type": "string", "description": "complete standalone instructions"},
            "tools": {"type": "string", "enum": ["read-only", "all"],
                      "description": "default read-only"},
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
        Ok(p) if !p.trim().is_empty() => p,
        Ok(_) => return ToolOutcome::err("parameter \"prompt\" is empty; resend the call"),
        Err(e) => return ToolOutcome::err(e),
    };
    let tools_mode = match opt_str(args, "tools") {
        Ok(None) => "read-only",
        Ok(Some(m @ ("read-only" | "all"))) => m,
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
        Ok(Some(n)) => (n as u32).clamp(1, cfg.max_turns),
        Ok(None) => cfg.max_turns,
        Err(e) => return ToolOutcome::err(e),
    };

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => return ToolOutcome::err(format!("cannot locate the noob binary: {e}")),
    };
    let mut child = match Command::new(exe)
        .arg("child")
        .env("NOOB_DEPTH", (cfg.depth + 1).to_string())
        .current_dir(&ctx.workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return ToolOutcome::err(format!("cannot spawn the sub-agent: {e}")),
    };

    // One JSON object in, then EOF: the child reads stdin to end.
    let payload = json!({"prompt": prompt, "tools": tools_mode, "max_turns": max_turns});
    {
        let mut stdin = child.stdin.take().expect("piped stdin");
        if stdin.write_all(format!("{payload}\n").as_bytes()).is_err() {
            kill_group(&mut child);
            return ToolOutcome::err(
                "the sub-agent exited before reading its task; try again",
            );
        }
    } // drop closes the pipe

    // Readers on threads so a chatty child never deadlocks on a full pipe.
    let stdout = child.stdout.take().expect("piped stdout");
    let stdout_reader = std::thread::spawn(move || read_capped(stdout, STDOUT_CAP));
    let stderr = child.stderr.take().expect("piped stderr");
    let verbose = cfg.verbose;
    let stderr_reader = std::thread::spawn(move || drain_stderr(stderr, verbose));

    // The wait loop owns the three exits: completion, wall clock, Ctrl-C.
    let deadline = Instant::now() + cfg.wall_clock;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
        if INTERRUPTED.load(Ordering::SeqCst) {
            kill_group(&mut child);
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return ToolOutcome::err("canceled by user");
        }
        if Instant::now() >= deadline {
            kill_group(&mut child);
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return ToolOutcome::err(format!(
                "the sub-agent exceeded the {}s wall clock and was killed; give it a \
                 smaller task or raise NOOB_TASK_WALL_CLOCK_S",
                cfg.wall_clock.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let stdout_text = stdout_reader.join().unwrap_or_default();
    let _ = stderr_reader.join();

    // The child's contract: exactly one JSON line on stdout. Parse the last
    // non-empty line so an accidental stray print does not break the parent.
    let result_line = stdout_text.lines().rev().find(|l| !l.trim().is_empty());
    let parsed = result_line.and_then(|l| serde_json::from_str::<Value>(l).ok());
    let Some(parsed) = parsed else {
        let code = status
            .map(|s| s.code().map_or("signal".to_string(), |c| c.to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        return ToolOutcome::err(format!(
            "the sub-agent produced no result (exit {code}); its task may have \
             crashed; retry with a simpler prompt"
        ));
    };
    let result = parsed.get("result").and_then(Value::as_str).unwrap_or("");
    let turns = parsed.get("turns").and_then(Value::as_u64).unwrap_or(0);
    if parsed.get("status").and_then(Value::as_str) == Some("ok") {
        ToolOutcome::ok(result, format!("task done ({turns} turns)"))
    } else {
        ToolOutcome::err(format!("sub-agent error: {result}"))
    }
}

/// SIGKILL the child's whole process group (it was spawned with
/// `process_group(0)`), then reap.
fn kill_group(child: &mut Child) {
    let pid = child.id() as libc::pid_t;
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Read a stream to end, keeping at most `cap` bytes (the rest is drained
/// and dropped so the child never blocks on a full pipe).
fn read_capped(mut stream: impl Read, cap: usize) -> String {
    let mut kept = Vec::new();
    let mut buf = [0u8; 8 * 1024];
    loop {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if kept.len() < cap {
                    let take = n.min(cap - kept.len());
                    kept.extend_from_slice(&buf[..take]);
                }
            }
        }
    }
    String::from_utf8_lossy(&kept).into_owned()
}

/// Child progress: relay as dim `[task]` lines when verbose, else discard.
/// Either way the pipe is drained to end so the child cannot wedge on it.
fn drain_stderr(stream: impl Read, verbose: bool) {
    let mut reader = BufReader::new(stream);
    if !verbose {
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
        return;
    }
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => eprintln!("[task] {}", line.trim_end()),
        }
    }
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
        assert!(out.content.contains("missing required parameter \"prompt\""));
        let out = run(&ctx, &json!({"prompt": "  "}));
        assert!(out.content.contains("is empty"));
        let out = run(&ctx, &json!({"prompt": "x", "tools": "everything"}));
        assert!(out.content.contains("\"read-only\" or \"all\""));
    }

    #[test]
    fn read_capped_keeps_the_head_and_drains_the_rest() {
        let big = vec![b'x'; 100];
        let got = read_capped(std::io::Cursor::new(big), 10);
        assert_eq!(got.len(), 10);
    }

    // Spawning real children is exercised end to end in tests/e2e_p6.rs;
    // unit tests here cannot use current_exe (it is the test harness, not
    // the noob binary).
}
