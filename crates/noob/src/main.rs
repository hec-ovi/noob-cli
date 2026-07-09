//! noob: argv dispatch only. Subcommands: (repl) | exec | child | doctor | debug.
//! Hand-rolled parsing; the whole surface is a handful of flags.

mod config;

use std::process::ExitCode;
use std::sync::atomic::Ordering;

use noob_provider::http::{Client, INTERRUPTED, Timeouts};
use noob_provider::types::{Overrides, ProviderError};
use serde_json::json;

fn main() -> ExitCode {
    install_sigint_handler();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") | Some("-V") => {
            println!("noob {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("exec") => cmd_exec(&args[1..]),
        Some("child") => not_yet("child", "P6"),
        Some("doctor") => not_yet("doctor", "P7"),
        Some("debug") => not_yet("debug", "P2"),
        None => {
            eprintln!(
                "the interactive REPL lands in P2; for now run: noob exec -p \"<prompt>\""
            );
            ExitCode::from(2)
        }
        Some(other) => {
            eprintln!(
                "noob: unknown command {other:?}; available: exec, doctor, debug, --version"
            );
            ExitCode::from(2)
        }
    }
}

fn not_yet(cmd: &str, phase: &str) -> ExitCode {
    eprintln!("noob {cmd} lands in {phase}; this build is the P0 scaffold");
    ExitCode::from(2)
}

fn cmd_exec(args: &[String]) -> ExitCode {
    const USAGE: &str = "usage: noob exec -p \"<prompt>\" [--model <name>] [--base-url <url>]";
    // A flag's value must exist and must not look like another flag;
    // consuming blindly turns one forgotten value into a silent misconfig.
    fn value_for(flag: &str, next: Option<&String>) -> Result<String, String> {
        match next {
            Some(v) if !v.starts_with('-') => Ok(v.clone()),
            _ => Err(format!("noob exec: {flag} needs a value; {USAGE}")),
        }
    }

    let mut prompt: Option<String> = None;
    let mut ov = Overrides::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let taken = match arg.as_str() {
            "-p" | "--prompt" => value_for(arg, it.next()).map(|v| prompt = Some(v)),
            "--model" => value_for(arg, it.next()).map(|v| ov.model = Some(v)),
            "--base-url" => value_for(arg, it.next()).map(|v| ov.base_url = Some(v)),
            other => Err(format!("noob exec: unknown flag {other:?}; {USAGE}")),
        };
        if let Err(msg) = taken {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    }
    let Some(prompt) = prompt.filter(|p| !p.is_empty()) else {
        eprintln!("noob exec: missing prompt; {USAGE}");
        return ExitCode::from(2);
    };

    let config_dir = config::config_dir();
    let client = Client::new(Timeouts::default());
    // P0: a single user message and one turn. The system prompt, tools, and
    // the agent loop land in P2.
    let messages = vec![json!({"role": "user", "content": prompt})];
    match noob_provider::run_turn(&client, &config_dir, &ov, &messages) {
        Ok(turn) => {
            println!("{}", turn.text);
            ExitCode::SUCCESS
        }
        Err(ProviderError::Interrupted) => {
            eprintln!("noob: interrupted");
            ExitCode::from(130)
        }
        Err(e) => {
            eprintln!("noob: {e}");
            ExitCode::FAILURE
        }
    }
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
