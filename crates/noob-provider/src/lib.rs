//! noob-provider: transcript in, events out.
//!
//! Sole owner of ureq. Resolves `.env` keys fresh on every request build, so
//! editing the config file applies on the next call with no restart.

pub mod assemble;
pub mod chat;
pub mod envfile;
pub mod http;
pub mod responses;
pub mod sse;
pub mod types;

use std::path::Path;

use types::{ApiStyle, Endpoint, Event, Overrides, ProviderError, Turn, TurnRequest};

/// Resolve the endpoint for one request. Called inside every request build:
/// the `.env` file is opened, parsed, and dropped here, which is what makes
/// hot key reload work with no restart.
///
/// Precedence for non-secret settings: override (CLI flag) > process env > `.env`.
/// The API key is read from `.env` only; secrets never enter the process
/// environment where bash subprocesses and child agents could read them.
pub fn resolve_endpoint(config_dir: &Path, ov: &Overrides) -> Result<Endpoint, ProviderError> {
    let env_path = config_dir.join(".env");
    let file = if env_path.is_file() {
        envfile::load(&env_path).map_err(|e| {
            ProviderError::Config(format!(
                "could not read {}: {e}; fix or comment out the offending line",
                env_path.display()
            ))
        })?
    } else {
        Default::default()
    };
    let setting = |key: &str| -> Option<String> {
        if let Some(v) = std::env::var(key).ok().filter(|v| !v.is_empty()) {
            return Some(v);
        }
        file.get(key).cloned().filter(|v| !v.is_empty())
    };

    let base_url = ov
        .base_url
        .clone()
        .or_else(|| setting("NOOB_BASE_URL"))
        .ok_or_else(|| {
            ProviderError::Config(format!(
                "NOOB_BASE_URL is not set and no local endpoint answered the autodetect \
                 probes (ports 8090, 8080, 11434, 1234, 8000); add it to {} (for example \
                 NOOB_BASE_URL=http://localhost:8090/v1)",
                env_path.display()
            ))
        })?;
    let base_url = base_url.trim_end_matches('/').to_string();

    let style_str = ov.api_style.clone().or_else(|| setting("NOOB_API_STYLE"));
    let style = match style_str.as_deref() {
        Some("chat") => ApiStyle::Chat,
        Some("responses") => ApiStyle::Responses,
        Some(other) => {
            return Err(ProviderError::Config(format!(
                "NOOB_API_STYLE is \"{other}\"; set it to \"chat\" or \"responses\", \
                 or remove it to pick by base URL"
            )));
        }
        None => {
            if base_url.contains("api.openai.com") {
                ApiStyle::Responses
            } else {
                ApiStyle::Chat
            }
        }
    };

    Ok(Endpoint {
        base_url,
        api_key: file.get("NOOB_API_KEY").cloned().unwrap_or_default(),
        model: ov
            .model
            .clone()
            .or_else(|| setting("NOOB_MODEL"))
            .unwrap_or_else(|| "default".to_string()),
        style,
    })
}

/// Run one model turn: resolve the endpoint fresh, pick the adapter by
/// api_style, stream events through `on`, return the assembled turn.
pub fn run_turn(
    client: &http::Client,
    config_dir: &Path,
    ov: &Overrides,
    req: &TurnRequest,
    on: &mut dyn FnMut(Event),
) -> Result<Turn, ProviderError> {
    let ep = resolve_endpoint(config_dir, ov)?;
    match ep.style {
        ApiStyle::Chat => chat::stream(client, &ep, req, on),
        ApiStyle::Responses => responses::stream(client, &ep, req, on),
    }
}
