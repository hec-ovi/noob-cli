//! noob: argv dispatch only. Subcommands: (repl) | exec | child | doctor |
//! debug. Hand-rolled parsing; the whole surface is a handful of flags.

mod agent;
mod config;
mod doctor;
mod mcp;
mod session;
mod skills;
mod task;
mod tools;
mod ui;

use std::io::Read;
use std::process::ExitCode;
use std::sync::atomic::Ordering;

use noob_provider::http::{Client, INTERRUPTED, Timeouts};
use noob_provider::types::Overrides;
use serde_json::json;

use agent::{Agent, RunEnd, prompt};
use session::Session;
use tools::ToolCtx;
use ui::{Mode, Ui};

fn main() -> ExitCode {
    install_sigint_handler();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") | Some("-V") => {
            println!("noob {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("exec") => cmd_exec(&args[1..]),
        Some("debug") => cmd_debug(&args[1..]),
        Some("child") => cmd_child(),
        Some("doctor") => doctor::run(),
        Some(flag) if flag.starts_with('-') => cmd_repl(&args),
        None => cmd_repl(&[]),
        Some(other) => {
            eprintln!(
                "noob: unknown command {other:?}; available: exec, debug, doctor, --version"
            );
            ExitCode::from(2)
        }
    }
}

/// A flag's value must exist and must not look like another flag; consuming
/// blindly turns one forgotten value into a silent misconfig.
fn value_for(flag: &str, next: Option<&String>, usage: &str) -> Result<String, String> {
    match next {
        Some(v) if !v.starts_with('-') => Ok(v.clone()),
        _ => Err(format!("noob: {flag} needs a value; {usage}")),
    }
}

// ---------------------------------------------------------------------------
// Session bootstrap shared by the REPL and exec
// ---------------------------------------------------------------------------

fn model_label(config_dir: &std::path::Path, ov: &Overrides) -> String {
    ov.model
        .clone()
        .or_else(|| config::setting(config_dir, "NOOB_MODEL"))
        .unwrap_or_else(|| "default".to_string())
}

struct BootArgs {
    ov: Overrides,
    yolo: bool,
    /// Start in plan mode (read-only tools until /go).
    plan: bool,
    /// Register only the read-only set (read-only children).
    read_only: bool,
    /// Relay sub-agent stderr as `[task] ...` lines.
    verbose: bool,
    /// None = no persistence; Some(None) = fresh id; Some(Some(id)) = resume.
    session: Option<Option<String>>,
}

impl BootArgs {
    fn new(ov: Overrides, yolo: bool, plan: bool, session: Option<Option<String>>) -> BootArgs {
        BootArgs { ov, yolo, plan, read_only: false, verbose: false, session }
    }
}

/// NOOB_DEPTH: 0 for the user's agent; children run at parent+1.
fn current_depth() -> u32 {
    std::env::var("NOOB_DEPTH")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

fn bootstrap(boot: BootArgs, ui: &mut Ui) -> Result<Agent, String> {
    let config_dir = config::config_dir();
    let mut ov = boot.ov;
    if ov.base_url.is_none()
        && config::setting(&config_dir, "NOOB_BASE_URL").is_none()
        && let Some(found) = config::autodetect_base_url()
    {
        ui.note(&format!("using {found} (autodetected)"));
        ov.base_url = Some(found);
    }
    let workspace = std::env::current_dir()
        .and_then(|d| d.canonicalize())
        .map_err(|e| format!("cannot resolve the working directory: {e}"))?;
    let (sandbox, sandbox_label) = config::detect_sandbox(&config_dir, boot.yolo);

    // The env-block model label follows the same precedence as the real
    // request (flag > env > .env) but is independent of base-url resolution,
    // so `debug prompt` and a live session print the identical head.
    let model = model_label(&config_dir, &ov);
    let discovered = skills::discover(&workspace, &config_dir);
    let (mcp_servers, mcp_warnings) = mcp::config::load(&workspace, &config_dir);
    for warning in &mcp_warnings {
        ui.note(&format!("mcp: {warning}"));
    }
    let inputs = prompt::PromptInputs {
        cwd: workspace.display().to_string(),
        model,
        sandbox: sandbox_label,
        global_agents: prompt::load_agents_md(&config_dir),
        project_agents: prompt::load_agents_md(&workspace),
        skills_index: skills::index(&discovered),
        // A read-only child has no mcp_call; naming servers would only
        // tempt it into calls it cannot make.
        mcp_line: if boot.read_only { None } else { prompt::mcp_line(&mcp_servers) },
    };
    let system = prompt::assemble(&inputs);
    // Registered set is decided here and stays byte-stable for the session:
    // the skill tool exists only when discovery found at least one skill,
    // the MCP pair only when mcp.json configured at least one server, and
    // the task tool only below the recursion ceiling with the full set.
    let depth = current_depth();
    let mut tool_specs = tools::specs();
    if !discovered.is_empty() {
        tool_specs.push(tools::skill::spec());
    }
    if !mcp_servers.is_empty() {
        tool_specs.push(tools::mcp::connect_spec());
        tool_specs.push(tools::mcp::call_spec());
    }
    let with_task = depth < task::MAX_DEPTH && !boot.read_only;
    if with_task {
        tool_specs.push(task::spec());
    }
    if boot.read_only {
        tool_specs.retain(|t| tools::READ_ONLY_SET.contains(&t.name.as_str()));
    }
    let mut tool_ctx = ToolCtx::new(workspace, sandbox);
    tool_ctx.skills = discovered;
    if !mcp_servers.is_empty() && !boot.read_only {
        tool_ctx.mcp = Some(mcp::Mcp::new(mcp_servers));
    }
    if with_task {
        tool_ctx.task = Some(task::TaskCfg {
            depth,
            concurrency: config::task_concurrency(&config_dir),
            max_turns: config::task_max_turns(&config_dir),
            wall_clock: config::task_wall_clock(&config_dir),
            verbose: boot.verbose,
        });
    }

    let (session, replayed) = match boot.session {
        None => (None, Vec::new()),
        Some(id) => {
            let (s, items) = Session::open(&config_dir, id.as_deref())?;
            (Some(s), items)
        }
    };
    let mut agent = Agent::new(
        Client::new(Timeouts::default()),
        config_dir.clone(),
        ov,
        system,
        tool_specs,
        replayed,
        tool_ctx,
        session,
        config::ctx_tokens(&config_dir),
    );
    if boot.plan {
        agent.enter_plan(ui);
    }
    // Read-only children: the schemas are already filtered above; this arms
    // the dispatcher's defense in depth against hallucinated mutations.
    agent.read_only = boot.read_only;
    Ok(agent)
}

// ---------------------------------------------------------------------------
// REPL
// ---------------------------------------------------------------------------

fn cmd_repl(args: &[String]) -> ExitCode {
    const USAGE: &str = "usage: noob [--model <name>] [--base-url <url>] [--session <id> | --restore <id>] \
                         [--plan] [--verbose] [--yolo]";
    let mut ov = Overrides::default();
    let mut yolo = false;
    let mut plan = false;
    let mut verbose = false;
    let mut session_id: Option<String> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let taken = match arg.as_str() {
            "--model" => value_for(arg, it.next(), USAGE).map(|v| ov.model = Some(v)),
            "--base-url" => value_for(arg, it.next(), USAGE).map(|v| ov.base_url = Some(v)),
            "--session" | "--restore" => {
                value_for(arg, it.next(), USAGE).map(|v| session_id = Some(v))
            }
            "--yolo" => {
                yolo = true;
                Ok(())
            }
            "--plan" => {
                plan = true;
                Ok(())
            }
            "--verbose" => {
                verbose = true;
                Ok(())
            }
            other => Err(format!("noob: unknown flag {other:?}; {USAGE}")),
        };
        if let Err(msg) = taken {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    }

    let mut ui = Ui::new(Mode::Repl);
    // The REPL always persists: a fresh id, or resume the one given so a closed
    // session can be picked up where it left off.
    let session = match session_id {
        Some(id) => Some(Some(id)),
        None => Some(None),
    };
    let mut boot = BootArgs::new(ov, yolo, plan, session);
    boot.verbose = verbose;
    let mut agent = match bootstrap(boot, &mut ui) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("noob: {e}");
            return ExitCode::from(2);
        }
    };
    greet(&agent, &mut ui);
    // The dock (fable.md): raw mode held for the session, a live input frame
    // during turns, output above it. NOOB_DOCK=0 keeps the classic per-prompt
    // editor and blocking turn below.
    let mut dock = ui.dock_session();

    loop {
        ui.end_line();
        // The reader draws the boxed termios editor at an interactive terminal
        // and falls back to cooked `read_line` when piped. EOF (Ctrl-D) exits;
        // a Ctrl-C at the prompt reprompts, kept distinct from EOF. In cooked
        // mode a second Ctrl-C before any input still hard-exits via the signal
        // handler; in raw mode Ctrl-C cancels the line and Ctrl-D or /quit exit.
        let line = match dock.as_mut() {
            Some(d) => d.read_prompt(&mut ui, agent.plan),
            None => ui.read_prompt(agent.plan),
        };
        let line = match line {
            ui::Input::Eof => break,
            ui::Input::Interrupted => {
                ui.note("(interrupted; /quit to exit)");
                continue;
            }
            ui::Input::Line(l) => l,
        };
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        // Bare exit/quit leave too: the zero-friction promise says nobody
        // should have to learn slash commands to get out.
        if matches!(input, "exit" | "quit") {
            break;
        }
        if let Some(cmd) = input.strip_prefix('/') {
            match cmd.trim() {
                "quit" | "q" | "exit" => break,
                "status" => status(&agent, &mut ui),
                "compact" => {
                    // Compaction makes a blocking summarizer request, so in
                    // dock mode it must run through the render loop like a
                    // turn: the tty is raw (ISIG off), so a keyboard Ctrl-C is
                    // a byte only the render loop turns into INTERRUPTED. Run
                    // straight on the main thread otherwise (the tty is cooked
                    // between prompts, so SIGINT still reaches the watchdog).
                    match dock.as_mut() {
                        Some(d) => {
                            let plan = agent.plan;
                            d.run_turn(&mut ui, plan, |tui| agent.compact(tui));
                            if INTERRUPTED.swap(false, Ordering::SeqCst) {
                                d.drain_queue_to_draft();
                            }
                        }
                        None => {
                            agent.compact(&mut ui);
                            INTERRUPTED.store(false, Ordering::SeqCst);
                        }
                    }
                }
                "plan" => {
                    if !agent.enter_plan(&mut ui) {
                        ui.note("already in plan mode; /go approves the plan");
                    }
                }
                "go" => {
                    if agent.exit_plan(&mut ui) {
                        let end =
                            run_repl_turn(&mut agent, &mut ui, &mut dock, agent::PLAN_APPROVED_MSG);
                        match end {
                            RunEnd::Completed(_) | RunEnd::Interrupted => {}
                            RunEnd::Aborted(msg) => ui.error(&format!("error: {msg}")),
                        }
                    } else {
                        ui.note("not in plan mode; /plan enters it");
                    }
                }
                s if s == "skills" || s.starts_with("skills ") => {
                    let args = s.strip_prefix("skills").unwrap_or("").trim();
                    // Installation can clone a remote repository or copy a
                    // large local tree. Keep the dock alive so Ctrl-C remains
                    // actionable and typed input is retained while it runs.
                    if args.starts_with("add ") {
                        match dock.as_mut() {
                            Some(d) => {
                                let plan = agent.plan;
                                d.run_turn(&mut ui, plan, |tui| {
                                    handle_skills(args, &mut agent, tui)
                                });
                                if INTERRUPTED.swap(false, Ordering::SeqCst) {
                                    d.drain_queue_to_draft();
                                }
                            }
                            None => {
                                handle_skills(args, &mut agent, &mut ui);
                                INTERRUPTED.store(false, Ordering::SeqCst);
                            }
                        }
                    } else {
                        handle_skills(args, &mut agent, &mut ui);
                    }
                }
                other => ui.note(&format!(
                    "unknown command /{other}; available: /plan /go /status /compact /skills /quit"
                )),
            }
            continue;
        }
        let end = run_repl_turn(&mut agent, &mut ui, &mut dock, input);
        match end {
            RunEnd::Completed(_) | RunEnd::Interrupted => {}
            RunEnd::Aborted(msg) => ui.error(&format!("error: {msg}")),
        }
    }
    // Leave raw mode before the exit hint so it prints on a cooked terminal.
    drop(dock);
    // On the way out, tell the human how to pick this session back up. Only at
    // an interactive terminal, so a piped REPL stays byte-identical.
    if ui.is_interactive()
        && let Some(s) = agent.session.as_ref()
    {
        let id = s.id().to_string();
        ui.note(&format!(
            "session {id} saved · resume with --session {id} · host install: noob --restore {id}"
        ));
    }
    ExitCode::SUCCESS
}

/// One REPL turn through the right driver: the dock (worker thread, live
/// input frame, output above it) when the session runs docked, else the
/// classic blocking turn bracketed by the thinking scanner. The scanner
/// brackets only the classic path; the dock draws its own liveness.
fn run_repl_turn(
    agent: &mut Agent,
    ui: &mut Ui,
    dock: &mut Option<ui::DockSession>,
    input: &str,
) -> RunEnd {
    match dock.as_mut() {
        Some(d) => {
            let plan = agent.plan;
            let end = d.run_turn(ui, plan, |tui| agent.run_input(input, tui));
            // A canceled turn hands any type-ahead back to the editor instead
            // of firing it: an interrupt means the human wants to steer.
            if matches!(end, RunEnd::Interrupted) {
                d.drain_queue_to_draft();
            }
            end
        }
        None => {
            // The thinking scanner sweeps from here until the first reply
            // byte, so the request-to-first-token gap is not dead air.
            // thinking_stop is the end-of-turn bracket for a turn that
            // streamed nothing at all.
            ui.thinking_start();
            let end = agent.run_input(input, ui);
            ui.thinking_stop();
            end
        }
    }
}

/// The `/skills` command family (REPL only): list, reload, add a skill from a
/// local path or git URL, or remove an installed one. Each mutation re-runs
/// discovery through `reload_skills`, which registers the tool if needed and
/// announces the change in-band. Runs between turns on the main thread, so it
/// never contends with the dock's stdin reader (no confirmation prompt: a
/// user-run install is the trust signal; the agent-authoring gate is separate).
fn handle_skills(args: &str, agent: &mut Agent, ui: &mut Ui) {
    let mut parts = args.splitn(2, char::is_whitespace);
    let verb = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();
    match verb {
        "" | "list" => {
            if agent.tool_ctx.skills.is_empty() {
                ui.note("no skills installed; /skills add <path|git-url> to install one");
                return;
            }
            let lines: Vec<String> = agent
                .tool_ctx
                .skills
                .iter()
                .map(|s| format!("  {}: {}", s.name, skills::clip_description(&s.description)))
                .collect();
            ui.note(&format!("skills ({}):\n{}", agent.tool_ctx.skills.len(), lines.join("\n")));
        }
        "reload" => {
            let (added, removed) = agent.reload_skills(ui);
            skills_delta(ui, &added, &removed);
        }
        "add" => {
            if rest.is_empty() {
                ui.error("usage: /skills add <path|git-url>");
                return;
            }
            match skills::install(&agent.tool_ctx.workspace, rest) {
                Ok(name) => {
                    ui.note(&format!("installed skill {name}"));
                    let (added, removed) = agent.reload_skills(ui);
                    skills_delta(ui, &added, &removed);
                }
                Err(e) => ui.error(&format!("skill add failed: {e}")),
            }
        }
        "remove" | "rm" => {
            if rest.is_empty() {
                ui.error("usage: /skills remove <name>");
                return;
            }
            let dir = agent.tool_ctx.skills.iter().find(|s| s.name == rest).map(|s| s.dir.clone());
            match dir {
                None => ui.error(&format!("no installed skill named {rest:?}; /skills lists them")),
                Some(dir) => match skills::remove(&agent.tool_ctx.workspace, &dir) {
                    Ok(()) => {
                        let (added, removed) = agent.reload_skills(ui);
                        skills_delta(ui, &added, &removed);
                    }
                    Err(e) => ui.error(&format!("skill remove failed: {e}")),
                },
            }
        }
        other => ui.error(&format!(
            "unknown /skills subcommand {other:?}; use: list, add, remove, reload"
        )),
    }
}

/// One line summarizing what a reload changed, so the human sees it even when
/// the in-band model note scrolled by.
fn skills_delta(ui: &mut Ui, added: &[String], removed: &[String]) {
    if added.is_empty() && removed.is_empty() {
        ui.note("skills: no change");
        return;
    }
    let mut parts = Vec::new();
    if !added.is_empty() {
        parts.push(format!("added {}", added.join(", ")));
    }
    if !removed.is_empty() {
        parts.push(format!("removed {}", removed.join(", ")));
    }
    ui.note(&format!("skills: {}", parts.join("; ")));
}

fn greet(agent: &Agent, ui: &mut Ui) {
    let endpoint = noob_provider::resolve_endpoint(&agent.config_dir, &agent.ov)
        .map(|ep| format!("{} · {}", ep.base_url, ep.model))
        .unwrap_or_else(|_| "no endpoint configured; set NOOB_BASE_URL in .env".to_string());
    let session = agent
        .session
        .as_ref()
        .map(|s| format!(" · session {}", s.id()))
        .unwrap_or_default();
    ui.greeting(&format!(
        "noob {} · {endpoint}{session}\ntype a task; /plan /go /status /compact /skills /quit",
        env!("CARGO_PKG_VERSION")
    ));
}

fn status(agent: &Agent, ui: &mut Ui) {
    let endpoint = noob_provider::resolve_endpoint(&agent.config_dir, &agent.ov)
        .map(|ep| {
            format!(
                "{} · {} · {}",
                ep.base_url,
                ep.model,
                match ep.style {
                    noob_provider::types::ApiStyle::Chat => "chat",
                    noob_provider::types::ApiStyle::Responses => "responses",
                }
            )
        })
        .unwrap_or_else(|e| e.to_string());
    let est = agent.context_estimate();
    let pct = est * 100 / agent.ctx_tokens.max(1);
    let usage = match agent.last_usage() {
        Some(u) => format!(
            "last turn: prompt {} (cached {}), completion {}",
            u.prompt_tokens, u.cached_prompt_tokens, u.completion_tokens
        ),
        None => "last turn: no usage reported yet".to_string(),
    };
    let skills_line = if agent.tool_ctx.skills.is_empty() {
        String::new()
    } else {
        let names: Vec<&str> = agent.tool_ctx.skills.iter().map(|s| s.name.as_str()).collect();
        format!("\nskills: {}", names.join(", "))
    };
    let mcp_line = match &agent.tool_ctx.mcp {
        None => String::new(),
        Some(mcp) => {
            let entries: Vec<String> = mcp
                .names()
                .iter()
                .map(|name| {
                    if mcp.connection(name).is_some() {
                        format!("{name} (connected)")
                    } else {
                        (*name).to_string()
                    }
                })
                .collect();
            format!("\nmcp servers: {}", entries.join(", "))
        }
    };
    let session = agent
        .session
        .as_ref()
        .map(|s| format!("\nsession: {}", s.path().display()))
        .unwrap_or_default();
    let plan_line = if agent.plan { "\nplan mode: on (read-only; /go approves)" } else { "" };
    ui.note(&format!(
        "endpoint: {endpoint}\ncontext: ~{est} of {} tokens ({pct}%)\n{usage}{skills_line}{mcp_line}{plan_line}{session}",
        agent.ctx_tokens
    ));
}

// ---------------------------------------------------------------------------
// exec
// ---------------------------------------------------------------------------

fn cmd_exec(args: &[String]) -> ExitCode {
    const USAGE: &str = "usage: noob exec -p \"<prompt>\" [--json] [--session <id>] \
                         [--plan] [--verbose] [--model <name>] [--base-url <url>] [--yolo]";
    let mut prompt_arg: Option<String> = None;
    let mut ov = Overrides::default();
    let mut json_mode = false;
    let mut yolo = false;
    let mut plan = false;
    let mut verbose = false;
    let mut session_id: Option<String> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let taken = match arg.as_str() {
            // The prompt is arbitrary user text relayed by wrappers; a
            // leading '-' must not be rejected as a missing value.
            "-p" | "--prompt" => match it.next() {
                Some(v) => {
                    prompt_arg = Some(v.clone());
                    Ok(())
                }
                None => Err(format!("noob exec: {arg} needs a value; {USAGE}")),
            },
            "--model" => value_for(arg, it.next(), USAGE).map(|v| ov.model = Some(v)),
            "--base-url" => value_for(arg, it.next(), USAGE).map(|v| ov.base_url = Some(v)),
            "--session" => value_for(arg, it.next(), USAGE).map(|v| session_id = Some(v)),
            "--json" => {
                json_mode = true;
                Ok(())
            }
            "--yolo" => {
                yolo = true;
                Ok(())
            }
            "--plan" => {
                plan = true;
                Ok(())
            }
            "--verbose" => {
                verbose = true;
                Ok(())
            }
            other => Err(format!("noob exec: unknown flag {other:?}; {USAGE}")),
        };
        if let Err(msg) = taken {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    }
    let Some(input) = prompt_arg.filter(|p| !p.is_empty()) else {
        eprintln!("noob exec: missing prompt; {USAGE}");
        return ExitCode::from(2);
    };

    let mut ui = Ui::new(if json_mode { Mode::ExecJson } else { Mode::Exec });
    let session = session_id.map(Some);
    let mut boot = BootArgs::new(ov, yolo, plan, session);
    boot.verbose = verbose;
    let mut agent = match bootstrap(boot, &mut ui) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("noob: {e}");
            return ExitCode::from(2);
        }
    };
    match agent.run_input(&input, &mut ui) {
        RunEnd::Completed(_) => ExitCode::SUCCESS,
        RunEnd::Interrupted => {
            eprintln!("noob: interrupted");
            ExitCode::from(130)
        }
        RunEnd::Aborted(msg) => {
            eprintln!("noob: {msg}");
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// child (P6)
// ---------------------------------------------------------------------------

/// The sub-agent entry point. Reads ONE JSON task object from stdin
/// (`{"prompt", "tools": "read-only"|"all", "max_turns"}`), runs a fresh
/// scoped context with no parent history, streams progress to stderr, and
/// writes exactly one JSON result line to stdout:
/// `{"status": "ok"|"error", "result", "turns", "usage"}`.
fn cmd_child() -> ExitCode {
    /// Bound on the task payload; a parent never sends anything near this.
    const STDIN_CAP: u64 = 8 * 1024 * 1024;
    let mut payload = String::new();
    let read = std::io::stdin().lock().take(STDIN_CAP).read_to_string(&mut payload);
    let parsed: Option<serde_json::Value> = match read {
        Ok(_) => serde_json::from_str(payload.trim()).ok(),
        Err(_) => None,
    };
    let Some(task_obj) = parsed.filter(|v| v.is_object()) else {
        return child_result("error", "no task: stdin must carry one JSON object", 0, None);
    };
    let Some(prompt_text) = task_obj
        .get("prompt")
        .and_then(serde_json::Value::as_str)
        .filter(|p| !p.trim().is_empty())
    else {
        return child_result("error", "no task: the JSON object needs a \"prompt\"", 0, None);
    };
    let read_only = match task_obj.get("tools").and_then(serde_json::Value::as_str) {
        None | Some("read-only") => true,
        Some("all") => false,
        Some(other) => {
            return child_result(
                "error",
                &format!("unknown tools mode {other:?}; use \"read-only\" or \"all\""),
                0,
                None,
            );
        }
    };

    let mut ui = Ui::new(Mode::Child);
    let mut boot = BootArgs::new(Overrides::default(), false, false, None);
    boot.read_only = read_only;
    let mut agent = match bootstrap(boot, &mut ui) {
        Ok(a) => a,
        Err(e) => return child_result("error", &e, 0, None),
    };
    // Both sides enforce the turn cap: the parent clamped its request; the
    // child clamps that against its own environment's ceiling.
    let env_cap = config::task_max_turns(&agent.config_dir);
    agent.max_rounds = task_obj
        .get("max_turns")
        .and_then(serde_json::Value::as_u64)
        .map(|n| (n as u32).clamp(1, env_cap))
        .unwrap_or(env_cap);

    let (status, result) = match agent.run_input(prompt_text, &mut ui) {
        RunEnd::Completed(text) => ("ok", text),
        RunEnd::Aborted(msg) => ("error", msg),
        RunEnd::Interrupted => ("error", "interrupted".to_string()),
    };
    child_result(status, &result, agent.last_rounds, agent.last_usage())
}

/// The single stdout line; everything else this process printed went to
/// stderr, so the parent's parse is mechanical.
fn child_result(
    status: &str,
    result: &str,
    turns: u32,
    usage: Option<noob_provider::types::Usage>,
) -> ExitCode {
    let usage = usage.map(|u| {
        json!({
            "prompt": u.prompt_tokens,
            "completion": u.completion_tokens,
            "cached_prompt": u.cached_prompt_tokens,
        })
    });
    println!(
        "{}",
        json!({"status": status, "result": result, "turns": turns, "usage": usage})
    );
    if status == "ok" { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

// ---------------------------------------------------------------------------
// debug
// ---------------------------------------------------------------------------

/// `noob debug prompt [--json]`: the EXACT assembled system prompt and wire
/// tools array this binary would send, so budget tests measure the shipped
/// artifact rather than a reimplementation.
fn cmd_debug(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("prompt") => {}
        _ => {
            eprintln!("usage: noob debug prompt [--json]");
            return ExitCode::from(2);
        }
    }
    let json_mode = args.iter().any(|a| a == "--json");
    let config_dir = config::config_dir();
    let ov = Overrides::default();
    let workspace = std::env::current_dir()
        .and_then(|d| d.canonicalize())
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    let (_, sandbox_label) = config::detect_sandbox(&config_dir, false);
    let model = model_label(&config_dir, &ov);
    // Same discovery as bootstrap: the printed artifact must match what a
    // real session sends, byte for byte.
    let discovered = skills::discover(&workspace, &config_dir);
    let (mcp_servers, mcp_warnings) = mcp::config::load(&workspace, &config_dir);
    for warning in &mcp_warnings {
        eprintln!("noob: mcp: {warning}");
    }
    let inputs = prompt::PromptInputs {
        cwd: workspace.display().to_string(),
        model,
        sandbox: sandbox_label,
        global_agents: prompt::load_agents_md(&config_dir),
        project_agents: prompt::load_agents_md(&workspace),
        skills_index: skills::index(&discovered),
        mcp_line: prompt::mcp_line(&mcp_servers),
    };
    // One head computation feeds both outputs: a date rollover between two
    // calls must not make "head" disagree with "system".
    let head = prompt::head(&inputs);
    let system = prompt::assemble_from(head.clone(), &inputs);
    if json_mode {
        let mut tool_specs = tools::specs();
        if !discovered.is_empty() {
            tool_specs.push(tools::skill::spec());
        }
        if !mcp_servers.is_empty() {
            tool_specs.push(tools::mcp::connect_spec());
            tool_specs.push(tools::mcp::call_spec());
        }
        if current_depth() < task::MAX_DEPTH {
            tool_specs.push(task::spec());
        }
        let out = json!({
            "system": system,
            "head": head,
            "tools": noob_provider::chat::wire_tools(&tool_specs),
        });
        println!("{out}");
    } else {
        println!("{system}");
    }
    ExitCode::SUCCESS
}

/// First Ctrl-C sets the watchdog flag (the in-flight request aborts within
/// one tick); a second Ctrl-C hard-exits. Only async-signal-safe calls here.
fn install_sigint_handler() {
    extern "C" fn on_sigint(_: libc::c_int) {
        if INTERRUPTED.swap(true, Ordering::SeqCst) {
            // A second Ctrl-C hard-exits; restore the terminal first so a raw
            // editor session does not leave the shell garbled. Restore touches
            // only atomics, tcsetattr, and write, all async-signal-safe.
            ui::prompt::restore_terminal();
            unsafe { libc::_exit(130) };
        }
    }
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_sigint as *const () as usize;
        // No SA_RESTART: blocked reads return EINTR so the tick loop sees the
        // flag immediately instead of after the socket timeout.
        sa.sa_flags = 0;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
    }
}
