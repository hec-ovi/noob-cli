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

/// How much of an `mcp.json` is read. These files are a handful of entries; a
/// gigabyte of JSON where one was expected is a mistake, not a configuration.
const MCP_BYTES: u64 = 1024 * 1024;

/// The key the panel lets anybody edit: where the model lives.
pub const ENDPOINT: &str = "NOOB_BASE_URL";

/// The context window the CLI budgets against before it compacts.
pub const CTX: &str = "NOOB_CTX";

/// How many sub-agent tasks the CLI runs at once.
pub const TASK_CONCURRENCY: &str = "NOOB_TASK_CONCURRENCY";

/// The settings in the agent's file the panel owns: the ones it draws as
/// controls rather than listing as readings, and the ones [`crate::link`] clears
/// out of the child's environment so the file is what the agent reads.
///
/// The endpoint is not one of them. It is typed rather than nudged, and a
/// machine that points the agent somewhere with an exported `NOOB_BASE_URL` is
/// a machine doing that on purpose.
pub const OWNED: [&str; 2] = [CTX, TASK_CONCURRENCY];

/// The CLI's own bounds for [`CTX`], read off `crates/noob/src/config/mod.rs`:
/// anything under 4096 is refused there and silently becomes the default, so
/// the panel does not offer one. The top is the panel's own: the CLI has no
/// ceiling, and a track has to end somewhere.
pub const CTX_LOW: f32 = 4096.0;
pub const CTX_HIGH: f32 = 1_048_576.0;
pub const CTX_STEP: f32 = 4096.0;
/// What the CLI uses when the key is not set.
pub const CTX_DEFAULT: u32 = 131_072;

/// The CLI's own bounds for [`TASK_CONCURRENCY`]: at least one, and capped at
/// sixteen there, so the right end of this track is the maximum the agent will
/// honour rather than a number to guess at.
pub const TASK_CONCURRENCY_LOW: f32 = 1.0;
pub const TASK_CONCURRENCY_HIGH: f32 = 16.0;
pub const TASK_CONCURRENCY_STEP: f32 = 1.0;
/// What the CLI uses when the key is not set (`subagent::DEFAULT_CONCURRENCY`).
pub const TASK_CONCURRENCY_DEFAULT: u32 = 4;

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

/// One skill installed under `skills/`.
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
}

/// Every skill directory under `at`, by directory name.
///
/// A directory with no `SKILL.md`, or one whose front matter is missing or
/// unreadable, is still a skill: it is installed, the agent will find it, and
/// leaving it off this list would say it is not there. It falls back to its
/// directory name.
pub fn read_skills(at: &Path) -> Vec<Skill> {
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
        let (name, about) = front_matter(&entry.path().join("SKILL.md"));
        out.push(Skill {
            name: name.unwrap_or_else(|| dir.clone()),
            about: about.unwrap_or_default(),
            dir,
        });
    }
    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    out
}

/// The `name` and `description` out of a skill's front matter.
///
/// The front matter is the block between the first `---` line and the next one.
/// Both YAML scalar shapes a description is written in are read: the value on
/// the same line, and a folded block (`>-`, `>`, `|`) whose indented lines
/// follow it. Nothing else about YAML is understood, and nothing else is
/// needed: two keys off the top of a file is not a reason to carry a parser.
fn front_matter(path: &Path) -> (Option<String>, Option<String>) {
    let Some(text) = head_of(path, SKILL_HEAD_BYTES) else {
        return (None, None);
    };
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }
    let (mut name, mut about) = (None, None);
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
            _ => continue,
        };
        let value = value.trim();
        if matches!(value, ">" | ">-" | "|" | "|-" | ">+" | "|+") || value.is_empty() {
            folding = Some(slot);
            continue;
        }
        *slot = Some(value.trim_matches(['"', '\'']).to_string());
    }
    (name.filter(|it| !it.is_empty()), about.filter(|it| !it.is_empty()))
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
        for (name, entry) in map {
            match how_of(entry) {
                Some(how) => {
                    mcp.servers.retain(|server| server.name != *name);
                    mcp.servers.push(Server {
                        name: name.clone(),
                        how,
                        project,
                    });
                }
                None => mcp.trouble.push(format!(
                    "{name} in {} has neither a url nor a command",
                    path.display()
                )),
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
    /// Where the skills live, and what is installed there.
    pub skills_at: Option<PathBuf>,
    pub skills: Vec<Skill>,
    pub mcp: Mcp,
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
        Agent {
            env_exists: env_path.as_deref().is_some_and(Path::is_file),
            env: env_path.as_deref().map(read_env).unwrap_or_default(),
            skills: dir
                .map(|dir| read_skills(&dir.join("skills")))
                .unwrap_or_default(),
            skills_at: dir.map(|dir| dir.join("skills")),
            mcp: read_mcp(dir, workspace),
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
    /// has one and by its directory name when it does not.
    #[test]
    fn skills_are_named_by_their_front_matter() {
        let dir = temp("skills");
        std::fs::create_dir_all(dir.join("coding")).expect("a directory");
        std::fs::write(
            dir.join("coding").join("SKILL.md"),
            "---\nname: coding\ndescription: >-\n  Changing code that already\n  exists.\n---\n\n# Changing code\n",
        )
        .expect("a file");
        std::fs::create_dir_all(dir.join("web-search")).expect("a directory");
        std::fs::write(
            dir.join("web-search").join("SKILL.md"),
            "---\nname: web search\ndescription: Search the live web.\n---\n",
        )
        .expect("a file");
        // Installed, undocumented, and still installed.
        std::fs::create_dir_all(dir.join("bare")).expect("a directory");
        std::fs::write(dir.join("loose.md"), "not a skill").expect("a file");

        assert_eq!(
            read_skills(&dir),
            vec![
                Skill {
                    dir: String::from("bare"),
                    name: String::from("bare"),
                    about: String::new(),
                },
                Skill {
                    dir: String::from("coding"),
                    name: String::from("coding"),
                    about: String::from("Changing code that already exists."),
                },
                Skill {
                    dir: String::from("web-search"),
                    name: String::from("web search"),
                    about: String::from("Search the live web."),
                },
            ]
        );
        assert!(read_skills(&dir.join("nowhere")).is_empty(), "no skills is not an error");
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
            mcp.servers,
            vec![
                Server {
                    name: String::from("docs"),
                    how: String::from("http://localhost:9100/mcp"),
                    project: true,
                },
                Server {
                    name: String::from("shell"),
                    how: String::from("mcp-shell --safe"),
                    project: false,
                },
            ],
            "the project file wins per server"
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
