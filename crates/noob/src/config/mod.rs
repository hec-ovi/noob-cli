//! Config-dir resolution. Keys themselves are read lazily per request inside
//! noob-provider; this module only decides which directory to read from.

use std::path::PathBuf;

/// Resolution order: NOOB_CONFIG_DIR > /config (the container bind mount) >
/// ~/.config/noob outside Docker. The directory does not have to exist yet;
/// `noob doctor` (P7) reports on it.
pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NOOB_CONFIG_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let container_default = PathBuf::from("/config");
    if container_default.is_dir() {
        return container_default;
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config/noob")
}
