//! Config-dir resolution, non-secret settings lookup, sandbox detection,
//! and localhost endpoint autodetect. API keys are NOT read here: they stay
//! lazy inside noob-provider so they never enter the process environment.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use std::io::Write;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::tools::guard::Sandbox;

/// User-facing `/config` names. Secrets are deliberately absent: putting an
/// API key in terminal history is not an acceptable configuration flow.
pub const EDITABLE: &[(&str, &str)] = &[
    ("base-url", "NOOB_BASE_URL"),
    ("model", "NOOB_MODEL"),
    ("api-style", "NOOB_API_STYLE"),
    ("ctx", "NOOB_CTX"),
    ("autodetect", "NOOB_AUTODETECT"),
    ("task-concurrency", "NOOB_TASK_CONCURRENCY"),
    ("task-max-turns", "NOOB_TASK_MAX_TURNS"),
    ("task-wall-clock", "NOOB_TASK_WALL_CLOCK_S"),
    ("tool-caps", "NOOB_TOOL_CAPS"),
    ("read-dedup", "NOOB_READ_DEDUP"),
];

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

pub fn editable_key(name: &str) -> Option<&'static str> {
    EDITABLE
        .iter()
        .find(|(alias, _)| *alias == name)
        .map(|(_, key)| *key)
}

/// Validate and atomically update one non-secret `.env` setting. Existing
/// comments and unrelated settings stay in place; an active value is replaced
/// in place, while a new value is appended. Rewrites normalize line endings.
pub fn write_setting(
    config_dir: &Path,
    name: &str,
    value: Option<&str>,
) -> Result<&'static str, String> {
    let key = editable_key(name).ok_or_else(|| {
        format!(
            "unknown setting {name:?}; available: {}",
            EDITABLE
                .iter()
                .map(|(alias, _)| *alias)
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    if let Some(value) = value {
        if value.contains('\n') || value.contains('\r') {
            return Err("the value cannot contain a newline".to_string());
        }
        validate_setting(name, value)?;
    }

    std::fs::create_dir_all(config_dir).map_err(|e| {
        format!(
            "cannot create config directory {}: {e}",
            config_dir.display()
        )
    })?;
    let path = config_dir.join(".env");
    let old = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let mut found = false;
    let mut lines = Vec::new();
    for line in old.lines() {
        let active = line
            .trim_start()
            .strip_prefix("export ")
            .unwrap_or_else(|| line.trim_start());
        if active
            .strip_prefix(key)
            .is_some_and(|tail| tail.starts_with('='))
        {
            if !found {
                if let Some(value) = value {
                    lines.push(format!("{key}={value}"));
                }
                found = true;
            }
        } else {
            lines.push(line.to_string());
        }
    }
    if !found {
        let Some(value) = value else {
            // Unsetting an absent key: say so instead of rewriting the file
            // and promising a restart that changes nothing.
            return Err(format!("{name} is not set; nothing to unset"));
        };
        lines.push(format!("{key}={value}"));
    }
    let mut next = lines.join("\n");
    if !next.is_empty() {
        next.push('\n');
    }
    let existing_permissions = std::fs::symlink_metadata(&path)
        .ok()
        .filter(|metadata| metadata.file_type().is_file())
        .map(|metadata| metadata.permissions());
    let (tmp, mut file) = open_config_temp(config_dir)
        .map_err(|e| format!("cannot create a temporary config file: {e}"))?;
    let replace = (|| -> std::io::Result<()> {
        file.write_all(next.as_bytes())?;
        if let Some(permissions) = existing_permissions {
            file.set_permissions(permissions)?;
        }
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &path)
    })();
    if let Err(error) = replace {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("cannot replace {}: {error}", path.display()));
    }
    Ok(key)
}

fn create_private_temp(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)
}

fn open_config_temp(config_dir: &Path) -> std::io::Result<(PathBuf, std::fs::File)> {
    static TMP_SERIAL: AtomicU64 = AtomicU64::new(1);
    for _ in 0..32 {
        let serial = TMP_SERIAL.fetch_add(1, Ordering::Relaxed);
        let path = config_dir.join(format!(".env.tmp-{}-{serial}", std::process::id()));
        match create_private_temp(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "too many stale .env temporary files",
    ))
}

fn validate_setting(name: &str, value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("the value is empty; use /config unset <name> instead".to_string());
    }
    let range = |min: u64, max: u64| {
        value
            .parse::<u64>()
            .ok()
            .filter(|n| (min..=max).contains(n))
            .map(|_| ())
            .ok_or_else(|| format!("{name} must be an integer from {min} to {max}"))
    };
    match name {
        "api-style" if !matches!(value, "chat" | "responses") => {
            Err("api-style must be chat or responses".to_string())
        }
        // No meaningful upper bound; naming u64::MAX in the error only
        // confuses, so the message states the floor.
        "ctx" => value
            .parse::<u64>()
            .ok()
            .filter(|&n| n >= 4_096)
            .map(|_| ())
            .ok_or_else(|| "ctx must be an integer of at least 4096".to_string()),
        "task-concurrency" => range(1, 16),
        "task-max-turns" => range(1, 50),
        "task-wall-clock" => range(1, 3_600),
        "autodetect" | "tool-caps" | "read-dedup"
            if !matches!(
                value.to_ascii_lowercase().as_str(),
                "0" | "1" | "true" | "false" | "on" | "off" | "yes" | "no"
            ) =>
        {
            Err(format!("{name} must be on/off, true/false, yes/no, or 1/0"))
        }
        _ => Ok(()),
    }
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

/// NOOB_READ_DEDUP: whether a `read` of content the model already holds in
/// full may answer with a short note instead of the body. 0/off/false/no
/// prints every read in full; anything else, or unset, keeps the short note.
///
/// This exists because the mechanism has a known failure mode in the field.
/// Claude Code ships the same short-circuit behind a remote killswitch, and a
/// released version of it was reported to send the model into a probe loop:
/// it read the notice as a broken tool result and compensated by re-reading
/// harder. noob keys on a content hash rather than mtime, which removes the
/// "the note lies" half of that, but not the "the model distrusts the note"
/// half. A local switch means a bad interaction with some model is a config
/// change, not a downgrade.
pub fn read_dedup(config_dir: &Path) -> bool {
    !setting(config_dir, "NOOB_READ_DEDUP").is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        )
    })
}

/// NOOB_TOOL_CAPS: the tool-result truncation policy. 0/off/false/no lifts
/// every cap (read, bash, grep, glob/ls, skill, and MCP results flow through
/// whole, so no truncation marker ever renders); anything else, or unset,
/// keeps the shipped defaults. Resolved once at bootstrap.
pub fn tool_caps(config_dir: &Path) -> crate::tools::truncate::Caps {
    let off = setting(config_dir, "NOOB_TOOL_CAPS").is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        )
    });
    if off {
        crate::tools::truncate::Caps::uncapped()
    } else {
        crate::tools::truncate::Caps::default()
    }
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
pub fn autodetect_base_url(config_dir: &Path) -> Option<String> {
    // An explicit off switch is useful for deterministic automation and for
    // hosts where another user's local model must not be selected. Normal
    // interactive behavior remains zero-configuration.
    if setting(config_dir, "NOOB_AUTODETECT").is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        )
    }) {
        return None;
    }
    let candidates = [
        "http://localhost:8090/v1",  // llama.cpp (this project's default)
        "http://localhost:8080/v1",  // llama.cpp default port
        "http://localhost:11434/v1", // Ollama
        "http://localhost:1234/v1",  // LM Studio
        "http://localhost:8000/v1",  // vLLM
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
    fn editable_settings_preserve_comments_replace_and_unset() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".env"),
            "# keep this\nNOOB_MODEL=old\nNOOB_CTX=8192\n",
        )
        .unwrap();
        assert_eq!(
            write_setting(tmp.path(), "model", Some("new-model")).unwrap(),
            "NOOB_MODEL"
        );
        write_setting(tmp.path(), "ctx", None).unwrap();
        write_setting(tmp.path(), "task-concurrency", Some("8")).unwrap();
        let got = std::fs::read_to_string(tmp.path().join(".env")).unwrap();
        assert_eq!(
            got,
            "# keep this\nNOOB_MODEL=new-model\nNOOB_TASK_CONCURRENCY=8\n"
        );
    }

    #[test]
    fn editable_settings_reject_secrets_unknown_names_and_invalid_ranges() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            write_setting(tmp.path(), "api-key", Some("secret"))
                .unwrap_err()
                .contains("unknown")
        );
        assert!(
            write_setting(tmp.path(), "ctx", Some("100"))
                .unwrap_err()
                .contains("4096")
        );
        assert!(
            write_setting(tmp.path(), "task-concurrency", Some("17"))
                .unwrap_err()
                .contains("16")
        );
        assert!(
            write_setting(tmp.path(), "api-style", Some("magic"))
                .unwrap_err()
                .contains("chat")
        );
        assert!(!tmp.path().join(".env").exists());
    }

    #[test]
    fn ctx_validation_names_the_floor_not_the_u64_ceiling() {
        let tmp = tempfile::tempdir().unwrap();
        for bad in ["potato", "100", "-1", "4095"] {
            let error = write_setting(tmp.path(), "ctx", Some(bad)).unwrap_err();
            assert_eq!(error, "ctx must be an integer of at least 4096", "{bad}");
        }
        // No upper bound: any integer at or above the floor is accepted.
        assert!(write_setting(tmp.path(), "ctx", Some("4096")).is_ok());
        assert!(write_setting(tmp.path(), "ctx", Some(&u64::MAX.to_string())).is_ok());
    }

    #[test]
    fn unset_of_an_absent_key_says_so_and_rewrites_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        // No .env at all: no file appears.
        let error = write_setting(tmp.path(), "model", None).unwrap_err();
        assert_eq!(error, "model is not set; nothing to unset");
        assert!(!tmp.path().join(".env").exists());
        // A file without the key stays byte-identical.
        std::fs::write(tmp.path().join(".env"), "# note\nNOOB_CTX=8192\n").unwrap();
        let error = write_setting(tmp.path(), "model", None).unwrap_err();
        assert_eq!(error, "model is not set; nothing to unset");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join(".env")).unwrap(),
            "# note\nNOOB_CTX=8192\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn editable_settings_preserve_mode_and_create_private_files() {
        use std::os::unix::fs::PermissionsExt;

        let existing = tempfile::tempdir().unwrap();
        let existing_path = existing.path().join(".env");
        std::fs::write(&existing_path, "NOOB_MODEL=old\n").unwrap();
        std::fs::set_permissions(&existing_path, std::fs::Permissions::from_mode(0o640)).unwrap();
        write_setting(existing.path(), "model", Some("new")).unwrap();
        assert_eq!(
            std::fs::metadata(&existing_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );

        let fresh = tempfile::tempdir().unwrap();
        write_setting(fresh.path(), "model", Some("new")).unwrap();
        assert_eq!(
            std::fs::metadata(fresh.path().join(".env"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600,
        );
    }

    #[cfg(unix)]
    #[test]
    fn config_temp_creation_never_follows_a_precreated_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        let link = tmp.path().join(".env.tmp-hostile");
        std::fs::write(&target, "secret stays intact").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let error = create_private_temp(&link).expect_err("create_new must reject the symlink");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(target).unwrap(),
            "secret stays intact"
        );
    }

    #[test]
    fn editable_settings_reject_newline_injection() {
        let tmp = tempfile::tempdir().unwrap();
        let error = write_setting(tmp.path(), "model", Some("safe\nNOOB_CTX=4096"))
            .expect_err("newlines must not create a second setting");
        assert!(error.contains("newline"));
        assert!(!tmp.path().join(".env").exists());
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
        assert_eq!(
            skill_paths(cfg.path(), ws.path()),
            vec![ws.path().join("cli")]
        );
    }

    #[test]
    fn tool_caps_zero_or_off_lifts_every_cap() {
        use crate::tools::truncate::Caps;

        let tmp = tempfile::tempdir().unwrap();
        // Unset: the shipped defaults.
        assert_eq!(tool_caps(tmp.path()), Caps::default());
        for off in ["0", "off", "false", "no", " OFF "] {
            std::fs::write(tmp.path().join(".env"), format!("NOOB_TOOL_CAPS={off}\n")).unwrap();
            assert_eq!(tool_caps(tmp.path()), Caps::uncapped(), "{off}");
        }
        // Anything else (on, 1, junk) keeps the defaults.
        for on in ["1", "on", "true", "yes", "potato"] {
            std::fs::write(tmp.path().join(".env"), format!("NOOB_TOOL_CAPS={on}\n")).unwrap();
            assert_eq!(tool_caps(tmp.path()), Caps::default(), "{on}");
        }
        // The /config alias validates like the other switches.
        assert!(write_setting(tmp.path(), "tool-caps", Some("off")).is_ok());
        assert!(
            write_setting(tmp.path(), "tool-caps", Some("potato"))
                .unwrap_err()
                .contains("tool-caps")
        );
    }

    #[test]
    fn read_dedup_defaults_on_and_switches_off() {
        let tmp = tempfile::tempdir().unwrap();
        // Unset: the short-circuit is active.
        assert!(read_dedup(tmp.path()));
        for off in ["0", "off", "false", "no", " OFF "] {
            std::fs::write(tmp.path().join(".env"), format!("NOOB_READ_DEDUP={off}\n")).unwrap();
            assert!(!read_dedup(tmp.path()), "{off}");
        }
        for on in ["1", "on", "true", "yes", "potato"] {
            std::fs::write(tmp.path().join(".env"), format!("NOOB_READ_DEDUP={on}\n")).unwrap();
            assert!(read_dedup(tmp.path()), "{on}");
        }
        assert!(write_setting(tmp.path(), "read-dedup", Some("off")).is_ok());
        assert!(
            write_setting(tmp.path(), "read-dedup", Some("potato"))
                .unwrap_err()
                .contains("read-dedup")
        );
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
