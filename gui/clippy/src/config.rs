//! The settings file, and a parser small enough to not be a dependency.
//!
//! `key = value`, one per line, `#` to end of line is a comment. That is the
//! whole format. A TOML parser is four crates and this file has no nesting in
//! it; when it grows one, that is the moment to reconsider, not before.
//!
//! Written on first run with every key present and commented, because a config
//! file you have to read the source to discover is not a config file. Unknown
//! keys are kept and reported rather than dropped, so a typo is visible instead
//! of silently doing nothing.
//!
//! The colors ship as commented defaults rather than live lines. An explicit
//! key beats the `theme` it belongs to, so a file that spelled all 35 colors
//! out would make every theme but the first one do nothing.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

/// Everything the window reads at startup.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// 0.0 fully see-through, 1.0 fully opaque. Scales every panel fill.
    pub opacity: f32,
    pub font_size: f32,
    pub pane_font_size: f32,
    /// The tallest the prompt grows before it scrolls inside itself, in rows.
    pub max_input_rows: usize,

    pub accent: [u8; 3],
    pub text: [u8; 3],
    pub dim: [u8; 3],
    pub bright: [u8; 3],
    pub good: [u8; 3],
    pub bad: [u8; 3],
    /// The color panels are filled with, under the text. Black by default: a
    /// green panel under green text is the thing that makes it hard to read.
    pub panel: [u8; 3],
    /// The title and status bars, which stay green so the window reads as noob.
    pub bar: [u8; 3],

    /// One color per tool, in the order [`Kind`](crate::state::Kind) declares
    /// them. Read by position, so the order is the same as [`TOOL_KEYS`].
    pub tools: [[u8; 3]; 14],

    /// One color per view, in the order [`View`](crate::dock::View) declares
    /// them. Read by position, so the order is the same as [`VIEW_KEYS`].
    pub views: [[u8; 3]; 8],

    pub syntax_comment: [u8; 3],
    pub syntax_string: [u8; 3],
    pub syntax_number: [u8; 3],
    pub syntax_keyword: [u8; 3],
    pub syntax_markup: [u8; 3],

    pub show_activity: bool,
    pub show_files: bool,
    /// Whether the avatar view exists at all. On by default, and the one
    /// setting somebody is definitely going to want off.
    pub show_avatar: bool,
    /// A clip to play instead of the one built in. Anything `gui/asciify`
    /// produced. Unset plays the built-in one.
    pub avatar: Option<PathBuf>,

    /// Keys in the file this build does not know. Reported, never dropped.
    pub unknown: Vec<String>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            opacity: 0.88,
            font_size: 14.0,
            pane_font_size: 13.0,
            max_input_rows: 8,
            accent: [0x7c, 0xd8, 0x94],
            text: [0x9a, 0xd6, 0xac],
            dim: [0x58, 0x96, 0x6e],
            bright: [0xce, 0xfa, 0xdb],
            good: [0x74, 0xd1, 0x94],
            bad: [0xe8, 0x7a, 0x6c],
            panel: [0x00, 0x00, 0x00],
            bar: [0x0e, 0x2e, 0x1e],
            tools: TOOLS,
            views: VIEWS,
            syntax_comment: [0x56, 0x84, 0x66],
            syntax_string: [0xd6, 0xc4, 0x7a],
            syntax_number: [0xb2, 0xce, 0xf0],
            syntax_keyword: [0x82, 0xce, 0xf0],
            syntax_markup: [0xba, 0xa0, 0xe8],
            show_activity: true,
            show_files: true,
            show_avatar: true,
            avatar: None,
            unknown: Vec::new(),
        }
    }
}

/// The key for each tool color, in the order `Kind::ALL` declares the tools.
/// The position here is the position in [`Config::tools`].
pub const TOOL_KEYS: [&str; 14] = [
    "tool_bash",
    "tool_read",
    "tool_ls",
    "tool_glob",
    "tool_grep",
    "tool_context",
    "tool_write",
    "tool_edit",
    "tool_web",
    "tool_skill",
    "tool_mcp",
    "tool_agent",
    "tool_plan",
    "tool_other",
];

/// One hue per tool, spread far enough apart to tell at a glance and far
/// enough from the window's own green not to read as ordinary text. Grouping
/// them by category was the first attempt and read as no color at all, because
/// most of a session is read, ls and grep.
///
/// A theme keeps the first twelve: they name tools rather than the window, and
/// a categorical color that moved with the palette would stop naming anything.
/// The last two are prose, so they follow the theme's `bright` and `text`.
const TOOLS: [[u8; 3]; 14] = [
    [0x4f, 0xd6, 0xc8], // bash
    [0x7f, 0xb5, 0xf0], // read
    [0x5f, 0x8f, 0xd0], // ls
    [0xa8, 0xc8, 0xf0], // glob
    [0xc8, 0xd8, 0x4f], // grep
    [0x9a, 0xa4, 0xae], // context
    [0xf5, 0xc2, 0x5a], // write
    [0xf5, 0x9a, 0x4f], // edit
    [0xc0, 0x90, 0xf5], // websearch
    [0xf5, 0x7f, 0xc8], // skill
    [0xf5, 0xd8, 0x4f], // mcp
    [0x7f, 0x7f, 0xf5], // subagent
    [0xce, 0xfa, 0xdb], // plan, the default `bright`
    [0x9a, 0xd6, 0xac], // anything else, the default `text`
];

/// The key for each view color, in the order `View::ALL` declares the views.
/// The position here is the position in [`Config::views`].
pub const VIEW_KEYS: [&str; 8] = [
    "view_talk",
    "view_activity",
    "view_plan",
    "view_agents",
    "view_hardware",
    "view_llm",
    "view_files",
    "view_avatar",
];

/// One hue per view. It marks the tab that is showing, so what a space is
/// holding is answerable from the corner of the eye rather than by reading
/// eight labels. Spread the way the tool hues are, and a theme leaves them
/// alone for the same reason: they name the views, not the window.
///
/// The avatar is the grey one. It is the view with nothing to report, so a hue
/// of its own would compete with the seven that do.
const VIEWS: [[u8; 3]; 8] = [
    [0x73, 0xde, 0x9f], // talk
    [0xf5, 0xc7, 0x5c], // activity
    [0xc6, 0x82, 0xed], // plan
    [0x5f, 0xa3, 0xf2], // agents
    [0x52, 0xe0, 0xe0], // hardware
    [0xf0, 0x75, 0xc3], // llm
    [0xf0, 0x7d, 0x4c], // files
    [0xa9, 0xb1, 0xbc], // avatar
];

fn prose_tools(bright: [u8; 3], text: [u8; 3]) -> [[u8; 3]; 14] {
    let mut tools = TOOLS;
    tools[12] = bright;
    tools[13] = text;
    tools
}

/// The palettes `theme = <name>` accepts.
pub const THEMES: [&str; 4] = ["noob", "amber", "ice", "plum"];

/// A named palette, as a whole `Config`. Resolved before the rest of the file
/// is read, so an explicit key still wins over the theme that set it.
///
/// Every preset is dark under readable text: the panel is near black and the
/// tones stand off it. A preset that is not is an unreadable window, which is
/// why the skin tests run these invariants over all of them.
pub fn theme(name: &str) -> Option<Config> {
    let base = Config::default();
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "noob" => base,
        "amber" => {
            let (bright, text) = ([0xff, 0xe9, 0xc4], [0xe2, 0xc4, 0x95]);
            Config {
                accent: [0xf0, 0xb4, 0x5a],
                text,
                dim: [0x96, 0x78, 0x4a],
                bright,
                good: [0xb6, 0xd0, 0x7a],
                bad: [0xef, 0x7a, 0x63],
                panel: [0x0a, 0x07, 0x04],
                bar: [0x33, 0x22, 0x0e],
                tools: prose_tools(bright, text),
                syntax_comment: [0x8a, 0x70, 0x48],
                syntax_string: [0xdc, 0xc0, 0x7e],
                syntax_number: [0xa8, 0xc8, 0xe8],
                syntax_keyword: [0xf2, 0x8f, 0x4b],
                syntax_markup: [0xd9, 0xa0, 0xd0],
                ..base
            }
        }
        "ice" => {
            let (bright, text) = ([0xdd, 0xf3, 0xff], [0xa8, 0xcc, 0xdf]);
            Config {
                accent: [0x62, 0xc8, 0xf0],
                text,
                dim: [0x5d, 0x7f, 0x96],
                bright,
                good: [0x6a, 0xd0, 0xa8],
                bad: [0xf0, 0x74, 0x8c],
                panel: [0x01, 0x05, 0x0a],
                bar: [0x10, 0x28, 0x3a],
                tools: prose_tools(bright, text),
                syntax_comment: [0x4f, 0x7a, 0x8c],
                syntax_string: [0x9f, 0xd8, 0xc0],
                syntax_number: [0xc0, 0xb8, 0xf0],
                syntax_keyword: [0x9a, 0xb8, 0xff],
                syntax_markup: [0xd5, 0xa8, 0xe8],
                ..base
            }
        }
        "plum" => {
            let (bright, text) = ([0xf2, 0xe2, 0xff], [0xcf, 0xb8, 0xe0]);
            Config {
                accent: [0xc5, 0x8c, 0xf0],
                text,
                dim: [0x86, 0x68, 0x9c],
                bright,
                good: [0x86, 0xd8, 0xa0],
                bad: [0xf0, 0x70, 0x8c],
                panel: [0x06, 0x03, 0x0a],
                bar: [0x2a, 0x14, 0x40],
                tools: prose_tools(bright, text),
                syntax_comment: [0x7a, 0x5c, 0x8e],
                syntax_string: [0xe0, 0xb0, 0xd8],
                syntax_number: [0xa8, 0xc0, 0xf0],
                syntax_keyword: [0xb0, 0xa0, 0xff],
                syntax_markup: [0xf0, 0xc0, 0x80],
                ..base
            }
        }
        _ => return None,
    })
}

/// Every key the parser knows. The writer refuses anything else, and a test
/// checks the shipped file documents all of them.
pub fn keys() -> Vec<&'static str> {
    let mut keys = vec![
        "opacity",
        "font_size",
        "pane_font_size",
        "max_input_rows",
        "theme",
        "accent",
        "text",
        "dim",
        "bright",
        "good",
        "bad",
        "panel",
        "bar",
        "syntax_comment",
        "syntax_string",
        "syntax_number",
        "syntax_keyword",
        "syntax_markup",
        "show_activity",
        "show_files",
        "show_avatar",
        "avatar",
    ];
    keys.extend(TOOL_KEYS);
    keys.extend(VIEW_KEYS);
    keys
}

/// Where the file lives. `$XDG_CONFIG_HOME` when set, `~/.config` otherwise,
/// beside noob's own settings rather than in a directory of its own.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("noob").join("clippy.conf"))
}

impl Config {
    /// Read the file, writing the commented default first if it is not there.
    ///
    /// Any failure returns the defaults: a settings file that cannot be read is
    /// a reason to use the defaults, never a reason to refuse to open a window.
    pub fn load() -> Config {
        let Some(path) = path() else {
            return Config::default();
        };
        if !path.exists() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, DEFAULT_FILE);
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => Config::parse(&text),
            Err(_) => Config::default(),
        }
    }

    pub fn parse(text: &str) -> Config {
        let pairs = pairs(text);
        // The theme is resolved first and every other line lands on top of it,
        // so a preset plus one explicit color is that color over the preset
        // whichever order the two lines are written in.
        let mut config = pairs
            .iter()
            .rev()
            .find(|(key, _)| key == "theme")
            .and_then(|(_, name)| theme(name))
            .unwrap_or_default();
        for (key, value) in pairs {
            let known = match key.as_str() {
                // Already applied above. Resolving it again here is how a name
                // this build does not have gets reported instead of ignored.
                "theme" => theme(&value).is_some(),
                "opacity" => set(&mut config.opacity, number(&value).map(|n| n.clamp(0.05, 1.0))),
                "font_size" => set(&mut config.font_size, number(&value).map(|n| n.clamp(8.0, 40.0))),
                "pane_font_size" => set(
                    &mut config.pane_font_size,
                    number(&value).map(|n| n.clamp(8.0, 40.0)),
                ),
                // A prompt taller than the window is not a prompt, and one of
                // zero rows has nowhere to put the caret.
                "max_input_rows" => set(
                    &mut config.max_input_rows,
                    value.parse::<usize>().ok().map(|rows| rows.clamp(1, 24)),
                ),
                "accent" => set(&mut config.accent, color(&value)),
                "text" => set(&mut config.text, color(&value)),
                "dim" => set(&mut config.dim, color(&value)),
                "bright" => set(&mut config.bright, color(&value)),
                "good" => set(&mut config.good, color(&value)),
                "bad" => set(&mut config.bad, color(&value)),
                "panel" => set(&mut config.panel, color(&value)),
                "bar" => set(&mut config.bar, color(&value)),
                "syntax_comment" => set(&mut config.syntax_comment, color(&value)),
                "syntax_string" => set(&mut config.syntax_string, color(&value)),
                "syntax_number" => set(&mut config.syntax_number, color(&value)),
                "syntax_keyword" => set(&mut config.syntax_keyword, color(&value)),
                "syntax_markup" => set(&mut config.syntax_markup, color(&value)),
                _ if key.starts_with("tool_") => {
                    match TOOL_KEYS.iter().position(|known| *known == key) {
                        Some(at) => set(&mut config.tools[at], color(&value)),
                        None => false,
                    }
                }
                _ if key.starts_with("view_") => {
                    match VIEW_KEYS.iter().position(|known| *known == key) {
                        Some(at) => set(&mut config.views[at], color(&value)),
                        None => false,
                    }
                }
                "show_activity" => set(&mut config.show_activity, boolean(&value)),
                "show_files" => set(&mut config.show_files, boolean(&value)),
                "show_avatar" => set(&mut config.show_avatar, boolean(&value)),
                // Empty means the built-in clip, which is what the shipped
                // file says, so a key left as written is not a broken path.
                "avatar" => {
                    config.avatar = Some(value.trim())
                        .filter(|path| !path.is_empty())
                        .map(PathBuf::from);
                    true
                }
                _ => false,
            };
            if !known {
                config.unknown.push(key);
            }
        }
        config
    }
}

fn set<T>(slot: &mut T, parsed: Option<T>) -> bool {
    match parsed {
        Some(value) => {
            *slot = value;
            true
        }
        // A key this build knows with a value it cannot read keeps the default
        // and is still reported, because a typed value is a typo worth seeing.
        None => false,
    }
}

/// Every `key = value` in the text, in file order, comments and blanks gone.
fn pairs(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| split(line).map(|(key, value, _)| (key, value)))
        .collect()
}

/// One line, as its key, its value, and whatever followed the value. `None`
/// for a blank line or a comment.
///
/// A comment is a line whose first non-space character is `#`, and the value is
/// the first whitespace-separated word after the key. That second half is what
/// lets `accent = #7cd894   # the accent` work: stripping at the first `#`
/// anywhere on the line ate every color in the file, since a hex color starts
/// with one. No value in this format contains a space, so taking the first word
/// is exact rather than a heuristic.
///
/// The writer splits lines through here too, so a line it rewrites can never
/// mean something different to the reader than it did before.
fn split(line: &str) -> Option<(String, String, String)> {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    // `=` optional, so `opacity 0.8` reads the same as `opacity = 0.8`.
    let (key, rest) = match line.split_once('=') {
        Some((key, rest)) => (key, rest),
        None => match line.split_once(char::is_whitespace) {
            Some(pair) => pair,
            None => (line, ""),
        },
    };
    let lead = rest.len() - rest.trim_start().len();
    let value = rest[lead..].split_whitespace().next().unwrap_or_default();
    Some((
        key.trim().to_ascii_lowercase(),
        value.trim_matches('"').to_string(),
        rest[lead + value.len()..].trim().to_string(),
    ))
}

/// The line under a `#`, which is what a commented default in the shipped file
/// is. `None` when the line is not a comment.
fn uncommented(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix('#')?;
    Some(rest.trim_start_matches(' ').to_string())
}

/// The same line carrying a new value, keeping the comment that came after it.
fn rewritten(line: &str, key: &str, value: &str) -> String {
    let trailer = split(line).map_or(String::new(), |(.., trailer)| trailer);
    let spacer = if trailer.is_empty() { "" } else { "   " };
    format!("{key} = {value}{spacer}{trailer}")
}

/// Change one setting in the file, keeping every comment and every other line.
///
/// Ported from noob's own `.env` writer in `crates/noob/src/config/mod.rs`: an
/// active line is replaced where it stands, a missing one is appended, and the
/// result arrives by rename so a crash mid-write cannot leave half a settings
/// file behind. `None` unsets, which here means commenting the line out rather
/// than deleting it, because the line carries the sentence that documents it.
///
/// The shipped file writes every color as a commented default, so a key that
/// exists only as a comment is uncommented in place: the value lands next to
/// its own documentation instead of alone at the end of the file.
pub fn write_setting(path: &Path, key: &str, value: Option<&str>) -> Result<(), String> {
    let key = key.trim().to_ascii_lowercase();
    if !keys().contains(&key.as_str()) {
        return Err(format!("unknown setting {key:?}"));
    }
    if let Some(value) = value {
        // The reader takes the first word after the key, so a value with a
        // space in it would read back as something shorter than it was.
        if value.chars().any(char::is_whitespace) {
            return Err("the value cannot contain a space or a newline".to_string());
        }
        // Ask the parser instead of repeating it: a value the writer accepts
        // and the reader refuses is a setting that silently does nothing.
        if !Config::parse(&format!("{key} = {value}")).unknown.is_empty() {
            return Err(match key.as_str() {
                "theme" => format!("theme must be one of {}", THEMES.join(", ")),
                _ => format!("{value:?} is not a value {key} understands"),
            });
        }
    }

    let old = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let mut done = false;
    let mut lines: Vec<String> = Vec::new();
    for line in old.lines() {
        if split(line).is_some_and(|(active, ..)| active == key) {
            // The reader takes the last line for a key, so a duplicate left
            // behind would win over the one just written.
            if done {
                continue;
            }
            done = true;
            lines.push(match value {
                Some(value) => rewritten(line, &key, value),
                None => format!("# {}", line.trim_end()),
            });
            continue;
        }
        lines.push(line.to_string());
    }
    if !done && let Some(value) = value {
        for line in lines.iter_mut() {
            let Some(bare) = uncommented(line) else {
                continue;
            };
            if split(&bare).is_some_and(|(commented, ..)| commented == key) {
                *line = rewritten(&bare, &key, value);
                done = true;
                break;
            }
        }
    }
    match (done, value) {
        (false, Some(value)) => lines.push(format!("{key} = {value}")),
        // Say so rather than rewriting the file and promising a restart that
        // changes nothing.
        (false, None) => return Err(format!("{key} is not set; nothing to unset")),
        _ => {}
    }

    let mut next = lines.join("\n");
    if !next.is_empty() {
        next.push('\n');
    }
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let permissions = std::fs::symlink_metadata(path)
        .ok()
        .filter(|metadata| metadata.file_type().is_file())
        .map(|metadata| metadata.permissions());
    let (tmp, mut file) =
        open_temp(path).map_err(|e| format!("cannot create a temporary settings file: {e}"))?;
    let replace = (|| -> std::io::Result<()> {
        file.write_all(next.as_bytes())?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if let Err(error) = replace {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("cannot replace {}: {error}", path.display()));
    }
    Ok(())
}

/// A private temporary file beside the settings file. `create_new`, so a name
/// somebody planted first is an error rather than a write through their
/// symlink.
fn open_temp(path: &Path) -> std::io::Result<(PathBuf, std::fs::File)> {
    static SERIAL: AtomicU64 = AtomicU64::new(1);
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map_or_else(|| String::from("clippy.conf"), |n| n.to_string_lossy().into());
    for _ in 0..32 {
        let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!(".{name}.tmp-{}-{serial}", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        match options.open(&tmp) {
            Ok(file) => return Ok((tmp, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "too many stale temporary settings files",
    ))
}

/// The `--set <key>=<value>` a launch may be carrying instead of a workspace.
/// `None` is an ordinary launch; the error is what to print before exiting.
pub fn set_request(args: &[String]) -> Option<Result<(&str, &str), String>> {
    let [flag, rest @ ..] = args else {
        return None;
    };
    if flag != "--set" {
        return None;
    }
    let [assignment] = rest else {
        return Some(Err("usage: clippy --set <key>=<value>".to_string()));
    };
    match assignment.split_once('=') {
        Some((key, value)) => Some(Ok((key.trim(), value.trim()))),
        None => Some(Err(format!("{assignment:?} is not <key>=<value>"))),
    }
}

fn number(value: &str) -> Option<f32> {
    // A percentage reads more naturally for opacity than a fraction does.
    match value.strip_suffix('%') {
        Some(percent) => percent.trim().parse::<f32>().ok().map(|n| n / 100.0),
        None => value.parse().ok(),
    }
}

fn boolean(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// `#rrggbb`, `rrggbb`, or `#rgb`.
fn color(value: &str) -> Option<[u8; 3]> {
    let hex = value.trim().trim_start_matches('#');
    let expand = |c: char| u8::from_str_radix(&format!("{c}{c}"), 16).ok();
    match hex.len() {
        3 => {
            let mut chars = hex.chars();
            Some([
                expand(chars.next()?)?,
                expand(chars.next()?)?,
                expand(chars.next()?)?,
            ])
        }
        6 => Some([
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
        ]),
        _ => None,
    }
}

/// Written on first run. Every key present, every key commented.
const DEFAULT_FILE: &str = "\
# CLIppy settings. `key = value`, one per line, `#` starts a comment.
# Delete this file to get it back with the defaults.

# How solid the window is. 5% is a ghost, 100% is a normal opaque window.
# The panels are drawn dark under green text, so lowering this shows more of
# your desktop through the reading surface. Below about 60% a busy wallpaper
# starts competing with the text; that is a taste call, not a bug.
opacity = 88%

font_size = 14          # the conversation
pane_font_size = 13     # the activity, plan, agents and file panes

# How tall the prompt is allowed to grow, in rows. Past this it scrolls inside
# itself rather than taking more of the conversation.
max_input_rows = 8

# The whole palette, by name: noob, amber, ice, plum.
theme = noob

# Every color the theme sets, #rrggbb, written out here as the noob theme.
# Uncomment a line to keep the theme and override that one color.
# accent = #7cd894        # focus edges, the caret, the context gauge
# text   = #9ad6ac        # ordinary content
# dim    = #58966e        # headers, timings, structure
# bright = #cefadb        # what just happened, and what you typed
# good   = #74d194        # a call that worked
# bad    = #e87a6c        # a call that did not
# panel  = #000000        # panel fill under the text
# bar    = #0e2e1e        # the title and status bars

# One color per tool. These name the tools rather than the window, so a theme
# leaves them alone; the last two are prose and follow bright and text.
# tool_bash    = #4fd6c8
# tool_read    = #7fb5f0
# tool_ls      = #5f8fd0
# tool_glob    = #a8c8f0
# tool_grep    = #c8d84f
# tool_context = #9aa4ae
# tool_write   = #f5c25a
# tool_edit    = #f59a4f
# tool_web     = #c090f5
# tool_skill   = #f57fc8
# tool_mcp     = #f5d84f
# tool_agent   = #7f7ff5
# tool_plan    = #cefadb
# tool_other   = #9ad6ac

# One color per view. It is the line along the top of the tab that is showing,
# so these name the views rather than the window and a theme leaves them alone.
# view_talk     = #73de9f
# view_activity = #f5c75c
# view_plan     = #c682ed
# view_agents   = #5fa3f2
# view_hardware = #52e0e0
# view_llm      = #f075c3
# view_files    = #f07d4c
# view_avatar   = #a9b1bc

# Code in a message: the five things the highlighter can name.
# syntax_comment = #568466
# syntax_string  = #d6c47a
# syntax_number  = #b2cef0
# syntax_keyword = #82cef0
# syntax_markup  = #baa0e8

# Panes. A hidden pane gives its room to the conversation.
show_activity = true
show_files    = true

# The animated ASCII avatar, as its own view. Off removes the tab entirely.
show_avatar = true

# A clip to play instead of the one built in. Any file `gui/asciify` produced:
#   cargo run -p asciify -- your.gif your.txt --cols 40
# Empty plays the built-in one.
avatar =
";

#[cfg(test)]
mod tests {
    use super::*;

    /// The file shipped on first run must parse into exactly the defaults, or
    /// a user who changes nothing gets something other than what was designed.
    #[test]
    fn the_written_default_file_parses_back_to_the_defaults() {
        let parsed = Config::parse(DEFAULT_FILE);
        assert_eq!(parsed, Config::default());
        assert!(parsed.unknown.is_empty(), "{:?}", parsed.unknown);
    }

    /// Every key the file names, live or as a commented default. A commented
    /// default documents the key just as well, and leaving the colors
    /// commented is what lets `theme` mean anything at all.
    fn documented(text: &str) -> Vec<String> {
        text.lines()
            .filter_map(|line| {
                let bare = uncommented(line).unwrap_or_else(|| line.to_string());
                split(&bare).map(|(key, ..)| key)
            })
            .collect()
    }

    /// Every key the struct has must appear in the file, or it is a setting
    /// nobody can discover.
    #[test]
    fn every_setting_is_present_in_the_written_file() {
        let named = documented(DEFAULT_FILE);
        for key in keys() {
            assert!(named.contains(&key.to_string()), "{key} is undocumented");
        }
        assert_eq!(keys().len(), 44, "a new key needs a line in the file");
    }

    /// The commented colors are the noob theme spelled out. A stale hex there
    /// is documentation that lies about what the window will look like.
    #[test]
    fn the_commented_colors_are_the_theme_they_claim_to_be() {
        let live: String = DEFAULT_FILE
            .lines()
            .map(|line| {
                uncommented(line)
                    .filter(|bare| {
                        split(bare).is_some_and(|(key, ..)| keys().contains(&key.as_str()))
                    })
                    .unwrap_or_else(|| line.to_string())
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(Config::parse(&live), Config::default());
    }

    #[test]
    fn comments_blank_lines_and_a_missing_equals_all_read() {
        let config = Config::parse(
            "\n# a comment\n\nopacity = 50%   # trailing comment\nfont_size 20\n\n",
        );
        assert_eq!(config.opacity, 0.5);
        assert_eq!(config.font_size, 20.0);
    }

    /// A hex color starts with the comment character. Stripping at the first
    /// `#` on the line ate every color in the file.
    #[test]
    fn a_color_is_not_eaten_by_the_comment_marker() {
        let config = Config::parse("accent = #7cd894   # focus edges and the caret");
        assert_eq!(config.accent, [0x7c, 0xd8, 0x94]);
        assert!(config.unknown.is_empty(), "{:?}", config.unknown);
        // And a line that really is only a comment is still a comment.
        assert_eq!(Config::parse("  # accent = #ff0000").accent, Config::default().accent);
    }

    #[test]
    fn colors_read_in_every_shape() {
        assert_eq!(color("#7cd894"), Some([0x7c, 0xd8, 0x94]));
        assert_eq!(color("7cd894"), Some([0x7c, 0xd8, 0x94]));
        assert_eq!(color("#0f8"), Some([0x00, 0xff, 0x88]));
        assert_eq!(color("green"), None);
        assert_eq!(color("#12345"), None);
    }

    /// A value out of range is clamped rather than obeyed: opacity 0 is a
    /// window nobody can find again to fix.
    #[test]
    fn opacity_is_clamped_to_something_you_can_still_see() {
        assert_eq!(Config::parse("opacity = 0").opacity, 0.05);
        assert_eq!(Config::parse("opacity = 900%").opacity, 1.0);
        assert_eq!(Config::parse("font_size = 2").font_size, 8.0);
    }

    /// A typo must be visible. Silently ignoring it is how a setting someone
    /// swears they changed does nothing forever.
    #[test]
    fn a_key_this_build_does_not_know_is_reported() {
        let config = Config::parse("opacty = 50%\ncolour = #fff\nopacity = 60%");
        assert_eq!(config.unknown, ["opacty", "colour"]);
        assert_eq!(config.opacity, 0.6, "the good key still applied");
    }

    /// A known key with an unreadable value keeps the default and is reported,
    /// rather than quietly becoming zero.
    #[test]
    fn a_known_key_with_a_bad_value_keeps_the_default_and_is_reported() {
        let config = Config::parse("opacity = very\naccent = chartreuse");
        assert_eq!(config.opacity, Config::default().opacity);
        assert_eq!(config.accent, Config::default().accent);
        assert_eq!(config.unknown, ["opacity", "accent"]);
    }

    #[test]
    fn booleans_read_the_ways_people_write_them() {
        for yes in ["true", "yes", "on", "1", "TRUE"] {
            assert!(Config::parse(&format!("show_files = {yes}")).show_files, "{yes}");
        }
        for no in ["false", "no", "off", "0"] {
            assert!(!Config::parse(&format!("show_files = {no}")).show_files, "{no}");
        }
    }

    /// A file full of nonsense still yields a usable window.
    #[test]
    fn garbage_still_produces_a_working_config() {
        let config = Config::parse("!!!\n===\n\0\n   \n#\n");
        assert_eq!(config.opacity, Config::default().opacity);
    }

    #[test]
    fn every_tool_and_syntax_color_reads_from_the_file() {
        let mut text = String::from("syntax_comment = #010203\nsyntax_markup = #040506\n");
        for (at, key) in TOOL_KEYS.iter().enumerate() {
            text.push_str(&format!("{key} = #{:02x}0000\n", at + 1));
        }
        let config = Config::parse(&text);
        assert!(config.unknown.is_empty(), "{:?}", config.unknown);
        assert_eq!(config.syntax_comment, [0x01, 0x02, 0x03]);
        assert_eq!(config.syntax_markup, [0x04, 0x05, 0x06]);
        for (at, _) in TOOL_KEYS.iter().enumerate() {
            assert_eq!(config.tools[at], [at as u8 + 1, 0, 0], "{}", TOOL_KEYS[at]);
        }
        // A tool key this build has no slot for is a typo, not a new tool.
        assert_eq!(Config::parse("tool_telepathy = #fff").unknown, ["tool_telepathy"]);
    }

    #[test]
    fn every_view_color_reads_from_the_file() {
        let mut text = String::new();
        for (at, key) in VIEW_KEYS.iter().enumerate() {
            text.push_str(&format!("{key} = #00{:02x}00\n", at + 1));
        }
        let config = Config::parse(&text);
        assert!(config.unknown.is_empty(), "{:?}", config.unknown);
        for (at, key) in VIEW_KEYS.iter().enumerate() {
            assert_eq!(config.views[at], [0, at as u8 + 1, 0], "{key}");
        }
        // The table is read by position, so it has to have one slot per view.
        assert_eq!(VIEW_KEYS.len(), crate::dock::View::ALL.len());
        assert_eq!(Config::parse("view_weather = #fff").unknown, ["view_weather"]);
    }

    /// The whole point of a preset: one word changes every color, and the one
    /// color you also wrote down is still yours.
    #[test]
    fn a_theme_sets_the_palette_and_an_explicit_key_still_wins() {
        let amber = Config::parse("theme = amber");
        assert!(amber.unknown.is_empty(), "{:?}", amber.unknown);
        assert_eq!(amber, theme("amber").unwrap());
        assert_ne!(amber.accent, Config::default().accent);
        assert_ne!(amber.syntax_keyword, Config::default().syntax_keyword);

        // Either order, because the theme is resolved before the file is read.
        for text in [
            "theme = amber\naccent = #ff0000",
            "accent = #ff0000\ntheme = amber",
        ] {
            let config = Config::parse(text);
            assert_eq!(config.accent, [0xff, 0x00, 0x00], "{text}");
            assert_eq!(config.text, theme("amber").unwrap().text, "{text}");
        }
    }

    /// `theme = noob` is what the shipped file says, so it has to be exactly
    /// the defaults or a fresh install is not the design.
    #[test]
    fn the_noob_theme_is_the_default_and_the_others_are_not() {
        assert_eq!(theme("noob"), Some(Config::default()));
        for name in THEMES {
            let preset = theme(name).expect(name);
            assert!(preset.unknown.is_empty(), "{name}");
            assert_eq!(preset.tools[12], preset.bright, "{name}: plan is prose");
            assert_eq!(preset.tools[13], preset.text, "{name}: the catch-all is prose");
            assert_eq!(preset.views, VIEWS, "{name}: a view hue names the view");
            if name != "noob" {
                assert_ne!(preset, Config::default(), "{name} is the default twice");
            }
        }
        assert_eq!(theme("chartreuse"), None);
    }

    /// A name this build does not have keeps the defaults and is reported, so
    /// a typo shows up in the window instead of looking like a no-op.
    #[test]
    fn an_unknown_theme_name_is_reported() {
        let config = Config::parse("theme = tangerine");
        assert_eq!(config.unknown, ["theme"]);
        assert_eq!(config.accent, Config::default().accent);
    }

    /// A directory of its own per test, so two tests writing settings at once
    /// cannot read each other's file.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!("clippy-conf-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn conf(&self) -> PathBuf {
            self.0.join("clippy.conf")
        }

        fn read(&self) -> String {
            std::fs::read_to_string(self.conf()).unwrap()
        }

        fn leftovers(&self) -> Vec<String> {
            std::fs::read_dir(&self.0)
                .unwrap()
                .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
                .filter(|name| name != "clippy.conf")
                .collect()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The file is mostly comments and they are the documentation, so a write
    /// that flattens them costs the user more than the setting gained.
    #[test]
    fn writing_a_setting_keeps_every_comment_and_the_other_lines() {
        let scratch = Scratch::new("comments");
        std::fs::write(scratch.conf(), DEFAULT_FILE).unwrap();
        write_setting(&scratch.conf(), "opacity", Some("40%")).unwrap();

        let after = scratch.read();
        assert!(after.contains("opacity = 40%"), "{after}");
        assert!(!after.contains("opacity = 88%"), "{after}");
        // Every comment line survived, including the one on the edited line.
        for line in DEFAULT_FILE.lines().filter(|line| line.starts_with('#')) {
            assert!(after.contains(line), "lost {line:?}");
        }
        assert_eq!(Config::parse(&after).opacity, 0.4);
        assert!(scratch.leftovers().is_empty(), "{:?}", scratch.leftovers());
    }

    /// The colors ship commented, so the first write of one has to land on its
    /// own documented line rather than orphaned at the end of the file.
    #[test]
    fn writing_a_key_that_is_only_documented_uncomments_that_line() {
        let scratch = Scratch::new("uncomment");
        std::fs::write(scratch.conf(), DEFAULT_FILE).unwrap();
        write_setting(&scratch.conf(), "accent", Some("#ff0000")).unwrap();

        let after = scratch.read();
        assert!(
            after.contains("accent = #ff0000   # focus edges, the caret, the context gauge"),
            "{after}"
        );
        assert_eq!(Config::parse(&after).accent, [0xff, 0x00, 0x00]);
        // And the rest of the palette is still commented, so the theme rules it.
        assert_eq!(Config::parse(&after).text, Config::default().text);
    }

    /// A key with no line at all is appended, and writing it again edits that
    /// line instead of stacking a second one the reader would prefer.
    #[test]
    fn a_new_key_is_appended_once_however_often_it_is_written() {
        let scratch = Scratch::new("append");
        std::fs::write(scratch.conf(), "# only a comment\n").unwrap();
        write_setting(&scratch.conf(), "theme", Some("ice")).unwrap();
        write_setting(&scratch.conf(), "theme", Some("plum")).unwrap();

        let after = scratch.read();
        assert_eq!(after.matches("theme = ").count(), 1, "{after}");
        assert!(after.starts_with("# only a comment\n"), "{after}");
        assert_eq!(Config::parse(&after).accent, theme("plum").unwrap().accent);
    }

    /// A file that already carries the same key twice is left with one live
    /// line, because the reader takes the last one.
    #[test]
    fn a_duplicate_line_does_not_survive_the_write() {
        let scratch = Scratch::new("duplicate");
        std::fs::write(scratch.conf(), "opacity = 10%\nfont_size = 20\nopacity = 20%\n").unwrap();
        write_setting(&scratch.conf(), "opacity", Some("50%")).unwrap();

        let after = scratch.read();
        assert_eq!(after, "opacity = 50%\nfont_size = 20\n");
    }

    #[test]
    fn unsetting_comments_the_line_out_and_an_absent_key_says_so() {
        let scratch = Scratch::new("unset");
        std::fs::write(scratch.conf(), "opacity = 10%   # a ghost\n").unwrap();
        write_setting(&scratch.conf(), "opacity", None).unwrap();

        let after = scratch.read();
        assert_eq!(after, "# opacity = 10%   # a ghost\n");
        assert_eq!(Config::parse(&after).opacity, Config::default().opacity);
        assert_eq!(
            write_setting(&scratch.conf(), "opacity", None),
            Err("opacity is not set; nothing to unset".to_string())
        );
    }

    /// The writer refuses anything the reader would refuse, so a written
    /// setting is a setting that took effect.
    #[test]
    fn the_writer_refuses_what_the_reader_cannot_read() {
        let scratch = Scratch::new("refuse");
        let conf = scratch.conf();
        assert!(write_setting(&conf, "colour", Some("#fff")).is_err());
        assert!(write_setting(&conf, "accent", Some("chartreuse")).is_err());
        assert!(write_setting(&conf, "opacity", Some("very")).is_err());
        assert!(write_setting(&conf, "avatar", Some("two words")).is_err());
        assert!(write_setting(&conf, "accent", Some("#ff0000\nbad = #fff")).is_err());
        let theme_error = write_setting(&conf, "theme", Some("tangerine")).unwrap_err();
        assert!(theme_error.contains("noob, amber, ice, plum"), "{theme_error}");
        assert!(!conf.exists(), "a refused write created a file");
    }

    /// A file that does not exist yet is a first write, not a failure.
    #[test]
    fn a_missing_file_is_created_by_the_first_write() {
        let scratch = Scratch::new("create");
        write_setting(&scratch.conf(), "show_files", Some("off")).unwrap();
        assert_eq!(scratch.read(), "show_files = off\n");
        assert!(!Config::parse(&scratch.read()).show_files);
    }

    #[test]
    fn the_set_flag_is_read_off_the_command_line() {
        let args = |args: &[&str]| -> Vec<String> { args.iter().map(|a| a.to_string()).collect() };
        assert_eq!(set_request(&args(&[])), None, "an ordinary launch");
        assert_eq!(set_request(&args(&["/home/me/code"])), None);
        assert_eq!(
            set_request(&args(&["--set", "theme=amber"])),
            Some(Ok(("theme", "amber")))
        );
        assert!(set_request(&args(&["--set"])).is_some_and(|r| r.is_err()));
        assert!(set_request(&args(&["--set", "theme"])).is_some_and(|r| r.is_err()));
        assert!(set_request(&args(&["--set", "a=b", "c=d"])).is_some_and(|r| r.is_err()));
    }
}
