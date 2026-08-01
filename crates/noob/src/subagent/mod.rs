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
use noob_provider::types::{Overrides, ToolSpec};

use crate::tools::{ToolOutcome, need_str, opt_str};

mod background;
#[cfg(test)]
pub use background::JobProgressSnapshot;
pub use background::{BackgroundHub, JobsSnapshot, ReadyResult};

/// Recursion ceiling: depth 0 (the user's agent) and depth 1 children may
/// spawn; at depth 2 the subagent tool is simply not registered.
pub const MAX_DEPTH: u32 = 2;
pub const DEFAULT_CONCURRENCY: usize = 4;
/// Per-child inference-round budget. 0 means no limit, and it is the
/// default: a bounded child dies mid-task with nothing to show, which costs
/// more than the rounds the bound was saving. What actually stops a child is
/// finishing, a cancel, or the parent exiting; NOOB_TASK_MAX_TURNS still
/// caps it for the tests and for anyone who wants a ceiling back.
pub const DEFAULT_MAX_TURNS: u32 = 0;
/// Tools a child gets when the model does not say: every tool. The narrower
/// modes exist for the model to choose deliberately; a padded or omitted
/// field must not strand a child that was asked to edit files in a
/// read-only harness. NOOB_TASK_TOOLS restores a narrower default.
pub const DEFAULT_TOOLS: &str = "all";
/// Per-child wall clock; the parent kills the whole process group on expiry.
/// 0 means no limit: the child runs until it finishes or is interrupted.
pub const DEFAULT_WALL_CLOCK_S: u64 = 0;
/// The child's stdin bound: `noob child` reads its task via
/// `take(CHILD_STDIN_CAP)`, so a bigger payload arrives truncated, fails to
/// parse, and surfaces as a misleading "no task" error. The parent
/// pre-checks against the same bound and reports the real reason instead.
pub(crate) const CHILD_STDIN_CAP: u64 = 8 * 1024 * 1024;

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
    /// Per-child round budget; 0 is unbounded.
    pub max_turns: u32,
    /// The tools mode a spawn gets when the model omits `tools`.
    pub tools_default: String,
    pub wall_clock: Duration,
    /// Surface bounded child stderr as `[subagent] ...` after the child exits.
    pub verbose: bool,
    /// Effective root CLI overrides. Children receive these over the private
    /// stdin protocol so a root `--model` or `--base-url` cannot silently send
    /// detached work to a different provider.
    pub overrides: Overrides,
    /// Preserve the root sandbox decision across the process boundary.
    pub yolo: bool,
    /// Skills loaded by ancestors are orchestration context, not child task
    /// context. Keep the whole chain so nested children cannot rediscover and
    /// recursively execute the skill that spawned them.
    pub ancestor_skills: Vec<String>,
    /// Present only for the default interactive dock. Other surfaces keep the
    /// original inline child contract.
    pub background: Option<BackgroundHub>,
}

#[derive(Clone)]
struct TaskRequest {
    prompt: String,
    /// What the model actually asked for, before `child_prompt` wrapped it in
    /// the runtime brief. The wrapper is four hundred characters of harness
    /// instruction and is the same on every child, so it is the one thing that
    /// must never be what a watcher is shown as the task.
    asked: String,
    tools_mode: String,
    max_turns: u32,
    excluded_skills: Vec<String>,
}

#[derive(Clone)]
struct RunCfg {
    /// Exact depth passed to the child. Dock jobs are structural leaves so a
    /// root fleet cannot multiply behind the user's back or oversubscribe the
    /// provider slots that `noob doctor` validated.
    child_depth: u32,
    wall_clock: Duration,
    verbose: bool,
    overrides: Overrides,
    yolo: bool,
    workspace: std::path::PathBuf,
    progress: Option<background::ProgressLog>,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "subagent".to_string(),
        description: "Spawn, inspect, or cancel detached sub-agents; reports return automatically."
            .to_string(),
        parameters: json!({"type": "object", "properties": {
            "prompt": {"type": "string", "description": "complete standalone instructions"},
            "tools": {"type": "string", "enum": ["read-only", "web", "all"],
                      "description": "read-only limits to inspection; web adds the websearch tool without mutations; all (the default) adds Bash and file changes"},
            "status": {"type": "boolean",
                       "description": "true returns one current snapshot; reports still arrive automatically"},
            "cancel": {"type": "string",
                       "description": "stop this job id only when no longer needed; never use as status"}
        }, "anyOf": [
            {"required": ["prompt"]},
            {"required": ["status"]},
            {"required": ["cancel"]}
        ]}),
    }
}

/// Everything the subagent tool needs from the session, as plain parameters.
/// The tools box builds this at dispatch; this box never sees the tool
/// context itself.
pub struct SpawnEnv<'a> {
    pub workspace: &'a std::path::Path,
    pub websearch: bool,
    pub task: Option<&'a TaskCfg>,
    pub loaded_skills: &'a std::sync::Mutex<Vec<String>>,
}

pub fn run(env: &SpawnEnv, args: &Value) -> ToolOutcome {
    let Some(cfg) = env.task else {
        return ToolOutcome::err(
            "sub-agents are not available here; do the work yourself with the other tools",
        );
    };
    let cancel = match opt_str(args, "cancel") {
        Ok(Some(id)) if !id.trim().is_empty() => Some(id.trim()),
        Ok(Some(_)) | Ok(None) => None,
        Err(e) => return ToolOutcome::err(e),
    };
    let status = match args.get("status") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        Some(_) => return ToolOutcome::err("parameter \"status\" must be true or false"),
    };
    let has_prompt = args
        .get("prompt")
        .and_then(Value::as_str)
        .is_some_and(|prompt| !prompt.trim().is_empty());
    // A padded status beside a real prompt is the same small-model tic a
    // padded status:false is: the prompt is the operation, the padding is
    // noise, and refusing the call starved real work over a filler field.
    // A cancel is different: destroying a job on a call that also asks to
    // start one is ambiguous enough to refuse.
    let status = status && !has_prompt;
    if usize::from(cancel.is_some()) + usize::from(status) + usize::from(has_prompt) > 1 {
        return ToolOutcome::err(
            "choose exactly one subagent operation: prompt to spawn, status:true to inspect, \
             or cancel with a job id",
        );
    }
    if status {
        let Some(hub) = &cfg.background else {
            return ToolOutcome::err("no detached sub-agents on this surface; no status available");
        };
        let snapshot = hub.snapshot();
        // The digest carries the fleet's real elapsed (its longest-running
        // child). The generic per-call timing is suppressed for detached
        // subagent calls, because a hub snapshot returns in microseconds and
        // its "0.0s" read as a broken child elapsed on screen.
        let mut summary = format!("{} active · {} ready", snapshot.active, snapshot.ready);
        if let Some((id, elapsed)) = &snapshot.oldest_active {
            summary.push_str(&format!(" · {id} {}", crate::ui::elapsed_label(*elapsed)));
        }
        return ToolOutcome::ok(
            json!({
                "active": snapshot.active,
                "queued": snapshot.queued,
                "running": snapshot.running,
                "ready": snapshot.ready,
                "active_ids": snapshot.active_ids,
                "oldest_active": snapshot
                    .oldest_active
                    .as_ref()
                    .map(|(id, elapsed)| json!({"job_id": id, "elapsed_s": elapsed.as_secs()})),
                "contract": "snapshot only; final reports arrive automatically; do not poll",
            })
            .to_string(),
            summary,
        );
    }
    // Fleet control without a second tool schema: {"cancel": "agent-N"} stops
    // a detached job through the same path as the human's /agents cancel, so
    // the model can act on "stop the research" instead of deferring to the
    // user. Spawning and canceling are mutually exclusive shapes of one call.
    if let Some(id) = cancel {
        let Some(hub) = &cfg.background else {
            return ToolOutcome::err("no detached sub-agents on this surface; nothing to cancel");
        };
        let id = id.to_string();
        return if hub.cancel(&id) {
            ToolOutcome::ok(
                json!({"job_id": id, "status": "canceling"}).to_string(),
                format!("canceling {id}"),
            )
        } else {
            ToolOutcome::err(format!(
                "no active job {id:?}; job ids come from your spawn acknowledgments"
            ))
        };
    }
    // A bare {"status": false} is schema-legal but does nothing; the generic
    // missing-prompt error below misread as "add a prompt" and seeded small-
    // model retry loops. A padded status:false alongside a real prompt or
    // cancel still takes those paths untouched.
    if args.get("status") == Some(&Value::Bool(false)) && !has_prompt {
        return ToolOutcome::err(
            "status:false does nothing: use status:true for one snapshot, prompt to spawn, \
             or cancel with a job id",
        );
    }
    let prompt = match need_str(args, "prompt") {
        Ok(p) if !p.trim().is_empty() => p.to_string(),
        Ok(_) => return ToolOutcome::err("parameter \"prompt\" is empty; resend the call"),
        Err(e) => return ToolOutcome::err(e),
    };
    let web_available = env.websearch;
    let requested_tools_mode = match opt_str(args, "tools") {
        Ok(None) => cfg.tools_default.clone(),
        Ok(Some(m @ ("read-only" | "web" | "all"))) => m.to_string(),
        Ok(Some(other)) => {
            return ToolOutcome::err(format!(
                "parameter \"tools\" must be \"read-only\", \"web\", or \"all\", got {other:?}; \
                 resend the call"
            ));
        }
        Err(e) => return ToolOutcome::err(e),
    };
    // The installed research workflow explicitly assigns storage to the
    // parent. Small models sometimes still give that child `all`; recognize
    // the workflow's required brief and enforce its nonmutating web profile.
    let research_investigation = research_investigation_loaded(env.loaded_skills, &prompt);
    let tools_mode = if research_investigation && web_available {
        "web".to_string()
    } else {
        requested_tools_mode
    };
    if tools_mode == "web" && !web_available {
        return ToolOutcome::err(
            "tools mode \"web\" needs the websearch CLI on PATH; install it with \
             `uv tool install websearch-skill`, or use \"all\" so the child can reach the \
             web through Bash",
        );
    }
    // The round budget is the session's setting alone. The tool schema
    // carries no per-spawn cap: models pad optional fields, and a padded low
    // cap starved children into cap aborts mid-task.
    let max_turns = cfg.max_turns;

    let excluded_skills = skill_exclusions(cfg, env.loaded_skills, web_available);
    let request = TaskRequest {
        asked: prompt.clone(),
        prompt: child_prompt(prompt, web_available, tools_mode == "web"),
        tools_mode,
        max_turns,
        excluded_skills,
    };
    let detached = cfg.background.is_some();
    let run_cfg = RunCfg {
        child_depth: if detached { MAX_DEPTH } else { cfg.depth + 1 },
        wall_clock: cfg.wall_clock,
        verbose: cfg.verbose,
        overrides: cfg.overrides.clone(),
        yolo: cfg.yolo,
        workspace: env.workspace.to_path_buf(),
        progress: None,
    };
    // Every dock child detaches. Full-tool children take the cross-process
    // workspace lease around write/edit calls. Bash remains available for
    // builds, tests, and exploration while children infer or mutate files.
    if let Some(hub) = &cfg.background {
        return hub.submit(run_cfg, request);
    }
    run_task(&run_cfg, &request, || INTERRUPTED.load(Ordering::SeqCst))
}

fn research_investigation_loaded(loaded: &std::sync::Mutex<Vec<String>>, prompt: &str) -> bool {
    if !loaded
        .lock()
        .unwrap()
        .iter()
        .any(|name| name == "research")
    {
        return false;
    }
    let lower = prompt.to_ascii_lowercase();
    ["zero prior context", "contrarian", "## sources"]
        .into_iter()
        .filter(|marker| lower.contains(marker))
        .count()
        >= 2
}

/// Tool names copied from another harness are a common source of small-model
/// loops. Put the actual noob calls at the start of the leaf's task, where the
/// child can act on them without rediscovering a compatibility skill.
fn child_prompt(prompt: String, web_available: bool, web_only: bool) -> String {
    if !web_available {
        return prompt;
    }
    let mutation_rule = if web_only {
        " This is a nonmutating research child: Bash, write, and edit are unavailable. Do not \
         create files; return the complete synthesis in your final message so the parent can \
         validate and store it."
    } else {
        ""
    };
    format!(
        "[noob child runtime: you are one leaf agent and cannot delegate. For live web access, \
         do not load a web-search skill and do not invent WebSearch or WebFetch calls. Call \
         websearch {{\"action\":\"init\"}} once, then websearch \
         {{\"action\":\"search\",\"query\":\"...\"}} and \
         {{\"action\":\"fetch\",\"url\":\"...\"}} to read the sources you found. Use the \
         minimum evidence required by the brief; once its requirements are met, stop gathering \
         and return the synthesis.{mutation_rule}]\n\n{prompt}"
    )
}

fn skill_exclusions(cfg: &TaskCfg, loaded: &std::sync::Mutex<Vec<String>>, mcp_replaces_web_skill: bool) -> Vec<String> {
    let mut excluded = cfg.ancestor_skills.clone();
    for name in loaded.lock().unwrap().iter() {
        if !excluded.contains(name) {
            excluded.push(name.clone());
        }
    }
    if mcp_replaces_web_skill && !excluded.iter().any(|name| name == "web-search") {
        excluded.push("web-search".to_string());
    }
    excluded
}

/// When the parent should kill this child, or `None` when it should not.
///
/// Zero means no limit, and it is the default. A child researching for twenty
/// minutes is doing its job, not hanging, and killing it at an arbitrary mark
/// destroys work the parent then has to pay to redo. What actually bounds a
/// child is its own turn budget, a Ctrl-C, and the parent exiting. A positive
/// value still caps it, which is how the tests avoid long waits.
fn deadline_for(wall_clock: Duration) -> Option<Instant> {
    (wall_clock > Duration::ZERO).then(|| Instant::now() + wall_clock)
}

fn run_task(
    cfg: &RunCfg,
    request: &TaskRequest,
    interrupted: impl Fn() -> bool + Copy,
) -> ToolOutcome {
    let deadline = deadline_for(cfg.wall_clock);
    // One JSON object in, then EOF: the child reads stdin to end. Built and
    // bounds-checked BEFORE the spawn, so an oversized task fails with the
    // real reason instead of the child's truncated-JSON "no task" error.
    let payload = json!({
        "prompt": request.prompt,
        "tools": request.tools_mode,
        "max_turns": request.max_turns,
        "_noob_runtime": {
            "base_url": cfg.overrides.base_url,
            "model": cfg.overrides.model,
            "api_style": cfg.overrides.api_style,
            "yolo": cfg.yolo,
            "verbose": cfg.verbose,
            "excluded_skills": request.excluded_skills,
        },
    });
    let bytes = format!("{payload}\n");
    if bytes.len() as u64 > CHILD_STDIN_CAP {
        return ToolOutcome::err(format!(
            "the sub-agent task payload is {} bytes, above the {} MiB child input \
             bound; send a smaller prompt (reference files by path instead of \
             inlining their contents)",
            bytes.len(),
            CHILD_STDIN_CAP / (1024 * 1024)
        ));
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => return ToolOutcome::err(format!("cannot locate the noob binary: {e}")),
    };
    let parent_pid = unsafe { libc::getpid() };
    let mut command = Command::new(exe);
    command
        .arg("child")
        .env("NOOB_DEPTH", cfg.child_depth.to_string())
        .current_dir(&cfg.workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    // If the parent dies, the child must not outlive it. On Linux PDEATHSIG
    // delivers SIGTERM to the child's watchdog; elsewhere the watchdog polls
    // its parent pid instead (see install_parent_death_cleanup). The
    // parent-pid recheck closes the fork-to-arm race on every platform.
    unsafe {
        command.pre_exec(move || {
            #[cfg(target_os = "linux")]
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
        Ok(child) => ChildGuard::new(child),
        Err(e) => return ToolOutcome::err(format!("cannot spawn the sub-agent: {e}")),
    };
    {
        let mut stdin = child.stdin.take().expect("piped stdin");
        if let Err(error) =
            write_child_input_with(&mut stdin, bytes.as_bytes(), deadline, interrupted)
        {
            child.terminate();
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
    let stdout_reader = match std::thread::Builder::new()
        .name("noob-agent-stdout".to_string())
        .spawn(move || read_all(stdout))
    {
        Ok(reader) => reader,
        Err(error) => {
            child.terminate();
            return ToolOutcome::err(format!("cannot start the sub-agent output reader: {error}"));
        }
    };
    let stderr = child.stderr.take().expect("piped stderr");
    let verbose = cfg.verbose;
    let live_progress = cfg.progress.clone();
    let stderr_reader = match std::thread::Builder::new()
        .name("noob-agent-stderr".to_string())
        .spawn(move || {
            drain_stderr_with(stderr, verbose, |bytes| {
                if let Some(progress) = &live_progress {
                    progress.push(bytes);
                }
            })
        }) {
        Ok(reader) => reader,
        Err(error) => {
            child.terminate();
            let _ = stdout_reader.join();
            return ToolOutcome::err(format!(
                "cannot start the sub-agent diagnostics reader: {error}"
            ));
        }
    };

    // The wait loop owns the three exits: completion, wall clock, Ctrl-C.
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                child.disarm();
                break Some(status);
            }
            Ok(None) => {}
            Err(_) => {
                child.terminate();
                break None;
            }
        }
        if interrupted() {
            child.terminate();
            let _ = stdout_reader.join();
            let progress = stderr_reader.join().unwrap_or_default();
            return with_progress(ToolOutcome::canceled(), verbose, progress);
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            child.terminate();
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
    deadline: Option<Instant>,
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
        if deadline.is_some_and(|d| Instant::now() >= d) {
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

/// A spawned child is armed until `try_wait` reaps it or an explicit
/// termination path runs. If an unexpected unwind crosses `run_task`, Drop
/// still kills and reaps the process group instead of orphaning it.
struct ChildGuard {
    child: Child,
    armed: bool,
}

impl ChildGuard {
    fn new(child: Child) -> ChildGuard {
        ChildGuard { child, armed: true }
    }

    fn terminate(&mut self) {
        if self.armed {
            kill_group(&mut self.child);
            self.armed = false;
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl std::ops::Deref for ChildGuard {
    type Target = Child;

    fn deref(&self) -> &Child {
        &self.child
    }
}

impl std::ops::DerefMut for ChildGuard {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.terminate();
    }
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
            // Without PDEATHSIG, reparenting is the death notice: the parent
            // recorded at spawn is gone the moment getppid answers with
            // someone else. One compare per tick costs nothing.
            #[cfg(not(target_os = "linux"))]
            let parent_at_start = unsafe { libc::getppid() };
            loop {
                #[cfg(not(target_os = "linux"))]
                if unsafe { libc::getppid() } != parent_at_start {
                    PARENT_DIED.store(true, Ordering::SeqCst);
                    INTERRUPTED.store(true, Ordering::SeqCst);
                }
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
#[cfg(target_os = "linux")]
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

/// Without /proc there is no descendant walk. The child is its own process
/// group leader, so the group is the killable unit: SIGTERM reaches every
/// member that did not make its own session (this process catches it and is
/// already exiting). A setsid escapee survives here where the Linux walk
/// would have found it; that residue is the platform's limit.
#[cfg(not(target_os = "linux"))]
fn kill_descendants(root: libc::pid_t) {
    unsafe {
        libc::kill(-root, libc::SIGTERM);
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

    /// Build the env the dispatch layer would: locals owned by the test.
    fn env_of<'a>(
        ws: &'a std::path::Path,
        task: Option<&'a TaskCfg>,
        loaded: &'a std::sync::Mutex<Vec<String>>,
    ) -> SpawnEnv<'a> {
        SpawnEnv {
            workspace: ws,
            websearch: false,
            task,
            loaded_skills: loaded,
        }
    }

    fn cfg() -> TaskCfg {
        TaskCfg {
            depth: 0,
            concurrency: DEFAULT_CONCURRENCY,
            max_turns: DEFAULT_MAX_TURNS,
            tools_default: String::from("all"),
            wall_clock: Duration::from_secs(DEFAULT_WALL_CLOCK_S),
            verbose: false,
            overrides: Overrides::default(),
            yolo: false,
            ancestor_skills: Vec::new(),
            background: None,
        }
    }

    #[test]
    fn without_task_cfg_the_tool_refuses() {
        let (_tmp, ctx) = test_ctx();
        let loaded = std::sync::Mutex::new(Vec::new());
        let env = env_of(&ctx.core.workspace, None, &loaded);
        let out = run(&env, &json!({"prompt": "do things"}));
        assert!(out.is_error);
        assert!(out.content.contains("not available here"));
    }

    #[test]
    fn argument_validation_teaches() {
        let (_tmp, ctx) = test_ctx();
        let task = cfg();
        let loaded = std::sync::Mutex::new(Vec::new());
        let env = env_of(&ctx.core.workspace, Some(&task), &loaded);
        let out = run(&env, &json!({}));
        assert!(
            out.content
                .contains("missing required parameter \"prompt\"")
        );
        let out = run(&env_of(&ctx.core.workspace, Some(&task), &loaded), &json!({"prompt": "  "}));
        assert!(out.content.contains("is empty"));
        let out = run(&env_of(&ctx.core.workspace, Some(&task), &loaded), &json!({"prompt": "x", "tools": "everything"}));
        assert!(out.content.contains("\"read-only\", \"web\", or \"all\""));
        let out = run(&env_of(&ctx.core.workspace, Some(&task), &loaded), &json!({"prompt": "x", "tools": "web"}));
        assert!(
            out.is_error && out.content.contains("needs the websearch CLI on PATH"),
            "{}",
            out.content
        );
        let out = run(&env_of(&ctx.core.workspace, Some(&task), &loaded), &json!({"prompt": "x", "cancel": "agent-1"}));
        assert!(out.is_error);
        assert!(out.content.contains("choose exactly one"));
        // A padded status:true beside a real prompt spawns: refusing the
        // call starved real work over a filler field. Proven without
        // spawning: the call walks past the exclusive-operation check and
        // trips on the bad tools value, the check after it.
        let out = run(
            &env_of(&ctx.core.workspace, Some(&task), &loaded),
            &json!({"prompt": "x", "status": true, "tools": "everything"}),
        );
        assert!(
            out.content.contains("\"read-only\", \"web\", or \"all\""),
            "{}",
            out.content
        );

        let schema = spec().parameters;
        assert_eq!(schema["anyOf"].as_array().unwrap().len(), 3);
        let tools = schema["properties"]["tools"]["description"]
            .as_str()
            .unwrap();
        assert!(tools.contains("websearch tool"), "{tools}");
        assert!(!tools.contains("MCP"), "{tools}");
    }

    #[test]
    fn cancel_shape_stops_a_hub_job_and_refuses_cleanly_without_one() {
        let (_tmp, ctx) = test_ctx();
        let mut task = cfg();
        let loaded = std::sync::Mutex::new(Vec::new());
        // No hub on this surface: a clear refusal, never a spawn.
        let env = env_of(&ctx.core.workspace, Some(&task), &loaded);
        let out = run(&env, &json!({"cancel": "agent-1"}));
        assert!(out.is_error && out.content.contains("nothing to cancel"));

        let hub = BackgroundHub::new(1);
        task.background = Some(hub.clone());
        let env = env_of(&ctx.core.workspace, Some(&task), &loaded);
        let status = run(&env, &json!({"status": true}));
        assert!(!status.is_error, "{}", status.content);
        assert!(status.content.contains("\"active\":0"));
        assert!(status.content.contains("do not poll"));
        // An unknown id is named as such, pointing back at the acks.
        let out = run(&env_of(&ctx.core.workspace, Some(&task), &loaded), &json!({"cancel": "agent-9"}));
        assert!(out.is_error && out.content.contains("no active job"));

        // A blank cancel is "not canceling": it must fall through to the
        // spawn shape (live catch: the model padded its spawn call with
        // "cancel": "" and the spawn was rejected). With a blank prompt the
        // spawn path then reports the prompt, never a bad cancel id.
        let out = run(&env_of(&ctx.core.workspace, Some(&task), &loaded), &json!({"cancel": "", "prompt": "  "}));
        assert!(out.content.contains("is empty"), "{}", out.content);
        assert!(!out.content.contains("no active job"), "{}", out.content);

        // A live job cancels through the same path as /agents cancel.
        let _ack = hub.submit_with("probe".to_string(), |cancel| {
            while !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            crate::tools::ToolOutcome::canceled()
        });
        let out = run(&env_of(&ctx.core.workspace, Some(&task), &loaded), &json!({"cancel": "agent-1"}));
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("canceling"));
        let results = hub.shutdown();
        assert!(results.iter().all(|r| r.outcome.canceled));
    }

    #[test]
    fn bare_status_false_is_taught_not_mistaken_for_a_missing_prompt() {
        let (_tmp, ctx) = test_ctx();
        let task = cfg();
        let loaded = std::sync::Mutex::new(Vec::new());
        let env = env_of(&ctx.core.workspace, Some(&task), &loaded);
        let out = run(&env, &json!({"status": false}));
        assert!(out.is_error);
        assert!(
            out.content.contains("status:false does nothing"),
            "{}",
            out.content
        );
        assert!(
            !out.content.contains("missing required parameter"),
            "{}",
            out.content
        );
        // Padded status:false alongside a real control still routes there.
        let out = run(&env_of(&ctx.core.workspace, Some(&task), &loaded), &json!({"status": false, "cancel": "agent-1"}));
        assert!(out.content.contains("nothing to cancel"), "{}", out.content);
    }

    #[test]
    fn status_digest_names_the_longest_running_child() {
        let (_tmp, ctx) = test_ctx();
        let mut task = cfg();
        let loaded = std::sync::Mutex::new(Vec::new());
        let hub = BackgroundHub::new(1);
        {
            let task = &mut task;
            task.background = Some(hub.clone());
        }
        let _ack = hub.submit_with("probe".to_string(), |cancel| {
            while !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            crate::tools::ToolOutcome::canceled()
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while hub.snapshot().running == 0 {
            assert!(Instant::now() < deadline, "child never started running");
            std::thread::sleep(Duration::from_millis(5));
        }
        let out = run(&env_of(&ctx.core.workspace, Some(&task), &loaded), &json!({"status": true}));
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.summary.starts_with("1 active · 0 ready · agent-1 "),
            "digest must carry the fleet elapsed: {}",
            out.summary
        );
        assert!(
            out.content.contains("\"job_id\":\"agent-1\""),
            "{}",
            out.content
        );
        let results = hub.shutdown();
        assert!(results.iter().all(|r| r.outcome.canceled));
    }

    #[test]
    fn loaded_and_ancestor_skills_are_excluded_from_descendants_once() {
        let mut task = cfg();
        task.ancestor_skills = vec!["research".into(), "shared".into()];
        let loaded = std::sync::Mutex::new(vec![
            "shared".to_string(),
            "domain".to_string(),
            "research".to_string(),
        ]);
        assert_eq!(
            skill_exclusions(&task, &loaded, false),
            vec!["research", "shared", "domain"]
        );
    }

    #[test]
    fn the_web_tool_replaces_the_descendant_compatibility_skill() {
        // A child that can call `websearch` must not also be handed the
        // web-search skill: the skill exists to teach a Bash-driven fallback,
        // and two ways to do one thing is how small models loop.
        let prompt = child_prompt("research current facts".into(), true, true);
        assert!(prompt.starts_with("[noob child runtime:"));
        assert!(
            prompt.contains("websearch {\"action\":\"init\"}"),
            "{prompt}"
        );
        assert!(prompt.contains("\"action\":\"fetch\""), "{prompt}");
        assert!(!prompt.contains("mcp_connect"), "{prompt}");
        assert!(prompt.contains("stop gathering and return the synthesis"));
        assert!(prompt.contains("Do not create files"));
        assert!(prompt.ends_with("research current facts"));
        let loaded = std::sync::Mutex::new(Vec::new());
        assert!(skill_exclusions(&cfg(), &loaded, true).contains(&"web-search".to_string()));
        // Without the tool the brief is passed through untouched: the child
        // would only be told to call something it does not have.
        assert_eq!(
            child_prompt("research current facts".into(), false, true),
            "research current facts"
        );
    }

    #[test]
    fn a_research_investigation_brief_is_recognized() {
        let loaded = std::sync::Mutex::new(vec!["research".to_string()]);
        assert!(research_investigation_loaded(
            &loaded,
            "You have zero prior context. Run a CONTRARIAN pass. End with ## Sources."
        ));
        assert!(!research_investigation_loaded(&loaded, "implement the parser"));
    }

    #[test]
    fn armed_child_guard_kills_and_reaps_on_drop() {
        use std::os::unix::process::CommandExt;

        let mut command = Command::new("sh");
        command.arg("-c").arg("sleep 30").process_group(0);
        let child = command.spawn().unwrap();
        let pid = child.id() as libc::pid_t;
        drop(ChildGuard::new(child));

        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[test]
    fn oversized_task_payloads_are_refused_before_spawning() {
        // Above the child's stdin bound the task JSON would arrive truncated
        // and misreport as "no task"; the parent must name the real reason.
        // Post-check, run_task returns before any spawn, so this cannot
        // accidentally exec the test binary as `noob child`.
        let request = TaskRequest {
            prompt: "x".repeat((CHILD_STDIN_CAP + 1) as usize),
            asked: String::from("oversized"),
            tools_mode: "read-only".to_string(),
            max_turns: 1,
            excluded_skills: Vec::new(),
        };
        let run_cfg = RunCfg {
            child_depth: 1,
            wall_clock: Duration::from_secs(5),
            verbose: false,
            overrides: Overrides::default(),
            yolo: false,
            workspace: std::env::temp_dir(),
            progress: None,
        };
        let started = Instant::now();
        let out = run_task(&run_cfg, &request, || false);
        assert!(out.is_error);
        assert!(
            out.content.contains("8 MiB child input bound"),
            "{}",
            out.content
        );
        assert!(out.content.contains("bytes"), "{}", out.content);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the refusal must not wait on a spawned child"
        );
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
            Some(Instant::now() + Duration::from_secs(2)),
            || canceled.load(Ordering::SeqCst),
        );
        setter.join().unwrap();
        let _ = child.kill();
        let _ = child.wait();

        assert!(matches!(result, Err(ChildInputError::Canceled)));
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    /// The wall clock now ships disabled, so `None` is the ordinary state of
    /// this path rather than an impossible one. A blocked write must still be
    /// interruptible with no deadline to fall back on.
    #[test]
    fn blocked_child_input_without_a_deadline_is_still_cancelable() {
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
        let result =
            write_child_input_with(&mut stdin, &input, None, || canceled.load(Ordering::SeqCst));
        setter.join().unwrap();
        let _ = child.kill();
        let _ = child.wait();

        assert!(matches!(result, Err(ChildInputError::Canceled)));
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    /// A configured wall clock still stops a child that never reads its task.
    /// Paired with the test above so the two states are pinned together: no
    /// deadline never times out, a deadline always does.
    #[test]
    fn blocked_child_input_times_out_when_a_wall_clock_is_set() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("exec sleep 5")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let input = vec![b'x'; 4 * 1024 * 1024];
        let result = write_child_input_with(
            &mut stdin,
            &input,
            Some(Instant::now() + Duration::from_millis(200)),
            || false,
        );
        let _ = child.kill();
        let _ = child.wait();

        assert!(matches!(result, Err(ChildInputError::Timeout)));
    }

    /// The default ships disabled; a configured value still produces a kill
    /// time. This is the whole behavior change, at the point it is decided.
    #[test]
    fn a_zero_wall_clock_produces_no_deadline() {
        assert_eq!(deadline_for(Duration::ZERO), None);
        assert_eq!(
            deadline_for(Duration::from_secs(DEFAULT_WALL_CLOCK_S)),
            None,
            "the shipped default must not cap a child"
        );
        assert!(deadline_for(Duration::from_secs(5)).is_some());
    }

    // Spawning real children is exercised end to end in tests/e2e_p6.rs;
    // unit tests here cannot use current_exe (it is the test harness, not
    // the noob binary).
}
