//! bash: `bash -c <cmd>` in the workspace. The process lifecycle (merged
//! stdout/stderr, own session, timeout, tail-heavy truncation) lives in
//! `exec`; what is here is the shell-specific part: the argv, the
//! command-not-found hint, and the one-time no-sandbox warning.

use std::process::Command;
use std::sync::atomic::Ordering;

use serde_json::Value;

use super::exec;
use super::{ToolCtx, ToolOutcome, fail, need_str, opt_u64};

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
    // Both sides are the outcome: a failure is classified where it is minted,
    // which is the only place that knows whether it was the model's argument,
    // the deadline, or the command's own exit status.
    match run_inner(ctx, args) {
        Ok(out) | Err(out) => out,
    }
}

/// A parameter the model got wrong, which is a different failure from anything
/// the command itself can do.
fn arg_error(message: String) -> ToolOutcome {
    ToolOutcome::err(message).classed(fail::INVALID_ARGUMENT)
}

// Both variants are the same type, so there is no smaller shape to move to:
// the usual advice (box the error) would only add an allocation to every
// failure and leave the success just as large.
#[allow(clippy::result_large_err)]
fn run_inner(ctx: &ToolCtx, args: &Value) -> Result<ToolOutcome, ToolOutcome> {
    let cmd = need_str(args, "cmd").map_err(arg_error)?;
    if cmd.trim().is_empty() {
        return Err(arg_error(
            "cmd is empty; send the shell command to run".to_string(),
        ));
    }
    let timeout_s = opt_u64(args, "timeout_s")
        .map_err(arg_error)?
        .unwrap_or(DEFAULT_TIMEOUT_S)
        .clamp(1, MAX_TIMEOUT_S);

    let mut command = Command::new("bash");
    command.arg("-c").arg(cmd).current_dir(&ctx.workspace);
    let run = match exec::run(
        command,
        "bash",
        timeout_s,
        ctx.caps.bash_head,
        ctx.caps.bash_tail,
        crate::emit::Progress::for_current_call(&ctx.emitter),
    ) {
        Ok(run) => run,
        Err(exec::RunError::Spawn(message)) => {
            return Err(ToolOutcome::err(message).classed(fail::INTERNAL));
        }
        Err(exec::RunError::Canceled { body, elapsed }) => {
            return Err(ToolOutcome::canceled_with(format!(
                "command canceled by user after {:.1}s; partial output:\n{body}",
                elapsed.as_secs_f32()
            )));
        }
        Err(exec::RunError::TimedOut { body, timeout_s }) => {
            return Err(ToolOutcome::err(format!(
                "command timed out after {timeout_s}s and was killed; raise timeout_s \
                 (max {MAX_TIMEOUT_S}) or run something faster; partial output:\n{body}"
            ))
            .classed(fail::TIMEOUT)
            .remedy(format!(
                "raise timeout_s (max {MAX_TIMEOUT_S}) or run something faster"
            )));
        }
    };
    let (mut body, code, elapsed) = (run.body, run.code, run.elapsed);

    // One-time workspace-mode warning, UI-only (never in the transcript).
    // Attached only when a command actually ran, so an early parameter
    // error cannot consume the one-shot silently.
    let warning = (ctx.sandbox == super::guard::Sandbox::Workspace
        && !ctx.bash_warned.swap(true, Ordering::SeqCst))
    .then(|| "no sandbox: commands run directly on your host".to_string());
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
        let mut out = ToolOutcome::err(format!("exit code {code}\n{body}{hint}"))
            .classed(fail::EXIT_STATUS)
            .coded(code);
        // The one exit status whose next action is knowable from the number
        // alone. Anything else, the command already said why.
        if code == 127 {
            out.remedy = Some(match available() {
                "" => "none of the usual interpreters are on PATH here".to_string(),
                found => format!("available here: {found}"),
            });
        }
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
    use std::time::{Duration, Instant};

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

    /// The one supported way to leave a server running: setsid for its own
    /// process group, and stdio off the pipe so the call still sees EOF. It
    /// must survive the straggler kill and must not earn either warning.
    #[test]
    fn a_detached_daemon_survives_with_a_clean_result() {
        let (_t, ctx) = test_ctx();
        let pidfile = ctx.workspace.join("daemon.pid");
        let started = std::time::Instant::now();
        let out = run(
            &ctx,
            &json!({"cmd": format!(
                "setsid sh -c 'echo $$ > {p}; sleep 30' </dev/null >/dev/null 2>&1 & echo up",
                p = pidfile.display()
            )}),
        );
        assert!(started.elapsed() < Duration::from_secs(2), "did not detach");
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("up"), "{}", out.content);
        assert!(!out.content.contains("were killed"), "{}", out.content);
        assert!(
            !out.content.contains("is still running and holding"),
            "redirected stdio must not hold the pipe: {}",
            out.content
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        while !pidfile.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let pid: i32 = std::fs::read_to_string(&pidfile)
            .expect("daemon wrote no pid")
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            0,
            "the detached daemon was killed with the call"
        );
        unsafe { libc::kill(pid, libc::SIGKILL) };
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
