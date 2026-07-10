//! noob: argv dispatch only. Subcommands: (repl) | exec | child | doctor |
//! debug. Hand-rolled parsing; the whole surface is a handful of flags.

mod agent;
mod config;
mod session;
mod skills;
mod tools;
mod ui;

use std::io::BufRead;
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
        Some("child") => not_yet("child", "P6"),
        Some("doctor") => not_yet("doctor", "P7"),
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

fn not_yet(cmd: &str, phase: &str) -> ExitCode {
    eprintln!("noob {cmd} lands in {phase}; this build is the P2 core loop");
    ExitCode::from(2)
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
    /// None = no persistence; Some(None) = fresh id; Some(Some(id)) = resume.
    session: Option<Option<String>>,
}

fn bootstrap(boot: BootArgs, ui: &mut Ui) -> Result<Agent, String> {
    let config_dir = config::config_dir();
    let mut ov = boot.ov;
    if ov.base_url.is_none() && config::setting(&config_dir, "NOOB_BASE_URL").is_none() {
        if let Some(found) = config::autodetect_base_url() {
            ui.note(&format!("using {found} (autodetected)"));
            ov.base_url = Some(found);
        }
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
    let inputs = prompt::PromptInputs {
        cwd: workspace.display().to_string(),
        model,
        sandbox: sandbox_label,
        global_agents: prompt::load_agents_md(&config_dir),
        project_agents: prompt::load_agents_md(&workspace),
        skills_index: skills::index(&discovered),
        mcp_line: None,
    };
    let system = prompt::assemble(&inputs);
    // Registered set is decided here and stays byte-stable for the session:
    // the skill tool exists only when discovery found at least one skill.
    let mut tool_specs = tools::specs();
    if !discovered.is_empty() {
        tool_specs.push(tools::skill::spec());
    }
    let mut tool_ctx = ToolCtx::new(workspace, sandbox);
    tool_ctx.skills = discovered;

    let (session, replayed) = match boot.session {
        None => (None, Vec::new()),
        Some(id) => {
            let (s, items) = Session::open(&config_dir, id.as_deref())?;
            (Some(s), items)
        }
    };
    Ok(Agent::new(
        Client::new(Timeouts::default()),
        config_dir.clone(),
        ov,
        system,
        tool_specs,
        replayed,
        tool_ctx,
        session,
        config::ctx_tokens(&config_dir),
    ))
}

// ---------------------------------------------------------------------------
// REPL
// ---------------------------------------------------------------------------

fn cmd_repl(args: &[String]) -> ExitCode {
    const USAGE: &str = "usage: noob [--model <name>] [--base-url <url>] [--yolo]";
    let mut ov = Overrides::default();
    let mut yolo = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let taken = match arg.as_str() {
            "--model" => value_for(arg, it.next(), USAGE).map(|v| ov.model = Some(v)),
            "--base-url" => value_for(arg, it.next(), USAGE).map(|v| ov.base_url = Some(v)),
            "--yolo" => {
                yolo = true;
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
    let mut agent = match bootstrap(BootArgs { ov, yolo, session: Some(None) }, &mut ui) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("noob: {e}");
            return ExitCode::from(2);
        }
    };
    greet(&agent, &mut ui);

    let stdin = std::io::stdin();
    loop {
        ui.end_line();
        prompt_marker();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF (Ctrl-D)
            Ok(_) => {}
            Err(_) => break,
        }
        // Ctrl-C while at the prompt: std's read_line retries EINTR
        // internally, so the flag is the only signal it happened. The tty
        // has already flushed the typed input (ISIG), so drop this line,
        // clear the flag, and prompt again; a second Ctrl-C before any
        // input hard-exits via the signal handler.
        if INTERRUPTED.swap(false, Ordering::SeqCst) {
            ui.note("(interrupted; /quit to exit)");
            continue;
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if let Some(cmd) = input.strip_prefix('/') {
            match cmd.trim() {
                "quit" | "q" | "exit" => break,
                "status" => status(&agent, &mut ui),
                "compact" => {
                    agent.compact(&mut ui);
                    // A Ctrl-C during a manual compaction was consumed by
                    // it; a stale flag would phantom-cancel the next input.
                    INTERRUPTED.store(false, Ordering::SeqCst);
                }
                "plan" | "go" => ui.note("plan mode lands in P5"),
                other => ui.note(&format!(
                    "unknown command /{other}; available: /status /compact /quit"
                )),
            }
            continue;
        }
        match agent.run_input(input, &mut ui) {
            RunEnd::Completed(_) | RunEnd::Interrupted => {}
            RunEnd::Aborted(msg) => ui.note(&format!("error: {msg}")),
        }
    }
    ExitCode::SUCCESS
}

fn prompt_marker() {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(b"> ");
    let _ = out.flush();
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
    ui.note(&format!(
        "noob {} · {endpoint}{session}\ntype a task; /status /compact /quit",
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
    let session = agent
        .session
        .as_ref()
        .map(|s| format!("\nsession: {}", s.path().display()))
        .unwrap_or_default();
    ui.note(&format!(
        "endpoint: {endpoint}\ncontext: ~{est} of {} tokens ({pct}%)\n{usage}{skills_line}{session}",
        agent.ctx_tokens
    ));
}

// ---------------------------------------------------------------------------
// exec
// ---------------------------------------------------------------------------

fn cmd_exec(args: &[String]) -> ExitCode {
    const USAGE: &str = "usage: noob exec -p \"<prompt>\" [--json] [--session <id>] \
                         [--model <name>] [--base-url <url>] [--yolo]";
    let mut prompt_arg: Option<String> = None;
    let mut ov = Overrides::default();
    let mut json_mode = false;
    let mut yolo = false;
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
    let mut agent = match bootstrap(BootArgs { ov, yolo, session }, &mut ui) {
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
    let inputs = prompt::PromptInputs {
        cwd: workspace.display().to_string(),
        model,
        sandbox: sandbox_label,
        global_agents: prompt::load_agents_md(&config_dir),
        project_agents: prompt::load_agents_md(&workspace),
        skills_index: skills::index(&discovered),
        mcp_line: None,
    };
    let system = prompt::assemble(&inputs);
    let head = prompt::head(&inputs);
    if json_mode {
        let mut tool_specs = tools::specs();
        if !discovered.is_empty() {
            tool_specs.push(tools::skill::spec());
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
