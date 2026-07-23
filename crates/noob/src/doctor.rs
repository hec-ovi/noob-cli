//! `noob doctor` (P7): diagnose the setup and print the one-line fix for
//! every problem. Checks run in dependency order (config dir, .env,
//! endpoint, llama.cpp capacity, wire shape, mcp.json, workspace, sandbox); nothing here
//! mutates state beyond a writability probe file that is removed again.
//! Exit code 0 when everything needed for a working chat is in place, 1
//! when at least one FAIL line printed.

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use noob_provider::types::{ApiStyle, Overrides, ProviderError};

use crate::{config, mcp};

const REACH_TIMEOUT: Duration = Duration::from_secs(2);

// Load-bearing literals: doctor classifies producer errors by matching their
// prose, so these strings MUST stay substrings of the producer's message.
// MISSING_BASE_URL is emitted by noob-provider resolve_endpoint (lib.rs, the
// missing-NOOB_BASE_URL Config error); MCP_INVALID_JSON by mcp::config::load
// (the unparseable-mcp.json warning). The tests below build the real producer
// errors, so rewording either producer breaks a test instead of silently
// changing doctor's classification.
const MISSING_BASE_URL: &str = "NOOB_BASE_URL is not set";
const MCP_INVALID_JSON: &str = "not valid JSON";

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
        Err(ProviderError::Config(msg)) if !msg.contains(MISSING_BASE_URL) => {
            checks.push(Check::Fail(format!("endpoint config: {msg}")));
            None
        }
        Err(ProviderError::Config(_)) => match config::autodetect_base_url(config_dir) {
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
        },
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
    let reachable = match noob_provider::http::get_status(
        &format!("{}/models", ep.base_url),
        &ep.api_key,
        REACH_TIMEOUT,
    ) {
        Ok((status, _)) if (200..300).contains(&status) => {
            checks.push(Check::Ok(format!(
                "endpoint {} answers /models (HTTP {status}); model {:?}, style {style}",
                ep.base_url, ep.model
            )));
            true
        }
        Ok((s @ (401 | 403), _)) => {
            checks.push(Check::Fail(format!(
                "endpoint {} rejects the key (HTTP {s}); fix: set NOOB_API_KEY in the \
                 config .env",
                ep.base_url
            )));
            false
        }
        Ok((status, _)) => {
            checks.push(Check::Fail(format!(
                "endpoint {} answered HTTP {status} on /models; fix: check that \
                 NOOB_BASE_URL points at an OpenAI-compatible /v1 base",
                ep.base_url
            )));
            false
        }
        Err(e) => {
            checks.push(Check::Fail(format!(
                "endpoint {} is unreachable ({e}); fix: start the server, or correct \
                 NOOB_BASE_URL in the config .env",
                ep.base_url
            )));
            false
        }
    };
    if reachable {
        checks.extend(check_llama_slots(config_dir, &ep));
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

/// llama.cpp documents `total_slots` on GET `/props` as the capacity set by
/// `--parallel`. The parent consumes one inference slot while it is thinking,
/// so fully concurrent detached work needs one more slot than the configured
/// child cap. Other OpenAI-compatible providers are silent here: a missing,
/// rejected, malformed, or non-llama `/props` response is not a diagnosis.
fn check_llama_slots(config_dir: &Path, ep: &noob_provider::types::Endpoint) -> Vec<Check> {
    // OpenAI is known not to expose llama.cpp properties. Avoid an irrelevant
    // request (and its latency) against the first-party endpoint entirely.
    if ep.base_url.contains("api.openai.com") {
        return Vec::new();
    }
    let root = ep.base_url.strip_suffix("/v1").unwrap_or(&ep.base_url);
    let model = encode_query_component(&ep.model);
    // Router GETs auto-load an unloaded model unless this per-request switch
    // is present. Diagnosis must remain read-only and must not consume VRAM.
    let url = format!("{root}/props?model={model}&autoload=false");
    let Ok((status, body)) = noob_provider::http::get_status(&url, &ep.api_key, REACH_TIMEOUT)
    else {
        return Vec::new();
    };
    if !(200..300).contains(&status) {
        return Vec::new();
    }
    let Some(props) = llama_properties(&body) else {
        return Vec::new();
    };

    let children = config::task_concurrency(config_dir) as u64;
    let needed = children + 1;
    let mut checks = Vec::new();
    if props.slots >= needed {
        checks.push(Check::Ok(format!(
            "llama.cpp slots: {} available; enough for the parent + {children} detached sub-agents",
            props.slots
        )));
    } else {
        let alternative = match props.slots.checked_sub(1) {
            Some(max_children @ 1..) => format!(
                "restart llama-server with --parallel {needed}, adequate --ctx-size, and \
                 --kv-unified where appropriate, or set NOOB_TASK_CONCURRENCY={max_children}"
            ),
            _ => format!(
                "restart llama-server with --parallel {needed}, adequate --ctx-size, and \
                 --kv-unified where appropriate; one slot cannot run the parent and a detached \
                 sub-agent together"
            ),
        };
        checks.push(Check::Warn(format!(
            "llama.cpp has {} slot(s), but the parent + {children} configured detached sub-agents need {needed}; fix: {alternative}",
            props.slots
        )));
    }

    let expected_ctx = config::ctx_tokens(config_dir);
    if let Some(n_ctx) = props.n_ctx
        && n_ctx < expected_ctx
    {
        checks.push(Check::Warn(format!(
            "llama.cpp reports n_ctx={n_ctx}, below NOOB_CTX={expected_ctx}; fix: set \
             NOOB_CTX={n_ctx}, or raise llama-server --ctx-size while preserving enough context \
             across its parallel slots"
        )));
    }

    if props.supports_tools == Some(false) || props.supports_tool_calls == Some(false) {
        checks.push(Check::Warn(
            "llama.cpp chat template reports tool calling disabled; multi-agent calls may fail; \
             fix: use a model/template whose /props chat_template_caps enables supports_tools \
             and supports_tool_calls"
                .to_string(),
        ));
    } else if children > 1 && props.supports_parallel_tool_calls == Some(false) {
        checks.push(Check::Warn(
            "llama.cpp chat template reports parallel tool calls disabled; sub-agent fan-out may \
             serialize; fix: use a model/template whose /props chat_template_caps enables \
             supports_parallel_tool_calls"
                .to_string(),
        ));
    }
    checks
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LlamaProperties {
    slots: u64,
    n_ctx: Option<u64>,
    supports_tools: Option<bool>,
    supports_tool_calls: Option<bool>,
    supports_parallel_tool_calls: Option<bool>,
}

/// Require multiple documented llama.cpp `/props` fields before treating an
/// arbitrary OpenAI-compatible endpoint as llama.cpp. Types are deliberately
/// strict: ambiguous provider JSON must not create a false warning.
fn llama_properties(body: &str) -> Option<LlamaProperties> {
    let props: serde_json::Value = serde_json::from_str(body).ok()?;
    let defaults = props.get("default_generation_settings")?.as_object()?;
    if props.get("build_info")?.as_str()?.is_empty() {
        return None;
    }
    let caps = props
        .get("chat_template_caps")
        .and_then(|value| value.as_object());
    Some(LlamaProperties {
        slots: props.get("total_slots")?.as_u64()?,
        n_ctx: defaults.get("n_ctx").and_then(serde_json::Value::as_u64),
        supports_tools: caps
            .and_then(|value| value.get("supports_tools"))
            .and_then(serde_json::Value::as_bool),
        supports_tool_calls: caps
            .and_then(|value| value.get("supports_tool_calls"))
            .and_then(serde_json::Value::as_bool),
        supports_parallel_tool_calls: caps
            .and_then(|value| value.get("supports_parallel_tool_calls"))
            .and_then(serde_json::Value::as_bool),
    })
}

fn encode_query_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[(byte >> 4) as usize]));
            encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

fn check_mcp(workspace: &Path, config_dir: &Path) -> Vec<Check> {
    let mut checks = Vec::new();
    let global = config_dir.join("mcp.json");
    let project = workspace.join(".noob/mcp.json");
    if !global.is_file() && !project.is_file() {
        checks.push(Check::Ok(
            "no mcp.json (MCP tools stay unregistered)".to_string(),
        ));
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
        if !w.contains(MCP_INVALID_JSON) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llama_capacity_requires_the_documented_props_shape_and_integer_slots() {
        assert_eq!(
            llama_properties(
                r#"{"default_generation_settings":{},"build_info":"b1-test","total_slots":5}"#
            )
            .map(|props| props.slots),
            Some(5)
        );
        for body in [
            r#"{"total_slots":5}"#,
            r#"{"default_generation_settings":{},"build_info":"","total_slots":5}"#,
            r#"{"default_generation_settings":{},"build_info":"b1-test","total_slots":"5"}"#,
            r#"{"default_generation_settings":{},"build_info":"b1-test","total_slots":-1}"#,
            "not json",
        ] {
            assert_eq!(llama_properties(body), None, "accepted {body}");
        }
    }

    #[test]
    fn llama_properties_reads_context_and_explicit_tool_capabilities() {
        let props = llama_properties(
            r#"{"default_generation_settings":{"n_ctx":32768},"build_info":"b1-test",
                "total_slots":3,"chat_template_caps":{"supports_tools":true,
                "supports_tool_calls":false,"supports_parallel_tool_calls":false}}"#,
        )
        .unwrap();
        assert_eq!(props.slots, 3);
        assert_eq!(props.n_ctx, Some(32768));
        assert_eq!(props.supports_tools, Some(true));
        assert_eq!(props.supports_tool_calls, Some(false));
        assert_eq!(props.supports_parallel_tool_calls, Some(false));
    }

    #[test]
    fn missing_base_url_classifier_matches_the_real_provider_error() {
        // A host-exported URL makes the error unproducible in-process; the
        // spawned-binary suites scrub, but this test shares the harness env.
        if std::env::var("NOOB_BASE_URL").is_ok_and(|v| !v.is_empty()) {
            return;
        }
        let config = tempfile::tempdir().unwrap();
        let err = noob_provider::resolve_endpoint(config.path(), &Overrides::default())
            .expect_err("no base URL is configured");
        match err {
            ProviderError::Config(msg) => assert!(
                msg.contains(MISSING_BASE_URL),
                "resolve_endpoint reworded its missing-URL error; update \
                 MISSING_BASE_URL and check_endpoint: {msg}"
            ),
            other => panic!("expected a Config error, got {other:?}"),
        }
    }

    #[test]
    fn invalid_json_classifier_matches_the_real_mcp_loader_warning() {
        let config = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(config.path().join("mcp.json"), "{not json").unwrap();
        let (servers, warnings) = mcp::config::load(workspace.path(), config.path());
        assert!(servers.is_empty());
        assert!(
            warnings.iter().any(|w| w.contains(MCP_INVALID_JSON)),
            "mcp::config::load reworded its invalid-JSON warning; update \
             MCP_INVALID_JSON and check_mcp: {warnings:?}"
        );
    }

    #[test]
    fn model_query_component_is_percent_encoded() {
        assert_eq!(
            encode_query_component("org/model:Q4 K/ñ"),
            "org%2Fmodel%3AQ4%20K%2F%C3%B1"
        );
    }
}
