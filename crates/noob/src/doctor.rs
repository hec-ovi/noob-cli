//! `noob doctor` (P7): diagnose the setup and print the one-line fix for
//! every problem. Checks run in dependency order (config dir, .env,
//! endpoint, wire shape, mcp.json, workspace, sandbox); nothing here
//! mutates state beyond a writability probe file that is removed again.
//! Exit code 0 when everything needed for a working chat is in place, 1
//! when at least one FAIL line printed.

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use noob_provider::types::{ApiStyle, Overrides, ProviderError};

use crate::{config, mcp};

const REACH_TIMEOUT: Duration = Duration::from_secs(2);

/// One check's outcome; doctor renders each as a single line.
enum Check {
    Ok(String),
    Warn(String),
    Fail(String),
}

pub fn run() -> ExitCode {
    let config_dir = config::config_dir();
    let workspace = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let mut checks: Vec<Check> = Vec::new();

    checks.push(check_config_dir(&config_dir));
    checks.push(check_env_file(&config_dir));
    checks.extend(check_endpoint(&config_dir));
    checks.extend(check_mcp(&workspace, &config_dir));
    checks.push(check_workspace(&workspace));
    let (_, sandbox_label) = config::detect_sandbox(&config_dir, false);
    checks.push(Check::Ok(format!("sandbox: {sandbox_label}")));

    let mut failed = false;
    for check in &checks {
        match check {
            Check::Ok(line) => println!("ok    {line}"),
            Check::Warn(line) => println!("warn  {line}"),
            Check::Fail(line) => {
                failed = true;
                println!("FAIL  {line}");
            }
        }
    }
    if failed {
        println!("\nfix the FAIL lines above, then run noob doctor again");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn check_config_dir(config_dir: &Path) -> Check {
    if !config_dir.is_dir() {
        return Check::Fail(format!(
            "config dir {} does not exist; fix: create it (or bind-mount it in \
             docker compose) and put a .env inside",
            config_dir.display()
        ));
    }
    // A writability probe: sessions and skills land here.
    let probe = config_dir.join(".noob-doctor-probe");
    match std::fs::write(&probe, b"probe") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Check::Ok(format!("config dir {} (writable)", config_dir.display()))
        }
        Err(e) => Check::Fail(format!(
            "config dir {} is not writable ({e}); fix: chown it to the uid you run \
             the container with (compose passes your uid when you use ./dev.sh repl)",
            config_dir.display()
        )),
    }
}

fn check_env_file(config_dir: &Path) -> Check {
    let path = config_dir.join(".env");
    if !path.is_file() {
        return Check::Warn(format!(
            "no {} (localhost autodetect will pick the endpoint); copy \
             config/.env.example there to pin one",
            path.display()
        ));
    }
    match noob_provider::envfile::load(&path) {
        Ok(map) => Check::Ok(format!("{} parsed ({} keys)", path.display(), map.len())),
        Err(e) => Check::Fail(format!(
            "{} does not parse: {e}; fix: correct or comment out the offending line",
            path.display()
        )),
    }
}

fn check_endpoint(config_dir: &Path) -> Vec<Check> {
    let mut ov = Overrides::default();
    let mut checks = Vec::new();
    let resolved = match noob_provider::resolve_endpoint(config_dir, &ov) {
        Ok(ep) => Some(ep),
        // Only the missing-URL case falls through to autodetect; a broken
        // .env (also a Config error) must surface, not be papered over.
        Err(ProviderError::Config(msg)) if !msg.contains("NOOB_BASE_URL is not set") => {
            checks.push(Check::Fail(format!("endpoint config: {msg}")));
            None
        }
        Err(ProviderError::Config(_)) => {
            match config::autodetect_base_url() {
                Some(found) => {
                    checks.push(Check::Ok(format!("endpoint autodetected: {found}")));
                    ov.base_url = Some(found);
                    noob_provider::resolve_endpoint(config_dir, &ov).ok()
                }
                None => {
                    checks.push(Check::Fail(
                        "no endpoint: NOOB_BASE_URL is unset and nothing answered the \
                         localhost probes (:8090 :8080 :11434 :1234 :8000); fix: start \
                         your model server or set NOOB_BASE_URL in the config .env"
                            .to_string(),
                    ));
                    None
                }
            }
        }
        Err(e) => {
            checks.push(Check::Fail(format!("endpoint config: {e}")));
            None
        }
    };
    let Some(ep) = resolved else {
        return checks;
    };
    let style = match ep.style {
        ApiStyle::Chat => "chat",
        ApiStyle::Responses => "responses",
    };
    match noob_provider::http::get_status(&format!("{}/models", ep.base_url), &ep.api_key, REACH_TIMEOUT)
    {
        Ok((status, _)) if (200..300).contains(&status) => {
            checks.push(Check::Ok(format!(
                "endpoint {} answers /models (HTTP {status}); model {:?}, style {style}",
                ep.base_url, ep.model
            )));
        }
        Ok((s @ (401 | 403), _)) => {
            checks.push(Check::Fail(format!(
                "endpoint {} rejects the key (HTTP {s}); fix: set NOOB_API_KEY in the \
                 config .env",
                ep.base_url
            )));
        }
        Ok((status, _)) => {
            checks.push(Check::Fail(format!(
                "endpoint {} answered HTTP {status} on /models; fix: check that \
                 NOOB_BASE_URL points at an OpenAI-compatible /v1 base",
                ep.base_url
            )));
        }
        Err(e) => {
            checks.push(Check::Fail(format!(
                "endpoint {} is unreachable ({e}); fix: start the server, or correct \
                 NOOB_BASE_URL in the config .env",
                ep.base_url
            )));
        }
    }
    // Wire-shape sanity: the responses style outside api.openai.com is
    // usually a misconfiguration (llama.cpp and friends do not serve it).
    if ep.style == ApiStyle::Responses && !ep.base_url.contains("api.openai.com") {
        checks.push(Check::Warn(format!(
            "style is \"responses\" against {}; most local servers only speak \
             chat; set NOOB_API_STYLE=chat if requests fail",
            ep.base_url
        )));
    }
    checks
}

fn check_mcp(workspace: &Path, config_dir: &Path) -> Vec<Check> {
    let mut checks = Vec::new();
    let global = config_dir.join("mcp.json");
    let project = workspace.join(".noob/mcp.json");
    if !global.is_file() && !project.is_file() {
        checks.push(Check::Ok("no mcp.json (MCP tools stay unregistered)".to_string()));
        return checks;
    }
    // Invalid JSON is a FAIL (the user wrote it wanting servers); per-entry
    // skips are warns (the rest of the file still works).
    for path in [&global, &project] {
        if path.is_file()
            && let Ok(text) = std::fs::read_to_string(path)
            && serde_json::from_str::<serde_json::Value>(&text).is_err()
        {
            checks.push(Check::Fail(format!(
                "{} is not valid JSON; fix: correct it or remove it",
                path.display()
            )));
        }
    }
    let (servers, warnings) = mcp::config::load(workspace, config_dir);
    for w in warnings {
        // The invalid-JSON case already failed above; entry skips warn here.
        if !w.contains("not valid JSON") {
            checks.push(Check::Warn(format!("mcp.json: {w}")));
        }
    }
    if !servers.is_empty() {
        let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
        checks.push(Check::Ok(format!(
            "mcp.json: {} server(s) configured ({})",
            servers.len(),
            names.join(", ")
        )));
    }
    checks
}

fn check_workspace(workspace: &Path) -> Check {
    let probe = workspace.join(".noob-doctor-probe");
    match std::fs::write(&probe, b"probe") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Check::Ok(format!("workspace {} (writable)", workspace.display()))
        }
        Err(e) => Check::Fail(format!(
            "workspace {} is not writable ({e}); fix: run the container with your \
             uid (./dev.sh repl does) or chown the mounted folder",
            workspace.display()
        )),
    }
}
