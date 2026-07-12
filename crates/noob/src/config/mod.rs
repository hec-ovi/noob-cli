//! Config-dir resolution, non-secret settings lookup, sandbox detection,
//! and localhost endpoint autodetect. API keys are NOT read here: they stay
//! lazy inside noob-provider so they never enter the process environment.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::tools::guard::Sandbox;

/// Resolution order: NOOB_CONFIG_DIR > /config (the container bind mount) >
/// ~/.config/noob outside Docker. The directory does not have to exist yet;
/// `noob doctor` (P7) reports on it.
pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NOOB_CONFIG_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    let container_default = PathBuf::from("/config");
    if container_default.is_dir() {
        return container_default;
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config/noob")
}

/// One non-secret setting: process env wins, then the config `.env`.
/// Read fresh on every call (the .env parse is cheap and hot reload is the
/// whole point of the flat file).
pub fn setting(config_dir: &Path, key: &str) -> Option<String> {
    if let Ok(v) = std::env::var(key)
        && !v.is_empty()
    {
        return Some(v);
    }
    let env_path = config_dir.join(".env");
    if !env_path.is_file() {
        return None;
    }
    noob_provider::envfile::load(&env_path)
        .ok()?
        .get(key)
        .cloned()
        .filter(|v| !v.is_empty())
}

/// NOOB_SKILL_PATHS: extra resolver/dispatcher skill directories to index, on
/// top of the four default roots. Colon-separated (PATH style). Each entry is
/// resolved against the workspace, so `cli` means `<workspace>/cli`; an
/// absolute entry is used as-is. Every entry points at ONE skill directory
/// (the dir must contain a `SKILL.md`); discovery does not scan it as a root.
/// Empty/whitespace entries are ignored. Order is preserved. This lets the
/// agent pick up a workflow skill that lives at a non-root path inside the
/// mounted workspace (e.g. a `cli/SKILL.md` dispatcher) without copying it
/// into a discovery root.
pub fn skill_paths(config_dir: &Path, workspace: &Path) -> Vec<PathBuf> {
    let Some(raw) = setting(config_dir, "NOOB_SKILL_PATHS") else {
        return Vec::new();
    };
    raw.split(':')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let p = Path::new(entry);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                workspace.join(p)
            }
        })
        .collect()
}

/// NOOB_CTX: the context window compaction budgets against.
pub fn ctx_tokens(config_dir: &Path) -> u64 {
    setting(config_dir, "NOOB_CTX")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&n| n >= 4096)
        .unwrap_or(131_072)
}

/// NOOB_TASK_CONCURRENCY: concurrent sub-agent cap (P6). Bounded: every
/// child is a full agent hitting the same endpoint.
pub fn task_concurrency(config_dir: &Path) -> usize {
    setting(config_dir, "NOOB_TASK_CONCURRENCY")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(crate::subagent::DEFAULT_CONCURRENCY)
        .min(16)
}

/// NOOB_TASK_MAX_TURNS: per-child inference-round cap (P6). A loop budget,
/// never an output-token cap.
pub fn task_max_turns(config_dir: &Path) -> u32 {
    setting(config_dir, "NOOB_TASK_MAX_TURNS")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(crate::subagent::DEFAULT_MAX_TURNS)
        .min(50)
}

/// NOOB_TASK_WALL_CLOCK_S: per-child wall clock before the parent kills the
/// process group. Settable mostly so tests do not wait five minutes.
pub fn task_wall_clock(config_dir: &Path) -> std::time::Duration {
    let secs = setting(config_dir, "NOOB_TASK_WALL_CLOCK_S")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(crate::subagent::DEFAULT_WALL_CLOCK_S)
        .min(3_600);
    std::time::Duration::from_secs(secs)
}

/// Two states, no permission DSL: the container is the wall. An explicit
/// NOOB_SANDBOX setting wins; otherwise /.dockerenv decides. `--yolo` lifts
/// the workspace restriction entirely.
pub fn detect_sandbox(config_dir: &Path, yolo: bool) -> (Sandbox, String) {
    if yolo {
        return (Sandbox::Container, "off (--yolo)".to_string());
    }
    match setting(config_dir, "NOOB_SANDBOX").as_deref() {
        Some("container") => (Sandbox::Container, "container".to_string()),
        Some(_) => (Sandbox::Workspace, "workspace".to_string()),
        None => {
            if Path::new("/.dockerenv").exists() {
                (Sandbox::Container, "container".to_string())
            } else {
                (Sandbox::Workspace, "workspace".to_string())
            }
        }
    }
}

/// The zero-friction path: when no base URL is configured, probe the usual
/// localhost ports with a short timeout. Loopback only, only when
/// unconfigured, never a remote call.
pub fn autodetect_base_url() -> Option<String> {
    // An explicit off switch is useful for deterministic automation and for
    // hosts where another user's local model must not be selected. Normal
    // interactive behavior remains zero-configuration.
    if std::env::var("NOOB_AUTODETECT")
        .ok()
        .is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no"))
    {
        return None;
    }
    let candidates = [
        "http://localhost:8090/v1", // llama.cpp (this project's default)
        "http://localhost:8080/v1", // llama.cpp default port
        "http://localhost:11434/v1", // Ollama
        "http://localhost:1234/v1", // LM Studio
        "http://localhost:8000/v1", // vLLM
    ];
    first_responding(&candidates)
}

/// Testable core: the first candidate whose /models answers HTTP.
pub fn first_responding(candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .find(|base| {
            noob_provider::http::probe(&format!("{base}/models"), Duration::from_millis(500))
        })
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctx_tokens_default_parse_and_floor() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(ctx_tokens(tmp.path()), 131_072);
        std::fs::write(tmp.path().join(".env"), "NOOB_CTX=32768\n").unwrap();
        assert_eq!(ctx_tokens(tmp.path()), 32_768);
        // Nonsense and sub-floor values fall back to the default.
        std::fs::write(tmp.path().join(".env"), "NOOB_CTX=potato\n").unwrap();
        assert_eq!(ctx_tokens(tmp.path()), 131_072);
        std::fs::write(tmp.path().join(".env"), "NOOB_CTX=100\n").unwrap();
        assert_eq!(ctx_tokens(tmp.path()), 131_072);
    }

    #[test]
    fn skill_paths_split_on_colon_and_resolve_against_workspace() {
        let cfg = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        // Unset: no extra paths.
        assert!(skill_paths(cfg.path(), ws.path()).is_empty());
        // Relative entries resolve against the workspace; an absolute entry is
        // kept as-is; order is preserved.
        std::fs::write(
            cfg.path().join(".env"),
            "NOOB_SKILL_PATHS=cli:tools/agent:/opt/shared/skill\n",
        )
        .unwrap();
        assert_eq!(
            skill_paths(cfg.path(), ws.path()),
            vec![
                ws.path().join("cli"),
                ws.path().join("tools/agent"),
                PathBuf::from("/opt/shared/skill"),
            ]
        );
        // Empty and whitespace-only entries are ignored.
        std::fs::write(cfg.path().join(".env"), "NOOB_SKILL_PATHS=: cli : :\n").unwrap();
        assert_eq!(skill_paths(cfg.path(), ws.path()), vec![ws.path().join("cli")]);
    }

    #[test]
    fn sandbox_explicit_setting_beats_dockerenv() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".env"), "NOOB_SANDBOX=workspace\n").unwrap();
        let (mode, label) = detect_sandbox(tmp.path(), false);
        assert_eq!(mode, Sandbox::Workspace);
        assert_eq!(label, "workspace");
        let (mode, label) = detect_sandbox(tmp.path(), true);
        assert_eq!(mode, Sandbox::Container);
        assert_eq!(label, "off (--yolo)");
    }

    #[test]
    fn first_responding_finds_a_live_listener_and_skips_dead_ports() {
        // A real listener on an ephemeral port, speaking minimal HTTP.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
                );
            }
        });
        let live = format!("http://{addr}");
        let dead = "http://127.0.0.1:9".to_string(); // discard port; nothing listens
        let got = first_responding(&[dead.as_str(), live.as_str()]);
        assert_eq!(got, Some(live));
        assert_eq!(first_responding(&[dead.as_str()]), None);
    }
}
