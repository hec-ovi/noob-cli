//! mcp.json loading and merge. Two files: `<config>/mcp.json` (global) and
//! `<workspace>/.noob/mcp.json` (project); the project entry wins per server
//! name. A malformed file or entry is a warning and a skip, never a crash:
//! a broken mcp.json must not take the whole session down.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

/// Per-call timeout when the entry does not set `timeout_s`.
pub const DEFAULT_TIMEOUT_S: u64 = 30;
/// Ceiling on `timeout_s`: a wedged server must never block the loop for
/// longer than the bash tool would be allowed to run.
pub const MAX_TIMEOUT_S: u64 = 600;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportConfig {
    /// `url` entry: MCP Streamable HTTP.
    Http { url: String },
    /// `command` entry: a stdio child process.
    Stdio { command: String, args: Vec<String> },
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub name: String,
    pub transport: TransportConfig,
    pub timeout: Duration,
}

/// Load and merge both files. Returns the configured servers (sorted by
/// name, deterministic) and human-readable warnings for everything skipped.
pub fn load(workspace: &Path, config_dir: &Path) -> (Vec<ServerConfig>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut servers: Vec<ServerConfig> = Vec::new();
    // Global first, project second: a later push with the same name replaces
    // the earlier one (project wins).
    for path in [
        config_dir.join("mcp.json"),
        workspace.join(".noob/mcp.json"),
    ] {
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                warnings.push(format!("cannot read {}: {e}", path.display()));
                continue;
            }
        };
        let parsed: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                warnings.push(format!(
                    "{} is not valid JSON ({e}); fix it or remove it",
                    path.display()
                ));
                continue;
            }
        };
        let Some(map) = parsed.get("servers").and_then(Value::as_object) else {
            warnings.push(format!(
                "{} has no \"servers\" object; expected {{\"servers\": {{\"name\": {{...}}}}}}",
                path.display()
            ));
            continue;
        };
        // serde_json's map is sorted, so iteration order is deterministic.
        for (name, entry) in map {
            match parse_entry(name, entry) {
                Ok(cfg) => {
                    servers.retain(|s| s.name != cfg.name);
                    servers.push(cfg);
                }
                Err(reason) => {
                    warnings.push(format!(
                        "skipping MCP server {name:?} in {}: {reason}",
                        path.display()
                    ));
                }
            }
        }
    }
    servers.sort_by(|a, b| a.name.cmp(&b.name));
    (servers, warnings)
}

fn parse_entry(name: &str, entry: &Value) -> Result<ServerConfig, String> {
    if name.is_empty() || name.len() > 64 {
        return Err("server names must be 1-64 characters".to_string());
    }
    let url = entry.get("url").and_then(Value::as_str);
    let command = entry.get("command").and_then(Value::as_str);
    let transport = match (url, command) {
        (Some(_), Some(_)) => {
            return Err("has both \"url\" and \"command\"; pick one transport".to_string());
        }
        (None, None) => {
            return Err("needs \"url\" (HTTP) or \"command\" (stdio)".to_string());
        }
        (Some(url), None) => {
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(format!("url {url:?} must start with http:// or https://"));
            }
            TransportConfig::Http {
                url: url.trim_end_matches('/').to_string(),
            }
        }
        (None, Some(command)) => {
            if command.trim().is_empty() {
                return Err("\"command\" is empty".to_string());
            }
            let args = match entry.get("args") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(items)) => {
                    let mut args = Vec::with_capacity(items.len());
                    for item in items {
                        match item.as_str() {
                            Some(s) => args.push(s.to_string()),
                            None => {
                                return Err(format!(
                                    "\"args\" must be an array of strings, got {item}"
                                ));
                            }
                        }
                    }
                    args
                }
                Some(other) => {
                    return Err(format!("\"args\" must be an array of strings, got {other}"));
                }
            };
            TransportConfig::Stdio {
                command: command.to_string(),
                args,
            }
        }
    };
    let timeout_s = match entry.get("timeout_s") {
        None | Some(Value::Null) => DEFAULT_TIMEOUT_S,
        Some(v) => v
            .as_u64()
            .filter(|&n| n >= 1)
            .ok_or_else(|| format!("\"timeout_s\" must be a positive integer, got {v}"))?
            .min(MAX_TIMEOUT_S),
    };
    Ok(ServerConfig {
        name: name.to_string(),
        transport,
        timeout: Duration::from_secs(timeout_s),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn loads_both_transports_with_defaults_and_overrides() {
        let cfg = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        write(
            cfg.path(),
            "mcp.json",
            r#"{"servers": {
                "websearch": {"url": "http://localhost:8000/"},
                "fs": {"command": "fs-mcp", "args": ["--root", "/data"], "timeout_s": 5}
            }}"#,
        );
        let (servers, warnings) = load(ws.path(), cfg.path());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(servers.len(), 2);
        // Sorted by name: fs, websearch.
        assert_eq!(servers[0].name, "fs");
        assert_eq!(
            servers[0].transport,
            TransportConfig::Stdio {
                command: "fs-mcp".into(),
                args: vec!["--root".into(), "/data".into()]
            }
        );
        assert_eq!(servers[0].timeout, Duration::from_secs(5));
        assert_eq!(servers[1].name, "websearch");
        // Trailing slash trimmed so joined RPC URLs stay clean.
        assert_eq!(
            servers[1].transport,
            TransportConfig::Http {
                url: "http://localhost:8000".into()
            }
        );
        assert_eq!(servers[1].timeout, Duration::from_secs(DEFAULT_TIMEOUT_S));
    }

    #[test]
    fn project_file_wins_per_name_and_adds_new_entries() {
        let cfg = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        write(
            cfg.path(),
            "mcp.json",
            r#"{"servers": {"shared": {"url": "http://global:1"}, "only-global": {"url": "http://g:2"}}}"#,
        );
        write(
            ws.path(),
            ".noob/mcp.json",
            r#"{"servers": {"shared": {"url": "http://project:1"}, "only-project": {"url": "http://p:2"}}}"#,
        );
        let (servers, warnings) = load(ws.path(), cfg.path());
        assert!(warnings.is_empty(), "{warnings:?}");
        let by_name: Vec<(&str, &TransportConfig)> = servers
            .iter()
            .map(|s| (s.name.as_str(), &s.transport))
            .collect();
        assert_eq!(servers.len(), 3);
        assert!(by_name.iter().any(|(n, t)| *n == "shared"
            && **t
                == TransportConfig::Http {
                    url: "http://project:1".into()
                }));
        assert!(by_name.iter().any(|(n, _)| *n == "only-global"));
        assert!(by_name.iter().any(|(n, _)| *n == "only-project"));
    }

    #[test]
    fn bad_entries_warn_and_skip_good_ones_survive() {
        let cfg = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        write(
            cfg.path(),
            "mcp.json",
            r#"{"servers": {
                "both": {"url": "http://x:1", "command": "y"},
                "neither": {},
                "badargs": {"command": "c", "args": [1]},
                "badurl": {"url": "ftp://x"},
                "badtimeout": {"url": "http://x:1", "timeout_s": 0},
                "good": {"url": "http://localhost:9"}
            }}"#,
        );
        let (servers, warnings) = load(ws.path(), cfg.path());
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "good");
        assert_eq!(warnings.len(), 5, "{warnings:?}");
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("both") && w.contains("pick one"))
        );
        assert!(warnings.iter().any(|w| w.contains("neither")));
    }

    #[test]
    fn malformed_file_warns_and_is_ignored() {
        let cfg = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        write(cfg.path(), "mcp.json", "{ not json");
        write(
            ws.path(),
            ".noob/mcp.json",
            r#"{"servers": {"ok": {"url": "http://x:1"}}}"#,
        );
        let (servers, warnings) = load(ws.path(), cfg.path());
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "ok");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("not valid JSON"));
    }

    #[test]
    fn missing_files_are_silent() {
        let cfg = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let (servers, warnings) = load(ws.path(), cfg.path());
        assert!(servers.is_empty());
        assert!(warnings.is_empty());
    }

    #[test]
    fn timeout_is_capped_at_the_ceiling() {
        let cfg = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        write(
            cfg.path(),
            "mcp.json",
            r#"{"servers": {"slow": {"url": "http://x:1", "timeout_s": 99999}}}"#,
        );
        let (servers, _) = load(ws.path(), cfg.path());
        assert_eq!(servers[0].timeout, Duration::from_secs(MAX_TIMEOUT_S));
    }
}
