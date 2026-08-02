//! `noob tokens`: what a file actually costs, through the model's own
//! tokenizer.
//!
//! Every other number about token cost in this project is an estimate: bytes
//! over four, or a tokenizer that no served model here uses. Those are drift
//! alarms, not measurements. A prompt is only 2,000 tokens through a particular
//! tokenizer, and the only tokenizer whose answer matters is the one the model
//! in front of you is loaded with.
//!
//! llama.cpp servers expose `/tokenize`, which is that tokenizer. When the
//! endpoint does not have it, the count is refused rather than guessed: a
//! number that might be the answer is worse than no number, because it gets
//! quoted later as if it were.

use noob_provider::types::Overrides;

use crate::config;

/// One file, and what it costs.
struct Counted {
    label: String,
    bytes: usize,
    lines: usize,
    tokens: usize,
}

pub fn run(args: &[String]) -> std::process::ExitCode {
    const USAGE: &str = "usage: noob tokens <path>... [--model <name>] [--base-url <url>]\n\
                         \x20      noob tokens -  (read stdin)";
    let mut ov = Overrides::default();
    let mut paths: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let taken = match arg.as_str() {
            "--model" => crate::value_for(arg, it.next(), USAGE).map(|v| ov.model = Some(v)),
            "--base-url" => crate::value_for(arg, it.next(), USAGE).map(|v| ov.base_url = Some(v)),
            other if other.starts_with("--") => {
                Err(format!("noob tokens: unknown flag {other:?}; {USAGE}"))
            }
            other => {
                paths.push(other.to_string());
                Ok(())
            }
        };
        if let Err(message) = taken {
            eprintln!("{message}");
            return std::process::ExitCode::from(2);
        }
    }
    if paths.is_empty() {
        eprintln!("noob tokens: nothing to measure; {USAGE}");
        return std::process::ExitCode::from(2);
    }

    let config_dir = config::config_dir();
    let endpoint = match noob_provider::resolve_endpoint(&config_dir, &ov) {
        Ok(endpoint) => endpoint,
        Err(e) => {
            eprintln!("noob: {e}");
            return std::process::ExitCode::from(2);
        }
    };
    let client = noob_provider::http::Client::new(noob_provider::http::Timeouts::default());
    let url = tokenize_url(&endpoint.base_url);

    let mut counted = Vec::new();
    for path in &paths {
        let (label, text) = match read(path) {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("noob: {e}");
                return std::process::ExitCode::FAILURE;
            }
        };
        match count(&client, &url, &endpoint.api_key, &endpoint.model, &text) {
            Ok(tokens) => counted.push(Counted {
                label,
                bytes: text.len(),
                lines: text.lines().count(),
                tokens,
            }),
            Err(e) => {
                eprintln!("noob: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
    }

    report(&counted, &endpoint.model);
    std::process::ExitCode::SUCCESS
}

fn read(path: &str) -> Result<(String, String), String> {
    if path == "-" {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
            .map_err(|e| format!("cannot read stdin: {e}"))?;
        return Ok((String::from("(stdin)"), text));
    }
    let text = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let text = String::from_utf8(text)
        .map_err(|_| format!("{path} is not valid UTF-8; a tokenizer has nothing to say about it"))?;
    Ok((path.to_string(), text))
}

/// `/tokenize` sits beside the OpenAI-compatible routes rather than under
/// them: a base URL of `http://host:8080/v1` serves it at `http://host:8080`.
pub fn tokenize_url(base_url: &str) -> String {
    let root = base_url
        .trim_end_matches('/')
        .strip_suffix("/v1")
        .unwrap_or(base_url.trim_end_matches('/'));
    format!("{root}/tokenize")
}

/// Ask the endpoint, and refuse to guess when it cannot answer.
fn count(
    client: &noob_provider::http::Client,
    url: &str,
    api_key: &str,
    model: &str,
    text: &str,
) -> Result<usize, String> {
    let body = serde_json::json!({ "content": text, "model": model });
    let (status, bytes) = client
        .post_json(url, api_key, &body)
        .map_err(|e| format!("{url}: {e}"))?;
    if status == 404 {
        return Err(format!(
            "{url} answered 404: this endpoint has no tokenizer route, so the only \
             answer is none. llama.cpp's server has one; hosted APIs generally \
             do not."
        ));
    }
    if !(200..300).contains(&status) {
        return Err(format!(
            "{url} answered {status}: {}",
            String::from_utf8_lossy(&bytes).trim()
        ));
    }
    parse(&bytes).ok_or_else(|| {
        format!(
            "{url} answered {status} with something that is not a token list: {}",
            String::from_utf8_lossy(&bytes).chars().take(200).collect::<String>()
        )
    })
}

/// The count out of a `/tokenize` reply.
///
/// llama.cpp returns `{"tokens":[...]}`; a bare array is accepted too, because
/// more than one server speaks this route and they do not all wrap it.
pub fn parse(bytes: &[u8]) -> Option<usize> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if let Some(tokens) = value.get("tokens").and_then(|t| t.as_array()) {
        return Some(tokens.len());
    }
    value.as_array().map(|tokens| tokens.len())
}

fn report(counted: &[Counted], model: &str) {
    let width = counted
        .iter()
        .map(|file| file.label.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    println!("{:<width$}  {:>8}  {:>7}  {:>8}", "file", "tokens", "lines", "bytes");
    for file in counted {
        println!(
            "{:<width$}  {:>8}  {:>7}  {:>8}",
            file.label, file.tokens, file.lines, file.bytes
        );
    }
    if counted.len() > 1 {
        let tokens: usize = counted.iter().map(|f| f.tokens).sum();
        let lines: usize = counted.iter().map(|f| f.lines).sum();
        let bytes: usize = counted.iter().map(|f| f.bytes).sum();
        println!("{:<width$}  {tokens:>8}  {lines:>7}  {bytes:>8}", "total");
    }
    // The tokenizer is the whole point, so say which one answered.
    println!("\nmeasured by {model}");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The route is beside the OpenAI-compatible paths, not under them.
    #[test]
    fn the_tokenize_route_hangs_off_the_server_root() {
        assert_eq!(
            tokenize_url("http://localhost:8080/v1"),
            "http://localhost:8080/tokenize"
        );
        assert_eq!(
            tokenize_url("http://localhost:8080/v1/"),
            "http://localhost:8080/tokenize"
        );
        assert_eq!(
            tokenize_url("http://localhost:8080"),
            "http://localhost:8080/tokenize"
        );
        // A base URL that is not a `/v1` keeps its path, since whatever is
        // serving it decided that shape.
        assert_eq!(
            tokenize_url("http://host/openai/v1"),
            "http://host/openai/tokenize"
        );
    }

    /// More than one server speaks this route and they do not all wrap the
    /// list the same way.
    #[test]
    fn a_token_list_is_counted_in_either_shape() {
        assert_eq!(parse(br#"{"tokens":[1,2,3,4]}"#), Some(4));
        assert_eq!(parse(br#"[5,6,7]"#), Some(3));
        assert_eq!(parse(br#"{"tokens":[]}"#), Some(0));
    }

    /// A reply that is not a token list must not be read as a count. A number
    /// that might be the answer is worse than none, because it gets quoted
    /// later as if it were.
    #[test]
    fn anything_that_is_not_a_token_list_is_refused() {
        assert_eq!(parse(b"not json"), None);
        assert_eq!(parse(br#"{"error":"no such route"}"#), None);
        assert_eq!(parse(br#"{"tokens":42}"#), None);
        assert_eq!(parse(b""), None);
    }
}
