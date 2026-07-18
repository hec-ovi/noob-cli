//! noob: argv dispatch only. Subcommands: (repl) | exec | sessions | child |
//! doctor | debug. Hand-rolled parsing; the whole surface is a handful of flags.

mod agent;
mod config;
mod doctor;
mod mcp;
mod session;
mod skills;
mod subagent;
mod tools;
mod ui;

use std::collections::HashSet;
use std::io::Read;
use std::process::ExitCode;
use std::sync::atomic::Ordering;

use noob_provider::http::{Client, INTERRUPTED, Timeouts};
use noob_provider::types::{Item, Overrides};
use serde_json::json;

use agent::{Agent, RunEnd, prompt};
use session::{ReplayReport, Session};
use tools::ToolCtx;
use ui::{Mode, Ui};

fn main() -> ExitCode {
    install_sigint_handler();
    install_sigwinch_handler();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") | Some("-V") => {
            println!("noob {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("exec") => cmd_exec(&args[1..]),
        Some("debug") => cmd_debug(&args[1..]),
        Some("child") => cmd_child(),
        Some("sessions") => cmd_sessions(),
        Some("doctor") => doctor::run(),
        Some(flag) if flag.starts_with('-') => cmd_repl(&args),
        None => cmd_repl(&[]),
        Some(other) => {
            eprintln!(
                "noob: unknown command {other:?}; available: exec, sessions, debug, doctor, --version"
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
    /// Nonmutating research child: local reads plus the uniquely configured
    /// web-search MCP server.
    web_only: bool,
    /// Relay sub-agent stderr as `[subagent] ...` diagnostics.
    verbose: bool,
    /// Skill names already loaded by ancestor agents. A child filters these
    /// from discovery so orchestration skills cannot recursively invoke
    /// themselves through nested delegation.
    excluded_skills: Vec<String>,
    /// None = no persistence; Some(None) = fresh id; Some(Some(id)) = resume.
    session: Option<Option<String>>,
}

impl BootArgs {
    fn new(ov: Overrides, yolo: bool, plan: bool, session: Option<Option<String>>) -> BootArgs {
        BootArgs {
            ov,
            yolo,
            plan,
            read_only: false,
            web_only: false,
            verbose: false,
            excluded_skills: Vec::new(),
            session,
        }
    }
}

/// NOOB_DEPTH: 0 for the user's agent; children run at parent+1.
fn current_depth() -> u32 {
    std::env::var("NOOB_DEPTH")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// Returns the assembled agent and whether an explicit `--resume <id>` missed
/// (the id was given but no session file existed), so the REPL can tell the
/// human it started fresh. The flag is display-only; `exec`/`child` ignore it.
fn bootstrap(boot: BootArgs, ui: &mut Ui) -> Result<(Agent, bool), String> {
    let config_dir = config::config_dir();
    let mut ov = boot.ov;
    if ov.base_url.is_none()
        && config::setting(&config_dir, "NOOB_BASE_URL").is_none()
        && let Some(found) = config::autodetect_base_url(&config_dir)
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
    let skill_paths = config::skill_paths(&config_dir, &workspace);
    let mut discovered = skills::discover(&workspace, &config_dir, &skill_paths);
    if !boot.excluded_skills.is_empty() {
        discovered.retain(|skill| !boot.excluded_skills.contains(&skill.name));
    }
    let (mut mcp_servers, mcp_warnings) = mcp::config::load(&workspace, &config_dir);
    for warning in &mcp_warnings {
        ui.note(&format!("mcp: {warning}"));
    }
    if boot.web_only {
        let Some(web_server) = mcp::unique_normalized_server(
            mcp_servers.iter().map(|server| server.name.as_str()),
            "websearch",
        )
        .map(str::to_string) else {
            return Err(
                "web research child needs one unambiguous MCP server named websearch".to_string(),
            );
        };
        mcp_servers.retain(|server| server.name == web_server);
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
        mcp_line: if boot.read_only && !boot.web_only {
            None
        } else {
            prompt::mcp_line(&mcp_servers)
        },
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
    let with_task = depth < subagent::MAX_DEPTH && !boot.read_only;
    if with_task {
        tool_specs.push(subagent::spec());
    }
    if boot.web_only {
        tool_specs.retain(|t| tools::WEB_RESEARCH_SET.contains(&t.name.as_str()));
    } else if boot.read_only {
        tool_specs.retain(|t| tools::READ_ONLY_SET.contains(&t.name.as_str()));
    }
    let mut tool_ctx = ToolCtx::new(workspace, sandbox);
    tool_ctx.skills = discovered;
    if !mcp_servers.is_empty() && (!boot.read_only || boot.web_only) {
        tool_ctx.mcp = Some(mcp::Mcp::new(mcp_servers));
    }
    if with_task {
        tool_ctx.task = Some(subagent::TaskCfg {
            depth,
            concurrency: config::task_concurrency(&config_dir),
            max_turns: config::task_max_turns(&config_dir),
            wall_clock: config::task_wall_clock(&config_dir),
            verbose: boot.verbose,
            overrides: ov.clone(),
            yolo: boot.yolo,
            ancestor_skills: boot.excluded_skills.clone(),
            background: None,
        });
    }

    let (session, replayed, resume_missed, replay_report) = match boot.session {
        None => (None, Vec::new(), false, ReplayReport::default()),
        Some(id) => {
            let requested = id.is_some();
            let resolved = match id.as_deref() {
                Some("latest") => Session::latest_id(&config_dir)?,
                _ => id,
            };
            let (s, items, existed, report) = Session::open(&config_dir, resolved.as_deref())?;
            (Some(s), items, requested && !existed, report)
        }
    };
    if let Some(warning) = replay_report.warning() {
        ui.error(&warning);
    }
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
    let orphaned = agent.repair_orphaned_background_results();
    if orphaned > 0 {
        agent.show_session_warning(ui);
        ui.note(&format!(
            "recovered {orphaned} unfinished background sub-agent(s) as canceled"
        ));
    }
    if boot.plan {
        agent.enter_plan(ui);
    }
    // Read-only children: the schemas are already filtered above; this arms
    // the dispatcher's defense in depth against hallucinated mutations.
    agent.read_only = boot.read_only;
    Ok((agent, resume_missed))
}

// ---------------------------------------------------------------------------
// REPL
// ---------------------------------------------------------------------------

fn cmd_repl(args: &[String]) -> ExitCode {
    const USAGE: &str = "usage: noob [--model <name>] [--base-url <url>] \
                         [--resume <id> | --session <id> | --restore <id>] \
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
            "--resume" | "--session" | "--restore" => {
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
    let session = match &session_id {
        Some(id) => Some(Some(id.clone())),
        None => Some(None),
    };
    let mut boot = BootArgs::new(ov, yolo, plan, session);
    boot.verbose = verbose;
    let (mut agent, resume_missed) = match bootstrap(boot, &mut ui) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("noob: {e}");
            return ExitCode::from(2);
        }
    };
    greet(&agent, &mut ui);
    // A resume that named an id with no saved session starts fresh; on the
    // interactive surface say so instead of silently opening an empty session.
    // Gated on is_interactive so a piped REPL and exec stay byte-identical.
    if resume_missed
        && ui.is_interactive()
        && let Some(id) = &session_id
    {
        ui.note(&format!("no saved session {id}; starting fresh"));
    }
    // Redisplay a resumed conversation on screen: interactive REPL only and
    // only when there is history. Display-only and off the token path; it never
    // touches the request body, the transcript, or the session log.
    if ui.is_interactive() && !agent.items.is_empty() {
        ui.replay_transcript(&agent.items);
    }
    // The dock (fable.md): raw mode held for the session, a live input frame
    // during turns, output above it. NOOB_DOCK=0 keeps the classic per-prompt
    // editor and blocking turn below.
    let mut dock = ui.dock_session();
    if dock.is_some() {
        agent.enable_background_agents(&mut ui);
    }

    loop {
        ui.end_line();
        // The reader draws the boxed termios editor at an interactive terminal
        // and falls back to cooked `read_line` when piped. EOF (Ctrl-D) exits;
        // a Ctrl-C at the prompt reprompts, kept distinct from EOF. In cooked
        // mode a second Ctrl-C before any input still hard-exits via the signal
        // handler; in raw mode Ctrl-C cancels the line and Ctrl-D or /quit exit.
        let background = agent.background_hub();
        let line = match dock.as_mut() {
            Some(d) => d.read_prompt(&mut ui, agent.plan, background.as_ref()),
            None => ui.read_prompt(agent.plan),
        };
        let line = match line {
            ui::Input::BackgroundReady => {
                let end = run_repl_background(&mut agent, &mut ui, &mut dock);
                match end {
                    RunEnd::Completed(_) | RunEnd::Interrupted => {}
                    RunEnd::Aborted(msg) => ui.error(&format!("error: {msg}")),
                }
                continue;
            }
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
                "context" => ui.note(&tools::context::report(
                    agent.context_estimate(),
                    agent.ctx_tokens,
                )),
                "sessions" => show_sessions(&agent.config_dir, &mut ui),
                "clear-plan" => {
                    let updates = agent.clear_plan_history(&mut ui);
                    if updates == 0 {
                        ui.note("no plan payloads in the current context");
                    } else {
                        ui.note(&format!(
                            "plan cleared from context: {updates} update(s) redacted; cache prefix reset"
                        ));
                    }
                }
                s if s == "config" || s.starts_with("config ") => handle_config(
                    s.strip_prefix("config").unwrap_or("").trim(),
                    &agent,
                    &mut ui,
                ),
                s if s == "agents" || s.starts_with("agents ") => handle_agents(
                    s.strip_prefix("agents").unwrap_or("").trim(),
                    &agent,
                    &mut ui,
                ),
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
                            let background = agent.background_hub();
                            d.run_turn(&mut ui, plan, background.as_ref(), |tui| {
                                agent.compact(tui)
                            });
                            let steered = d.take_steering();
                            if INTERRUPTED.swap(false, Ordering::SeqCst) && !steered {
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
                s if s == "mcp" || s.starts_with("mcp ") => {
                    let args = s.strip_prefix("mcp").unwrap_or("").trim();
                    // A connect makes a network round-trip that can hang until
                    // the timeout. Keep the dock alive so Ctrl-C stays
                    // actionable while it runs, exactly like a skill clone.
                    if args.starts_with("connect ") {
                        match dock.as_mut() {
                            Some(d) => {
                                let plan = agent.plan;
                                let background = agent.background_hub();
                                d.run_turn(&mut ui, plan, background.as_ref(), |tui| {
                                    handle_mcp(args, &mut agent, tui)
                                });
                                let steered = d.take_steering();
                                if INTERRUPTED.swap(false, Ordering::SeqCst) && !steered {
                                    d.drain_queue_to_draft();
                                }
                            }
                            None => {
                                handle_mcp(args, &mut agent, &mut ui);
                                INTERRUPTED.store(false, Ordering::SeqCst);
                            }
                        }
                    } else {
                        handle_mcp(args, &mut agent, &mut ui);
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
                                let background = agent.background_hub();
                                d.run_turn(&mut ui, plan, background.as_ref(), |tui| {
                                    handle_skills(args, &mut agent, tui)
                                });
                                let steered = d.take_steering();
                                if INTERRUPTED.swap(false, Ordering::SeqCst) && !steered {
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
                    "unknown command /{other}; available: {}",
                    ui::commands::banner()
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
    agent.shutdown_background_agents(&mut ui);
    // Leave raw mode before the exit hint so it prints on a cooked terminal.
    drop(dock);
    // On the way out, tell the human how to pick this session back up. Only at
    // an interactive terminal, so a piped REPL stays byte-identical.
    if ui.is_interactive()
        && let Some(s) = agent.session.as_ref()
    {
        let id = s.id().to_string();
        ui.note(&format!(
            "session {id} saved · resume with: noob --resume {id}"
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
            let background = agent.background_hub();
            let end = d.run_turn(ui, plan, background.as_ref(), |tui| {
                agent.run_input(input, tui)
            });
            // Enter during a turn is explicit steering and leaves the queued
            // message ready for immediate dispatch. Ctrl-C still hands any
            // type-ahead back to the editor.
            let steered = d.take_steering();
            if steered {
                // The worker normally consumes the interrupt. Clear the
                // shared tail explicitly for abort/error races after the
                // steering message was already accepted.
                INTERRUPTED.store(false, Ordering::SeqCst);
            } else if matches!(end, RunEnd::Interrupted) {
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

fn run_repl_background(
    agent: &mut Agent,
    ui: &mut Ui,
    dock: &mut Option<ui::DockSession>,
) -> RunEnd {
    match dock.as_mut() {
        Some(d) => {
            let plan = agent.plan;
            let background = agent.background_hub();
            let end = d.run_turn(ui, plan, background.as_ref(), |tui| {
                agent.continue_after_background(tui)
            });
            let steered = d.take_steering();
            if steered {
                INTERRUPTED.store(false, Ordering::SeqCst);
            } else if matches!(end, RunEnd::Interrupted) {
                d.drain_queue_to_draft();
            }
            end
        }
        None => agent.continue_after_background(ui),
    }
}

fn handle_agents(args: &str, agent: &Agent, ui: &mut Ui) {
    if args.is_empty() || args == "list" {
        let snapshot = agent.background_snapshot();
        if snapshot.rows.is_empty() {
            ui.note("no background sub-agents");
        } else {
            ui.note(&format!("agents:\n  {}", snapshot.rows.join("\n  ")));
        }
        return;
    }
    let Some(target) = args.strip_prefix("cancel ").map(str::trim) else {
        ui.error("usage: /agents [list | cancel <agent-N|all>]");
        return;
    };
    if target == "all" {
        let count = agent.cancel_all_background();
        ui.note(&format!("canceling {count} background sub-agent(s)"));
    } else if agent.cancel_background(target) {
        ui.note(&format!("canceling {target}"));
    } else {
        ui.error(&format!(
            "unknown background sub-agent {target:?}; run /agents to list them"
        ));
    }
}

fn session_lines(config_dir: &std::path::Path) -> Result<Vec<String>, String> {
    let sessions = Session::list(config_dir)?;
    Ok(sessions
        .into_iter()
        .enumerate()
        .map(|(index, session)| {
            let newest = if index == 0 { " (latest)" } else { "" };
            format!(
                "{}{} · {:.1} KiB",
                session.id,
                newest,
                session.bytes as f64 / 1024.0
            )
        })
        .collect())
}

fn show_sessions(config_dir: &std::path::Path, ui: &mut Ui) {
    match session_lines(config_dir) {
        Ok(lines) if lines.is_empty() => ui.note("no saved sessions"),
        Ok(lines) => ui.note(&format!(
            "saved sessions (newest first):\n  {}\nresume newest: noob --resume latest",
            lines.join("\n  "),
        )),
        Err(error) => ui.error(&error),
    }
}

fn cmd_sessions() -> ExitCode {
    match session_lines(&config::config_dir()) {
        Ok(lines) if lines.is_empty() => {
            println!("no saved sessions");
            ExitCode::SUCCESS
        }
        Ok(lines) => {
            println!("{}", lines.join("\n"));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("noob: {error}");
            ExitCode::FAILURE
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
    let _workspace_lease = if matches!(verb, "add" | "remove" | "rm") {
        match tools::guard::workspace_write_lease(
            &agent.tool_ctx.workspace,
            std::time::Duration::ZERO,
            || INTERRUPTED.load(Ordering::SeqCst),
        ) {
            Ok(lease) => Some(lease),
            Err(tools::guard::WorkspaceLeaseError::Canceled) => {
                ui.note("skill change canceled before touching the workspace");
                return;
            }
            Err(tools::guard::WorkspaceLeaseError::Busy) => {
                ui.error(
                    "skill change blocked: another parent or sub-agent mutation is active; \
                     wait for it or cancel the relevant agent, then retry",
                );
                return;
            }
            Err(tools::guard::WorkspaceLeaseError::Io(error)) => {
                ui.error(&format!(
                    "cannot lock the workspace for the skill change: {error}; nothing changed"
                ));
                return;
            }
        }
    } else {
        None
    };
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
            ui.note(&format!(
                "skills ({}):\n{}",
                agent.tool_ctx.skills.len(),
                lines.join("\n")
            ));
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
            let dir = agent
                .tool_ctx
                .skills
                .iter()
                .find(|s| s.name == rest)
                .map(|s| s.dir.clone());
            match dir {
                None => ui.error(&format!(
                    "no installed skill named {rest:?}; /skills lists them"
                )),
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

fn handle_mcp(args: &str, agent: &mut Agent, ui: &mut Ui) {
    let mut parts = args.splitn(2, char::is_whitespace);
    let verb = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();
    let project = mcp::config::project_path(&agent.tool_ctx.workspace);
    match verb {
        "" | "list" => {
            let Some(mgr) = &agent.tool_ctx.mcp else {
                ui.note(
                    "no MCP servers configured; /mcp add <name> <url|command> installs one \
                     (persisted in .noob/mcp.json)",
                );
                return;
            };
            let lines: Vec<String> = mgr
                .names()
                .iter()
                .map(|name| {
                    let state = match mgr.connection(name) {
                        Some(conn) => format!("connected, {} tools", conn.tools().len()),
                        None => "not connected".to_string(),
                    };
                    format!("  {name}: {state}")
                })
                .collect();
            ui.note(&format!(
                "mcp servers ({}):\n{}\nconnect one with /mcp connect <name> (the model uses mcp_connect)",
                lines.len(),
                lines.join("\n")
            ));
        }
        "add" => {
            let Some((name, spec)) = rest
                .split_once(char::is_whitespace)
                .map(|(n, s)| (n.trim(), s.trim()))
            else {
                ui.error(
                    "usage: /mcp add <name> <url|command...> \
                     (e.g. /mcp add deepwiki https://mcp.deepwiki.com/mcp)",
                );
                return;
            };
            let transport = match mcp::config::parse_spec(spec) {
                Ok(t) => t,
                Err(e) => {
                    ui.error(&format!("mcp add failed: {e}"));
                    return;
                }
            };
            if let Err(e) = mcp::config::add_server(&project, name, &transport) {
                ui.error(&format!("mcp add failed: {e}"));
                return;
            }
            let (added, removed) = agent.reload_mcp(ui);
            mcp_delta(ui, &added, &removed);
        }
        "remove" | "rm" => {
            if rest.is_empty() {
                ui.error("usage: /mcp remove <name>");
                return;
            }
            match mcp::config::remove_server(&project, rest) {
                Ok(true) => {
                    let (added, removed) = agent.reload_mcp(ui);
                    mcp_delta(ui, &added, &removed);
                }
                Ok(false) => {
                    let hint = if agent
                        .tool_ctx
                        .mcp
                        .as_ref()
                        .is_some_and(|m| m.names().contains(&rest))
                    {
                        format!(
                            "{rest:?} is not in the project file; it comes from the global \
                             <config>/mcp.json, edit that file to remove it"
                        )
                    } else {
                        format!("no MCP server named {rest:?}; /mcp lists them")
                    };
                    ui.error(&hint);
                }
                Err(e) => ui.error(&format!("mcp remove failed: {e}")),
            }
        }
        "connect" => {
            if rest.is_empty() {
                ui.error("usage: /mcp connect <name>");
                return;
            }
            let Some(mgr) = &agent.tool_ctx.mcp else {
                ui.error("no MCP servers configured; /mcp add <name> <url|command> installs one");
                return;
            };
            match mgr.connect(rest) {
                Ok(info) => {
                    let names: Vec<&str> = info.tools.iter().map(|t| t.name.as_str()).collect();
                    ui.note(&format!(
                        "connected {rest} (protocol {}): {} tools: {}",
                        info.protocol,
                        info.tools.len(),
                        names.join(", ")
                    ));
                }
                Err(e) => ui.error(&format!("mcp connect failed: {e}")),
            }
        }
        other => ui.error(&format!(
            "unknown /mcp subcommand {other:?}; use: list, add, remove, connect"
        )),
    }
}

/// One line summarizing what an `/mcp` change did, mirroring `skills_delta`.
fn mcp_delta(ui: &mut Ui, added: &[String], removed: &[String]) {
    if added.is_empty() && removed.is_empty() {
        ui.note("mcp: no change");
        return;
    }
    let mut parts = Vec::new();
    if !added.is_empty() {
        parts.push(format!("added {}", added.join(", ")));
    }
    if !removed.is_empty() {
        parts.push(format!("removed {}", removed.join(", ")));
    }
    ui.note(&format!("mcp: {}", parts.join("; ")));
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
        "noob {} · {endpoint} · context {}{session}\ntype a task; {}",
        env!("CARGO_PKG_VERSION"),
        tools::context::token_label(agent.ctx_tokens),
        ui::commands::banner()
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
        let names: Vec<&str> = agent
            .tool_ctx
            .skills
            .iter()
            .map(|s| s.name.as_str())
            .collect();
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
    let plan_line = if agent.plan {
        "\nplan mode: on (read-only; /go approves)"
    } else {
        ""
    };
    ui.note(&format!(
        "endpoint: {endpoint}\ncontext: ~{} / {} tokens ({pct}%; {est} / {})\n{usage}{skills_line}{mcp_line}{plan_line}{session}",
        tools::context::token_label(est),
        tools::context::token_label(agent.ctx_tokens),
        agent.ctx_tokens,
    ));
}

fn handle_config(args: &str, agent: &Agent, ui: &mut Ui) {
    let path = agent.config_dir.join(".env");
    if args.is_empty() || args == "list" {
        let lines = config::EDITABLE
            .iter()
            .map(|(name, key)| {
                let value =
                    config::setting(&agent.config_dir, key).unwrap_or_else(|| "(default)".into());
                format!("  {name} = {value}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        ui.note(&format!(
            "config: {}\n{lines}\nuse /config set <name> <value> or /config unset <name>; API keys stay in the file, never terminal history",
            path.display()
        ));
        return;
    }

    let mut parts = args.split_whitespace();
    let verb = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    let result = match verb {
        "set" => {
            let value = parts.collect::<Vec<_>>().join(" ");
            if name.is_empty() || value.is_empty() {
                Err("usage: /config set <name> <value>".to_string())
            } else {
                config::write_setting(&agent.config_dir, name, Some(&value))
            }
        }
        "unset" if !name.is_empty() && parts.next().is_none() => {
            config::write_setting(&agent.config_dir, name, None)
        }
        "unset" => Err("usage: /config unset <name>".to_string()),
        _ => Err("usage: /config [list|set <name> <value>|unset <name>]".to_string()),
    };
    match result {
        Ok(key) => {
            let reload = match key {
                "NOOB_BASE_URL" if verb == "unset" => {
                    "restart noob to run localhost autodetect; an exported variable or CLI flag still overrides it"
                }
                "NOOB_BASE_URL" if agent.ov.base_url.is_some() => {
                    "restart noob without a CLI override to apply it; this process is pinned to its startup endpoint"
                }
                "NOOB_BASE_URL" => {
                    "applies on the next model request unless an exported variable overrides it"
                }
                "NOOB_MODEL" | "NOOB_API_STYLE" => {
                    "applies on the next model request unless a CLI flag or exported variable overrides it"
                }
                _ => "restart noob to apply it",
            };
            ui.note(&format!("saved {key} in {} · {reload}", path.display()));
        }
        Err(error) => ui.error(&format!("config: {error}")),
    }
}

// ---------------------------------------------------------------------------
// exec
// ---------------------------------------------------------------------------

fn cmd_exec(args: &[String]) -> ExitCode {
    const USAGE: &str = "usage: noob exec -p \"<prompt>\" [--json] [--resume <id>] \
                         [--session <id> | --restore <id>] \
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
            "--resume" | "--session" | "--restore" => {
                value_for(arg, it.next(), USAGE).map(|v| session_id = Some(v))
            }
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

    let mut ui = Ui::new(if json_mode {
        Mode::ExecJson
    } else {
        Mode::Exec
    });
    let session = session_id.map(Some);
    let mut boot = BootArgs::new(ov, yolo, plan, session);
    boot.verbose = verbose;
    // exec never redisplays a resumed transcript (byte-identity), so the
    // resume-miss flag is dropped here.
    let (mut agent, _) = match bootstrap(boot, &mut ui) {
        Ok(pair) => pair,
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
/// (`{"prompt", "tools": "read-only"|"web"|"all", "max_turns"}`), runs a fresh
/// scoped context with no parent history, streams progress to stderr, and
/// writes exactly one JSON result line to stdout:
/// `{"status": "ok"|"error", "result", "turns", "usage"}`.
fn cmd_child() -> ExitCode {
    subagent::install_parent_death_cleanup();
    /// Bound on the task payload; a parent never sends anything near this.
    const STDIN_CAP: u64 = 8 * 1024 * 1024;
    let mut payload = String::new();
    let read = std::io::stdin()
        .lock()
        .take(STDIN_CAP)
        .read_to_string(&mut payload);
    let parsed: Option<serde_json::Value> = match read {
        Ok(_) => serde_json::from_str(payload.trim()).ok(),
        Err(_) => None,
    };
    let Some(task_obj) = parsed.filter(|v| v.is_object()) else {
        return child_result(
            "error",
            "no task: stdin must carry one JSON object",
            0,
            None,
        );
    };
    let Some(prompt_text) = task_obj
        .get("prompt")
        .and_then(serde_json::Value::as_str)
        .filter(|p| !p.trim().is_empty())
    else {
        return child_result(
            "error",
            "no task: the JSON object needs a \"prompt\"",
            0,
            None,
        );
    };
    let (read_only, web_only) = match task_obj.get("tools").and_then(serde_json::Value::as_str) {
        None | Some("read-only") => (true, false),
        Some("web") => (true, true),
        Some("all") => (false, false),
        Some(other) => {
            return child_result(
                "error",
                &format!("unknown tools mode {other:?}; use \"read-only\", \"web\", or \"all\""),
                0,
                None,
            );
        }
    };

    let mut ui = Ui::new(Mode::Child);
    let runtime = task_obj.get("_noob_runtime");
    let mut overrides = Overrides::default();
    if let Some(runtime) = runtime {
        overrides.base_url = runtime
            .get("base_url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        overrides.model = runtime
            .get("model")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        overrides.api_style = runtime
            .get("api_style")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
    }
    let mut boot = BootArgs::new(
        overrides,
        runtime
            .and_then(|value| value.get("yolo"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        false,
        None,
    );
    boot.read_only = read_only;
    boot.web_only = web_only;
    boot.verbose = runtime
        .and_then(|value| value.get("verbose"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    boot.excluded_skills = runtime
        .and_then(|value| value.get("excluded_skills"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .collect();
    let (mut agent, _) = match bootstrap(boot, &mut ui) {
        Ok(pair) => pair,
        Err(e) => return child_result("error", &e, 0, None),
    };
    // The sub-agent lifecycle contract, layered system-level: one goal in,
    // one final report out, the instance closes. Deliberately hard language;
    // a small model treats soft phrasing as optional.
    agent.system.push_str(
        "\n\n# Sub-agent contract\n\
         You are a sub-agent with exactly ONE goal: the task in the next message. \
         You MUST complete it and end with a single final message carrying your \
         complete result; that message is returned to the orchestrator that spawned \
         you and closes this instance. NEVER wait for further input, NEVER ask \
         questions (nobody can answer), and NEVER idle in sleep or polling loops: \
         goal done, report, stop.",
    );
    // Both sides enforce the turn cap: the parent clamped its request; the
    // child clamps that against its own environment's ceiling.
    let env_cap = config::task_max_turns(&agent.config_dir);
    agent.max_rounds = task_obj
        .get("max_turns")
        .and_then(serde_json::Value::as_u64)
        .map(|n| (n as u32).clamp(1, env_cap))
        .unwrap_or(env_cap);
    // A child that runs out of rounds mid-gathering delivers nothing; nudge
    // it to write the report while budget remains (also covers the web
    // evidence-gate correction run, which reuses the unused budget).
    agent.budget_nudge = true;

    let original_round_cap = agent.max_rounds;
    let mut end = agent.run_input(prompt_text, &mut ui);
    let mut total_rounds = agent.last_rounds;

    // A web-research child must prove that it actually consulted sources.
    // Small local models sometimes answer from memory even with the MCP tools
    // in their schema, so give one explicit correction without extending the
    // original child budget. Aborts and interrupts are terminal and pass
    // through untouched.
    if web_only
        && matches!(&end, RunEnd::Completed(_))
        && completed_mcp_call_count(&agent.items) < 2
    {
        let remaining = original_round_cap.saturating_sub(total_rounds);
        if remaining == 0 {
            end = RunEnd::Aborted(format!(
                "web research returned without the required 2 mcp_call evidence calls, and the original {original_round_cap}-round budget is exhausted"
            ));
        } else {
            let server = agent
                .tool_ctx
                .mcp
                .as_ref()
                .and_then(|mcp| mcp.names().into_iter().next())
                .unwrap_or("websearch")
                .to_string();
            agent.max_rounds = remaining;
            let correction = format!(
                "[web research evidence gate] No usable web evidence was gathered. Use mcp_connect on the configured server {server:?}, then make at least 2 successful mcp_call operations that gather usable source evidence by searching and fetching primary sources before returning a corrected synthesis. Do not answer from memory."
            );
            end = agent.run_input(&correction, &mut ui);
            total_rounds = total_rounds.saturating_add(agent.last_rounds);
            if matches!(&end, RunEnd::Completed(_)) && completed_mcp_call_count(&agent.items) < 2 {
                end = RunEnd::Aborted(
                    "web research returned without the required 2 mcp_call evidence calls after one corrective follow-up"
                        .to_string(),
                );
            }
        }
    }

    let (status, result) = match end {
        RunEnd::Completed(text) => ("ok", text),
        RunEnd::Aborted(msg) => ("error", msg),
        RunEnd::Interrupted => ("error", "interrupted".to_string()),
    };
    child_result(status, &result, total_rounds, agent.last_usage())
}

fn completed_mcp_call_count(items: &[Item]) -> usize {
    let call_ids: HashSet<&str> = items
        .iter()
        .filter_map(|item| match item {
            Item::Assistant { tool_calls, .. } => Some(tool_calls),
            _ => None,
        })
        .flatten()
        .filter(|call| call.name == "mcp_call")
        .map(|call| call.id.as_str())
        .collect();
    let completed_ids: HashSet<&str> = items
        .iter()
        .filter_map(|item| match item {
            Item::ToolResult { call_id, content }
                if content.starts_with("[untrusted content from MCP server ") =>
            {
                Some(call_id.as_str())
            }
            _ => None,
        })
        .collect();
    call_ids.intersection(&completed_ids).count()
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
    if status == "ok" {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
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
    let skill_paths = config::skill_paths(&config_dir, &workspace);
    let discovered = skills::discover(&workspace, &config_dir, &skill_paths);
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
        if current_depth() < subagent::MAX_DEPTH {
            tool_specs.push(subagent::spec());
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

/// A terminal resize flips a flag the dock's stdin reader consumes on EINTR, so
/// an idle prompt reflows its box to the new width without a keystroke. SIGWINCH
/// is blocked in this (main) thread and therefore in every thread spawned after
/// this call, so the only thread that can catch it is the reader, which unblocks
/// it for itself: that guarantees the signal interrupts the read rather than
/// racing an unrelated blocking call. Cheap and event-driven: no idle polling.
fn install_sigwinch_handler() {
    extern "C" fn on_sigwinch(_: libc::c_int) {
        ui::WINCH.store(true, Ordering::SeqCst);
    }
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_sigwinch as *const () as usize;
        // No SA_RESTART: the reader's blocked read returns EINTR and injects the
        // resize event instead of resuming as if nothing happened.
        sa.sa_flags = 0;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGWINCH, &sa, std::ptr::null_mut());
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGWINCH);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}
