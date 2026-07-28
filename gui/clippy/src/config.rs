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

use std::path::PathBuf;

/// Everything the window reads at startup.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// 0.0 fully see-through, 1.0 fully opaque. Scales every panel fill.
    pub opacity: f32,
    pub font_size: f32,
    pub pane_font_size: f32,

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
            accent: [0x7c, 0xd8, 0x94],
            text: [0x9a, 0xd6, 0xac],
            dim: [0x58, 0x96, 0x6e],
            bright: [0xce, 0xfa, 0xdb],
            good: [0x74, 0xd1, 0x94],
            bad: [0xe8, 0x7a, 0x6c],
            panel: [0x00, 0x00, 0x00],
            bar: [0x0e, 0x2e, 0x1e],
            show_activity: true,
            show_files: true,
            show_avatar: true,
            avatar: None,
            unknown: Vec::new(),
        }
    }
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
        let mut config = Config::default();
        for (key, value) in pairs(text) {
            let known = match key.as_str() {
                "opacity" => set(&mut config.opacity, number(&value).map(|n| n.clamp(0.05, 1.0))),
                "font_size" => set(&mut config.font_size, number(&value).map(|n| n.clamp(8.0, 40.0))),
                "pane_font_size" => set(
                    &mut config.pane_font_size,
                    number(&value).map(|n| n.clamp(8.0, 40.0)),
                ),
                "accent" => set(&mut config.accent, color(&value)),
                "text" => set(&mut config.text, color(&value)),
                "dim" => set(&mut config.dim, color(&value)),
                "bright" => set(&mut config.bright, color(&value)),
                "good" => set(&mut config.good, color(&value)),
                "bad" => set(&mut config.bad, color(&value)),
                "panel" => set(&mut config.panel, color(&value)),
                "bar" => set(&mut config.bar, color(&value)),
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
///
/// A comment is a line whose first non-space character is `#`, and the value is
/// the first whitespace-separated word after the key. That second half is what
/// lets `accent = #7cd894   # the accent` work: stripping at the first `#`
/// anywhere on the line ate every color in the file, since a hex color starts
/// with one. No value in this format contains a space, so taking the first word
/// is exact rather than a heuristic.
fn pairs(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // `=` optional, so `opacity 0.8` reads the same as `opacity = 0.8`.
        let (key, rest) = match line.split_once('=') {
            Some((key, rest)) => (key, rest),
            None => match line.split_once(char::is_whitespace) {
                Some(pair) => pair,
                None => (line, ""),
            },
        };
        let value = rest.split_whitespace().next().unwrap_or_default();
        out.push((
            key.trim().to_ascii_lowercase(),
            value.trim_matches('"').to_string(),
        ));
    }
    out
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

/// Reported by the window on first run, so the file is findable without
/// reading this source.
pub fn describe() -> String {
    match path() {
        Some(path) => format!("settings  {}", path.display()),
        None => String::from("settings  no HOME, using defaults"),
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

# Colors, #rrggbb.
accent = #7cd894        # focus edges, the caret, the context gauge
text   = #9ad6ac        # ordinary content
dim    = #58966e        # headers, timings, structure
bright = #cefadb        # what just happened, and what you typed
good   = #74d194        # a call that worked
bad    = #e87a6c        # a call that did not
panel  = #000000        # panel fill under the text
bar    = #0e2e1e        # the title and status bars

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

    /// Every key the struct has must appear in the file, or it is a setting
    /// nobody can discover.
    #[test]
    fn every_setting_is_present_in_the_written_file() {
        let keys: Vec<String> = pairs(DEFAULT_FILE).into_iter().map(|(k, _)| k).collect();
        for key in [
            "opacity",
            "font_size",
            "pane_font_size",
            "accent",
            "text",
            "dim",
            "bright",
            "good",
            "bad",
            "panel",
            "bar",
            "show_activity",
            "show_files",
            "show_avatar",
            "avatar",
        ] {
            assert!(keys.contains(&key.to_string()), "{key} is undocumented");
        }
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
}
