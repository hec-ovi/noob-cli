# config

contractVersion: 1.0.0

## Purpose

Where the agent's non-secret configuration comes from: the config directory,
one lookup rule over process env and the flat `.env`, validated atomic writes,
the sandbox mode, and localhost endpoint autodetect.

## Public surface

```rust
pub fn config_dir() -> PathBuf;
    // NOOB_CONFIG_DIR if set and non-empty, else /config when that directory
    // exists (the container bind mount), else $HOME/.config/noob.
    // Besides .env the directory holds the user's files read by other boxes:
    // AGENTS.md and TOOLS.md (agent box), mcp.json, skills/, sessions/
pub fn setting(config_dir: &Path, key: &str) -> Option<String>;
    // process env wins, then the .env file; empty values count as unset;
    // the file is reparsed on every call, so an edit lands mid-session
pub const EDITABLE: &[(&str, &str)];         // "/config" alias -> env key
pub fn editable_key(name: &str) -> Option<&'static str>;
pub fn write_setting(config_dir: &Path, name: &str, value: Option<&str>)
    -> Result<&'static str, String>;
    // Some(value) sets after validation, None unsets; returns the env key.
    // Comments and unrelated lines survive; an active line is replaced in
    // place, a new key is appended; the rewrite is atomic

pub fn skill_paths(config_dir: &Path, workspace: &Path) -> Vec<PathBuf>;
    // NOOB_SKILL_PATHS, colon split, relative entries against the workspace
pub fn ctx_tokens(config_dir: &Path) -> u64;             // floor 4096, default 131072
pub fn max_rounds(config_dir: &Path, default: u32) -> u32;
    // NOOB_MAX_ROUNDS, the user agent's rounds per input; 0 is unbounded
pub fn task_concurrency(config_dir: &Path, default: usize) -> usize;  // 1..=64
pub fn task_max_turns(config_dir: &Path, default: u32) -> u32;
    // per-child round budget; 0 is unbounded
pub fn task_tools(config_dir: &Path, default: &str) -> String;
    // read-only | web | all: the mode a spawn gets when the model omits it
pub fn task_wall_clock(config_dir: &Path, default_s: u64) -> Duration;
    // 0 disables; the caller passes its shipped default
pub fn read_dedup(config_dir: &Path) -> bool;            // on unless 0/off/false/no
pub fn tool_caps_lifted(config_dir: &Path) -> bool;      // true when 0/off/false/no

pub enum SandboxMode { Container, Workspace }
pub fn detect_sandbox(config_dir: &Path, yolo: bool) -> (SandboxMode, String);
    // yolo -> Container "off (--yolo)"; NOOB_SANDBOX=container -> Container;
    // any other set value -> Workspace; unset -> /.dockerenv decides.
    // The label is what doctor and the prompt head print

pub fn autodetect_base_url(config_dir: &Path) -> Option<String>;
    // localhost candidates 8080, 8090, 11434, 1234, 8000 (each /v1),
    // first whose /models answers within 500 ms; None when NOOB_AUTODETECT
    // is 0/off/false/no
pub fn first_responding(candidates: &[&str]) -> Option<String>;
```

## Errors

Only `write_setting` fails, with a closed set of strings: unknown setting
(lists the aliases), value contains a newline, empty value (points at unset),
per-name validation (`api-style must be chat or responses`, `ctx must be an
integer of at least 4096`, integer-range messages naming min and max, the
on/off switch wording), unsetting an absent key (`<name> is not set; nothing
to unset`), and `cannot create/read/replace ...` on io failures. Every getter
totals: invalid or missing input falls back to the default, never an error.

## Invariants

1. Secrets never cross this surface. `NOOB_API_KEY` is not an editable alias
   and is not read here; keys stay lazy inside noob-provider.
2. The `.env` rewrite is atomic: a private temp in the config directory, then
   rename, so a concurrent reader sees the old file or the new one, never a
   mix. A fresh file is created 0o600; an existing file keeps its mode; a
   symlink squatting on the temp path is refused, not followed.
3. Ceilings hold regardless of the caller's default: concurrency 16, turns
   50, wall clock 3600 s. Defaults come from the caller; this box owns only
   the parse, floors, and ceilings.
4. Autodetect never leaves loopback, and never runs when a base URL is
   configured or the switch is off.

## Dependencies

Contracts: [`crates/noob-provider/CONTRACT.md`](../../../noob-provider/CONTRACT.md)
for the `.env` parser (`envfile::load`) and the HTTP probe. The tools box maps
`SandboxMode` onto its write policy; callers pass their own shipped defaults
into the task getters.

## Tests

The module's inline tests cover each getter, the write/unset/validation
paths, atomicity, and permissions. `/config` end to end lives in
`crates/noob/tests/ui_commands.rs`.
