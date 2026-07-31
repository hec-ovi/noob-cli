//! The settings file, and a parser small enough to not be a dependency.
//!
//! `key = value`, one per line, `#` to end of line is a comment. That is the
//! whole format. A TOML parser is four crates and this file has no nesting in
//! it; when it grows one, that is the moment to reconsider, not before.
//!
//! Written on first run with every key present and commented, because a config
//! file you have to read the source to discover is not a config file. Unknown
//! keys are kept and reported rather than dropped, so a typo is visible instead
//! of silently doing nothing. A key a past build wrote and this one dropped is
//! not a typo, so [`RETIRED`] names those and the parser passes over them. A key
//! that was merely renamed is not a typo either and its value is still wanted,
//! so [`ALIASES`] maps the old name onto the current one.
//!
//! The colors ship as commented defaults rather than live lines. An explicit
//! key beats the `theme` it belongs to, so a file that spelled all 46 colors
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
    /// How tall the prompt is, in rows. Exactly that many, empty or full.
    ///
    /// One by default: the prompt is one line of the window until you say
    /// otherwise, and a longer prompt scrolls inside that line rather than
    /// pushing the conversation up as you type.
    ///
    /// A height rather than a ceiling. The strip used to climb to this number a
    /// row at a time as characters arrived, which moved the conversation every
    /// time a line wrapped and put the box at a different size every time you
    /// looked at it. Three rows means three rows.
    ///
    /// The one thing that overrides it is the window running out of height:
    /// the panes keep a floor of their own, and a row count that would eat into
    /// it takes what is left over instead. It goes back to the number here as
    /// soon as the window is tall enough for it again.
    ///
    /// Named `max_input_rows` until this build, which is where the confusion
    /// came from: it read as a ceiling and the window never obeyed it at all.
    /// The old name is retired rather than aliased, because the number beside it
    /// was never a choice anybody made.
    pub prompt_rows: usize,
    /// Where the dividers were left, so the arrangement survives a restart the
    /// way every other preference in this window does.
    ///
    /// Four, because each half of the grid has a line of its own: `left_width`
    /// is how much of the width the left column takes and `left_width_bottom`
    /// the same in the bottom row, `top_height` how much of the height the top
    /// space takes in the left column and `top_height_right` the same in the
    /// right one. Only three are live at once, since one line always runs the
    /// whole way across.
    pub left_width: f32,
    pub left_width_bottom: f32,
    pub top_height: f32,
    pub top_height_right: f32,
    /// How much of the settings panel the rail of section names takes, as a
    /// fraction of the panel's width. Dragged by the line between the rail and
    /// the settings beside it, and held to [`RAIL_LOW`]..[`RAIL_HIGH`].
    pub settings_rail: f32,

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

    /// The gauge palette: one color per slot, which a monitor reading names for
    /// itself. Read by position, so the order is the same as [`GAUGE_KEYS`].
    pub gauges: [[u8; 3]; 10],

    pub syntax_comment: [u8; 3],
    pub syntax_string: [u8; 3],
    pub syntax_number: [u8; 3],
    pub syntax_keyword: [u8; 3],
    pub syntax_markup: [u8; 3],

    pub show_activity: bool,
    pub show_files: bool,

    /// Keys in the file this build does not know. Reported, never dropped.
    pub unknown: Vec<String>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            opacity: 0.90,
            font_size: 14.0,
            pane_font_size: 13.0,
            prompt_rows: 1,
            left_width: crate::view::LEFT_WIDTH,
            left_width_bottom: crate::view::LEFT_WIDTH,
            top_height: crate::view::TOP_HEIGHT,
            top_height_right: crate::view::TOP_HEIGHT,
            settings_rail: crate::view::SETTINGS_RAIL,
            accent: [0x7c, 0xd8, 0x94],
            text: [0x9a, 0xd6, 0xac],
            dim: [0x58, 0x96, 0x6e],
            bright: [0xce, 0xfa, 0xdb],
            good: [0x74, 0xd1, 0x94],
            bad: [0xe8, 0x7a, 0x6c],
            panel: [0x00, 0x00, 0x00],
            bar: [0x0e, 0x2e, 0x1e],
            tools: TOOLS,
            gauges: GAUGES,
            syntax_comment: [0x56, 0x84, 0x66],
            syntax_string: [0xd6, 0xc4, 0x7a],
            syntax_number: [0xb2, 0xce, 0xf0],
            syntax_keyword: [0x82, 0xce, 0xf0],
            syntax_markup: [0xba, 0xa0, 0xe8],
            show_activity: true,
            show_files: true,
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

/// The key for each gauge slot. The position here is the position in
/// [`Config::gauges`], and a reading says which slot it wants.
pub const GAUGE_KEYS: [&str; 10] = [
    "gauge_1", "gauge_2", "gauge_3", "gauge_4", "gauge_5", "gauge_6", "gauge_7", "gauge_8",
    "gauge_9", "gauge_10",
];

/// The gauge palette. One hue per reading rather than one for every gauge in
/// the window: a pane of eight readings in a single colour reads as one texture,
/// and which block belongs to which label is then a matter of counting rows.
///
/// Ten slots so no pane ever has to repeat one: the widest carries five
/// readings today (hardware on an AMD card, and both LLM monitors), and nine of
/// the ten are spoken for.
///
/// A slot is a slot rather than a severity, so a set does not have to run from
/// red to green: what a set has to do is stay ten colours you can tell apart
/// side by side, since the blocks are read as a column. This one is the whole
/// spectrum, which is what noob-matrix draws in; the other two themes carry the
/// same ten readings into their own palette below.
const GAUGES: [[u8; 3]; 10] = [
    [0xf0, 0x65, 0x5c], // 1, red
    [0xf5, 0x9a, 0x4f], // 2, orange
    [0xf5, 0xe0, 0x5a], // 3, yellow
    [0xb9, 0xe0, 0x4f], // 4, lime
    [0x7c, 0xd8, 0x94], // 5, green
    [0x4f, 0xd6, 0xc8], // 6, teal
    [0x5f, 0xa3, 0xf2], // 7, blue
    [0x8f, 0x8f, 0xf5], // 8, indigo
    [0xc6, 0x82, 0xed], // 9, violet
    [0xf0, 0x75, 0xc3], // 10, pink
];

/// noob-cool's ten. The cold half of the wheel, green through cyan and blue to
/// violet and rose. A slate sits between the blues, because that stretch is
/// where ten cold hues run closest together and a grey one splits it.
const COOL_GAUGES: [[u8; 3]; 10] = [
    [0x3f, 0xbf, 0x6a], // 1, spring green
    [0x3f, 0xd9, 0xc4], // 2, turquoise
    [0x5c, 0xc8, 0xf5], // 3, sky, the theme's accent
    [0xa8, 0xd8, 0xff], // 4, ice
    [0x8f, 0xa8, 0xbc], // 5, slate
    [0x6f, 0x8f, 0xf0], // 6, periwinkle
    [0xa8, 0x6e, 0xf5], // 7, violet
    [0xd9, 0x8c, 0xf5], // 8, orchid
    [0xf5, 0x7f, 0xc8], // 9, rose
    [0x7c, 0xf2, 0xc0], // 10, mint
];

/// noob-red's ten. Warm the whole way, red through amber and wheat to a leaf
/// green and back out through clay and rose, so the block belongs to the red
/// window the way its syntax colours do.
const RED_GAUGES: [[u8; 3]; 10] = [
    [0xf0, 0x55, 0x4c], // 1, red
    [0xff, 0x94, 0x40], // 2, orange
    [0xf5, 0xc2, 0x5a], // 3, amber
    [0xe8, 0xe8, 0x8c], // 4, wheat
    [0xc8, 0xd8, 0x6a], // 5, warm lime
    [0x82, 0xd4, 0x72], // 6, leaf
    [0xb0, 0x8f, 0x7a], // 7, clay
    [0xe8, 0x96, 0x8f], // 8, dusty rose
    [0xf5, 0x66, 0xb4], // 9, magenta
    [0xcf, 0x8f, 0xd8], // 10, plum
];

fn prose_tools(bright: [u8; 3], text: [u8; 3]) -> [[u8; 3]; 14] {
    let mut tools = TOOLS;
    tools[12] = bright;
    tools[13] = text;
    tools
}

/// The palettes `theme = <name>` accepts. Three, all of them noob: the window
/// is one thing in three colours rather than a swatch book.
pub const THEMES: [&str; 3] = ["noob-matrix", "noob-cool", "noob-red"];

/// Theme names an earlier build wrote, and the standard theme each one is read
/// as now. Same reasoning as [`RETIRED`]: a name in somebody's file was valid
/// when it was written, and answering it with "not a theme" blames them for a
/// rename this window made.
///
/// `noob` is the green the window has always been, which is `noob-matrix`
/// unchanged. `amber` is the warm one, so it lands on `noob-red`; `ice` and
/// `plum` are both cold, so they land on `noob-cool`. None of them is offered on
/// the panel any more: they load, and the row then names the palette in hand.
pub const RETIRED_THEMES: [(&str, &str); 4] = [
    ("noob", "noob-matrix"),
    ("amber", "noob-red"),
    ("ice", "noob-cool"),
    ("plum", "noob-cool"),
];

/// How far from the middle either divider can be put by hand, as a fraction.
///
/// The layout clamps again, against the window it actually has, because a space
/// has a floor in pixels rather than in fractions and a 15% column is plenty on
/// a wide screen and unreadable on a small one. These two only keep the file's
/// own numbers inside what any window could use, and they are also what the
/// settings panel offers, so the panel cannot show a value the file would pull
/// back.
pub const SPLIT_LOW: f32 = 0.15;
pub const SPLIT_HIGH: f32 = 0.85;

/// The same for the settings panel's rail, which is held to a range of its own.
///
/// It goes narrower than a pane divider because a column of section names needs
/// far less room than a pane, and it stops at half the panel because past that
/// the names have more room than the settings they name. The layout clamps again
/// against the window it has, so neither side is ever narrower than the longest
/// section name.
pub const RAIL_LOW: f32 = 0.05;
pub const RAIL_HIGH: f32 = 0.5;

/// A named palette, as a whole `Config`. Resolved before the rest of the file
/// is read, so an explicit key still wins over the theme that set it.
///
/// Every preset is dark under readable text: the panel is near black and the
/// tones stand off it. A preset that is not is an unreadable window, which is
/// why the skin tests run these invariants over all of them.
pub fn theme(name: &str) -> Option<Config> {
    let base = Config::default();
    let asked = name.trim().to_ascii_lowercase();
    // An older build's name first, so a file nobody has opened since the rename
    // still gets a palette instead of an unknown-key report.
    let name = RETIRED_THEMES
        .iter()
        .find(|(was, _)| *was == asked)
        .map_or(asked.as_str(), |(_, now)| *now);
    Some(match name {
        // The window's own green, and the defaults verbatim. A window that
        // opens on nothing at all is this one.
        "noob-matrix" => base,
        // The same window at night: cyan where matrix is green, over a panel
        // with just enough blue in it to be a colour rather than black. `good`
        // stays green because it is the window's yes and reads as one on a cold
        // palette; the cyan next to it is the accent, which is a different job.
        // The gauges come with it: a cold window drawing its meters in the
        // green window's ten was the one thing a theme did not reach.
        "noob-cool" => {
            let (bright, text) = ([0xdf, 0xf2, 0xff], [0xa6, 0xc8, 0xe8]);
            Config {
                accent: [0x5c, 0xc8, 0xf5],
                text,
                dim: [0x62, 0x8a, 0xa8],
                bright,
                good: [0x62, 0xd8, 0xa0],
                bad: [0xf0, 0x74, 0x84],
                panel: [0x02, 0x05, 0x0b],
                bar: [0x0f, 0x24, 0x3c],
                tools: prose_tools(bright, text),
                gauges: COOL_GAUGES,
                syntax_comment: [0x5c, 0x82, 0x9c],
                syntax_string: [0x8f, 0xd8, 0xd0],
                syntax_number: [0xb8, 0xbe, 0xf5],
                syntax_keyword: [0x8f, 0xbc, 0xff],
                syntax_markup: [0xd4, 0xa8, 0xf0],
                ..base
            }
        }
        // Red as the window's colour, not as its alarm. The tones are warm and
        // the bar is a deep maroon, while `bad` stays the one thing louder than
        // the palette around it: a hot orange-red, so a failed call is still
        // the brightest thing in a red window. `good` is green here too, for
        // the same reason it is everywhere: the drop target, the picked row and
        // the showing tab's line are one colour between them.
        "noob-red" => {
            let (bright, text) = ([0xff, 0xdf, 0xd6], [0xe6, 0xae, 0xa6]);
            Config {
                accent: [0xf2, 0x5c, 0x50],
                text,
                dim: [0xa8, 0x6a, 0x64],
                bright,
                good: [0x8f, 0xd8, 0x7a],
                bad: [0xff, 0x8f, 0x5e],
                panel: [0x0a, 0x03, 0x03],
                bar: [0x36, 0x11, 0x10],
                tools: prose_tools(bright, text),
                gauges: RED_GAUGES,
                syntax_comment: [0x9a, 0x68, 0x64],
                syntax_string: [0xe8, 0xc0, 0x86],
                syntax_number: [0xc8, 0xb4, 0xf0],
                syntax_keyword: [0xff, 0x9a, 0x92],
                syntax_markup: [0xf0, 0xc8, 0x70],
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
        "prompt_rows",
        "left_width",
        "left_width_bottom",
        "top_height",
        "top_height_right",
        "settings_rail",
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
    ];
    keys.extend(TOOL_KEYS);
    keys.extend(GAUGE_KEYS);
    keys
}

/// Keys an earlier build wrote into the file and this one deliberately no longer
/// reads. The parser treats them as known and ignores them.
///
/// A settings file is written once and read for years. When a feature goes, the
/// line it owned is still sitting in everybody's file, and reporting it as "not
/// a setting" blames the user for a change this window made. Silencing every
/// unknown key instead would cost the one thing the report is for, which is
/// making a typo visible, so the retired names are listed here by hand and
/// nothing else gets the benefit.
///
/// The writer refuses these too: writing one back would put a line in the file
/// that nothing reads.
///
/// A name here must not also be in [`keys`]. That would be a live setting
/// documented as dead, and which of the two checks won would depend on their
/// order; `no_key_is_both_live_and_retired` fails if it ever happens.
///
/// What removed each one:
///
/// * `show_avatar`, `avatar`: the ASCII avatar was a docked view with a tab and
///   a clip path of its own. Removing the CLIPPY tab removed the view, and
///   nothing has read either key since.
/// * every `view_*`: the tab that is showing carried an accent line in a hue of
///   its own, nine of them, and nine hues on nine tabs is a harlequin strip. The
///   line is one green now ([`Skin::tab_accent`](crate::skin::Skin)), so there is
///   no per-view colour left for any of these to set. `view_avatar` and
///   `view_llm` were already dead when the views they named went; `view_talk`
///   and `view_overall` were the two the renames aliased.
/// * `max_input_rows`: the prompt's height, under a name that read like a
///   ceiling, and the window never read it. Now that it is read, it is
///   `prompt_rows` and it starts at 1. Retired rather than aliased on purpose:
///   every file written before this build carries a number that nothing obeyed
///   when it was written, so carrying it forward would answer a request for a
///   one row prompt with whatever the file happened to say.
pub const RETIRED: [&str; 16] = [
    "show_avatar",
    "avatar",
    "view_avatar",
    "view_llm",
    "view_talk",
    "view_overall",
    "view_output",
    "view_activity",
    "view_plan",
    "view_agents",
    "view_hardware",
    "view_context",
    "view_session",
    "view_debug",
    "view_files",
    "max_input_rows",
];

/// Names an earlier build wrote that this one reads under a different name.
/// `(what the file may say, what this build calls it)`.
///
/// Not the same mechanism as [`RETIRED`] and deliberately kept apart from it: a
/// retired key is dropped on the floor because nothing is behind it any more,
/// while an alias still applies its value to the slot its current name owns. A
/// name in one list must not be in the other, and neither may be in [`keys`],
/// which `an_alias_is_not_a_live_key_and_not_a_retired_one` holds.
///
/// Empty today. Both entries were view colours, whose keys are all retired now
/// that the accent is one colour; the machinery stays because the next rename
/// wants it and because a file written by a build that had it must keep parsing.
const ALIASES: [(&str, &str); 0] = [];

/// What this build calls a key. An older name comes back as its current one,
/// anything else comes back unchanged.
fn canonical(key: &str) -> &str {
    ALIASES
        .iter()
        .find(|(was, _)| *was == key)
        .map_or(key, |(_, now)| *now)
}

/// Where the file lives. `$XDG_CONFIG_HOME` when set, `~/.config` otherwise,
/// beside noob's own settings rather than in a directory of its own.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("noob").join("no0b.conf"))
}

/// The prefix these files carried while the window was called CLIppy.
const LEGACY_PREFIX: &str = "clippy";
/// The prefix they carry now. `no0b.conf` and `no0b.recent`.
const PREFIX: &str = "no0b";

/// Take over the file the same reading used to live in under the old name.
///
/// The window was CLIppy and wrote `clippy.conf` and `clippy.recent`. Renaming
/// it must not read as a fresh install: the palette somebody tuned and the
/// folders they have opened are still theirs and still valid, and there is no
/// version of this where losing them is acceptable.
///
/// Called by each reader on the way in, so it runs once per file and only until
/// the new name exists. It carried a third file, `clippy.totals`, until the
/// all-time counts came off the settings panel and the file went with them; the
/// rename is by prefix and never named any of them, so nothing here changed.
///
/// Nothing here is an error. A rename that cannot happen leaves the new name
/// absent, which every caller already treats as a first run and answers with
/// defaults; refusing to open a window over a leftover file would be far worse
/// than starting fresh. A new file that already exists wins and the old one is
/// left where it is, because the current file is the one the user has been
/// editing.
pub fn adopt_legacy(path: &Path) {
    if path.exists() {
        return;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let Some(rest) = name.strip_prefix(PREFIX) else {
        return;
    };
    let legacy = path.with_file_name(format!("{LEGACY_PREFIX}{rest}"));
    if !legacy.exists() {
        return;
    }
    let _ = std::fs::rename(&legacy, path);
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
        Config::load_from(&path)
    }

    /// The half of [`Config::load`] that knows a path, so a test can hand it one
    /// instead of setting `$XDG_CONFIG_HOME` for the whole process.
    pub fn load_from(path: &Path) -> Config {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Before the existence check below, or the first run under the new name
        // writes a defaults file over settings the user still has.
        adopt_legacy(path);
        if !path.exists() {
            let _ = std::fs::write(path, DEFAULT_FILE);
        }
        match std::fs::read_to_string(path) {
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
            // Before the match, because two of the retired names start with
            // `view_` and the prefix arm below would call them typos.
            if RETIRED.contains(&key.as_str()) {
                continue;
            }
            // A name an earlier build wrote still applies its value, at the slot
            // its current name owns. Resolved here rather than in each arm, so
            // every key gets the same treatment and none of them can forget.
            let name = canonical(&key);
            let known = match name {
                // Already applied above. Resolving it again here is how a name
                // this build does not have gets reported instead of ignored.
                "theme" => theme(&value).is_some(),
                _ => config.apply(name, &value),
            };
            if !known {
                // Reported the way the file spells it, alias or not, so the
                // message names a line somebody can go and find.
                config.unknown.push(key);
            }
        }
        config
    }

    /// Put one `key = value` into this config, held to the same bounds the file
    /// is read with, and say whether the key was one this build knows.
    ///
    /// The whole of [`Config::parse`] except the theme, which is resolved before
    /// the loop so an explicit colour lands on top of it. It is a method rather
    /// than an arm of the loop because a slider being dragged applies its value
    /// to the live config on every motion event and only writes the file when
    /// the button comes up: both paths clamp here, so the window cannot show a
    /// value the file would refuse to carry.
    pub fn apply(&mut self, key: &str, value: &str) -> bool {
        let name = canonical(key);
        match name {
            "opacity" => set(&mut self.opacity, number(value).map(|n| n.clamp(0.05, 1.0))),
            "font_size" => set(&mut self.font_size, number(value).map(|n| n.clamp(8.0, 40.0))),
            "pane_font_size" => set(
                &mut self.pane_font_size,
                number(value).map(|n| n.clamp(8.0, 40.0)),
            ),
            // A prompt taller than the window is not a prompt, and one of
            // zero rows has nowhere to put the caret.
            "prompt_rows" => set(
                &mut self.prompt_rows,
                value.parse::<usize>().ok().map(|rows| rows.clamp(1, 24)),
            ),
            "left_width" => set(
                &mut self.left_width,
                number(value).map(|n| n.clamp(SPLIT_LOW, SPLIT_HIGH)),
            ),
            "left_width_bottom" => set(
                &mut self.left_width_bottom,
                number(value).map(|n| n.clamp(SPLIT_LOW, SPLIT_HIGH)),
            ),
            "top_height" => set(
                &mut self.top_height,
                number(value).map(|n| n.clamp(SPLIT_LOW, SPLIT_HIGH)),
            ),
            "top_height_right" => set(
                &mut self.top_height_right,
                number(value).map(|n| n.clamp(SPLIT_LOW, SPLIT_HIGH)),
            ),
            "settings_rail" => set(
                &mut self.settings_rail,
                number(value).map(|n| n.clamp(RAIL_LOW, RAIL_HIGH)),
            ),
            "accent" => set(&mut self.accent, color(value)),
            "text" => set(&mut self.text, color(value)),
            "dim" => set(&mut self.dim, color(value)),
            "bright" => set(&mut self.bright, color(value)),
            "good" => set(&mut self.good, color(value)),
            "bad" => set(&mut self.bad, color(value)),
            "panel" => set(&mut self.panel, color(value)),
            "bar" => set(&mut self.bar, color(value)),
            "syntax_comment" => set(&mut self.syntax_comment, color(value)),
            "syntax_string" => set(&mut self.syntax_string, color(value)),
            "syntax_number" => set(&mut self.syntax_number, color(value)),
            "syntax_keyword" => set(&mut self.syntax_keyword, color(value)),
            "syntax_markup" => set(&mut self.syntax_markup, color(value)),
            _ if name.starts_with("tool_") => {
                match TOOL_KEYS.iter().position(|known| *known == name) {
                    Some(at) => set(&mut self.tools[at], color(value)),
                    None => false,
                }
            }
            _ if name.starts_with("gauge_") => {
                match GAUGE_KEYS.iter().position(|known| *known == name) {
                    Some(at) => set(&mut self.gauges[at], color(value)),
                    None => false,
                }
            }
            "show_activity" => set(&mut self.show_activity, boolean(value)),
            "show_files" => set(&mut self.show_files, boolean(value)),
            // Including `theme`, which is not a slot: it is a whole palette,
            // resolved by [`Config::parse`] before any line lands on top of it.
            _ => false,
        }
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
    // Said apart from an unknown key, because the name was right once. Either
    // way it is refused: the reader ignores it, so the line would do nothing.
    if RETIRED.contains(&key.as_str()) {
        return Err(format!("{key} is no longer a setting"));
    }
    // An older name is accepted and written under the name this build reads, so
    // the value lands on the commented default that documents it rather than
    // adding a second line for the same slot.
    let key = canonical(&key).to_string();
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
    replace_file(path, &next)
}

/// Put `text` in the file, by rename, keeping the permissions the old file had.
///
/// Extracted from the settings writer because every other file the window keeps
/// has the same requirement and no business having its own opinion about it: a
/// crash mid-write must not leave a half-written file where a whole one was.
pub(crate) fn replace_file(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let permissions = std::fs::symlink_metadata(path)
        .ok()
        .filter(|metadata| metadata.file_type().is_file())
        .map(|metadata| metadata.permissions());
    let (tmp, mut file) =
        open_temp(path).map_err(|e| format!("cannot create a temporary file: {e}"))?;
    let replace = (|| -> std::io::Result<()> {
        file.write_all(text.as_bytes())?;
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

/// A private temporary file beside the file being replaced. `create_new`, so a
/// name somebody planted first is an error rather than a write through their
/// symlink.
fn open_temp(path: &Path) -> std::io::Result<(PathBuf, std::fs::File)> {
    static SERIAL: AtomicU64 = AtomicU64::new(1);
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map_or_else(|| String::from("no0b.conf"), |n| n.to_string_lossy().into());
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
        return Some(Err(format!("usage: {} --set <key>=<value>", crate::BINARY)));
    };
    match assignment.split_once('=') {
        Some((key, value)) => Some(Ok((key.trim(), value.trim()))),
        None => Some(Err(format!("{assignment:?} is not <key>=<value>"))),
    }
}

fn number(value: &str) -> Option<f32> {
    // A percentage reads more naturally for opacity than a fraction does.
    let parsed = match value.strip_suffix('%') {
        Some(percent) => percent.trim().parse::<f32>().ok().map(|n| n / 100.0),
        None => value.parse().ok(),
    };
    // `nan` and `inf` parse. Clamping does not tame either of them, and a NaN
    // that reaches the layout takes a rectangle with it, so they are refused
    // here and reported like any other value the parser cannot use.
    parsed.filter(|n: &f32| n.is_finite())
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
# NO0B settings. `key = value`, one per line, `#` starts a comment.
# Delete this file to get it back with the defaults.

# How solid the window is. 5% is a ghost, 100% is a normal opaque window.
# The panels are drawn dark under green text, so lowering this shows more of
# your desktop through the reading surface. Below about 60% a busy wallpaper
# starts competing with the text; that is a taste call, not a bug.
opacity = 90%

font_size = 14          # the conversation
pane_font_size = 13     # the activity, plan, agents and file panes

# How tall the prompt is, in rows. Exactly this many, typed into or not: the box
# is the size you set rather than one that moves every time a line wraps. A
# longer message scrolls inside it rather than taking room from the
# conversation, and the row the caret is on is the row you see. Raise it if you
# would rather see more of what you are typing at once. On a window too short
# for the number set here, the panes keep their floor and the prompt takes what
# is left; it goes back to this number when there is room for it.
prompt_rows = 1

# Where the dividers sit, as fractions. One line runs the whole way across the
# grid and each half of it is then cut by a line of its own, so the left column
# and the right one can break at different heights. left_width is the left
# column's width and left_width_bottom the same in the bottom row; top_height is
# the top space's height in the left column and top_height_right the same in the
# right one. Dragging a divider writes it back here, so the arrangement survives
# a restart. A space is never dragged below the room it needs to be read,
# whatever these say.
left_width = 0.54
left_width_bottom = 0.54
top_height = 0.46
top_height_right = 0.46

# How much of the settings panel goes to the rail of section names down its
# left, as a fraction of the panel's width. Dragging the line between the rail
# and the settings beside it writes it back here.
settings_rail = 0.10

# The whole palette, by name: noob-matrix, noob-cool, noob-red. An older file
# saying noob, amber, ice or plum still opens; each one reads as the closest of
# the three.
theme = noob-matrix

# Every color the theme sets, #rrggbb, written out here as the noob-matrix theme.
# Uncomment a line to keep the theme and override that one color.
# accent = #7cd894        # focus edges, the caret, the context gauge
# text   = #9ad6ac        # ordinary content
# dim    = #58966e        # headers, timings, structure
# bright = #cefadb        # what just happened, and what you typed
# good   = #74d194        # a call that worked, and the showing tab's line
# bad    = #e87a6c        # a call that did not, and a turn in flight
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

# The gauge palette, in slot order. Every reading in a monitor names one of
# these, so a block and its number carry the metric's own colour rather than one
# colour shared by every gauge in the window. A theme sets these too: these ten
# are noob-matrix's, and the other two themes have ten of their own.
# gauge_1  = #f0655c
# gauge_2  = #f59a4f
# gauge_3  = #f5e05a
# gauge_4  = #b9e04f
# gauge_5  = #7cd894
# gauge_6  = #4fd6c8
# gauge_7  = #5fa3f2
# gauge_8  = #8f8ff5
# gauge_9  = #c682ed
# gauge_10 = #f075c3

# Code in a message: the five things the highlighter can name.
# syntax_comment = #568466
# syntax_string  = #d6c47a
# syntax_number  = #b2cef0
# syntax_keyword = #82cef0
# syntax_markup  = #baa0e8

# Panes. A hidden pane gives its room to the conversation.
show_activity = true
show_files    = true
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

    /// The prompt is one row until somebody asks for more, in the struct and in
    /// the file both. Eight rows of prompt on a window nobody has typed into is
    /// eight rows of conversation nobody can see.
    ///
    /// Written against `max_input_rows`, which is the name this build retired.
    /// The three assertions are the same three; the key they name is the one the
    /// window now reads.
    #[test]
    fn the_prompt_is_one_row_until_it_is_asked_for_more() {
        assert_eq!(Config::default().prompt_rows, 1);
        assert_eq!(Config::parse(DEFAULT_FILE).prompt_rows, 1);
        assert_eq!(Config::parse("prompt_rows = 6").prompt_rows, 6);
    }

    /// A settings file written before the prompt's height was read starts at one
    /// row, and says nothing on the way in.
    ///
    /// The hole this closes: `max_input_rows` sat in every file that was ever
    /// written, the window ignored it, and the first build to read it would have
    /// obeyed a number nobody chose. Hector's own file says 2. So the name is
    /// retired, which is the same treatment a dropped key gets: no unknown-key
    /// report, no value carried over, and the rest of the file read as usual.
    #[test]
    fn an_old_file_asking_for_more_input_rows_still_opens_at_one() {
        let old = "opacity = 70%\nmax_input_rows = 2\nshow_files = off\n";
        let config = Config::parse(old);
        assert_eq!(config.prompt_rows, 1, "a number nobody chose was obeyed");
        assert!(config.unknown.is_empty(), "{:?}", config.unknown);
        // The lines around it are read, which is what separates a retired key
        // from a file the parser gave up on.
        assert_eq!(config.opacity, 0.7);
        assert!(!config.show_files);
        assert!(RETIRED.contains(&"max_input_rows"), "it has to stay retired");
        assert!(!keys().contains(&"max_input_rows"), "and it is not a live key");
        // Whatever it was left carrying, including the value the new key would
        // have taken.
        for value in ["", "2", "8", "24", "nonsense"] {
            let config = Config::parse(&format!("max_input_rows = {value}"));
            assert_eq!(config.prompt_rows, 1, "max_input_rows = {value:?} was obeyed");
            assert!(config.unknown.is_empty(), "{:?}", config.unknown);
        }
    }

    /// The writer never puts the old name back. A file it touches after this
    /// build carries `prompt_rows` and nothing else about the prompt's height.
    #[test]
    fn the_retired_input_row_name_never_reappears_in_a_written_file() {
        let scratch = Scratch::new("prompt-rows");
        let path = scratch.conf();
        std::fs::write(&path, "opacity = 70%\nmax_input_rows = 2\n").expect("the file");
        assert_eq!(
            write_setting(&path, "max_input_rows", Some("2")),
            Err("max_input_rows is no longer a setting".to_string())
        );
        write_setting(&path, "prompt_rows", Some("4")).expect("the new name is written");
        let text = std::fs::read_to_string(&path).expect("the file");
        assert!(text.contains("prompt_rows = 4"), "{text}");
        // The dead line is left exactly as it was: rewriting somebody's file
        // around it is not the writer's job, and the reader passes over it.
        assert_eq!(text.matches("max_input_rows").count(), 1, "{text}");
        assert_eq!(Config::parse(&text).prompt_rows, 4);
        // And a fresh file names only the new key.
        assert!(!DEFAULT_FILE.contains("max_input_rows"), "the shipped file teaches it");
        assert!(DEFAULT_FILE.contains("prompt_rows = 1"));
    }

    /// A value applied while a slider is dragged and the same value read out of
    /// the file land in the same place.
    ///
    /// [`Config::apply`] is the setter [`Config::parse`] uses, and this is what
    /// holds it there: a drag applies its value to the live window without
    /// writing anything, so a second copy of the clamps would let the window
    /// show a number the next launch pulls back.
    #[test]
    fn the_live_setter_holds_a_value_where_the_file_reader_does() {
        for (key, value) in [
            ("opacity", "5"),
            ("opacity", "0"),
            ("opacity", "0.62"),
            ("font_size", "400"),
            ("font_size", "2"),
            ("pane_font_size", "9"),
            ("prompt_rows", "99"),
            ("prompt_rows", "0"),
            ("prompt_rows", "3"),
            ("left_width", "0.99"),
            ("left_width_bottom", "0.01"),
            ("top_height", "0.99"),
            ("top_height_right", "0.42"),
            ("settings_rail", "0.99"),
            ("accent", "#123456"),
            ("show_files", "off"),
        ] {
            let mut live = Config::default();
            assert!(live.apply(key, value), "{key} is not a key this build knows");
            assert_eq!(
                live,
                Config::parse(&format!("{key} = {value}")),
                "{key} = {value} means one thing live and another in the file"
            );
        }
        // A name nothing knows changes nothing and says so, which is how the
        // file reader turns it into a reported typo.
        let mut live = Config::default();
        assert!(!live.apply("no_such_setting", "1"));
        assert!(!live.apply("opacity", "green"), "an unreadable value is not a value");
        assert_eq!(live, Config::default());
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
        assert_eq!(keys().len(), 49, "a new key needs a line in the file");
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

    /// Item 16: where the dividers were left survives a restart. The value a
    /// finished drag writes is the value the next launch lays the window out
    /// with, and a number nobody could use is pulled back rather than obeyed.
    #[test]
    fn the_divider_ratios_are_clamped_and_come_back_out_of_the_file() {
        assert_eq!(Config::default().left_width, crate::view::LEFT_WIDTH);
        assert_eq!(Config::default().top_height, crate::view::TOP_HEIGHT);

        let config = Config::parse("left_width = 0.72\ntop_height = 30%\n");
        assert_eq!(config.left_width, 0.72);
        assert_eq!(config.top_height, 0.3);
        assert!(config.unknown.is_empty(), "{:?}", config.unknown);

        // A whole column of nothing, or none at all, is not an arrangement.
        assert_eq!(Config::parse("left_width = 0").left_width, SPLIT_LOW);
        assert_eq!(Config::parse("left_width = 4").left_width, SPLIT_HIGH);
        assert_eq!(Config::parse("top_height = -1").top_height, SPLIT_LOW);
        // `nan` parses as a number and poisons every rectangle it reaches.
        let mad = Config::parse("top_height = nan\nleft_width = inf\n");
        assert_eq!(mad.top_height, Config::default().top_height);
        assert_eq!(mad.left_width, Config::default().left_width);
        assert_eq!(mad.unknown, ["top_height", "left_width"]);

        // Through the writer and back, which is the round trip a drag makes.
        // All four halves, each its own key: a drag of one line must not be
        // read back as a drag of the line beside it.
        let scratch = Scratch::new("dividers");
        let _ = Config::load_from(&scratch.conf());
        for (key, value) in [
            ("left_width", "0.317"),
            ("left_width_bottom", "0.742"),
            ("top_height", "0.628"),
            ("top_height_right", "0.211"),
        ] {
            write_setting(&scratch.conf(), key, Some(value)).unwrap();
        }
        let after = Config::load_from(&scratch.conf());
        assert_eq!(after.left_width, 0.317);
        assert_eq!(after.left_width_bottom, 0.742);
        assert_eq!(after.top_height, 0.628);
        assert_eq!(after.top_height_right, 0.211);
        // The new pair is clamped by the same rule as the old one.
        let far = Config::parse("left_width_bottom = 0\ntop_height_right = 9\n");
        assert_eq!(far.left_width_bottom, SPLIT_LOW);
        assert_eq!(far.top_height_right, SPLIT_HIGH);
        // And the file the drag wrote through still documents itself.
        assert!(scratch.read().contains("# Where the dividers sit"), "{}", scratch.read());
    }

    /// The settings panel's rail is remembered the same way, through a range of
    /// its own: a column of names is narrower than a pane and stops at half the
    /// panel.
    #[test]
    fn the_settings_rail_is_read_and_written_like_a_divider() {
        assert_eq!(Config::default().settings_rail, crate::view::SETTINGS_RAIL);
        assert_eq!(Config::parse("settings_rail = 0.22").settings_rail, 0.22);
        assert_eq!(Config::parse("settings_rail = 0").settings_rail, RAIL_LOW);
        assert_eq!(Config::parse("settings_rail = 9").settings_rail, RAIL_HIGH);
        // Narrower than a pane divider is allowed to be, which is the point of
        // the second pair of bounds.
        const { assert!(RAIL_LOW < SPLIT_LOW && RAIL_HIGH < SPLIT_HIGH) };

        // Through the writer and back, which is the round trip a drag of it
        // makes, and it comes back as the value the drag left.
        let scratch = Scratch::new("rail");
        let _ = Config::load_from(&scratch.conf());
        write_setting(&scratch.conf(), "settings_rail", Some("0.283")).unwrap();
        assert_eq!(Config::load_from(&scratch.conf()).settings_rail, 0.283);
        assert!(
            scratch.read().contains("settings_rail = 0.283"),
            "{}",
            scratch.read()
        );
    }

    /// A typo must be visible. Silently ignoring it is how a setting someone
    /// swears they changed does nothing forever.
    #[test]
    fn a_key_this_build_does_not_know_is_reported() {
        let config = Config::parse("opacty = 50%\ncolour = #fff\nopacity = 60%");
        assert_eq!(config.unknown, ["opacty", "colour"]);
        assert_eq!(config.opacity, 0.6, "the good key still applied");
    }

    /// A key in both sets is a bug. Whichever check runs first would decide, so
    /// the setting is either live and refused by the writer or dead and still
    /// applied by the reader, depending on nothing anybody can see.
    #[test]
    fn no_key_is_both_live_and_retired() {
        for key in RETIRED {
            assert!(!keys().contains(&key), "{key} is live and retired at once");
        }
    }

    /// The startup report a settings file from an older build used to produce.
    /// The line was correct when it was written, so it says nothing now, and the
    /// typo sitting next to it in the same file still does.
    #[test]
    fn a_retired_key_says_nothing_and_a_typo_still_does() {
        let config = Config::parse("show_avatar = true\navatar =\nopacity = 40%\nopacty = 50%\n");
        assert_eq!(config.unknown, ["opacty"]);
        assert_eq!(config.opacity, 0.4, "the good key still applied");

        // Each retired name on its own, whatever value it was left carrying,
        // reports nothing and changes nothing.
        for key in RETIRED {
            for value in ["", "true", "off", "#ff0000", "/home/me/clip.txt"] {
                let config = Config::parse(&format!("{key} = {value}"));
                assert!(config.unknown.is_empty(), "{key} = {value:?}: {:?}", config.unknown);
                assert_eq!(config, Config::default(), "{key} = {value:?} changed something");
            }
        }
    }

    /// A retired key is not documented in the shipped file. A fresh install that
    /// wrote one would be planting the line the migration above has to forgive.
    #[test]
    fn the_written_file_documents_no_retired_key() {
        let named = documented(DEFAULT_FILE);
        for key in RETIRED {
            assert!(!named.contains(&key.to_string()), "{key} is retired and still documented");
        }
    }

    /// The two mechanisms are different and must stay so. A retired name is
    /// dropped on the floor, an alias applies its value, and a name that ended
    /// up in both lists or in [`keys`] would be decided by whichever check the
    /// parser happens to run first.
    #[test]
    fn an_alias_is_not_a_live_key_and_not_a_retired_one() {
        let named = documented(DEFAULT_FILE);
        for (was, now) in ALIASES {
            assert!(!keys().contains(&was), "{was} is an alias and a live key");
            assert!(!RETIRED.contains(&was), "{was} is an alias and retired");
            assert!(keys().contains(&now), "{was} points at {now}, which is dead");
            assert!(!RETIRED.contains(&now), "{now} is aliased to and retired");
            assert_eq!(canonical(was), now);
            assert_eq!(canonical(now), now, "a current name is left alone");
            // The shipped file teaches the current name only, or a fresh install
            // would write a line that only exists to be translated.
            assert!(!named.contains(&was.to_string()), "{was} is still documented");
            assert!(named.contains(&now.to_string()), "{now} is not documented");
        }
        assert_eq!(canonical("opacity"), "opacity", "and so is anything else");
    }

    /// A file that still carries a view colour keeps parsing and says nothing.
    ///
    /// Nine `view_*` keys coloured the accent line on the showing tab, one hue
    /// each, and that line is one green now. The keys are retired rather than
    /// removed: they are sitting in every file that was ever written, and a
    /// build that called them typos would be blaming the user for a change this
    /// window made. Both spellings the renames left behind are retired with
    /// them, since there is no longer a slot for an alias to land in.
    #[test]
    fn a_view_colour_in_an_old_file_is_ignored_rather_than_reported() {
        let text = "view_talk = #010203\nview_overall = #040506\nview_output = #0a0b0c\n\
                    view_session = #111213\nview_context = #141516\nview_files = #f07d4c\n";
        let config = Config::parse(text);
        assert!(config.unknown.is_empty(), "{:?}", config.unknown);
        // And it changed nothing: a retired key is read off the floor.
        assert_eq!(config, Config::default());

        // Retirement is not a licence for anything else that starts the same
        // way: a name nobody ever wrote is still a typo.
        assert_eq!(Config::parse("view_chatter = #fff").unknown, ["view_chatter"]);
        assert_eq!(Config::parse("view_weather = #fff").unknown, ["view_weather"]);
    }

    /// A file written while the DEBUG pane existed still loads, in full and
    /// without a word about it.
    ///
    /// That pane is gone, and every file this window has ever written carries
    /// `view_debug` because the defaults were written commented into it. The
    /// key stays retired for exactly that reason: a build that called it a typo
    /// would report a line in the settings file on every start, for a change
    /// this window made.
    #[test]
    fn a_file_that_still_names_the_debug_pane_loads_without_complaint() {
        let text = "view_debug = #ff0000\nopacity = 80%\nshow_files = off\n";
        let config = Config::parse(text);
        assert!(config.unknown.is_empty(), "{:?}", config.unknown);
        // The rest of the file was read: one dead key does not cost the lines
        // around it.
        assert_eq!(config.opacity, 0.8);
        assert!(!config.show_files);
        assert!(RETIRED.contains(&"view_debug"), "it has to stay retired");
        assert!(!keys().contains(&"view_debug"), "and it is not a live key");
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
    fn every_gauge_hue_reads_from_the_file() {
        let mut text = String::new();
        for (at, key) in GAUGE_KEYS.iter().enumerate() {
            text.push_str(&format!("{key} = #0000{:02x}\n", at + 1));
        }
        let config = Config::parse(&text);
        assert!(config.unknown.is_empty(), "{:?}", config.unknown);
        for (at, key) in GAUGE_KEYS.iter().enumerate() {
            assert_eq!(config.gauges[at], [0, 0, at as u8 + 1], "{key}");
        }
        // Read by position, so a slot a reading asks for has to exist.
        assert_eq!(GAUGE_KEYS.len(), GAUGES.len());
        assert_eq!(Config::parse("gauge_11 = #fff").unknown, ["gauge_11"]);
    }

    /// A theme paints the meters too. It did not until this build: all three
    /// arms ended at `..base`, so a red window drew its readings in the green
    /// window's ten.
    ///
    /// noob-matrix's ten are written out here rather than compared against the
    /// table, so the window in use keeps exactly the colours it had.
    #[test]
    fn each_theme_draws_the_gauges_in_its_own_ten() {
        assert_eq!(
            theme("noob-matrix").unwrap().gauges,
            [
                [0xf0, 0x65, 0x5c],
                [0xf5, 0x9a, 0x4f],
                [0xf5, 0xe0, 0x5a],
                [0xb9, 0xe0, 0x4f],
                [0x7c, 0xd8, 0x94],
                [0x4f, 0xd6, 0xc8],
                [0x5f, 0xa3, 0xf2],
                [0x8f, 0x8f, 0xf5],
                [0xc6, 0x82, 0xed],
                [0xf0, 0x75, 0xc3],
            ],
            "the green window's meters moved"
        );

        // Every slot is a different colour in every theme, so no reading keeps
        // a hue the palette around it left behind.
        for (i, one) in THEMES.iter().enumerate() {
            let a = theme(one).expect(one);
            for other in &THEMES[i + 1..] {
                let b = theme(other).expect(other);
                for slot in 0..a.gauges.len() {
                    assert_ne!(
                        a.gauges[slot], b.gauges[slot],
                        "{one} and {other} share gauge {}",
                        slot + 1
                    );
                }
            }
        }
    }

    /// Every colour in a `Config`, by the key that writes it.
    ///
    /// Destructured with no `..`, so a field added to the struct stops this
    /// compiling until somebody names it here and the test below decides
    /// whether a theme has to set it.
    fn colours(config: &Config) -> Vec<(&'static str, Vec<u8>)> {
        let Config {
            opacity: _,
            font_size: _,
            pane_font_size: _,
            prompt_rows: _,
            left_width: _,
            left_width_bottom: _,
            top_height: _,
            top_height_right: _,
            settings_rail: _,
            accent,
            text,
            dim,
            bright,
            good,
            bad,
            panel,
            bar,
            tools,
            gauges,
            syntax_comment,
            syntax_string,
            syntax_number,
            syntax_keyword,
            syntax_markup,
            show_activity: _,
            show_files: _,
            unknown: _,
        } = config;
        let flat = |rows: &[[u8; 3]]| rows.iter().flatten().copied().collect::<Vec<u8>>();
        vec![
            ("accent", accent.to_vec()),
            ("text", text.to_vec()),
            ("dim", dim.to_vec()),
            ("bright", bright.to_vec()),
            ("good", good.to_vec()),
            ("bad", bad.to_vec()),
            ("panel", panel.to_vec()),
            ("bar", bar.to_vec()),
            ("tools", flat(tools)),
            ("gauges", flat(gauges)),
            ("syntax_comment", syntax_comment.to_vec()),
            ("syntax_string", syntax_string.to_vec()),
            ("syntax_number", syntax_number.to_vec()),
            ("syntax_keyword", syntax_keyword.to_vec()),
            ("syntax_markup", syntax_markup.to_vec()),
        ]
    }

    /// `..base` says nothing out loud: a theme arm that never mentions a colour
    /// inherits the default's, and there is no line anywhere to notice. That is
    /// how all three themes ended up drawing their gauges in noob-matrix's ten
    /// for four releases.
    ///
    /// So every colour a theme carries has to be a colour the theme chose. A
    /// colour that genuinely belongs to every palette goes in `SHARED` by name,
    /// where it can be argued with, rather than passing in silence.
    #[test]
    fn every_theme_sets_every_colour_in_the_window() {
        /// Colours every theme is meant to share. Empty today: everything the
        /// window is painted in moves with the palette.
        const SHARED: &[&str] = &[];

        let base = colours(&Config::default());
        // noob-matrix is the defaults on purpose, which is its own test above.
        for name in THEMES.iter().filter(|name| **name != "noob-matrix") {
            let preset = colours(&theme(name).expect(name));
            assert_eq!(preset.len(), base.len());
            for ((key, mine), (was, default)) in preset.iter().zip(&base) {
                assert_eq!(key, was, "the two lists are the same fields in order");
                if SHARED.contains(key) {
                    continue;
                }
                assert_ne!(
                    mine, default,
                    "{name}: {key} is still the default's, so the theme arm does not set it"
                );
            }
        }
    }

    /// The whole point of a preset: one word changes every color, and the one
    /// color you also wrote down is still yours.
    #[test]
    fn a_theme_sets_the_palette_and_an_explicit_key_still_wins() {
        let red = Config::parse("theme = noob-red");
        assert!(red.unknown.is_empty(), "{:?}", red.unknown);
        assert_eq!(red, theme("noob-red").unwrap());
        assert_ne!(red.accent, Config::default().accent);
        assert_ne!(red.syntax_keyword, Config::default().syntax_keyword);

        // Either order, because the theme is resolved before the file is read.
        for text in [
            "theme = noob-red\naccent = #ff0000",
            "accent = #ff0000\ntheme = noob-red",
        ] {
            let config = Config::parse(text);
            assert_eq!(config.accent, [0xff, 0x00, 0x00], "{text}");
            assert_eq!(config.text, theme("noob-red").unwrap().text, "{text}");
        }
    }

    /// `theme = noob-matrix` is what the shipped file says, so it has to be
    /// exactly the defaults or a fresh install is not the design.
    #[test]
    fn the_matrix_theme_is_the_default_and_the_others_are_not() {
        assert_eq!(theme("noob-matrix"), Some(Config::default()));
        for name in THEMES {
            let preset = theme(name).expect(name);
            assert!(preset.unknown.is_empty(), "{name}");
            assert_eq!(preset.tools[12], preset.bright, "{name}: plan is prose");
            assert_eq!(preset.tools[13], preset.text, "{name}: the catch-all is prose");
            // The gauges belong to the palette like everything else in it, so
            // only the green window draws them in the green window's ten.
            if name == "noob-matrix" {
                assert_eq!(preset.gauges, GAUGES, "{name}");
            } else {
                assert_ne!(preset.gauges, GAUGES, "{name}: the matrix meters");
            }
            // Every preset opens at the opacity the defaults do, since each one
            // is the defaults with the colours changed.
            assert_eq!(preset.opacity, 0.90, "{name}");
            if name != "noob-matrix" {
                assert_ne!(preset, Config::default(), "{name} is the default twice");
            }
        }
        assert_eq!(theme("chartreuse"), None);
        assert_eq!(THEMES.len(), 3, "three standard themes, all of them noob");
    }

    /// A theme name an older build wrote still opens a window. The names went
    /// when the palettes were cut to three; a settings file is read for years,
    /// so each one is read as the closest of the three rather than reported as
    /// a typo and dropped for the defaults.
    #[test]
    fn a_theme_name_an_older_build_wrote_still_loads() {
        for (was, now) in RETIRED_THEMES {
            let config = Config::parse(&format!("theme = {was}"));
            assert!(config.unknown.is_empty(), "{was}: {:?}", config.unknown);
            assert_eq!(config, theme(now).expect(now), "{was} is not {now}");
        }
        // And the panel then names the palette in hand, which is one of three.
        assert_eq!(theme("noob"), Some(Config::default()));
        assert!(!THEMES.contains(&"noob"), "the old name is not offered");
    }

    /// The window opens at 90% solid, in the struct and in the file it ships,
    /// and every preset opens there too since each one is the defaults with the
    /// colours changed.
    ///
    /// It was 88%, which is not a value the panel's own slider can reach: the
    /// opacity track moves in twentieths, so the first press of an arrow key
    /// left 88 behind and nothing could get back to it.
    #[test]
    fn the_window_opens_at_ninety_percent_solid() {
        assert_eq!(Config::default().opacity, 0.90);
        assert_eq!(Config::parse(DEFAULT_FILE).opacity, 0.90);
        assert_eq!(Config::parse("opacity = 90%").opacity, 0.90);
        for name in THEMES {
            assert_eq!(theme(name).expect(name).opacity, 0.90, "{name}");
        }
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
            let dir = std::env::temp_dir().join(format!("no0b-conf-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn conf(&self) -> PathBuf {
            self.0.join("no0b.conf")
        }

        /// The same file under the name it had while the window was CLIppy.
        fn legacy(&self) -> PathBuf {
            self.0.join("clippy.conf")
        }

        fn read(&self) -> String {
            std::fs::read_to_string(self.conf()).unwrap()
        }

        fn leftovers(&self) -> Vec<String> {
            std::fs::read_dir(&self.0)
                .unwrap()
                .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
                .filter(|name| name != "no0b.conf")
                .collect()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The rename must not throw a tuned palette away. Somebody's `clippy.conf`
    /// is their settings whatever the window is called this week, so the first
    /// run under the new name moves the file across and reads it.
    #[test]
    fn settings_written_under_the_old_name_move_to_the_new_one() {
        let scratch = Scratch::new("adopt");
        std::fs::write(scratch.legacy(), "opacity = 40%\ntheme = noob-red\n").unwrap();

        let config = Config::load_from(&scratch.conf());

        assert_eq!(config.opacity, 0.4);
        assert_eq!(config.accent, theme("noob-red").unwrap().accent);
        // The file is under the new name, with what was in it, and the old name
        // is gone rather than left to be read by nothing.
        assert_eq!(scratch.read(), "opacity = 40%\ntheme = noob-red\n");
        assert!(!scratch.legacy().exists(), "the old file is still there");
    }

    /// Both names present is not a migration. The new file is the one being
    /// edited, so it wins, and the old one is left alone rather than deleted:
    /// this code does not get to decide that a file it did not write is rubbish.
    #[test]
    fn the_current_file_wins_over_a_leftover_old_one() {
        let scratch = Scratch::new("both");
        std::fs::write(scratch.conf(), "opacity = 30%\n").unwrap();
        std::fs::write(scratch.legacy(), "opacity = 40%\n").unwrap();

        let config = Config::load_from(&scratch.conf());

        assert_eq!(config.opacity, 0.3);
        assert_eq!(
            std::fs::read_to_string(scratch.legacy()).unwrap(),
            "opacity = 40%\n"
        );
    }

    /// With neither file there it is a first run, which writes the documented
    /// defaults. The migration must not have turned that into a no-op.
    #[test]
    fn a_first_run_with_no_file_either_way_writes_the_defaults() {
        let scratch = Scratch::new("fresh");
        let config = Config::load_from(&scratch.conf());
        assert_eq!(config.opacity, Config::default().opacity);
        assert_eq!(scratch.read(), DEFAULT_FILE);
    }

    /// The migration is one function for every file the window keeps, so it
    /// carries any of them and touches nothing it was not asked about.
    #[test]
    fn the_migration_carries_every_file_the_window_keeps() {
        let scratch = Scratch::new("every");
        for (legacy, current) in [
            ("clippy.recent", "no0b.recent"),
            ("clippy.conf", "no0b.conf"),
        ] {
            std::fs::write(scratch.0.join(legacy), format!("from {legacy}\n")).unwrap();
            adopt_legacy(&scratch.0.join(current));
            assert_eq!(
                std::fs::read_to_string(scratch.0.join(current)).unwrap(),
                format!("from {legacy}\n")
            );
            assert!(!scratch.0.join(legacy).exists());
        }
        // A name that is not one of ours is not renamed onto, whatever sits
        // beside it.
        std::fs::write(scratch.0.join("clippy.txt"), "not settings").unwrap();
        adopt_legacy(&scratch.0.join("something.txt"));
        assert!(scratch.0.join("clippy.txt").exists());
        assert!(!scratch.0.join("something.txt").exists());
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
        assert!(!after.contains("opacity = 90%"), "{after}");
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
        write_setting(&scratch.conf(), "theme", Some("noob-cool")).unwrap();
        write_setting(&scratch.conf(), "theme", Some("noob-red")).unwrap();

        let after = scratch.read();
        assert_eq!(after.matches("theme = ").count(), 1, "{after}");
        assert!(after.starts_with("# only a comment\n"), "{after}");
        assert_eq!(Config::parse(&after).accent, theme("noob-red").unwrap().accent);
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
        assert!(write_setting(&conf, "theme", Some("two words")).is_err());
        assert!(write_setting(&conf, "accent", Some("#ff0000\nbad = #fff")).is_err());
        let theme_error = write_setting(&conf, "theme", Some("tangerine")).unwrap_err();
        assert!(
            theme_error.contains("noob-matrix, noob-cool, noob-red"),
            "{theme_error}"
        );
        assert!(!conf.exists(), "a refused write created a file");
    }

    /// The reader forgives a retired key; the writer must not write one. A line
    /// nothing reads is worse than an error, because it looks like it worked.
    #[test]
    fn the_writer_refuses_a_retired_key() {
        let scratch = Scratch::new("retired");
        let conf = scratch.conf();
        for key in RETIRED {
            let error = write_setting(&conf, key, Some("true")).unwrap_err();
            assert!(error.contains("no longer a setting"), "{key}: {error}");
            // And unsetting one is not a way in either.
            assert!(write_setting(&conf, key, None).is_err(), "{key} unset");
        }
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
            set_request(&args(&["--set", "theme=noob-red"])),
            Some(Ok(("theme", "noob-red")))
        );
        assert!(set_request(&args(&["--set"])).is_some_and(|r| r.is_err()));
        assert!(set_request(&args(&["--set", "theme"])).is_some_and(|r| r.is_err()));
        assert!(set_request(&args(&["--set", "a=b", "c=d"])).is_some_and(|r| r.is_err()));
    }
}
