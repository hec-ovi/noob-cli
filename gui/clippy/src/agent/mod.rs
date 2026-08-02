//! The agent's own files, read so the window can say what it is a front end
//! for: the endpoint it talks to, the skills installed beside it and the MCP
//! servers it would connect to.
//!
//! None of this belongs to the window. `noob` keeps its settings in a flat
//! `.env` under its config directory, its skills in `skills/` and its MCP
//! servers in `mcp.json`, and every rule here is read off the CLI rather than
//! invented: [`config_dir`] is the CLI's own resolution order, [`write_env`] is
//! a port of the CLI's `.env` writer, and [`read_mcp`] merges the two files the
//! way `crates/noob/src/mcp/config.rs` does.
//!
//! **Nothing here writes a secret and nothing here shows one.** The CLI keeps
//! API keys out of settable config on purpose, so a key in the file is reported
//! as set and never as its value, and [`write_env`] refuses to write one at all.
//!
//! Nothing here is fatal either. A missing file is a machine where that thing is
//! not configured, which is worth saying plainly; a malformed one is a line of
//! trouble on the panel rather than a window that will not open.

use std::path::{Path, PathBuf};

/// How much of a `SKILL.md` is read to find its front matter. The front matter
/// is the first few lines of the file and the rest is a document; reading a
/// whole skill to print its one line description is work nobody asked for.
const SKILL_HEAD_BYTES: u64 = 16 * 1024;

/// How much of a `SKILL.md` is read for the column that shows the skill itself.
/// The whole document this time, up to a size no skill anybody wrote is: a
/// pane cannot show a megabyte of prose and reading one to draw forty lines is
/// work nobody asked for either.
const SKILL_BODY_BYTES: u64 = 256 * 1024;

/// The agent's own instructions file, the one it puts at the top of every
/// prompt. One name, in the config directory and in the workspace, and it is
/// hardcoded in the CLI: no setting anywhere names another file.
pub const AGENTS_MD: &str = "AGENTS.md";

/// The tools text, appended after the instructions: the second user-owned
/// layer of the prompt, in the config directory beside [`AGENTS_MD`].
pub const TOOLS_MD: &str = "TOOLS.md";

/// How much of each prompt file the CLI actually reads.
///
/// `crates/noob/src/agent/prompt.rs` caps `AGENTS.md` and `TOOLS.md` at 16 KiB
/// and appends a truncation notice, so a window showing more than this would be
/// showing text the model never sees.
pub const AGENTS_CAP: u64 = 16 * 1024;

/// The texts the CLI ships for the two prompt files, used whenever a file is
/// absent. Included from the CLI's own sources, so what the panel shows as the
/// built-in text and what the agent runs with cannot drift.
pub const AGENTS_DEFAULT: &str = include_str!("../../../../crates/noob/prompts/agents-default.md");
pub const TOOLS_DEFAULT: &str = include_str!("../../../../crates/noob/prompts/tools-default.md");

/// What is on the end of the directory a turned-off skill is moved to.
///
/// The agent has no idea of a skill being off. It discovers `<ws>/.noob/skills`,
/// `<ws>/.claude/skills`, `<ws>/.agents/skills` and `<config>/skills`, plus each
/// entry of `NOOB_SKILL_PATHS` as one directory, and a skill is on when its
/// directory is in one of those and off when it is not. So off is a move: the
/// directory goes to a sibling of the skills directory with this on the end of
/// its name, which is not any of those roots, and comes back the same way.
/// Reversible, visible in a file manager, and nothing in the agent had to learn
/// a new concept for it.
pub const OFF: &str = ".off";

/// Where the file keeps the servers that are configured but turned off.
///
/// A sibling of `servers` in the same `mcp.json`. The CLI's loader reads
/// `parsed.get("servers")` and nothing else off the top level, so an entry
/// moved in here is one the next session does not connect to, and its writer
/// (`add_server`/`remove_server`) parses the whole file into a value and writes
/// it back, so this key survives anything the CLI does to that file.
pub const DISABLED: &str = "disabled";

/// How much of an `mcp.json` is read. These files are a handful of entries; a
/// gigabyte of JSON where one was expected is a mistake, not a configuration.
const MCP_BYTES: u64 = 1024 * 1024;

/// The key the panel lets anybody edit: where the model lives.
pub const ENDPOINT: &str = "NOOB_BASE_URL";

/// The credential the endpoint is called with. Never drawn: the panel says
/// whether it is set ([`is_secret`] covers it by name as well).
pub const API_KEY: &str = "NOOB_API_KEY";

/// Which model the endpoint is asked for, by the name that endpoint knows it
/// by.
pub const MODEL: &str = "NOOB_MODEL";

/// Which of the two request shapes the endpoint speaks, `chat` or `responses`.
/// With nothing set the provider picks by the address.
pub const API_STYLE: &str = "NOOB_API_STYLE";

/// Whether the thinking switch is sent with a request, `on` or `off`. With
/// nothing set the provider leaves it to the server's own flags.
pub const REASONING: &str = "NOOB_REASONING";

/// The context window the CLI budgets against before it compacts.
pub const CTX: &str = "NOOB_CTX";

/// How many sub-agent tasks the CLI runs at once.
pub const TASK_CONCURRENCY: &str = "NOOB_TASK_CONCURRENCY";

/// The user agent's inference rounds per input; 0 is unbounded.
pub const MAX_ROUNDS: &str = "NOOB_MAX_ROUNDS";

/// Each sub-agent's inference-round budget; 0 is unbounded.
pub const TASK_MAX_TURNS: &str = "NOOB_TASK_MAX_TURNS";

/// The tools a sub-agent gets when the model does not choose.
pub const TASK_TOOLS: &str = "NOOB_TASK_TOOLS";

/// Seconds a sub-agent may run before the CLI kills it; 0 is no limit.
pub const TASK_WALL_CLOCK: &str = "NOOB_TASK_WALL_CLOCK_S";

/// The settings in the agent's file the panel owns: the ones it draws as
/// controls rather than listing as readings, and the ones [`crate::link`] clears
/// out of the child's environment so the file is what the agent reads.
///
/// The endpoint is not one of them. It is typed rather than nudged, and a
/// machine that points the agent somewhere with an exported `NOOB_BASE_URL` is
/// a machine doing that on purpose.
pub const OWNED: [&str; 8] = [
    API_STYLE,
    REASONING,
    CTX,
    MAX_ROUNDS,
    TASK_CONCURRENCY,
    TASK_MAX_TURNS,
    TASK_TOOLS,
    TASK_WALL_CLOCK,
];

/// The CLI's own bounds for [`CTX`], read off `crates/noob/src/config/mod.rs`:
/// anything under 4096 is refused there and silently becomes the default, so
/// the panel does not offer one. The top is the panel's own: the CLI has no
/// ceiling, and a track has to end somewhere.
pub const CTX_LOW: f32 = 4096.0;
pub const CTX_HIGH: f32 = 1_048_576.0;
pub const CTX_STEP: f32 = 4096.0;
/// What the CLI uses when the key is not set.
pub const CTX_DEFAULT: u32 = 131_072;
/// The context windows models actually ship with: detents on the panel's
/// track, so a drag lands on 64k, 128k or 256k instead of four thousand
/// either side of one.
pub const CTX_STOPS: [f32; 3] = [65_536.0, 131_072.0, 262_144.0];

/// The CLI's own bounds for [`TASK_CONCURRENCY`]: at least one, and capped at
/// sixty-four there, so the right end of this track is the maximum the agent
/// will honour rather than a number to guess at.
pub const TASK_CONCURRENCY_LOW: f32 = 1.0;
pub const TASK_CONCURRENCY_HIGH: f32 = 64.0;
pub const TASK_CONCURRENCY_STEP: f32 = 1.0;
/// What the CLI uses when the key is not set (`subagent::DEFAULT_CONCURRENCY`).
pub const TASK_CONCURRENCY_DEFAULT: u32 = 4;
/// The counts worth reaching for: detents on the panel's track.
pub const TASK_CONCURRENCY_STOPS: [f32; 3] = [4.0, 8.0, 16.0];

/// Round budgets: 0 is the CLI's "no limit" and its default for both the
/// user agent ([`MAX_ROUNDS`]) and each child ([`TASK_MAX_TURNS`]). The top
/// is the panel's own: the CLI accepts far more, and a track has to end
/// somewhere.
pub const ROUNDS_LOW: f32 = 0.0;
pub const ROUNDS_HIGH: f32 = 200.0;
pub const ROUNDS_STEP: f32 = 1.0;
pub const ROUNDS_DEFAULT: u32 = 0;
pub const ROUNDS_STOPS: [f32; 2] = [25.0, 50.0];

/// What a spawned child may touch when the model does not say
/// (`subagent::DEFAULT_TOOLS`).
pub const TASK_TOOLS_CHOICES: [&str; 3] = ["read-only", "web", "all"];
pub const TASK_TOOLS_DEFAULT: &str = "all";

/// The wall clock in seconds; 0 is the CLI's "no limit" and its default.
pub const WALL_CLOCK_LOW: f32 = 0.0;
pub const WALL_CLOCK_HIGH: f32 = 7_200.0;
pub const WALL_CLOCK_STEP: f32 = 30.0;
pub const WALL_CLOCK_DEFAULT: u32 = 0;
pub const WALL_CLOCK_STOPS: [f32; 2] = [600.0, 1_800.0];

/// The agent's config directory.
///
/// The agent's own rule, not the window's: `noob` resolves it as
/// `$NOOB_CONFIG_DIR`, then `/config` when that directory is there (the bind
/// mount it is given inside a container), then `~/.config/noob`. Deriving it
/// from where the window keeps its own settings would come apart the moment
/// either rule changed, and on a machine with `$XDG_CONFIG_HOME` set it would
/// already be looking in the wrong place.
pub fn config_dir() -> Option<PathBuf> {
    config_dir_from(
        std::env::var_os("NOOB_CONFIG_DIR").map(PathBuf::from),
        Some(PathBuf::from("/config")).filter(|path| path.is_dir()),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// That rule with the machine's answers passed in, so it can be checked without
/// setting environment variables under a test runner that shares them.
fn config_dir_from(
    named: Option<PathBuf>,
    container: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    named
        .filter(|path| !path.as_os_str().is_empty())
        .or(container)
        .or_else(|| Some(home?.join(".config").join("noob")))
}

/// Whether a key holds a credential, by name.
///
/// By name because the window never gets to see the file's meaning: anything
/// that reads as a key, a token, a secret or a password is one, and the panel
/// says whether it is set rather than what it is. Wrong in the safe direction:
/// a setting misread as a secret is a setting the window shows as "set", while
/// the other way round is a key on a screen.
pub fn is_secret(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    ["KEY", "TOKEN", "SECRET", "PASSWORD"]
        .iter()
        .any(|word| key.contains(word))
}

/// Every active assignment in a `.env`, in file order.
///
/// Comments, blanks and commented-out defaults are gone: those are the file
/// documenting itself, and the panel is showing what the agent will actually
/// read. A later line for a key wins, the way the CLI's own parser reads it, so
/// what is listed is what the agent would use.
pub fn read_env(path: &Path) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = assignment(line) else {
            continue;
        };
        match out.iter_mut().find(|(known, _)| *known == key) {
            Some(slot) => slot.1 = value,
            None => out.push((key, value)),
        }
    }
    out
}

/// One `KEY=value` line, as its key and its value, or nothing for a blank line,
/// a comment or anything that is not an assignment.
///
/// `export ` in front is accepted because a `.env` is often sourced by a shell.
/// The value is cleaned the way the CLI cleans it: quotes come off, and an
/// unquoted value can carry a trailing comment after whitespace.
fn assignment(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((key.to_string(), clean(value)))
}

/// A value as the agent reads it: quoted to the closing quote, or up to a
/// trailing comment.
fn clean(raw: &str) -> String {
    let value = raw.trim();
    for quote in ['"', '\''] {
        if let Some(rest) = value.strip_prefix(quote)
            && let Some(end) = rest.find(quote)
        {
            return rest[..end].to_string();
        }
    }
    match value
        .char_indices()
        .find(|(at, ch)| *ch == '#' && value[..*at].ends_with(char::is_whitespace))
    {
        Some((at, _)) => value[..at].trim_end().to_string(),
        None => value.to_string(),
    }
}

/// Change one setting in the agent's `.env`, keeping every other line.
///
/// A port of the CLI's own writer (`crates/noob/src/config/mod.rs`): the active
/// line for the key is replaced where it stands, a key that is not there is
/// appended, and everything else in the file, comments included, is left exactly
/// as it was. There may be keys in that file this window knows nothing about,
/// and losing one would break somebody's agent.
///
/// The result arrives by rename, through the same writer the window's own
/// settings go through, so a crash mid-write cannot leave half a config file.
pub fn write_env(path: &Path, key: &str, value: &str) -> Result<(), String> {
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("{key:?} is not a setting name"));
    }
    // The CLI keeps credentials out of settable config on purpose. This window
    // is a second front end on the same file and does not get to be the loose
    // one.
    if is_secret(key) {
        return Err(format!("{key} is a credential; edit the file to change it"));
    }
    if value.trim().is_empty() {
        return Err(String::from("give a value, or edit the file to unset it"));
    }
    if value.chars().any(|ch| ch == '\n' || ch == '\r') {
        return Err(String::from("the value cannot contain a newline"));
    }
    let old = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let mut done = false;
    let mut lines: Vec<String> = Vec::new();
    for line in old.lines() {
        if assignment(line).is_some_and(|(known, _)| known == key) {
            // The agent takes the last line for a key, so a duplicate left
            // behind would win over the one just written.
            if done {
                continue;
            }
            done = true;
            lines.push(rewritten(line, key, value));
            continue;
        }
        lines.push(line.to_string());
    }
    if !done {
        lines.push(format!("{key}={value}"));
    }
    let mut next = lines.join("\n");
    if !next.is_empty() {
        next.push('\n');
    }
    crate::config::replace_file(path, &next)
}

/// The same line carrying a new value, keeping whatever comment followed the old
/// one. A line is often the only place a setting is documented.
fn rewritten(line: &str, key: &str, value: &str) -> String {
    let trailer = line
        .split_once('=')
        .and_then(|(_, rest)| {
            rest.char_indices()
                .find(|(at, ch)| *ch == '#' && rest[..*at].ends_with(char::is_whitespace))
                .map(|(at, _)| rest[at..].trim_end().to_string())
        })
        .unwrap_or_default();
    let spacer = if trailer.is_empty() { "" } else { "   " };
    format!("{key}={value}{spacer}{trailer}")
}

/// One skill installed under `skills/`, or under the sibling directory a
/// turned-off one is moved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    /// The directory it lives in, which is what the agent loads it by.
    pub dir: String,
    /// What its front matter calls it, or the directory name when it says
    /// nothing.
    pub name: String,
    /// What its front matter says it is for, on one line. Empty when there is
    /// no description to read.
    pub about: String,
    /// The repository its front matter records, when it records one.
    ///
    /// Nothing on disk records where an installed skill came from: the CLI's
    /// skill model is name, description and two paths, its git installer
    /// deletes the clone it staged and copies without `.git`, and neither skill
    /// this repo ships carries the key. So this is read where a skill's author
    /// wrote one and is nothing for every skill nobody did, which is why the
    /// panel says the directory it was found in instead.
    pub repo: Option<String>,
    /// Where it is on disk.
    pub path: PathBuf,
    /// Whether the agent would load it: true in the skills directory, false in
    /// the [`OFF`] sibling beside it. Read off the disk every time, never
    /// remembered: the directory it is in is the whole of the state.
    pub on: bool,
    /// Its `SKILL.md` with the front matter taken off, one line per line, which
    /// is what the panel shows beside the list.
    pub doc: Vec<String>,
}

/// Every skill directory under `at`, by directory name, marked `on` or not.
///
/// A directory with no `SKILL.md`, or one whose front matter is missing or
/// unreadable, is still a skill: it is installed, the agent will find it, and
/// leaving it off this list would say it is not there. It falls back to its
/// directory name.
pub fn read_skills(at: &Path, on: bool) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(at) else {
        return Vec::new();
    };
    let mut out: Vec<Skill> = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Some(dir) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if dir.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let file = path.join("SKILL.md");
        let (name, about, repo) = front_matter(&file);
        out.push(Skill {
            name: name.unwrap_or_else(|| dir.clone()),
            about: about.unwrap_or_default(),
            repo,
            doc: skill_doc(&file),
            path,
            on,
            dir,
        });
    }
    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    out
}

/// Where a turned-off skill lives: beside the skills directory, named the same
/// with [`OFF`] on the end.
pub fn skills_off(skills_at: &Path) -> PathBuf {
    let mut name = skills_at
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_else(|| std::ffi::OsString::from("skills"));
    name.push(OFF);
    skills_at.with_file_name(name)
}

/// One skill directory inside one of those two roots, or why it is not one.
///
/// The guard in front of every move and every delete, and it is deliberately
/// narrow. `dir` has to be one plain directory name, so nothing built from it
/// can walk out of the root with `..` or a path of its own; the thing at that
/// name has to be a real directory and not a symlink, so nothing can be moved
/// or deleted through a link; and the directory's own canonical path has to sit
/// directly inside the canonical root, which is what a link in the root itself
/// or anywhere above it cannot get past. The CLI's own remove has the same
/// shape and refuses every directory this window lists, which is why this is
/// written here rather than sent there.
fn skill_at(root: &Path, dir: &str) -> Result<PathBuf, String> {
    if dir.is_empty()
        || dir.starts_with('.')
        || dir.contains('/')
        || dir.contains('\\')
        || Path::new(dir).components().count() != 1
    {
        return Err(format!("{dir:?} is not the name of a skill directory"));
    }
    let path = root.join(dir);
    let kind = std::fs::symlink_metadata(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .file_type();
    if kind.is_symlink() {
        return Err(format!("{} is a link, not a skill", path.display()));
    }
    if !kind.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    let root_real = std::fs::canonicalize(root)
        .map_err(|e| format!("cannot read {}: {e}", root.display()))?;
    let real =
        std::fs::canonicalize(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if real.parent() != Some(root_real.as_path()) {
        return Err(format!(
            "{} is not inside {}",
            real.display(),
            root_real.display()
        ));
    }
    Ok(path)
}

/// Turn one skill on or off, which is a move between the skills directory and
/// the [`OFF`] sibling beside it. `on` is the state it should end in.
pub fn set_skill(skills_at: &Path, dir: &str, on: bool) -> Result<(), String> {
    let off = skills_off(skills_at);
    let (from_root, to_root) = match on {
        true => (off.as_path(), skills_at),
        false => (skills_at, off.as_path()),
    };
    let from = skill_at(from_root, dir)?;
    std::fs::create_dir_all(to_root)
        .map_err(|e| format!("cannot create {}: {e}", to_root.display()))?;
    let to = to_root.join(dir);
    if std::fs::symlink_metadata(&to).is_ok() {
        return Err(format!("{} is already there", to.display()));
    }
    std::fs::rename(&from, &to).map_err(|e| {
        format!(
            "cannot move {} to {}: {e}",
            from.display(),
            to.display()
        )
    })
}

/// Delete one skill directory. `on` says which of the two roots it is in now.
///
/// The only thing in this window that deletes anything, so it goes through
/// [`skill_at`] and nothing else does the arithmetic: no path this cannot name
/// is ever handed to `remove_dir_all`.
pub fn remove_skill(skills_at: &Path, dir: &str, on: bool) -> Result<(), String> {
    let root = match on {
        true => skills_at.to_path_buf(),
        false => skills_off(skills_at),
    };
    let path = skill_at(&root, dir)?;
    std::fs::remove_dir_all(&path).map_err(|e| format!("cannot remove {}: {e}", path.display()))
}

/// A skill's own document: its `SKILL.md` with the front matter block taken
/// off, since that block is what the name and the description on the row beside
/// it already say.
fn skill_doc(path: &Path) -> Vec<String> {
    let Some(text) = head_of(path, SKILL_BODY_BYTES) else {
        return Vec::new();
    };
    let mut lines = text.lines();
    let mut out: Vec<String> = Vec::new();
    match lines.next() {
        None => return out,
        Some(first) if first.trim() == "---" => {
            for line in lines.by_ref() {
                if line.trim() == "---" {
                    break;
                }
            }
        }
        Some(first) => out.push(first.to_string()),
    }
    out.extend(lines.map(str::to_string));
    while out.first().is_some_and(|line| line.trim().is_empty()) {
        out.remove(0);
    }
    out
}

/// The `name`, `description` and `repo` out of a skill's front matter.
///
/// The front matter is the block between the first `---` line and the next one.
/// Both YAML scalar shapes a description is written in are read: the value on
/// the same line, and a folded block (`>-`, `>`, `|`) whose indented lines
/// follow it. Nothing else about YAML is understood, and nothing else is
/// needed: three keys off the top of a file is not a reason to carry a parser.
fn front_matter(path: &Path) -> (Option<String>, Option<String>, Option<String>) {
    let Some(text) = head_of(path, SKILL_HEAD_BYTES) else {
        return (None, None, None);
    };
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None, None);
    }
    let (mut name, mut about, mut repo) = (None, None, None);
    let mut folding: Option<&mut Option<String>> = None;
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        // An indented line under a folded scalar is part of it.
        if line.starts_with(char::is_whitespace)
            && let Some(slot) = folding.as_deref_mut()
        {
            let piece = line.trim();
            if !piece.is_empty() {
                match slot {
                    Some(text) => {
                        text.push(' ');
                        text.push_str(piece);
                    }
                    None => *slot = Some(piece.to_string()),
                }
            }
            continue;
        }
        folding = None;
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let slot = match key.trim() {
            "name" => &mut name,
            "description" => &mut about,
            "repo" => &mut repo,
            _ => continue,
        };
        let value = value.trim();
        if matches!(value, ">" | ">-" | "|" | "|-" | ">+" | "|+") || value.is_empty() {
            folding = Some(slot);
            continue;
        }
        *slot = Some(value.trim_matches(['"', '\'']).to_string());
    }
    let said = |it: Option<String>| it.filter(|it: &String| !it.is_empty());
    (said(name), said(about), said(repo))
}

/// The agent's global instructions: where the file is, and the text of it the
/// agent would actually get.
///
/// Whitespace-only counts as nothing at all, because that is what the CLI makes
/// of it: `load_agents_md` trims and returns nothing, so a file of blank lines
/// contributes no heading to the prompt and is the same thing as no file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Instructions {
    /// Where the file is or would go, or nothing with no config directory.
    pub path: Option<PathBuf>,
    /// The lines of it, or empty when there is nothing there to read.
    pub body: Vec<String>,
    /// Whether the file is longer than the CLI reads. The tail past
    /// [`AGENTS_CAP`] is in the file and not in the prompt.
    pub capped: bool,
}

/// Read the global `AGENTS.md`, capped the way the CLI caps it.
///
/// The path is the CLI's own: `<config dir>/AGENTS.md`, resolved by
/// [`config_dir`], which is why nothing here spells out a home directory.
pub fn read_instructions(dir: Option<&Path>) -> Instructions {
    read_prompt_file(dir, AGENTS_MD)
}

/// Read the global `TOOLS.md` the same way: the CLI reads the two files with
/// one loader, so the window does too.
pub fn read_tools(dir: Option<&Path>) -> Instructions {
    read_prompt_file(dir, TOOLS_MD)
}

fn read_prompt_file(dir: Option<&Path>, name: &str) -> Instructions {
    let path = dir.map(|dir| dir.join(name));
    let text = path
        .as_deref()
        .and_then(|path| head_of(path, AGENTS_CAP))
        .unwrap_or_default();
    let capped = path
        .as_deref()
        .and_then(|path| std::fs::metadata(path).ok())
        .is_some_and(|it| it.len() > AGENTS_CAP);
    Instructions {
        body: match text.trim().is_empty() {
            true => Vec::new(),
            false => text.lines().map(str::to_string).collect(),
        },
        path,
        capped,
    }
}

/// Where a restore parks the current file: beside it, the same name with
/// `.bak` on the end. One slot, overwritten each time: the bak is the last
/// text a restore replaced, not a history.
pub fn bak_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(".bak");
    path.with_file_name(name)
}

/// Write the shipped default over a prompt file, parking what is there in the
/// `.bak` beside it first. A file that is not there gets no bak, because there
/// is nothing to lose; a bak that cannot be written stops the restore, so the
/// one copy of somebody's text is never traded for the default.
pub fn restore_prompt(path: &Path, default: &str) -> Result<(), String> {
    if path.is_file() {
        let bak = bak_path(path);
        std::fs::copy(path, &bak)
            .map_err(|e| format!("cannot park {} in {}: {e}", path.display(), bak.display()))?;
    }
    write_instructions(path, default)
}

/// Read an `.md` file somebody named, for the panel's load action: the text
/// comes back as lines for the editor and nothing is written anywhere.
///
/// Refused when it is not an `.md` file, cannot be read, or goes past the
/// [`AGENTS_CAP`] the CLI reads to: a loaded buffer past the cap would lose
/// its tail the moment it was saved.
pub fn load_md(path: &Path) -> Result<Vec<String>, String> {
    if path.as_os_str().is_empty() {
        return Err(String::from("type the path to an .md file first"));
    }
    let md = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
    if !md {
        return Err(format!("{} is not an .md file", path.display()));
    }
    let size = std::fs::metadata(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .len();
    if size > AGENTS_CAP {
        return Err(format!(
            "{} goes past the {} KiB the CLI reads; cut it down first",
            path.display(),
            AGENTS_CAP / 1024
        ));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(text.lines().map(str::to_string).collect())
}

/// Write one prompt file whole, for the panel's editor: the text lands at
/// once, through the same rename every write here arrives by, so a crash
/// mid-write cannot leave half of somebody's instructions. The directory is
/// created on the way when it is missing, and a file that ends without a
/// newline is given one, because that is how every writer here leaves a file.
pub fn write_instructions(path: &Path, text: &str) -> Result<(), String> {
    let mut whole = String::from(text);
    if !whole.is_empty() && !whole.ends_with('\n') {
        whole.push('\n');
    }
    crate::config::replace_file(path, &whole)
}

/// The head of a file as text, or nothing when it cannot be read. Lossy, since
/// the cap can land in the middle of a character.
fn head_of(path: &Path, cap: u64) -> Option<String> {
    use std::io::Read;
    let mut head = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(cap)
        .read_to_end(&mut head)
        .ok()?;
    Some(String::from_utf8_lossy(&head).into_owned())
}

/// One MCP server the agent would connect to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    pub name: String,
    /// What it is: a URL, or the command line that starts it.
    pub how: String,
    /// Whether it came from the project file rather than the global one. The
    /// project file wins per server, so this is why a global entry is not the
    /// one in use.
    pub project: bool,
    /// Whether the agent would connect to it: true under `servers`, false under
    /// the [`DISABLED`] sibling in the same file. Read off the file every time.
    pub on: bool,
    /// The entry exactly as the file carries it, pretty printed, which is what
    /// the panel shows beside the list.
    pub entry: String,
}

/// The MCP servers configured for one workspace, and where they were read from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mcp {
    /// `<config>/mcp.json`, when there is a config directory at all.
    pub global: Option<PathBuf>,
    /// `<workspace>/.noob/mcp.json`, when there is a workspace.
    pub project: Option<PathBuf>,
    /// Whether either file is there. False is the ordinary case on a machine
    /// where nobody has added a server, and is not trouble.
    pub any_file: bool,
    pub servers: Vec<Server>,
    /// One line per file or entry that could not be read, and why.
    pub trouble: Vec<String>,
}

/// Read both files and merge them the way the CLI does: global first, project
/// second, and the project entry wins for a name in both.
///
/// A file that is not there is not trouble. A file that is there and malformed
/// is one line of trouble and no servers from it: a broken `mcp.json` must not
/// take the panel down, the same way it must not take a session down.
pub fn read_mcp(config_dir: Option<&Path>, workspace: Option<&Path>) -> Mcp {
    let mut mcp = Mcp {
        global: config_dir.map(|dir| dir.join("mcp.json")),
        project: workspace.map(|dir| dir.join(".noob").join("mcp.json")),
        ..Mcp::default()
    };
    let files = [
        (mcp.global.clone(), false),
        (mcp.project.clone(), true),
    ];
    for (path, project) in files.into_iter() {
        let Some(path) = path else {
            continue;
        };
        let Some(text) = head_of(&path, MCP_BYTES) else {
            // Not there is the ordinary case. Something else (a directory, no
            // permission) is worth saying, and telling the two apart is what
            // this second look is for.
            if path.exists() {
                mcp.trouble.push(format!("cannot read {}", path.display()));
            }
            continue;
        };
        mcp.any_file = true;
        let parsed: serde_json::Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(e) => {
                mcp.trouble
                    .push(format!("{} is not valid JSON ({e})", path.display()));
                continue;
            }
        };
        let Some(map) = parsed.get("servers").and_then(serde_json::Value::as_object) else {
            mcp.trouble.push(format!(
                "{} has no \"servers\" object",
                path.display()
            ));
            continue;
        };
        // The servers the agent would connect to, then the ones this window
        // moved out of its way. The second key is not the CLI's: its loader
        // reads `servers` and nothing else, which is exactly what makes an
        // entry in there an entry that is off.
        let off = parsed.get(DISABLED).and_then(serde_json::Value::as_object);
        for (map, on) in [(Some(map), true), (off, false)] {
            let Some(map) = map else {
                continue;
            };
            for (name, entry) in map {
                match how_of(entry) {
                    Some(how) => {
                        mcp.servers.retain(|server| server.name != *name);
                        mcp.servers.push(Server {
                            name: name.clone(),
                            how,
                            project,
                            on,
                            entry: serde_json::to_string_pretty(entry)
                                .unwrap_or_else(|_| entry.to_string()),
                        });
                    }
                    None => mcp.trouble.push(format!(
                        "{name} in {} has neither a url nor a command",
                        path.display()
                    )),
                }
            }
        }
    }
    mcp.servers.sort_by(|a, b| a.name.cmp(&b.name));
    mcp
}

/// What one entry says it is, or nothing when it says neither of the two things
/// an entry can be.
fn how_of(entry: &serde_json::Value) -> Option<String> {
    if let Some(url) = entry.get("url").and_then(serde_json::Value::as_str) {
        return Some(url.to_string());
    }
    let command = entry.get("command").and_then(serde_json::Value::as_str)?;
    let args = entry
        .get("args")
        .and_then(serde_json::Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    match args.is_empty() {
        true => Some(command.to_string()),
        false => Some(format!("{command} {args}")),
    }
}

/// Put a new server into the file's live `servers` object, creating the file
/// when there is none. `how` is either a URL (`http://` or `https://`, kept
/// as a `url` entry) or a command line (the first word the command, the rest
/// its `args`). A name already in the file, on or off, is refused rather
/// than replaced: adding must never silently rewrite lines somebody typed.
///
/// The same whole-file rules as every other write here: parse, one key in,
/// serialize before anything opens, and the rename in
/// [`crate::config::replace_file`] so a crash mid-write cannot leave half an
/// `mcp.json`.
pub fn add_server(path: &Path, name: &str, how: &str) -> Result<(), String> {
    let (name, how) = (name.trim(), how.trim());
    if name.is_empty() {
        return Err(String::from("a server has to have a name"));
    }
    if how.is_empty() {
        return Err(String::from("a server has to have a command or a URL"));
    }
    let mut root: serde_json::Value = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| format!("{} is not valid JSON ({e}); fix it first", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            serde_json::Value::Object(serde_json::Map::new())
        }
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let Some(object) = root.as_object_mut() else {
        return Err(format!("{} is not a JSON object", path.display()));
    };
    for held in ["servers", DISABLED] {
        let taken = object
            .get(held)
            .and_then(serde_json::Value::as_object)
            .is_some_and(|map| map.contains_key(name));
        if taken {
            return Err(format!("{name} is already in {}", path.display()));
        }
    }
    let entry = match how.starts_with("http://") || how.starts_with("https://") {
        true => serde_json::json!({ "url": how }),
        false => {
            let mut words = how.split_whitespace().map(String::from);
            let command = words.next().expect("a non-empty command line");
            let args: Vec<String> = words.collect();
            match args.is_empty() {
                true => serde_json::json!({ "command": command }),
                false => serde_json::json!({ "command": command, "args": args }),
            }
        }
    };
    let into = object
        .entry("servers")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let Some(into) = into.as_object_mut() else {
        return Err(format!("\"servers\" in {} is not an object", path.display()));
    };
    into.insert(name.to_string(), entry);
    let mut text = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    text.push('\n');
    crate::config::replace_file(path, &text)
}

/// Turn one server on or off, by moving its entry between `servers` and the
/// [`DISABLED`] object beside it in the same file. `on` is the state it should
/// end in.
///
/// The entry is moved whole and nothing else in the file is touched: the file
/// is parsed, one key changes object, and the whole value is written back, so
/// every other server, every `timeout_s` and anything the CLI keeps in there
/// that this window has never heard of comes back out unchanged. The write goes
/// through the same rename the settings file goes through, so a crash mid-write
/// cannot leave half an `mcp.json`.
pub fn set_server(path: &Path, name: &str, on: bool) -> Result<(), String> {
    if name.is_empty() {
        return Err(String::from("a server has to have a name"));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut root: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not valid JSON ({e}); fix it first", path.display()))?;
    let (from, to) = match on {
        true => (DISABLED, "servers"),
        false => ("servers", DISABLED),
    };
    let Some(object) = root.as_object_mut() else {
        return Err(format!("{} is not a JSON object", path.display()));
    };
    let entry = object
        .get_mut(from)
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|map| map.remove(name))
        .ok_or_else(|| format!("{name} is not in {:?} in {}", from, path.display()))?;
    let into = object
        .entry(to)
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let Some(into) = into.as_object_mut() else {
        return Err(format!("{:?} in {} is not an object", to, path.display()));
    };
    into.insert(name.to_string(), entry);
    let mut text = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    text.push('\n');
    crate::config::replace_file(path, &text)
}

/// Take one server out of its file, out of whichever of the two objects it sits
/// in: `servers` when it is on, the [`DISABLED`] sibling when this window has
/// moved it out of the way.
///
/// The one thing this window does to an `mcp.json` that the window cannot undo.
/// A server turned off is still in the file to turn back on; this deletes the
/// lines somebody typed by hand. So it refuses rather than guesses: a file that
/// is not there, is not JSON, or is not an object is an error and nothing is
/// written, and a name in neither object is an error too rather than a quiet
/// success, because a button that answered nothing is worse than a line on the
/// footer.
///
/// Everything else in the file survives. The file is parsed whole, one key is
/// dropped, and the whole value is serialized into a string before anything is
/// opened, so a file that could not be built is a file that was never touched.
/// The write itself is [`crate::config::replace_file`]: a private temporary file
/// beside it, then a rename. Nothing here truncates the file that is there.
pub fn remove_server(path: &Path, name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err(String::from("a server has to have a name"));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut root: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not valid JSON ({e}); fix it first", path.display()))?;
    let Some(object) = root.as_object_mut() else {
        return Err(format!("{} is not a JSON object", path.display()));
    };
    // Both objects, not the one the row said it was in: the row is a snapshot
    // and the file is the truth, and a name left behind in the other one would
    // come straight back as a row the moment the panel read the file again.
    let mut gone = false;
    for key in ["servers", DISABLED] {
        if let Some(map) = object
            .get_mut(key)
            .and_then(serde_json::Value::as_object_mut)
        {
            gone |= map.remove(name).is_some();
        }
    }
    if !gone {
        return Err(format!("{name} is not in {}", path.display()));
    }
    let mut text = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    text.push('\n');
    crate::config::replace_file(path, &text)
}

/// Everything the panel says about the agent, read once when the panel opens.
///
/// A snapshot rather than a live view: the panel is a takeover that is up for a
/// few seconds, and re-reading four places on every frame to draw the same rows
/// would be a disk read per redraw.
#[derive(Clone, Debug)]
pub struct Agent {
    /// The agent's `.env`, whether or not the file exists.
    pub env_path: Option<PathBuf>,
    /// Whether that file is there. Not the same as having no settings: a file
    /// with everything commented out is a working agent that probes for a
    /// model.
    pub env_exists: bool,
    /// Every active assignment in it, in file order.
    pub env: Vec<(String, String)>,
    /// Where the skills live, and what is installed there. The list carries the
    /// turned-off ones as well, out of the [`OFF`] sibling of that directory.
    pub skills_at: Option<PathBuf>,
    pub skills: Vec<Skill>,
    pub mcp: Mcp,
    /// The global `AGENTS.md`, which is the first thing in every prompt.
    pub instructions: Instructions,
    /// The global `TOOLS.md`, appended after it.
    pub tools: Instructions,
    /// The sessions on disk, read with the same reader the folder picker uses.
    pub sessions: crate::sessions::Listing,
    /// When the snapshot was taken, which is what the ages on the session rows
    /// are measured against. Carried rather than read while drawing, so a row's
    /// age is decided once and a test can say what time it is.
    pub now: std::time::SystemTime,
}

impl Default for Agent {
    fn default() -> Agent {
        Agent {
            env_path: None,
            env_exists: false,
            env: Vec::new(),
            skills_at: None,
            skills: Vec::new(),
            mcp: Mcp::default(),
            instructions: Instructions::default(),
            tools: Instructions::default(),
            sessions: crate::sessions::Listing::default(),
            now: std::time::SystemTime::UNIX_EPOCH,
        }
    }
}

impl Agent {
    /// Read all of it. `sessions` comes from the caller because the window
    /// already reads that list for the picker and there is no second reader.
    pub fn read(
        dir: Option<&Path>,
        workspace: Option<&Path>,
        sessions: crate::sessions::Listing,
    ) -> Agent {
        let env_path = dir.map(|dir| dir.join(".env"));
        let skills_at = dir.map(|dir| dir.join("skills"));
        Agent {
            env_exists: env_path.as_deref().is_some_and(Path::is_file),
            env: env_path.as_deref().map(read_env).unwrap_or_default(),
            // Both roots, in one list: a skill that has been turned off is still
            // installed, and a list that dropped it would read as one that lost
            // it. Sorted by directory so the two come back interleaved the way
            // they would be if nothing were off.
            skills: skills_at
                .as_deref()
                .map(|at| {
                    let mut all = read_skills(at, true);
                    all.extend(read_skills(&skills_off(at), false));
                    all.sort_by(|a, b| a.dir.cmp(&b.dir));
                    all
                })
                .unwrap_or_default(),
            skills_at,
            mcp: read_mcp(dir, workspace),
            instructions: read_instructions(dir),
            tools: read_tools(dir),
            env_path,
            sessions,
            now: std::time::SystemTime::now(),
        }
    }

    /// What the file says one setting is, or nothing when the file does not
    /// carry it. An empty value counts as unset, the way the CLI's own lookup
    /// counts it.
    pub fn setting(&self, key: &str) -> Option<&str> {
        self.env
            .iter()
            .find(|(known, _)| known == key)
            .map(|(_, value)| value.as_str())
            .filter(|value| !value.is_empty())
    }

    /// What the file says the endpoint is, or nothing when it is unset. Unset is
    /// a working agent: with no base URL the CLI probes the usual local ports.
    pub fn endpoint(&self) -> Option<&str> {
        self.setting(ENDPOINT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "no0b-agent-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    /// The window looks where the agent writes, which is the agent's rule and
    /// not the window's: a named directory wins, then the mount a container is
    /// given, then the home directory.
    #[test]
    fn the_config_directory_is_the_agents_own_rule() {
        let named = Some(PathBuf::from("/somewhere/noob"));
        let container = Some(PathBuf::from("/config"));
        let home = Some(PathBuf::from("/home/hec"));
        assert_eq!(
            config_dir_from(named.clone(), container.clone(), home.clone()),
            named
        );
        assert_eq!(
            config_dir_from(None, container, home.clone()),
            Some(PathBuf::from("/config"))
        );
        assert_eq!(
            config_dir_from(None, None, home.clone()),
            Some(PathBuf::from("/home/hec/.config/noob"))
        );
        assert_eq!(
            config_dir_from(Some(PathBuf::new()), None, home),
            Some(PathBuf::from("/home/hec/.config/noob")),
            "an empty variable is a variable nobody set"
        );
        assert_eq!(config_dir_from(None, None, None), None, "and nowhere is nowhere");
    }

    /// The shape of the file this window is a second front end on: the keys it
    /// carries are read, the comments around them are not settings, and a
    /// commented-out default is not a setting either.
    #[test]
    fn the_env_reads_as_the_agent_reads_it() {
        let dir = temp("read-env");
        let path = dir.join(".env");
        std::fs::write(
            &path,
            "# noob configuration\n\
             \n\
             # Where the model lives.\n\
             NOOB_BASE_URL=http://localhost:8080/v1\n\
             #NOOB_MODEL=llm\n\
             NOOB_CTX=262144\n\
             export NOOB_TASK_CONCURRENCY=2   # sub-agents\n\
             NOOB_API_KEY=\"sk-secret\"\n",
        )
        .expect("a file");
        let env = read_env(&path);
        assert_eq!(
            env,
            vec![
                (String::from("NOOB_BASE_URL"), String::from("http://localhost:8080/v1")),
                (String::from("NOOB_CTX"), String::from("262144")),
                (String::from("NOOB_TASK_CONCURRENCY"), String::from("2")),
                (String::from("NOOB_API_KEY"), String::from("sk-secret")),
            ],
            "a commented default is not a setting the agent reads"
        );
        assert!(is_secret("NOOB_API_KEY"));
        assert!(!is_secret(ENDPOINT));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The agent's global instructions are one file with one name in the
    /// directory the CLI resolves, and the window reads it the way the CLI does:
    /// capped where the CLI caps it, and whitespace-only counted as nothing at
    /// all, because that is what `load_agents_md` makes of it.
    #[test]
    fn the_global_instructions_are_read_the_way_the_agent_reads_them() {
        let dir = temp("instructions");
        let path = dir.join(AGENTS_MD);

        // Nothing there: the path is still named, so there is somewhere to put
        // one, and there is nothing to show.
        let missing = read_instructions(Some(&dir));
        assert_eq!(missing.path.as_deref(), Some(path.as_path()));
        assert!(missing.body.is_empty());
        assert!(!missing.capped);

        // Written by hand, and read back as its lines.
        std::fs::write(&path, "# Global instructions\n\nbe brief\n").expect("a file");
        let read = read_instructions(Some(&dir));
        assert_eq!(read.body, ["# Global instructions", "", "be brief"]);
        assert!(!read.capped);

        // A file of blank lines is the same thing as no file: the CLI trims it
        // and it contributes no heading to the prompt.
        std::fs::write(&path, "\n \n\t\n").expect("a file");
        assert!(read_instructions(Some(&dir)).body.is_empty());

        // Past the cap, what is on the panel is what the model gets, and the
        // window says the file goes further.
        let long = "x".repeat(AGENTS_CAP as usize + 500);
        std::fs::write(&path, &long).expect("a file");
        let big = read_instructions(Some(&dir));
        assert!(big.capped, "a file the CLI cuts is shown whole");
        assert_eq!(
            big.body.iter().map(|line| line.len()).sum::<usize>(),
            AGENTS_CAP as usize,
            "more of the file is on the panel than the CLI reads"
        );

        // TOOLS.md is read with the same loader from the same directory,
        // because the CLI reads the two files with one.
        std::fs::write(dir.join(TOOLS_MD), "tools text\n").expect("a file");
        let tools = read_tools(Some(&dir));
        assert_eq!(tools.body, ["tools text"]);
        assert_eq!(tools.path.as_deref(), Some(dir.join(TOOLS_MD).as_path()));

        // And with no config directory there is nowhere to read one from, which
        // is said rather than guessed at.
        let nowhere = read_instructions(None);
        assert_eq!(nowhere.path, None);
        assert!(nowhere.body.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The restore parks the current file in the `.bak` beside it and writes
    /// the shipped default in its place. A file that is not there gets no bak,
    /// and an older bak is overwritten rather than kept.
    #[test]
    fn a_restore_parks_a_bak_and_writes_the_default() {
        let dir = temp("restore");
        let path = dir.join(AGENTS_MD);
        let bak = bak_path(&path);
        assert_eq!(bak, dir.join("AGENTS.md.bak"));

        // No file yet: the default lands and no bak appears, because there was
        // nothing to lose.
        restore_prompt(&path, AGENTS_DEFAULT).expect("the default lands");
        assert_eq!(
            std::fs::read_to_string(&path).expect("the file"),
            AGENTS_DEFAULT
        );
        assert!(!bak.exists(), "a bak of a file that was not there");

        // A file somebody wrote: the restore parks it first, then writes the
        // default, and a second restore overwrites the older bak.
        std::fs::write(&path, "mine\n").expect("a file");
        restore_prompt(&path, AGENTS_DEFAULT).expect("the default lands");
        assert_eq!(std::fs::read_to_string(&bak).expect("the bak"), "mine\n");
        assert_eq!(
            std::fs::read_to_string(&path).expect("the file"),
            AGENTS_DEFAULT
        );
        std::fs::write(&path, "newer\n").expect("a file");
        restore_prompt(&path, TOOLS_DEFAULT).expect("the default lands");
        assert_eq!(std::fs::read_to_string(&bak).expect("the bak"), "newer\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The load action reads an `.md` file into lines and writes nothing:
    /// anything that is not a readable `.md` inside the CLI's cap is refused
    /// with the reason.
    #[test]
    fn loading_a_custom_md_reads_lines_and_refuses_the_rest() {
        let dir = temp("load-md");
        let path = dir.join("mine.md");
        std::fs::write(&path, "# Mine\n\nbe brief\n").expect("a file");
        assert_eq!(
            load_md(&path).expect("it reads"),
            ["# Mine", "", "be brief"]
        );

        // Not markdown, not there, or past the cap: refused, with the reason.
        let txt = dir.join("notes.txt");
        std::fs::write(&txt, "text\n").expect("a file");
        assert!(load_md(&txt).expect_err("not md").contains("not an .md file"));
        assert!(load_md(&dir.join("gone.md")).is_err());
        assert!(load_md(Path::new("")).is_err());
        let big = dir.join("big.md");
        std::fs::write(&big, "x".repeat(AGENTS_CAP as usize + 1)).expect("a file");
        assert!(load_md(&big).expect_err("past the cap").contains("16 KiB"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The panel's editor saves the whole file at once: what was handed in is
    /// what the agent reads back, a missing directory is made on the way, and
    /// a write that cannot land leaves whatever was there untouched.
    #[test]
    fn the_whole_instructions_file_is_written_at_once() {
        let dir = temp("write-instructions");
        let path = dir.join("fresh").join(AGENTS_MD);
        write_instructions(&path, "# Mine\n\nbe brief").expect("a new file");
        assert_eq!(
            std::fs::read_to_string(&path).expect("the file"),
            "# Mine\n\nbe brief\n",
            "the file ends in the newline every writer here leaves"
        );
        assert_eq!(
            read_instructions(path.parent()).body,
            ["# Mine", "", "be brief"]
        );

        // Written again, the file is the new text whole: nothing of the old
        // one survives into it.
        write_instructions(&path, "shorter\n").expect("the file takes it");
        assert_eq!(read_instructions(path.parent()).body, ["shorter"]);

        // A path that cannot be written refuses and touches nothing: here the
        // file's name is taken by a directory.
        let blocked = dir.join(AGENTS_MD);
        std::fs::create_dir_all(&blocked).expect("a directory in the way");
        assert!(write_instructions(&blocked, "text").is_err());
        assert!(blocked.is_dir(), "the refusal replaced the directory");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The round trip that matters most: writing the endpoint keeps every other
    /// key and every comment in a file the window did not write.
    #[test]
    fn writing_the_endpoint_keeps_the_rest_of_the_file() {
        let dir = temp("write-env");
        let path = dir.join(".env");
        let before = "# noob configuration. Comments here document the keys.\n\
             \n\
             # Where the model lives (OpenAI-compatible /v1 base URL).\n\
             NOOB_BASE_URL=http://localhost:8080/v1\n\
             #NOOB_API_KEY=noauth\n\
             NOOB_CTX=262144   # context window\n\
             NOOB_SOMETHING_THIS_WINDOW_NEVER_HEARD_OF=keep-me\n";
        std::fs::write(&path, before).expect("a file");

        write_env(&path, ENDPOINT, "http://10.0.0.2:11434/v1").expect("the file takes it");
        let after = std::fs::read_to_string(&path).expect("the file");
        assert!(
            after.contains("NOOB_BASE_URL=http://10.0.0.2:11434/v1"),
            "{after}"
        );
        for kept in [
            "# noob configuration. Comments here document the keys.",
            "# Where the model lives (OpenAI-compatible /v1 base URL).",
            "#NOOB_API_KEY=noauth",
            "NOOB_SOMETHING_THIS_WINDOW_NEVER_HEARD_OF=keep-me",
        ] {
            assert!(after.contains(kept), "{kept:?} was lost:\n{after}");
        }
        // The comment that documents a line survives that line being rewritten.
        assert!(after.contains("# context window"), "{after}");
        assert_eq!(
            read_env(&path).len(),
            3,
            "a key was lost or a second one was added:\n{after}"
        );
        assert_eq!(
            read_env(&path)
                .iter()
                .find(|(key, _)| key == ENDPOINT)
                .map(|(_, value)| value.clone()),
            Some(String::from("http://10.0.0.2:11434/v1"))
        );

        // A key the file does not carry is appended rather than dropped.
        write_env(&path, "NOOB_MODEL", "llm").expect("the file takes it");
        assert_eq!(read_env(&path).len(), 4);

        // And no credential goes through this window, whatever it is called.
        assert!(write_env(&path, "NOOB_API_KEY", "sk-nope").is_err());
        assert!(write_env(&path, "SOME_TOKEN", "nope").is_err());
        assert!(!std::fs::read_to_string(&path).expect("the file").contains("sk-nope"));
        assert!(write_env(&path, ENDPOINT, "").is_err(), "empty is not a URL");
        assert!(write_env(&path, ENDPOINT, "a\nb").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file that is not there yet is written from nothing rather than refused:
    /// an agent configured entirely by environment has no `.env` at all.
    #[test]
    fn a_missing_env_is_written_rather_than_refused() {
        let dir = temp("new-env");
        let path = dir.join(".env");
        write_env(&path, ENDPOINT, "http://localhost:8080/v1").expect("a new file");
        assert_eq!(
            read_env(&path),
            vec![(
                String::from(ENDPOINT),
                String::from("http://localhost:8080/v1")
            )]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A skill is its directory, named and described by its front matter when it
    /// has one and by its directory name when it does not. Its document is the
    /// rest of the file, which is what the panel shows beside the list, and the
    /// repository is read only where its author wrote one.
    #[test]
    fn skills_are_named_by_their_front_matter() {
        let dir = temp("skills");
        std::fs::create_dir_all(dir.join("coding")).expect("a directory");
        std::fs::write(
            dir.join("coding").join("SKILL.md"),
            "---\nname: coding\ndescription: >-\n  Changing code that already\n  exists.\n---\n\n# Changing code\n\nRead it first.\n",
        )
        .expect("a file");
        std::fs::create_dir_all(dir.join("web-search")).expect("a directory");
        std::fs::write(
            dir.join("web-search").join("SKILL.md"),
            "---\nname: web search\ndescription: Search the live web.\nrepo: https://github.com/someone/web-search\n---\n",
        )
        .expect("a file");
        // Installed, undocumented, and still installed.
        std::fs::create_dir_all(dir.join("bare")).expect("a directory");
        std::fs::write(dir.join("loose.md"), "not a skill").expect("a file");

        let read = read_skills(&dir, true);
        assert_eq!(
            read.iter().map(|skill| skill.dir.as_str()).collect::<Vec<_>>(),
            vec!["bare", "coding", "web-search"]
        );
        assert!(read.iter().all(|skill| skill.on), "read as the on list");
        assert_eq!(read[0].name, "bare", "a skill with no front matter is its directory");
        assert_eq!(read[0].about, "");
        assert!(read[0].doc.is_empty(), "no SKILL.md is no document");
        assert_eq!(read[1].name, "coding");
        assert_eq!(read[1].about, "Changing code that already exists.");
        assert_eq!(read[1].path, dir.join("coding"));
        assert_eq!(
            read[1].doc,
            vec![
                String::from("# Changing code"),
                String::new(),
                String::from("Read it first."),
            ],
            "the front matter is not the document"
        );
        // Nothing the CLI writes records where a skill came from, so this is
        // read where a skill's author wrote it and is nothing everywhere else.
        assert_eq!(read[1].repo, None);
        assert_eq!(read[2].name, "web search");
        assert_eq!(
            read[2].repo.as_deref(),
            Some("https://github.com/someone/web-search")
        );

        // The off sibling is read the same way and marked for what it is.
        std::fs::create_dir_all(skills_off(&dir).join("noisy")).expect("a directory");
        let off = read_skills(&skills_off(&dir), false);
        assert_eq!(off.len(), 1);
        assert!(!off[0].on);
        assert!(
            read_skills(&dir.join("nowhere"), true).is_empty(),
            "no skills is not an error"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Turning a skill off moves its directory out of the one place the agent
    /// looks, and turning it back on puts it back. The state is where the
    /// directory is and nothing is remembered anywhere.
    #[test]
    fn a_skill_is_turned_off_by_moving_it_out_of_the_way() {
        let dir = temp("skill-toggle");
        let skills = dir.join("skills");
        std::fs::create_dir_all(skills.join("coding")).expect("a directory");
        std::fs::write(
            skills.join("coding").join("SKILL.md"),
            "---\nname: coding\ndescription: Change code.\n---\n\n# Changing code\n",
        )
        .expect("a file");
        assert_eq!(skills_off(&skills), dir.join("skills.off"));

        set_skill(&skills, "coding", false).expect("it moves out of the way");
        assert!(!skills.join("coding").exists(), "it is still where it was");
        assert!(dir.join("skills.off/coding/SKILL.md").is_file(), "it was lost");
        let agent = Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        assert_eq!(agent.skills.len(), 1, "a skill that is off is still installed");
        assert!(!agent.skills[0].on);
        assert_eq!(agent.skills[0].name, "coding", "and still reads its own file");

        set_skill(&skills, "coding", true).expect("it comes back");
        assert!(skills.join("coding/SKILL.md").is_file());
        assert!(!dir.join("skills.off/coding").exists());
        assert!(
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()).skills[0].on
        );

        // Nothing to move is said rather than done quietly.
        assert!(set_skill(&skills, "coding", true).is_err(), "it is already on");
        assert!(set_skill(&skills, "nothing-here", false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The delete refuses everything that is not a directory sitting directly
    /// in one of the two roots: a path of its own, a walk up out of the root,
    /// and a link pointing anywhere else.
    #[test]
    fn uninstalling_refuses_any_path_that_is_not_a_skill() {
        let dir = temp("skill-remove");
        let skills = dir.join("skills");
        let keep = dir.join("source");
        std::fs::create_dir_all(skills.join("coding")).expect("a directory");
        std::fs::write(skills.join("coding").join("SKILL.md"), "---\n---\n").expect("a file");
        std::fs::create_dir_all(&keep).expect("a directory");
        std::fs::write(keep.join("main.rs"), "fn main() {}").expect("a file");

        for name in ["", ".", "..", "../source", "coding/..", "/etc", ".hidden"] {
            assert!(
                remove_skill(&skills, name, true).is_err(),
                "{name:?} was taken as a skill"
            );
        }
        assert!(keep.join("main.rs").is_file(), "real source was deleted");

        // A link in the skills directory pointing at somebody's project is not
        // a skill and is never followed.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&keep, skills.join("linked")).expect("a link");
            assert!(remove_skill(&skills, "linked", true).is_err());
            assert!(set_skill(&skills, "linked", false).is_err());
            assert!(keep.join("main.rs").is_file(), "the link was followed");
            assert!(skills.join("linked").exists(), "the link itself was removed");
        }

        // And the one thing it does do.
        remove_skill(&skills, "coding", true).expect("a skill in the root goes");
        assert!(!skills.join("coding").exists());
        assert!(remove_skill(&skills, "coding", true).is_err(), "it is gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Neither file is the ordinary case, and it is not trouble: it is a machine
    /// where nobody has added a server.
    #[test]
    fn no_mcp_file_is_none_configured_rather_than_an_error() {
        let dir = temp("mcp-none");
        let mcp = read_mcp(Some(&dir), Some(&dir.join("work")));
        assert!(!mcp.any_file, "nothing was read");
        assert!(mcp.servers.is_empty());
        assert!(mcp.trouble.is_empty(), "{:?}", mcp.trouble);
        // Both paths are named, so the panel can say where to put one.
        assert_eq!(mcp.global, Some(dir.join("mcp.json")));
        assert_eq!(mcp.project, Some(dir.join("work/.noob/mcp.json")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The two files merge the way the CLI merges them, and a malformed one is a
    /// line of trouble rather than a panic or an empty list.
    #[test]
    fn the_two_mcp_files_merge_and_a_broken_one_is_survived() {
        let dir = temp("mcp-merge");
        let work = dir.join("work");
        std::fs::create_dir_all(work.join(".noob")).expect("a directory");
        std::fs::write(
            dir.join("mcp.json"),
            r#"{"servers": {"docs": {"url": "http://localhost:9000/mcp"},
                            "shell": {"command": "mcp-shell", "args": ["--safe"]}}}"#,
        )
        .expect("a file");
        std::fs::write(
            work.join(".noob").join("mcp.json"),
            r#"{"servers": {"docs": {"url": "http://localhost:9100/mcp"}}}"#,
        )
        .expect("a file");
        let mcp = read_mcp(Some(&dir), Some(&work));
        assert!(mcp.any_file);
        assert!(mcp.trouble.is_empty(), "{:?}", mcp.trouble);
        assert_eq!(
            mcp.servers
                .iter()
                .map(|server| (server.name.as_str(), server.how.as_str(), server.project, server.on))
                .collect::<Vec<_>>(),
            vec![
                ("docs", "http://localhost:9100/mcp", true, true),
                ("shell", "mcp-shell --safe", false, true),
            ],
            "the project file wins per server"
        );
        assert!(
            mcp.servers[1].entry.contains("--safe"),
            "the entry itself is not carried: {}",
            mcp.servers[1].entry
        );

        // Half a file, which is what an editor killed mid-save leaves.
        std::fs::write(dir.join("mcp.json"), "{\"servers\": {\"docs\":").expect("a file");
        let broken = read_mcp(Some(&dir), Some(&work));
        assert_eq!(broken.trouble.len(), 1, "{:?}", broken.trouble);
        assert!(broken.trouble[0].contains("not valid JSON"), "{:?}", broken.trouble);
        assert_eq!(broken.servers.len(), 1, "the good file still reads");

        // A file that is JSON and not a config, and an entry that is neither
        // shape: both are said, and neither stops the rest.
        std::fs::write(dir.join("mcp.json"), "[]").expect("a file");
        assert!(read_mcp(Some(&dir), Some(&work)).trouble[0].contains("servers"));
        std::fs::write(dir.join("mcp.json"), r#"{"servers": {"odd": {"port": 1}}}"#)
            .expect("a file");
        let odd = read_mcp(Some(&dir), Some(&work));
        assert_eq!(odd.trouble.len(), 1, "{:?}", odd.trouble);
        assert!(odd.trouble[0].contains("odd"), "{:?}", odd.trouble);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A server is turned off by moving its entry to a key beside `servers`,
    /// which the CLI's loader does not read, and the rest of the file comes back
    /// out of the rewrite exactly as it went in.
    #[test]
    fn a_server_is_added_into_a_fresh_file_and_a_taken_name_is_refused() {
        let dir = temp("mcp-add");
        let path = dir.join("mcp.json");

        // No file yet: the add creates one, a URL as a url entry.
        add_server(&path, "search", "https://localhost:8888/mcp").expect("a url server");
        // A command line: the first word the command, the rest its args.
        add_server(&path, "files", "npx -y files-mcp --root .").expect("a command server");
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("the file"))
                .expect("valid json");
        assert_eq!(root["servers"]["search"]["url"], "https://localhost:8888/mcp");
        assert_eq!(root["servers"]["files"]["command"], "npx");
        assert_eq!(
            root["servers"]["files"]["args"],
            serde_json::json!(["-y", "files-mcp", "--root", "."])
        );

        // A name already there, on or off, is refused rather than replaced.
        let taken = add_server(&path, "search", "something-else").expect_err("a taken name");
        assert!(taken.contains("already"), "{taken}");
        assert!(add_server(&path, "", "x").is_err());
        assert!(add_server(&path, "x", " ").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_server_is_turned_off_without_losing_the_rest_of_the_file() {
        let dir = temp("mcp-toggle");
        let path = dir.join("mcp.json");
        std::fs::write(
            &path,
            r#"{
  "servers": {
    "docs": {"url": "http://localhost:9000/mcp", "timeout_s": 120},
    "shell": {"command": "mcp-shell", "args": ["--safe"]}
  },
  "something_this_window_never_heard_of": {"keep": "me"}
}"#,
        )
        .expect("a file");

        set_server(&path, "docs", false).expect("it moves out of the way");
        let text = std::fs::read_to_string(&path).expect("the file");
        let root: serde_json::Value = serde_json::from_str(&text).expect("still JSON");
        assert!(
            root["servers"].get("docs").is_none(),
            "a server that is off is still in servers: {text}"
        );
        assert!(root[DISABLED]["docs"]["url"].is_string(), "{text}");
        assert_eq!(
            root[DISABLED]["docs"]["timeout_s"], 120,
            "the entry was not moved whole: {text}"
        );
        assert!(root["servers"]["shell"]["command"].is_string(), "{text}");
        assert_eq!(
            root["something_this_window_never_heard_of"]["keep"], "me",
            "the rewrite lost a key nobody here understands: {text}"
        );

        // The panel reads it back as configured and off, not as gone.
        let mcp = read_mcp(Some(&dir), None);
        assert_eq!(mcp.servers.len(), 2);
        let docs = mcp.servers.iter().find(|s| s.name == "docs").expect("still listed");
        assert!(!docs.on);
        assert!(docs.entry.contains("timeout_s"), "{}", docs.entry);
        assert!(mcp.trouble.is_empty(), "{:?}", mcp.trouble);

        set_server(&path, "docs", true).expect("it comes back");
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("the file"))
                .expect("still JSON");
        assert!(root["servers"]["docs"]["url"].is_string());
        assert!(root[DISABLED].as_object().is_some_and(|map| map.is_empty()));
        assert!(read_mcp(Some(&dir), None).servers.iter().all(|s| s.on));

        assert!(set_server(&path, "nothing-here", false).is_err());
        assert!(set_server(&dir.join("nowhere.json"), "docs", false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Uninstalling a server takes its entry out of the file and leaves every
    /// other server, every other key and everything this window has never heard
    /// of exactly where it was.
    #[test]
    fn a_server_is_removed_and_the_rest_of_the_file_survives() {
        let dir = temp("mcp-remove");
        let path = dir.join("mcp.json");
        std::fs::write(
            &path,
            r#"{
  "servers": {
    "docs": {"url": "http://localhost:9000/mcp", "timeout_s": 120},
    "shell": {"command": "mcp-shell", "args": ["--safe"]}
  },
  "something_this_window_never_heard_of": {"keep": "me"}
}"#,
        )
        .expect("a file");

        remove_server(&path, "docs").expect("it goes");
        let text = std::fs::read_to_string(&path).expect("the file");
        let root: serde_json::Value = serde_json::from_str(&text).expect("still JSON");
        assert!(
            root["servers"].get("docs").is_none(),
            "the removed server is still there: {text}"
        );
        assert!(
            root.get(DISABLED).is_none(),
            "removing put it in the off key instead of taking it out: {text}"
        );
        assert_eq!(
            root["servers"]["shell"]["args"][0], "--safe",
            "the other server did not come back whole: {text}"
        );
        assert_eq!(
            root["something_this_window_never_heard_of"]["keep"], "me",
            "the rewrite lost a key nobody here understands: {text}"
        );

        // And the CLI's own rule reads what is left: one server, no trouble.
        let mcp = read_mcp(Some(&dir), None);
        assert_eq!(
            mcp.servers.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["shell"]
        );
        assert!(mcp.trouble.is_empty(), "{:?}", mcp.trouble);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A server that was turned off first is in the other object, and uninstall
    /// still finds it there: a name left behind in the off key would come back
    /// as a row the next time the panel read the file.
    #[test]
    fn a_server_that_is_off_is_removed_out_of_the_key_it_was_moved_to() {
        let dir = temp("mcp-remove-off");
        let path = dir.join("mcp.json");
        std::fs::write(
            &path,
            r#"{"servers": {"docs": {"url": "http://localhost:9000/mcp"}, "shell": {"command": "mcp-shell"}}}"#,
        )
        .expect("a file");
        set_server(&path, "docs", false).expect("it moves out of the way");
        assert!(read_mcp(Some(&dir), None).servers.iter().any(|s| s.name == "docs" && !s.on));

        remove_server(&path, "docs").expect("an off server goes too");
        let text = std::fs::read_to_string(&path).expect("the file");
        let root: serde_json::Value = serde_json::from_str(&text).expect("still JSON");
        assert!(root["servers"].get("docs").is_none(), "{text}");
        assert!(
            root[DISABLED].get("docs").is_none(),
            "it is still in the off key: {text}"
        );
        assert!(root["servers"]["shell"]["command"].is_string(), "{text}");
        assert!(
            read_mcp(Some(&dir), None).servers.iter().all(|s| s.name != "docs"),
            "the panel would still list it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Everything that can go wrong leaves the file byte for byte as it was.
    /// This is the one thing in the window that deletes a line somebody typed by
    /// hand, so a refusal has to be a refusal and not half a rewrite.
    #[test]
    fn a_removal_that_cannot_be_finished_leaves_the_file_alone() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp("mcp-remove-fails");
        let path = dir.join("mcp.json");
        let whole = "{\n  \"servers\": {\n    \"docs\": {\"url\": \"http://localhost:9000/mcp\"}\n  }\n}";
        std::fs::write(&path, whole).expect("a file");

        // A name in neither object, and a name at all.
        assert!(remove_server(&path, "nothing-here").is_err());
        assert!(remove_server(&path, "").is_err());
        assert!(remove_server(&dir.join("nowhere.json"), "docs").is_err());
        assert_eq!(
            std::fs::read_to_string(&path).expect("the file"),
            whole,
            "a refusal rewrote the file"
        );

        // A file that is not JSON at all is a file to fix by hand, never one to
        // overwrite with what this window could parse out of it.
        let half = "{\"servers\": {\"docs\":";
        std::fs::write(&path, half).expect("a file");
        assert!(remove_server(&path, "docs").is_err());
        assert_eq!(std::fs::read_to_string(&path).expect("the file"), half);
        std::fs::write(&path, "[]").expect("a file");
        assert!(remove_server(&path, "docs").is_err());
        assert_eq!(std::fs::read_to_string(&path).expect("the file"), "[]");

        // And a write that cannot happen at all: the temporary file goes beside
        // the real one, so a directory nothing can be created in is a removal
        // that fails with the whole file still there. Skipped where permissions
        // do not apply, which is a machine running the tests as root.
        std::fs::write(&path, whole).expect("a file");
        let locked = dir.join("locked");
        std::fs::create_dir_all(&locked).expect("a directory");
        let inside = locked.join("mcp.json");
        std::fs::write(&inside, whole).expect("a file");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555))
            .expect("read only");
        let refused = std::fs::write(locked.join("probe"), "x").is_err();
        if refused {
            let why = remove_server(&inside, "docs").expect_err("it cannot write there");
            assert!(why.contains("temporary"), "{why}");
            assert_eq!(
                std::fs::read_to_string(&inside).expect("the file"),
                whole,
                "a failed write truncated the file"
            );
        }
        let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The whole snapshot off one directory, which is what the panel opens over.
    #[test]
    fn the_snapshot_reads_every_corner_of_the_agents_directory() {
        let dir = temp("snapshot");
        std::fs::write(dir.join(".env"), "NOOB_BASE_URL=http://localhost:8080/v1\n")
            .expect("a file");
        std::fs::create_dir_all(dir.join("skills").join("coding")).expect("a directory");
        let agent = Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        assert!(agent.env_exists);
        assert_eq!(agent.endpoint(), Some("http://localhost:8080/v1"));
        assert_eq!(agent.skills.len(), 1);
        assert_eq!(agent.skills_at, Some(dir.join("skills")));
        assert!(!agent.mcp.any_file);
        assert_eq!(agent.mcp.project, None, "no workspace, no project file");

        // A machine with no home directory: everything is empty and nothing
        // panics, which is the same shape a fresh install has.
        let nowhere = Agent::read(None, None, crate::sessions::Listing::default());
        assert!(nowhere.env.is_empty());
        assert_eq!(nowhere.endpoint(), None);
        assert!(!nowhere.env_exists);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
