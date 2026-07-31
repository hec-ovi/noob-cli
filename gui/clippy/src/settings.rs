//! The settings panel: five sections, what each one carries, and what changing
//! a row writes back.
//!
//! A full screen takeover rather than a popup or a second OS window. A second
//! window means a second wgpu surface with its own renderer and its own event
//! routing, for a list of rows; a popup over the panes means a scroll region
//! floating over another scroll region. While the panel is up it is the whole
//! window, the way the folder picker is before a folder has been chosen.
//!
//! **Sections, not one scroll.** The panel was a single list sixty rows long:
//! four unlabelled groups of settings and then forty six colours, with nothing
//! above it saying where you were and nothing on it about the agent the window
//! is a front end for. It is a rail of section names now, with the chosen
//! section's rows beside it, and each section is short enough to read at a
//! glance. Four of them ([`AGENT`], [`SESSIONS`], [`SKILLS`], [`MCP`]) are the
//! agent's own files rather than the window's; the last one ([`APPEARANCE`]) is
//! the window's own settings file, the palette included, minus the keys the
//! window itself already sets ([`OFF_PANEL`]).
//!
//! **A setting the window already sets is not a row.** Which panes are open and
//! where the dividers sit were a section of their own and then a pair of groups
//! under [`APPEARANCE`], and both are set by using the window: a closed widget
//! comes back off the right click menu, and a divider is dragged and writes its
//! own key on the way up. So the keys stayed and the rows went. Nothing on the
//! panel offers a number for something a pointer already moved.
//!
//! **Two of those sections are two columns.** [`SKILLS`] and [`MCP`] were lists
//! of text nothing could be done to. They are [`Row::Entry`] rows now: a name
//! with what is under it, a toggle that really turns the thing off, and an
//! uninstall beside it. Beside the list is whatever the row under the cursor
//! is, a skill's own `SKILL.md` or a server's entry out of its file.
//!
//! **Both lists uninstall.** A server had the toggle and the column and no way
//! to take it off the machine, which left the one verb the skills have and the
//! servers did not as the one anybody would want: a server nobody wants is
//! lines in a file, and the window that lists it should be able to take them
//! out. It is the same two-press button, and it deletes something the window
//! cannot write back, so [`agent::remove_server`] refuses the whole operation
//! rather than write a file it could not fully build.
//!
//! **Off is a place on the disk, never a flag.** There is no enabled or
//! disabled anywhere in the CLI: a skill is on when its directory is in one of
//! the four places the agent looks, and a server is on when its entry is under
//! `servers`. So a toggle moves it, to the `.off` sibling of the skills
//! directory or to a key beside `servers` in the same file, neither of which
//! the agent reads. Nothing here remembers which is which: what a row says is
//! read off the disk every time the rows are built, so the panel and the next
//! session cannot disagree. [`Deed`] is what a press asks for and `main` is
//! what does it, the same way a [`Change`] is written there and not here.
//!
//! **The agent's section is a form, and it shows what the agent is really
//! told.** Four things anybody opens it for were seven rows apart, with a
//! heading and three notes standing between them: a section more than half
//! prose. They are two rows of two now ([`Row::Pair`]), the endpoint and the
//! file the CLI reads down one column and the two numbers that decide what the
//! agent gets down the other, with Tab crossing between them because the arrow
//! keys are the nudge. Under the form are two blocks ([`Row::Paper`]): the
//! global `AGENTS.md`, which the CLI already reads and puts at the top of every
//! prompt, and the whole assembled prompt out of `noob debug prompt`. The file
//! is one capped layer of that prompt, so it is named as the file and never as
//! the prompt, and the block that has neither says which of the two it is
//! waiting on.
//!
//! **Two of the agent's own settings are controls, not readings.** Everything
//! in the CLI's `.env` was listed as text with the endpoint as the only thing
//! anybody could change, which left the two numbers that decide what the agent
//! actually gets, its context window and how many sub-agent tasks it runs at
//! once, as lines to read and edit somewhere else. They are tracks on the AGENT
//! section now, held to the CLI's own bounds ([`crate::agent`] carries them), so
//! the right end of the concurrency track is the maximum the agent will honour.
//! A [`Change`] says which [`File`] it belongs to and `main` writes it there.
//!
//! Nothing here draws and only [`commit`] and [`write_endpoint`] touch a disk.
//! [`crate::view`] turns these rows into rectangles and `main` routes keys and
//! clicks at them, so the whole model can be driven in a test with no window.
//!
//! **The panel never shows a value the file does not carry.** A change is not
//! applied here: [`Settings::change`] says what the file should say, `main`
//! writes it through the settings writer (which keeps every comment) and reads
//! the whole file back, and [`Settings::refresh`] rebuilds these rows from that.
//! So a value the parser clamps, or spells differently than it was typed, is the
//! value on the panel a frame later instead of the panel and the next launch
//! quietly disagreeing.
//!
//! **A slider moves the window while it is being dragged.** It used to hold its
//! value until the button came up, which read as a control that did nothing:
//! you drag the opacity and the window sits there until you let go. So the file
//! is still written once, on the way up, and the value the drag is holding is
//! applied to the live config on every motion event through [`Config::apply`],
//! which is the same setter and the same clamps the file is read with. The panel
//! itself still shows the file's value under a preview, so what a row says and
//! what the file carries cannot drift.
//!
//! The colours are listed and not editable here. Changing one means typing a hex
//! value into a field, and the one field this window has is the agent's
//! endpoint; thirty seven of them is a form. So the palette is on the panel as
//! swatches you can read, with the path of the file to edit beside them, and the
//! four presets are one row away in `theme`.
//!
//! **The palette is a grid, not a list.** Thirty seven colours one to a row was
//! a column of hex strings four screens long, and a hex string does not say what
//! it colours. They are [`Row::Swatches`] rows now: several to a row, each one a
//! block of the colour with a plain-words label beside it, grouped under the
//! headings they belong to. Pressing one says which key in the file writes it,
//! which is the only thing a hex string was there for.

use std::path::{Path, PathBuf};

use crate::agent::{self, Agent};
use crate::config::{self, Config};

/// The sections, in the order the rail lists them: the agent first, because
/// what the window is a front end for matters more than what colour it is.
pub const AGENT: &str = "AGENT";
pub const SESSIONS: &str = "SESSIONS";
pub const SKILLS: &str = "SKILLS";
pub const MCP: &str = "MCP";
pub const APPEARANCE: &str = "APPEARANCE";

/// Every section name, in rail order.
///
/// There were three more. PANES listed which views were open and where the
/// dividers sat, both of which are already set by doing the thing itself, so the
/// section went and its rows went with it (see [`OFF_PANEL`]). ALL TIME read the
/// counts of every session that ever ran, which answered a question nobody was
/// asking on a settings panel, and went with the file behind it. COLOURS was the
/// palette, which is what the window looks like: it is the last block of
/// [`APPEARANCE`] now, under its own headings.
pub const SECTIONS: [&str; 5] = [AGENT, SESSIONS, SKILLS, MCP, APPEARANCE];

/// What a setting holds, which is what decides how its row changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// One name out of a list, wrapping in both directions.
    Choice(&'static [&'static str]),
    /// A number, nudged by `step` and held between `low` and `high`.
    ///
    /// Drawn as a slider, because a number with a range is a position on a
    /// track: opacity nudged one key at a time is nineteen presses from end to
    /// end. The bounds are the parser's own (`Config::parse` clamps every one of
    /// these), so the panel cannot offer a value the file would silently pull
    /// back. `a_number_reads_back_as_what_the_panel_showed` walks each of them
    /// to both ends through the real file and fails if the two ever drift.
    Number {
        step: f32,
        low: f32,
        high: f32,
        /// Decimals to write. Zero, so `font_size` is `14` rather than `14.0`.
        places: usize,
    },
}

impl Kind {
    /// Where along a track a value sits, 0 at the low end and 1 at the high.
    /// Nothing for a kind that has no range, which is what stops anything else
    /// being drawn as a slider.
    pub fn fraction(self, value: f32) -> Option<f32> {
        let Kind::Number { low, high, .. } = self else {
            return None;
        };
        if high <= low {
            return None;
        }
        Some(((value - low) / (high - low)).clamp(0.0, 1.0))
    }

    /// The value a position along the track means, snapped to the step and
    /// spelled the way the file spells it.
    ///
    /// Snapped rather than free, so a dragged slider writes one of the values
    /// the arrow keys reach and the two cannot disagree about what opacity is.
    /// `the_slider_maps_a_position_to_a_value_and_back` walks the whole track
    /// and fails if a position and its value ever drift by more than the one
    /// step that snapping costs.
    pub fn at(self, fraction: f32) -> Option<String> {
        let Kind::Number {
            step,
            low,
            high,
            places,
        } = self
        else {
            return None;
        };
        if high <= low || step <= 0.0 {
            return None;
        }
        let raw = low + fraction.clamp(0.0, 1.0) * (high - low);
        let value = (low + ((raw - low) / step).round() * step).clamp(low, high);
        Some(format!("{value:.places$}"))
    }
}

/// One row of a section.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A group's name, inside a section that has more than one.
    Heading(&'static str),
    /// Prose: what a section is, or why it is empty. `bad` when it is something
    /// wrong rather than something explained.
    Note { text: String, bad: bool },
    /// One entry of a list read off the disk: a saved session, an installed
    /// skill, a configured server. Its own row rather than a label and a value,
    /// because what identifies one is a sentence and not a number.
    Item(String),
    /// Something read out rather than set: what the agent's own file says, and
    /// where the files behind all this live.
    Reading { label: String, value: String },
    /// A setting spelled the way the file that carries it spells it. Most of
    /// them are the window's own; [`File::Agent`] marks the few that are the
    /// CLI's, which are nudged exactly the same way and land in the other file.
    Setting {
        key: &'static str,
        value: String,
        kind: Kind,
        file: File,
    },
    /// A line of text in the agent's file, edited by typing. The endpoint, and
    /// nothing else: it is the one setting here whose value is not a number, a
    /// flag or a name from a list.
    Field { key: &'static str, value: String },
    /// One row of the palette grid: several colours side by side, each one a
    /// block of the colour and a plain-words label. Up to [`SWATCH_COLUMNS`] of
    /// them, so the row is always as wide as every other row and the list is
    /// still one panel per row index.
    Swatches(Vec<Swatch>),
    /// One installed skill or one configured server: two lines of text, a
    /// toggle that really turns it off, and an uninstall beside it. The row the
    /// column on the right belongs to.
    Entry(Entry),
    /// Two rows side by side, each in half the width: a form rather than a
    /// column of far apart lines.
    ///
    /// The AGENT section was one thing per row with prose between them, so the
    /// four things anybody opens it to set were seven rows apart. They are two
    /// rows of two now, which is what a form is. Never nested: a half is one of
    /// the plain rows above, and [`cell`] is what reads one out.
    Pair(Box<Row>, Box<Row>),
    /// A block of text under a title of its own: the agent's own instructions
    /// file, and the prompt it is a layer of. The one row that is more than a
    /// line or two of text.
    Paper(Paper),
}

/// A block of text on the panel, with a title over it.
///
/// Its own row rather than one row per line: the assembled prompt is thousands
/// of lines, and a section that carried them would be a text file with four
/// settings buried at the top of it. The block is a fixed [`PAPER_LINES`] tall
/// and scrolls inside itself, so the rows under it stay where they are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paper {
    /// What this is, on the first line.
    pub title: String,
    /// The line under it: where the text came from, or why there is none.
    pub under: String,
    /// The text itself, one entry per line, already capped by whoever read it.
    pub body: Vec<String>,
    /// Which line of `body` the block starts on.
    pub first: usize,
    /// The file a press would write, when there is nothing to show and writing
    /// one is the thing to do about it.
    pub offer: Option<PathBuf>,
    /// Whether `under` is something wrong rather than something explained.
    pub bad: bool,
}

impl Paper {
    /// The furthest down it can be scrolled and still be full.
    fn most(&self) -> usize {
        self.body.len().saturating_sub(PAPER_LINES)
    }
}

/// How many lines of a [`Paper`] are on screen at once.
///
/// A number rather than what fits, because a row's height cannot depend on the
/// width or the height of the window: [`lines`] is what the scroll window counts
/// in and what the layout places with, and a block that grew when the window did
/// would put every click under it on another row.
pub const PAPER_LINES: usize = 12;

/// Which half of a [`Row::Pair`] something is in. Left for every row that is not
/// one, so a press on an ordinary row is still a press on the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// One half of a row, or the row itself when it is not a pair.
///
/// Asked by the model, the layout and the drawing, so what a key changes, what a
/// click lands on and what is drawn are the same thing.
pub fn cell(row: &Row, side: Side) -> &Row {
    match (row, side) {
        (Row::Pair(left, _), Side::Left) => left,
        (Row::Pair(_, right), Side::Right) => right,
        (row, _) => row,
    }
}

/// One thing off the agent's disk that can be turned on and off.
///
/// Everything on it is read off the disk as the rows are built. There is no
/// remembered flag anywhere: a skill is on when its directory is in the one
/// place the agent looks and a server is on when its entry is under `servers`,
/// so what the toggle shows is what the next session will do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// The line under the name: the repository a skill records, or the
    /// directory it was found in when it records none; the address or the
    /// command line for a server, and which file it came from.
    pub under: String,
    pub on: bool,
    /// What turning it on and off means on the disk.
    pub what: Which,
    /// Whether there is an uninstall beside the toggle.
    pub removable: bool,
    /// What the column beside the list shows while this is the entry the cursor
    /// is on: a skill's own `SKILL.md`, or a server's entry out of its file.
    pub doc: Vec<String>,
}

/// Which of the two kinds of entry it is, and what naming it on the disk takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Which {
    /// A skill, named by the directory it lives in.
    Skill { dir: String },
    /// A server, named by [`Entry::name`] inside one of the two `mcp.json`
    /// files. Which file is the whole of what `project` says.
    Server { project: bool },
}

/// What a press on an entry's toggle or its uninstall asks the disk to do.
///
/// The panel decides what should happen and `main` is what makes it happen, the
/// same way a [`Change`] is written by `main` rather than here: nothing in this
/// file touches a disk except [`commit`] and [`write_endpoint`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Deed {
    /// Move a skill's directory between the skills directory and the sibling
    /// beside it. `on` is the state it should end in.
    TurnSkill { dir: String, on: bool },
    /// Delete a skill's directory. `on` says which of the two it is in now.
    RemoveSkill { dir: String, on: bool },
    /// Take a server's entry out of its file for good. No `on`: the entry goes
    /// out of both objects, so which one it happened to sit in when the row was
    /// built cannot leave half of it behind.
    RemoveServer { name: String, project: bool },
    /// Move a server's entry between the two objects in its own file. `on` is
    /// the state it should end in.
    TurnServer {
        name: String,
        project: bool,
        on: bool,
    },
    /// Write a starter `AGENTS.md` where the agent looks for one. Only ever
    /// asked for by a block that found nothing there.
    StartInstructions { path: PathBuf },
}

/// One colour on the grid: the key the file writes it under, what it actually
/// colours said in words, and the colour itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swatch {
    pub key: &'static str,
    pub about: &'static str,
    pub rgb: [u8; 3],
}

/// How many colours share a row.
///
/// Three rather than as many as fit: the layout would have to hand the model a
/// width for that, and a grid that reflows as the window is resized is a grid
/// whose rows change height while it is being read. Three holds the longest
/// label there is at the width the list has on a window this runs on, and a
/// narrow window clips a label rather than moving it.
pub const SWATCH_COLUMNS: usize = 3;

/// How many rows of text one row of the panel takes.
///
/// A heading is two, because it is drawn larger than the settings under it: a
/// heading measured at one height and drawn at another puts every click below it
/// on the wrong row. [`Settings::heights`] and `view::place_settings` both read
/// this, which is what keeps the two agreeing.
pub fn lines(row: &Row) -> usize {
    match row {
        // A heading is drawn larger; an entry is a name with what it is
        // underneath, which is two lines of the ordinary text.
        Row::Heading(_) | Row::Entry(_) => 2,
        // As tall as the taller half, so the two columns of a form sit on the
        // same lines and the rows under them do not move when one half changes.
        Row::Pair(left, right) => lines(left).max(lines(right)),
        // Its title, the line under it, and the text.
        Row::Paper(_) => PAPER_LINES + 2,
        _ => 1,
    }
}

/// Which file a setting lives in.
///
/// The panel writes two: the window's own settings file, and the `.env` the CLI
/// reads. They are written by different writers with different rules, so a
/// change carries the answer with it rather than having `main` guess it back out
/// of a key name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum File {
    /// The window's own settings file.
    Window,
    /// The agent's `.env`, which the CLI re-reads on every request.
    Agent,
}

/// What a nudge on the row under the cursor should write, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub key: &'static str,
    pub value: String,
    pub file: File,
}

/// Which half of the panel the keyboard is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The rail: up and down choose a section, right goes into it.
    Rail,
    /// The chosen section's rows.
    Content,
}

/// What the `theme` row says when the palette in the file is not one of the
/// presets. Not a value that can be written: the writer refuses it, and the row
/// only ever hands it back to itself.
const CUSTOM: &str = "custom";

/// What a field with nothing in it reads as. Drawn instead of an empty row,
/// which reads as a value that failed to load rather than one nobody set.
pub const UNSET: &str = "not set";

/// The settings of the window's own file that two arrow keys can cover, grouped
/// the way their sections list them.
///
/// Every other key in that file is a colour and is listed from the live
/// [`Config`] instead, so a colour added to the palette appears here with no
/// edit at all. `every_key_in_the_file_is_on_the_panel` fails if a key ends up
/// in neither list, which is what stops a setting being added to the file and
/// forgotten here.
const LOOKS: [(&str, Kind); 5] = [
    ("theme", Kind::Choice(&config::THEMES)),
    (
        "opacity",
        Kind::Number {
            step: 0.05,
            low: 0.05,
            high: 1.0,
            places: 2,
        },
    ),
    (
        "font_size",
        Kind::Number {
            step: 1.0,
            low: 8.0,
            high: 40.0,
            places: 0,
        },
    ),
    (
        "pane_font_size",
        Kind::Number {
            step: 1.0,
            low: 8.0,
            high: 40.0,
            places: 0,
        },
    ),
    (
        "prompt_rows",
        Kind::Number {
            step: 1.0,
            low: 1.0,
            high: 24.0,
            places: 0,
        },
    ),
];

/// Keys the window's file carries that this panel deliberately does not list.
///
/// Both groups came off the panel with PANES. Which panes are open is the right
/// click menu's job: it names every widget and reopens a closed one, which is
/// where anyone already goes to get a pane back, and two rows saying the same
/// thing in a form is the worse of the two. Where the dividers sit is set by
/// dragging the lines, and the drag writes these keys itself, so the rows were
/// a number to type for something a pointer already does.
///
/// The keys are alive: the parser reads them, a drag writes them, and a layout
/// left somewhere survives a restart. Only the rows are gone.
/// `the_panes_and_the_dividers_are_off_the_panel` fails if one comes back.
pub const OFF_PANEL: [&str; 7] = [
    "show_activity",
    "show_files",
    "left_width",
    "left_width_bottom",
    "top_height",
    "top_height_right",
    "settings_rail",
];

/// The two settings of the agent's own file that are numbers with a range, so
/// the panel can offer them as tracks instead of asking for a number to be
/// typed: the context window the CLI budgets against, and how many sub-agent
/// tasks it runs at once.
///
/// The bounds are the CLI's own ([`crate::agent`] reads them off it), so the
/// right end of the concurrency track is the maximum the agent will honour and
/// there is nothing to guess. Every other key in that file is listed as a
/// reading, because the window does not know what the CLI would accept for it.
const AGENT_SETTINGS: [(&str, Kind); 2] = [
    (
        agent::CTX,
        Kind::Number {
            step: agent::CTX_STEP,
            low: agent::CTX_LOW,
            high: agent::CTX_HIGH,
            places: 0,
        },
    ),
    (
        agent::TASK_CONCURRENCY,
        Kind::Number {
            step: agent::TASK_CONCURRENCY_STEP,
            low: agent::TASK_CONCURRENCY_LOW,
            high: agent::TASK_CONCURRENCY_HIGH,
            places: 0,
        },
    ),
];

/// Every colour in the file, in the order the panel lists them: the tones the
/// whole window is drawn from, then the five the highlighter uses, then one per
/// tool and one per gauge slot.
///
/// Read off a [`Config`] rather than written out, so this is also what names the
/// theme: a file whose colours are a preset's colours is that preset, whether it
/// says so or not (see [`theme_name`]).
pub fn colours(config: &Config) -> Vec<(&'static str, [u8; 3])> {
    let mut out = vec![
        ("accent", config.accent),
        ("text", config.text),
        ("dim", config.dim),
        ("bright", config.bright),
        ("good", config.good),
        ("bad", config.bad),
        ("panel", config.panel),
        ("bar", config.bar),
        ("syntax_comment", config.syntax_comment),
        ("syntax_string", config.syntax_string),
        ("syntax_number", config.syntax_number),
        ("syntax_keyword", config.syntax_keyword),
        ("syntax_markup", config.syntax_markup),
    ];
    out.extend(config::TOOL_KEYS.into_iter().zip(config.tools));
    out.extend(config::GAUGE_KEYS.into_iter().zip(config.gauges));
    out
}

/// How many of [`colours`] are the window's own tones, and how many of the rest
/// belong to the highlighter. The tools and the gauges are the two lists after
/// them, and both name their own keys.
const WINDOW_TONES: usize = 8;
const SYNTAX_TONES: usize = 5;

/// What a colour actually colours, in words.
///
/// The key is what the file wants and says nothing on its own: `bar`, `dim` and
/// `gauge_7` are three things nobody can pick out of a palette. The grid is
/// labelled with these and says the key only for the swatch that was pressed,
/// which is when the key is the thing being asked for.
///
/// Every key in [`colours`] is answered here.
/// `every_colour_says_what_it_colours` fails on one that is not.
fn about(key: &str) -> &'static str {
    match key {
        "accent" => "the accent",
        "text" => "ordinary text",
        "dim" => "quiet text",
        "bright" => "loud text",
        "good" => "it worked",
        "bad" => "it failed",
        "panel" => "behind everything",
        "bar" => "the title bar",
        "syntax_comment" => "comments",
        "syntax_string" => "strings",
        "syntax_number" => "numbers",
        "syntax_keyword" => "keywords",
        "syntax_markup" => "markup",
        "tool_bash" => "running a command",
        "tool_read" => "reading a file",
        "tool_ls" => "listing a folder",
        "tool_glob" => "finding files",
        "tool_grep" => "searching in files",
        "tool_context" => "reading the context",
        "tool_write" => "writing a file",
        "tool_edit" => "editing a file",
        "tool_web" => "searching the web",
        "tool_skill" => "running a skill",
        "tool_mcp" => "an mcp server",
        "tool_agent" => "a subagent",
        "tool_plan" => "planning",
        "tool_other" => "anything else",
        "gauge_1" => "reading 1",
        "gauge_2" => "reading 2",
        "gauge_3" => "reading 3",
        "gauge_4" => "reading 4",
        "gauge_5" => "reading 5",
        "gauge_6" => "reading 6",
        "gauge_7" => "reading 7",
        "gauge_8" => "reading 8",
        "gauge_9" => "reading 9",
        "gauge_10" => "reading 10",
        // A colour added to the file and not to this list. Said rather than
        // left blank, and `every_colour_says_what_it_colours` fails on it so it
        // is not a label anybody sees.
        _ => "a colour of its own",
    }
}

/// Which preset the file is carrying, or [`CUSTOM`] when it is carrying a
/// palette nobody named.
///
/// The `theme` key is resolved into colours as the file is read and is not kept
/// anywhere afterwards, so the only honest way to fill this row in is to ask
/// which preset the colours in hand match. A file that sets `theme = ice` and
/// then one colour of its own is custom, which is exactly what it is.
fn theme_name(config: &Config) -> &'static str {
    let mine = colours(config);
    config::THEMES
        .into_iter()
        .find(|name| config::theme(name).is_some_and(|preset| colours(&preset) == mine))
        .unwrap_or(CUSTOM)
}

/// One section: its name on the rail and the rows beside it.
///
/// The cursor and the scroll window belong to the section rather than to the
/// panel, so walking to COLOURS and back does not lose where you were in
/// APPEARANCE.
pub struct Section {
    pub name: &'static str,
    rows: Vec<Row>,
    cursor: usize,
    first: usize,
    /// Where the column beside the list is scrolled to, in lines of that
    /// document. Its own number because the two columns scroll separately: the
    /// wheel over a skill's own text must not walk the list of skills.
    doc_first: usize,
    /// Which half of a [`Row::Pair`] the keyboard is in. Kept while the cursor
    /// walks rows, so going down a form column stays in that column.
    side: Side,
}

impl Section {
    fn new(name: &'static str, rows: Vec<Row>) -> Section {
        let cursor = rows.iter().position(landable).unwrap_or(0);
        Section {
            name,
            rows,
            cursor,
            first: 0,
            doc_first: 0,
            side: Side::Left,
        }
    }
}

/// Where a section was left, so a rebuild does not throw it away.
struct Place {
    cursor: usize,
    first: usize,
    doc_first: usize,
    side: Side,
    /// Where each block of text was scrolled to, by the row it was on.
    papers: Vec<(usize, usize)>,
}

/// What `noob debug prompt` answered, which is the only place the whole
/// assembled prompt exists.
///
/// Not a file and not a frame: the protocol carries no prompt, and `AGENTS.md`
/// is one capped layer of one. The window runs the CLI's own subcommand off the
/// interface thread and this is what comes back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Assembled {
    /// The command has been started and has not answered yet.
    Waiting,
    /// It printed a prompt, read in the folder named.
    Got { at: String, body: Vec<String> },
    /// It failed, and why.
    Failed { at: String, why: String },
}

pub struct Settings {
    sections: Vec<Section>,
    /// Which section the rail is on, which is the one whose rows are beside it.
    chosen: usize,
    focus: Focus,
    /// What has been typed into the field being edited, or nothing when no field
    /// is being edited. The row itself keeps saying what the file says until the
    /// edit lands.
    editing: Option<String>,
    /// The row and half a slider is being dragged on, and the value it is being
    /// dragged to. Nothing is written until the button comes up.
    dragging: Option<(usize, Side, String)>,
    /// The swatch that was last pressed, as the row it is on and the cell along
    /// it. Nothing is changed by it: the footer says which key in the file
    /// writes that colour, which is what a grid of blocks cannot say on its own.
    picked: Option<(usize, usize)>,
    /// The entry row whose uninstall has been pressed once.
    ///
    /// The one thing on this panel that cannot be undone, so it takes two
    /// presses: the first says what is about to be deleted on the footer and
    /// the second deletes it. Anything else at all, a key, another row, a
    /// refresh, puts it back to nothing, so an armed button cannot be left
    /// sitting there for the next pointer that goes past.
    arming: Option<usize>,
    /// The settings file, or nothing when there is no home directory to put one
    /// in. Kept so [`Settings::refresh`] does not have to be handed it again.
    file: Option<PathBuf>,
    /// What the agent's own files said when the panel opened.
    agent: Agent,
    /// Why the last change did not land. Cleared by the next refresh, since a
    /// refresh only happens after a write that worked.
    trouble: Option<String>,
    /// The whole prompt the agent is given, once the CLI has printed it.
    prompt: Assembled,
}

impl Settings {
    /// Open the panel over the settings as they are now.
    ///
    /// `agent` is a snapshot of the CLI's own files, read once here rather than
    /// on every frame.
    pub fn open(config: &Config, file: Option<&Path>, agent: Agent) -> Settings {
        let mut panel = Settings {
            sections: Vec::new(),
            chosen: 0,
            focus: Focus::Rail,
            editing: None,
            dragging: None,
            picked: None,
            arming: None,
            file: file.map(PathBuf::from),
            agent,
            trouble: None,
            prompt: Assembled::Waiting,
        };
        panel.sections = panel.build(config);
        panel
    }

    /// Take what `noob debug prompt` answered, from the thread that ran it.
    ///
    /// The rows are rebuilt rather than patched, the same as every other thing
    /// that arrives: the block says what the command said, and nothing about it
    /// is assembled here.
    pub fn adopt_prompt(
        &mut self,
        at: String,
        answer: Result<Vec<String>, String>,
        config: &Config,
    ) {
        self.prompt = match answer {
            Ok(body) => Assembled::Got { at, body },
            Err(why) => Assembled::Failed { at, why },
        };
        self.refresh(config);
    }

    /// Rebuild the rows from the files as they now read, keeping the cursor
    /// where it was. Called after a change has been written and read back.
    pub fn refresh(&mut self, config: &Config) {
        let places: Vec<Place> = self
            .sections
            .iter()
            .map(|section| Place {
                cursor: section.cursor,
                first: section.first,
                doc_first: section.doc_first,
                side: section.side,
                papers: section
                    .rows
                    .iter()
                    .enumerate()
                    .filter_map(|(at, row)| match row {
                        Row::Paper(paper) => Some((at, paper.first)),
                        _ => None,
                    })
                    .collect(),
            })
            .collect();
        self.sections = self.build(config);
        self.trouble = None;
        self.dragging = None;
        self.picked = None;
        self.arming = None;
        for (section, place) in self.sections.iter_mut().zip(places) {
            let Place {
                cursor,
                first,
                doc_first,
                side,
                papers,
            } = place;
            let last = section.rows.len().saturating_sub(1);
            section.first = first.min(last);
            section.cursor = cursor.min(last);
            section.side = side;
            // Where each block of text was left, so a write somewhere else on
            // the panel does not scroll the prompt back to its first line under
            // whoever was reading it.
            for (at, was) in papers {
                if let Some(Row::Paper(paper)) = section.rows.get_mut(at) {
                    paper.first = was.min(paper.most());
                }
            }
            // Clamped where it is read, since the document that comes back may
            // be a different length or another entry's altogether.
            section.doc_first = doc_first;
            if !section.rows.get(section.cursor).is_some_and(landable) {
                section.cursor = landing_from(&section.rows, section.cursor, true)
                    .or_else(|| landing_from(&section.rows, section.cursor, false))
                    .unwrap_or(0);
            }
        }
    }

    /// Take a fresh reading of the agent's files, after the endpoint was
    /// written. Same rule as [`Settings::refresh`]: what the panel shows comes
    /// back off the disk rather than out of what was typed.
    pub fn adopt_agent(&mut self, agent: Agent, config: &Config) {
        self.agent = agent;
        self.refresh(config);
    }

    /// One section per name on the rail, in rail order. Driven off [`SECTIONS`]
    /// so the rail and what is behind it cannot come apart: a name with no rows
    /// would be a section that opens on nothing.
    fn build(&self, config: &Config) -> Vec<Section> {
        SECTIONS
            .into_iter()
            .map(|name| {
                let rows = match name {
                    AGENT => self.agent_rows(),
                    SESSIONS => self.session_rows(),
                    SKILLS => self.skill_rows(),
                    MCP => self.mcp_rows(),
                    APPEARANCE => self.appearance_rows(config),
                    // A name on the rail with no builder behind it opens on
                    // nothing, which `every_section_is_reachable` fails on. The
                    // arm this replaced was a catch-all that built the ALL TIME
                    // rows, so a name added and forgotten got that section's
                    // contents under its own heading and nothing said so.
                    _ => Vec::new(),
                };
                Section::new(name, rows)
            })
            .collect()
    }

    /// What the agent is pointed at, out of the file the CLI owns.
    ///
    /// A form, not a column. The four things anybody opens this section to look
    /// at were seven rows apart with a heading and three notes between them,
    /// which is a section more than half prose. They are two rows of two now:
    /// where the model is and which file says so on the left, how much the agent
    /// gets on the right. What the notes were saying is either on the row itself
    /// or gone, and the two blocks under the form are the instructions the agent
    /// really receives.
    fn agent_rows(&self) -> Vec<Row> {
        let mut unset = Vec::new();
        let mut numbers = Vec::new();
        for (key, kind) in AGENT_SETTINGS {
            numbers.push(Row::Setting {
                key,
                value: match self.agent.setting(key) {
                    Some(value) => value.to_string(),
                    None => {
                        unset.push(key);
                        agent_default(key)
                    }
                },
                kind,
                file: File::Agent,
            });
        }
        let tasks = numbers.pop().expect("both of the agent's numbers");
        let ctx = numbers.pop().expect("both of the agent's numbers");
        let mut rows = vec![
            Row::Pair(
                Box::new(Row::Field {
                    key: agent::ENDPOINT,
                    value: self.agent.endpoint().unwrap_or_default().to_string(),
                }),
                Box::new(ctx),
            ),
            Row::Pair(
                Box::new(Row::Reading {
                    label: String::from("main file"),
                    value: match (&self.agent.env_path, self.agent.env_exists) {
                        // Not there yet is worth saying: an agent configured
                        // entirely by environment has no file at all, and the
                        // first save writes one.
                        (Some(path), false) => format!("{} (not there yet)", path.display()),
                        (Some(path), true) => path.display().to_string(),
                        (None, _) => String::from("nowhere: no config directory to read one in"),
                    },
                }),
                Box::new(tasks),
            ),
        ];
        if self.agent.endpoint().is_none() {
            rows.push(note(
                "with no endpoint set, noob probes the usual local ports and takes the first that answers",
            ));
        }
        if !unset.is_empty() {
            rows.push(note(&format!(
                "not in the file yet: {}. Until then those rows read what the CLI falls back to, and nudging one writes the line",
                unset.join(" and ")
            )));
        }
        rows.push(Row::Paper(self.instructions_paper()));
        rows.push(Row::Paper(self.prompt_paper()));
        for (key, value) in &self.agent.env {
            if key == agent::ENDPOINT || agent::OWNED.contains(&key.as_str()) {
                continue;
            }
            rows.push(Row::Reading {
                label: key.clone(),
                // A credential is reported as set and never as itself. The CLI
                // keeps secrets out of settable config on purpose, and a window
                // is a worse place for one than a terminal: it is on a screen
                // somebody else can be standing behind.
                value: match agent::is_secret(key) {
                    true => String::from("set, and not shown here"),
                    false => value.clone(),
                },
            });
        }
        rows
    }

    /// The agent's global instructions, under a title naming the file they are
    /// in.
    ///
    /// The answer to "is the global AGENTS.md a thing": it already is, and this
    /// is it. The CLI reads `<config dir>/AGENTS.md` and puts it at the top of
    /// every prompt, before the project's own. Nothing was built to make that
    /// true; the window only names the path, shows what is in it, and offers to
    /// write one when there is none.
    ///
    /// Not called the prompt. It is one capped layer of one, which is what the
    /// block under it is for.
    fn instructions_paper(&self) -> Paper {
        let it = &self.agent.instructions;
        let title = String::from("GLOBAL INSTRUCTIONS \u{2022} AGENTS.md");
        let Some(path) = it.path.as_deref() else {
            return Paper {
                title,
                under: String::from("nowhere: no config directory to keep one in"),
                body: Vec::new(),
                first: 0,
                offer: None,
                bad: true,
            };
        };
        // Empty and missing are one thing here because they are one thing to the
        // agent: it trims the file and a blank one contributes no heading at all.
        if it.body.is_empty() {
            return Paper {
                title,
                under: format!("nothing at {} yet", path.display()),
                body: vec![
                    String::from("The agent reads this file first, in every folder, before the"),
                    String::from("project's own AGENTS.md. There is none here yet."),
                ],
                first: 0,
                offer: Some(path.to_path_buf()),
                bad: false,
            };
        }
        let mut body = it.body.clone();
        if it.capped {
            body.push(String::new());
            body.push(format!(
                "[the CLI stops reading at {} KiB, so the rest of this file is not in the prompt]",
                agent::AGENTS_CAP / 1024
            ));
        }
        Paper {
            title,
            // The `.env` is re-read on every request; this is not. The prompt is
            // assembled once when `serve` starts, so an edit here lands on the
            // next session rather than the next message, and a block that did
            // not say so would be the panel telling somebody their change was
            // live when it is not.
            under: format!(
                "{} \u{2022} read when a session starts, so an edit lands on the next one",
                path.display()
            ),
            body,
            first: 0,
            offer: None,
            bad: false,
        }
    }

    /// The whole prompt, exactly as the CLI assembles it.
    ///
    /// `AGENTS.md` is one layer of this: the prompt also carries the CLI's own
    /// base instructions, the environment block, the project's own AGENTS.md,
    /// the skills resolver and the MCP line. Only `noob debug prompt` returns
    /// all of it, so that is what this block shows, and while it is running or
    /// after it has failed the block says which of the two happened.
    fn prompt_paper(&self) -> Paper {
        let title = String::from("THE PROMPT THE AGENT GETS");
        match &self.prompt {
            Assembled::Waiting => Paper {
                title,
                under: String::from("running noob debug prompt\u{2026}"),
                body: Vec::new(),
                first: 0,
                offer: None,
                bad: false,
            },
            Assembled::Got { at, body } => Paper {
                title,
                under: format!("noob debug prompt, run in {at}"),
                body: body.clone(),
                first: 0,
                offer: None,
                bad: false,
            },
            Assembled::Failed { at, why } => Paper {
                title,
                under: format!("{why} (run in {at})"),
                body: Vec::new(),
                first: 0,
                offer: None,
                bad: true,
            },
        }
    }

    /// The conversations the agent has already written, read with the same
    /// reader the folder picker offers them with.
    fn session_rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Reading {
            label: String::from("sessions"),
            value: match crate::sessions::dir() {
                Some(dir) => dir.display().to_string(),
                None => String::from("nowhere: no config directory"),
            },
        }];
        if self.agent.sessions.sessions.is_empty() {
            rows.push(note("no saved sessions yet"));
        }
        for saved in &self.agent.sessions.sessions {
            rows.push(Row::Item(session_line(saved, self.agent.now)));
        }
        for why in &self.agent.sessions.skipped {
            rows.push(Row::Note {
                text: why.clone(),
                bad: true,
            });
        }
        rows
    }

    /// What is installed under the agent's `skills/`, and what has been turned
    /// off into the sibling beside it.
    ///
    /// Two columns: these rows are the left one, and the skill under the cursor
    /// carries its own `SKILL.md` for the right one. Each row is the skill's
    /// name with the repository it records underneath, or, since nothing the
    /// CLI writes records where a skill came from, the directory it was found
    /// in instead.
    fn skill_rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Reading {
            label: String::from("skills"),
            value: match &self.agent.skills_at {
                Some(path) => path.display().to_string(),
                None => String::from("nowhere: no config directory"),
            },
        }];
        if self.agent.skills.is_empty() {
            rows.push(note(
                "none installed: a skill is a directory here with a SKILL.md in it",
            ));
        } else {
            rows.push(note(
                "turning one off moves its directory beside the skills directory, where the agent does not look; uninstall deletes it",
            ));
        }
        for skill in &self.agent.skills {
            rows.push(Row::Entry(Entry {
                name: skill_line(skill),
                under: match &skill.repo {
                    Some(repo) => repo.clone(),
                    // Nothing on disk records the repository of an installed
                    // skill, so where it is is the truthful second line.
                    None => skill.path.display().to_string(),
                },
                on: skill.on,
                what: Which::Skill {
                    dir: skill.dir.clone(),
                },
                removable: true,
                doc: skill.doc.clone(),
            }));
        }
        rows
    }

    /// The MCP servers, out of the two files the CLI merges.
    ///
    /// The same two columns the skills are: a row per server with what it is
    /// underneath, and that server's entry out of its own file beside them.
    fn mcp_rows(&self) -> Vec<Row> {
        let mcp = &self.agent.mcp;
        let mut rows = vec![Row::Reading {
            label: String::from("global"),
            value: match &mcp.global {
                Some(path) => path.display().to_string(),
                None => String::from("nowhere: no config directory"),
            },
        }];
        rows.push(Row::Reading {
            label: String::from("project"),
            value: match &mcp.project {
                Some(path) => path.display().to_string(),
                None => String::from("nowhere until a folder is open"),
            },
        });
        // Said in full rather than shown as an empty list, which reads as a
        // panel that failed to load one.
        if !mcp.any_file {
            rows.push(note(
                "none configured: neither file exists. put a server in either one and the next session loads it",
            ));
        } else if mcp.servers.is_empty() && mcp.trouble.is_empty() {
            rows.push(note("none configured: the files carry no servers"));
        }
        for server in &mcp.servers {
            rows.push(Row::Entry(Entry {
                name: server.name.clone(),
                under: server_line(server),
                on: server.on,
                what: Which::Server {
                    project: server.project,
                },
                // A server is a few lines somebody wrote in a file, and taking
                // those lines out is the one thing this list could not do. It
                // is the same two-press uninstall a skill carries, and it goes
                // through the same rename, so a press that fails leaves the
                // file exactly as it was.
                removable: true,
                doc: server_doc(server, self.mcp_file(server.project)),
            }));
        }
        for why in &mcp.trouble {
            rows.push(Row::Note {
                text: why.clone(),
                bad: true,
            });
        }
        if !mcp.servers.is_empty() {
            rows.push(note(
                "the project file wins for a server named in both; turning one off moves its entry to a key the CLI does not read, and uninstall takes the entry out of the file",
            ));
        }
        rows
    }

    /// Everything about what the window looks like: the sizes, the theme and
    /// the palette.
    ///
    /// COLOURS was a section of its own and is the last block here, under its
    /// own headings, because a palette is what the window looks like. The two
    /// groups PANES held are not here and are not anywhere: see [`OFF_PANEL`]
    /// for which keys those were and what sets them instead.
    fn appearance_rows(&self, config: &Config) -> Vec<Row> {
        let mut rows = settings_rows(config, &LOOKS);
        rows.extend(self.colour_rows(config));
        rows
    }

    /// The palette, as a grid: one heading per group and then that group's
    /// colours [`SWATCH_COLUMNS`] to a row.
    ///
    /// It was one colour per row, thirty seven rows of hex string. That is four
    /// screens of a column half of which is empty, and no row said what it
    /// coloured. Grouped blocks of labelled blocks read as a palette, which is
    /// what this is.
    fn colour_rows(&self, config: &Config) -> Vec<Row> {
        let all = colours(config);
        let mut rows = Vec::new();
        let mut at = 0;
        for (heading, count) in [
            ("THE WINDOW", WINDOW_TONES),
            ("THE HIGHLIGHTER", SYNTAX_TONES),
            ("ONE PER TOOL", config::TOOL_KEYS.len()),
            ("ONE PER GAUGE", config::GAUGE_KEYS.len()),
        ] {
            rows.push(Row::Heading(heading));
            // Chunked inside the group rather than across the whole palette, so
            // a row never carries the end of one group and the start of the
            // next: a grid whose blocks run into each other is the list again.
            for chunk in all[at..at + count].chunks(SWATCH_COLUMNS) {
                rows.push(Row::Swatches(
                    chunk
                        .iter()
                        .map(|(key, rgb)| Swatch {
                            key,
                            about: about(key),
                            rgb: *rgb,
                        })
                        .collect(),
                ));
            }
            at += count;
        }
        rows.push(note(
            "press a colour to see which key writes it: colours are edited in the file, since a hex value needs a keyboard this window has nowhere to put",
        ));
        rows.push(Row::Reading {
            label: String::from("settings"),
            value: match &self.file {
                Some(path) => path.display().to_string(),
                // Not a failure worth refusing to open the panel over: every
                // reading is still true and the presets still apply for as long
                // as the window is up. It is why nothing can be saved.
                None => String::from("nowhere: no home directory to write one in"),
            },
        });
        rows
    }

    /// Every section's name, in rail order.
    pub fn section_names(&self) -> Vec<&'static str> {
        self.sections.iter().map(|section| section.name).collect()
    }

    pub fn chosen(&self) -> usize {
        self.chosen
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// The section the rail is on.
    fn here(&self) -> &Section {
        &self.sections[self.chosen.min(self.sections.len() - 1)]
    }

    fn here_mut(&mut self) -> &mut Section {
        let at = self.chosen.min(self.sections.len() - 1);
        &mut self.sections[at]
    }

    /// Every row of every section, which only the tests want: what is on screen
    /// is one section, and a whole-panel accessor in the window would be a
    /// second way to draw rows nobody clamped.
    /// A form row is its two halves here rather than the pair: what a test asks
    /// about is the setting, and which row of a form it happens to sit on is the
    /// layout's business.
    #[cfg(test)]
    pub fn all_rows(&self) -> impl Iterator<Item = (&'static str, &Row)> {
        self.sections.iter().flat_map(|section| {
            section.rows.iter().flat_map(move |row| match row {
                Row::Pair(left, right) => vec![
                    (section.name, left.as_ref()),
                    (section.name, right.as_ref()),
                ],
                row => vec![(section.name, row)],
            })
        })
    }

    #[cfg(test)]
    pub fn rows(&self) -> &[Row] {
        &self.here().rows
    }

    pub fn row(&self, index: usize) -> Option<&Row> {
        self.here().rows.get(index)
    }

    pub fn cursor(&self) -> usize {
        self.here().cursor
    }

    /// Which half of the row under the cursor the keyboard is in.
    ///
    /// Resolved rather than remembered: the half it was left in when that half
    /// can hold the cursor, the other one when it cannot. So walking down a form
    /// column stays in that column, and a row whose left half is a reading puts
    /// the cursor on the control beside it instead of on nothing.
    pub fn side(&self) -> Side {
        let here = self.here();
        let Some(row) = here.rows.get(here.cursor) else {
            return Side::Left;
        };
        if !matches!(row, Row::Pair(_, _)) {
            return Side::Left;
        }
        match landable(cell(row, here.side)) {
            true => here.side,
            false => here.side.other(),
        }
    }

    /// The half of the row under the cursor the keys act on, which is the row
    /// itself everywhere but in a form.
    pub fn at_cursor(&self) -> Option<&Row> {
        Some(cell(self.row(self.cursor())?, self.side()))
    }

    /// One half of one row, for the layout and the drawing.
    pub fn cell(&self, index: usize, side: Side) -> Option<&Row> {
        Some(cell(self.row(index)?, side))
    }

    /// Move across a form row, which is the one thing on this panel the arrow
    /// keys cannot do: left and right are the nudge. False when the cursor is
    /// not on a form, or when the other half of it is a reading.
    pub fn swap(&mut self) -> bool {
        if self.focus == Focus::Rail || self.editing.is_some() {
            return false;
        }
        let side = self.side();
        let here = self.here();
        let Some(row @ Row::Pair(_, _)) = here.rows.get(here.cursor) else {
            return false;
        };
        if !landable(cell(row, side.other())) {
            return false;
        }
        self.picked = None;
        self.arming = None;
        self.here_mut().side = side.other();
        true
    }

    /// Whether the cursor is on a row at all: a section of readings has nothing
    /// to land on, and a band drawn on a row nothing can be done to is a lie.
    pub fn on_row(&self) -> bool {
        self.focus == Focus::Content && self.row(self.cursor()).is_some_and(landable)
    }

    /// Where the list is scrolled to, as a row of the section rather than a row
    /// of text. Only the tests want it on its own: what the window asks for is
    /// [`Settings::window`], which is that row and everything that fits under it.
    #[cfg(test)]
    pub fn first(&self) -> usize {
        self.here().first
    }

    pub fn trouble(&self) -> Option<&str> {
        self.trouble.as_deref()
    }

    /// Say why a change did not land. Shown on the panel rather than dropped: a
    /// row that stays where it was with nothing said reads as a dead panel.
    pub fn say_trouble(&mut self, why: String) {
        self.trouble = Some(why);
    }

    /// What has been typed into the field being edited.
    pub fn editing(&self) -> Option<&str> {
        self.editing.as_deref()
    }

    /// What a slider is being dragged to, while it is being dragged. The row
    /// still says what the file says, which is what this is drawn instead of.
    pub fn preview(&self, index: usize, side: Side) -> Option<&str> {
        self.dragging
            .as_ref()
            .filter(|(at, half, _)| *at == index && *half == side)
            .map(|(_, _, value)| value.as_str())
    }

    /// The swatch at a place on the grid, or nothing when that row is not a
    /// grid row or has no such cell.
    pub fn swatch(&self, row: usize, cell: usize) -> Option<&Swatch> {
        let Row::Swatches(cells) = self.row(row)? else {
            return None;
        };
        cells.get(cell)
    }

    /// Press one swatch: which colour it is stays on the footer until something
    /// else is pressed.
    ///
    /// Nothing is changed by it. A block of colour cannot say which line of the
    /// file writes it, and that line is the whole of what somebody looking at
    /// the palette wants next.
    pub fn pick(&mut self, row: usize, cell: usize) -> bool {
        if self.swatch(row, cell).is_none() {
            return false;
        }
        let moved = self.picked != Some((row, cell));
        self.picked = Some((row, cell));
        moved
    }

    /// Which swatch is pressed, for the block the grid draws around it.
    pub fn picked(&self) -> Option<(usize, usize)> {
        self.picked
    }

    /// What the footer says about the pressed swatch: the key that writes it and
    /// the value it is carrying.
    fn picked_says(&self) -> Option<String> {
        let (row, cell) = self.picked?;
        let swatch = self.swatch(row, cell)?;
        Some(format!(
            "{}: the settings file writes it as {} = {}",
            swatch.about,
            swatch.key,
            hex(swatch.rgb)
        ))
    }

    /// What the keys under the cursor do, spelled out for the footer. The panel
    /// is the one surface in this window where there is nothing to experiment on
    /// safely, so it says what a key will do before it is pressed.
    ///
    /// A pressed swatch answers instead, because a press that said nothing at
    /// all would read as a grid that does not answer the pointer.
    pub fn says(&self) -> String {
        // An armed uninstall answers before anything else: it is the one press
        // on this panel that cannot be taken back, and what it would take with
        // it is the only thing worth reading at that moment.
        if let Some(Row::Entry(entry)) = self.arming.and_then(|at| self.row(at)) {
            // Named by the thing that is about to go, and said as what would
            // actually happen to it. A skill goes by its directory, since the
            // line under the name is the repository where there is one and
            // "delete https://github.com/..." is not what would happen; a
            // server is an entry in a file and no directory is going anywhere.
            let what = match &entry.what {
                Which::Skill { dir } => format!("delete the {dir} directory"),
                Which::Server { .. } => {
                    format!("take {} out of its mcp.json", entry.name)
                }
            };
            return format!("press uninstall again to {what}; anything else leaves it alone");
        }
        self.picked_says()
            .unwrap_or_else(|| String::from(self.hint()))
    }

    pub fn hint(&self) -> &'static str {
        if self.editing.is_some() {
            return "type it \u{2022} enter saves it \u{2022} esc leaves it alone";
        }
        if self.focus == Focus::Rail {
            return "up and down choose a section \u{2022} right goes in \u{2022} esc closes";
        }
        // On a form row, the one thing the arrow keys cannot say is how to get
        // to the other half of it, because left and right are the nudge.
        let across = matches!(self.row(self.cursor()), Some(Row::Pair(_, _)))
            && self.here().rows.get(self.cursor()).is_some_and(|row| {
                landable(cell(row, Side::Left)) && landable(cell(row, Side::Right))
            });
        match self.at_cursor() {
            Some(Row::Setting { kind, .. }) => match (kind, across) {
                (Kind::Choice(_), _) => "left and right walk the presets",
                (Kind::Number { .. }, false) => "left and right nudge it, or drag the slider",
                (Kind::Number { .. }, true) => {
                    "left and right nudge it \u{2022} tab crosses to the other column"
                }
            },
            Some(Row::Paper(paper)) => match paper.offer.is_some() {
                true => "enter writes a starter AGENTS.md there",
                false => "page up and page down read it \u{2022} up and down leave it",
            },
            Some(Row::Field { .. }) if across => {
                "enter edits it \u{2022} tab crosses to the other column"
            }
            Some(Row::Field { .. }) => "enter edits it \u{2022} left goes back to the sections",
            Some(Row::Entry(entry)) => match (entry.removable, &entry.what) {
                (false, _) => "enter turns it on and off",
                (true, Which::Skill { .. }) => {
                    "enter turns it on and off \u{2022} uninstall deletes its directory"
                }
                (true, Which::Server { .. }) => {
                    "enter turns it on and off in its file \u{2022} uninstall takes it out of that file"
                }
            },
            _ => "up and down move \u{2022} left goes back to the sections \u{2022} esc closes",
        }
    }

    /// Put the rail on one section, which is what a click on it does.
    pub fn choose(&mut self, index: usize) -> bool {
        if index >= self.sections.len() {
            return false;
        }
        let moved = index != self.chosen || self.focus != Focus::Rail;
        self.chosen = index;
        self.focus = Focus::Rail;
        self.editing = None;
        self.dragging = None;
        self.picked = None;
        self.arming = None;
        self.rewind_doc();
        moved
    }

    /// Into the chosen section, which is where the arrow keys act on rows.
    /// False when it is already there.
    pub fn enter(&mut self) -> bool {
        if self.focus == Focus::Content {
            return false;
        }
        self.focus = Focus::Content;
        true
    }

    /// Back out to the rail, which is how the sections are walked again.
    pub fn leave(&mut self) -> bool {
        if self.focus == Focus::Rail {
            return false;
        }
        self.focus = Focus::Rail;
        self.editing = None;
        true
    }

    /// What the row under the cursor becomes when it is nudged, or nothing when
    /// the cursor is on a row that cannot change.
    ///
    /// Takes `&self`: the row is not touched here. What the panel shows comes
    /// back from the file, so a write that fails leaves the row reading what the
    /// file still says instead of the value it was asked for.
    pub fn change(&self, forward: bool) -> Option<Change> {
        if self.focus == Focus::Rail || self.editing.is_some() {
            return None;
        }
        let Row::Setting {
            key,
            value,
            kind,
            file,
        } = self.at_cursor()?
        else {
            return None;
        };
        let next = match kind {
            Kind::Choice(names) => {
                let at = names.iter().position(|name| name == value);
                let next = match (at, forward) {
                    (Some(at), true) => (at + 1) % names.len(),
                    (Some(at), false) => (at + names.len() - 1) % names.len(),
                    // A value that is not in the list is what CUSTOM is: the
                    // first name going forward and the last going back, so one
                    // nudge off a hand-tuned palette lands on a preset either
                    // way instead of doing nothing.
                    (None, true) => 0,
                    (None, false) => names.len() - 1,
                };
                names[next].to_string()
            }
            Kind::Number {
                step,
                low,
                high,
                places,
            } => {
                let now = value.parse::<f32>().unwrap_or(*low);
                let next = (now + if forward { *step } else { -*step }).clamp(*low, *high);
                // Unchanged at the end of its range rather than wrapping: a
                // font size that jumps from 40 to 8 because the key was pressed
                // once more is a window nobody can read, reached by the key that
                // was making it larger.
                if (next - now).abs() < f32::EPSILON {
                    return None;
                }
                format!("{next:.places$}")
            }
        };
        Some(Change {
            key,
            value: next,
            file: *file,
        })
    }

    /// Where along its track the value of a row sits, for drawing the thumb.
    /// Nothing for a row that is not a slider.
    pub fn fraction(&self, index: usize, side: Side) -> Option<f32> {
        let Row::Setting { value, kind, .. } = self.cell(index, side)? else {
            return None;
        };
        let value = self.preview(index, side).unwrap_or(value);
        kind.fraction(value.parse::<f32>().ok()?)
    }

    /// Drag the slider on one row to a position along its track.
    ///
    /// The value is held here and not written: a drag across a window is
    /// hundreds of motion events, and writing the settings file at each one is
    /// hundreds of rename-over-the-file writes for one decision. What the drag
    /// is holding is applied to the window on every one of those events
    /// ([`Settings::previewed`] is what `main` reads it back with); it is only
    /// the file that waits for the button. The cursor follows the drag, so
    /// letting go and pressing an arrow key carries on from where the slider was
    /// left.
    pub fn slide(&mut self, index: usize, side: Side, fraction: f32) -> bool {
        let Some(Row::Setting { kind, .. }) = self.cell(index, side) else {
            return false;
        };
        let Some(next) = kind.at(fraction) else {
            return false;
        };
        self.focus = Focus::Content;
        let section = self.here_mut();
        section.cursor = index;
        section.side = side;
        let moved = self.preview(index, side) != Some(next.as_str());
        self.dragging = Some((index, side, next));
        moved
    }

    /// What the slider under the button is being dragged to, while it is still
    /// being dragged.
    ///
    /// The same change [`Settings::drop_slider`] returns, without ending the
    /// drag, so the window can take the value while the pointer moves and the
    /// file can still be written once. Nothing when no slider is down; the
    /// change is returned even when it matches the file, since a drag that went
    /// away and came back has to put the window back too.
    pub fn previewed(&self) -> Option<Change> {
        let (index, side, value) = self.dragging.as_ref()?;
        let Some(Row::Setting { key, file, .. }) = self.cell(*index, *side) else {
            return None;
        };
        Some(Change {
            key,
            value: value.clone(),
            file: *file,
        })
    }

    /// The button came up: what the drag decided, or nothing when it decided
    /// what the file already said.
    pub fn drop_slider(&mut self) -> Option<Change> {
        let (index, side, value) = self.dragging.take()?;
        let Some(Row::Setting {
            key,
            value: was,
            file,
            ..
        }) = self.cell(index, side)
        else {
            return None;
        };
        if *was == value {
            return None;
        }
        Some(Change {
            key,
            value,
            file: *file,
        })
    }

    /// Start typing into the field under the cursor, from what it says now.
    pub fn edit(&mut self) -> bool {
        if self.editing.is_some() {
            return false;
        }
        let Some(Row::Field { value, .. }) = self.at_cursor() else {
            return false;
        };
        self.editing = Some(value.clone());
        true
    }

    /// Add what was typed. Whitespace is dropped rather than typed: no value in
    /// either of these files can contain a space, and a URL pasted with a
    /// newline on the end would otherwise be refused at the write.
    pub fn type_text(&mut self, text: &str) -> bool {
        let Some(buffer) = self.editing.as_mut() else {
            return false;
        };
        let mut typed = false;
        for ch in text.chars() {
            if ch.is_whitespace() || ch.is_control() {
                continue;
            }
            buffer.push(ch);
            typed = true;
        }
        typed
    }

    /// Take one character back off it, by character rather than by byte.
    pub fn backspace(&mut self) -> bool {
        let Some(buffer) = self.editing.as_mut() else {
            return false;
        };
        buffer.pop().is_some()
    }

    /// Stop editing and keep what the file says. True when there was an edit to
    /// stop, which is what makes Escape close the panel only when there is not.
    pub fn cancel_edit(&mut self) -> bool {
        self.editing.take().is_some()
    }

    /// Finish the edit: the key and what was typed, for whoever writes the file.
    /// The row is left saying what the file says until the write has landed and
    /// been read back.
    pub fn finish_edit(&mut self) -> Option<(&'static str, String)> {
        let typed = self.editing.take()?;
        let Some(Row::Field { key, .. }) = self.at_cursor() else {
            return None;
        };
        Some((key, typed))
    }

    /// Where the agent's own file is, for the write the field asks for.
    pub fn agent_file(&self) -> Option<&Path> {
        self.agent.env_path.as_deref()
    }

    /// Where the skills live, for the move a toggle asks for and the delete an
    /// uninstall asks for. Both are done against this directory and the sibling
    /// beside it, and nowhere else.
    pub fn skills_at(&self) -> Option<&Path> {
        self.agent.skills_at.as_deref()
    }

    /// Which `mcp.json` a server belongs to, which is the only file a toggle on
    /// its row writes.
    pub fn mcp_file(&self, project: bool) -> Option<&Path> {
        match project {
            true => self.agent.mcp.project.as_deref(),
            false => self.agent.mcp.global.as_deref(),
        }
    }

    /// What turning the entry on one row on or off means on the disk, or
    /// nothing when that row is not an entry.
    pub fn toggle(&self, index: usize) -> Option<Deed> {
        let Row::Entry(entry) = self.row(index)? else {
            return None;
        };
        Some(match &entry.what {
            Which::Skill { dir } => Deed::TurnSkill {
                dir: dir.clone(),
                on: !entry.on,
            },
            Which::Server { project } => Deed::TurnServer {
                name: entry.name.clone(),
                project: *project,
                on: !entry.on,
            },
        })
    }

    /// Press the uninstall on one row. The first press arms it and answers with
    /// nothing; the second one on the same row is the delete.
    ///
    /// Two presses because these are the only things on the panel that cannot be
    /// undone: a skill turned off can be turned back on, a server turned off is
    /// still in its file, a setting written the wrong way can be written again,
    /// and a directory or an entry that has been removed is gone. In between the
    /// two the footer says what is about to go, which is the whole of what a
    /// confirmation is for.
    pub fn uninstall(&mut self, index: usize) -> Option<Deed> {
        let Some(Row::Entry(entry)) = self.row(index) else {
            return None;
        };
        if !entry.removable {
            return None;
        }
        let deed = match &entry.what {
            Which::Skill { dir } => Deed::RemoveSkill {
                dir: dir.clone(),
                on: entry.on,
            },
            Which::Server { project } => Deed::RemoveServer {
                name: entry.name.clone(),
                project: *project,
            },
        };
        if self.arming == Some(index) {
            self.arming = None;
            return Some(deed);
        }
        self.arming = Some(index);
        None
    }

    /// Which uninstall is armed, for the button that says so.
    pub fn arming(&self) -> Option<usize> {
        self.arming
    }

    /// The block of text on one row, for whoever draws it.
    pub fn paper(&self, index: usize) -> Option<&Paper> {
        match self.row(index)? {
            Row::Paper(paper) => Some(paper),
            _ => None,
        }
    }

    /// Move a block of text inside its own box, without moving the list.
    ///
    /// Its own scroll for the same reason the column beside the entry list has
    /// one: the pointer is on the thing being scrolled, and reading a prompt
    /// must not walk the rows under it.
    pub fn scroll_paper(&mut self, index: usize, by: usize, down: bool) -> bool {
        let section = self.here_mut();
        let Some(Row::Paper(paper)) = section.rows.get_mut(index) else {
            return false;
        };
        let most = paper.most();
        let next = match down {
            true => (paper.first + by).min(most),
            false => paper.first.saturating_sub(by),
        };
        let moved = next != paper.first;
        paper.first = next;
        moved
    }

    /// What a press on a block with nothing in it asks for: the file it offered
    /// to write. Nothing on a block that has something to show, so the press
    /// cannot land on a file that is already there.
    pub fn make(&self, index: usize) -> Option<Deed> {
        let path = self.paper(index)?.offer.clone()?;
        Some(Deed::StartInstructions { path })
    }

    /// The entry the column beside the list is showing: the one under the
    /// cursor, or the first in the section when the cursor is not on one, so
    /// the column has something in it the moment the section opens.
    pub fn showing(&self) -> Option<&Entry> {
        let here = self.here();
        match here.rows.get(here.cursor) {
            Some(Row::Entry(entry)) => Some(entry),
            _ => here.rows.iter().find_map(|row| match row {
                Row::Entry(entry) => Some(entry),
                _ => None,
            }),
        }
    }

    /// Which line of that document the column starts on, in a column `rows`
    /// tall. Clamped here rather than where it is set, so a document that got
    /// shorter cannot leave the column showing nothing.
    pub fn doc_first(&self, rows: usize) -> usize {
        let lines = self.showing().map(|entry| entry.doc.len()).unwrap_or(0);
        self.here().doc_first.min(lines.saturating_sub(rows))
    }

    /// Move that column, for a wheel with the pointer over it.
    pub fn scroll_doc(&mut self, by: usize, down: bool, rows: usize) -> bool {
        let most = self
            .showing()
            .map(|entry| entry.doc.len())
            .unwrap_or(0)
            .saturating_sub(rows);
        let section = self.here_mut();
        let next = match down {
            true => (section.doc_first + by).min(most),
            false => section.doc_first.saturating_sub(by),
        };
        let moved = next != section.doc_first;
        section.doc_first = next;
        moved
    }

    /// Back to the top of that column, which is what moving to another entry
    /// means: the lines on screen belong to the entry the cursor was on.
    fn rewind_doc(&mut self) {
        self.here_mut().doc_first = 0;
    }

    /// Move the cursor one row, over anything it cannot land on, or the rail one
    /// section. Clamped at both ends: a list that wraps under an arrow key held
    /// down is a cursor that arrives somewhere nobody was looking.
    pub fn step(&mut self, down: bool) -> bool {
        // The footer goes back to saying what the keys do the moment a key is
        // pressed: a swatch that was pressed a screen ago is not what the
        // keyboard is on. The column beside the list goes back to the top of
        // its document for the same reason: the cursor is about to be on
        // another entry, and that entry's text starts at its own first line.
        self.picked = None;
        self.arming = None;
        self.rewind_doc();
        if self.focus == Focus::Rail {
            let next = match down {
                true => (self.chosen + 1).min(self.sections.len() - 1),
                false => self.chosen.saturating_sub(1),
            };
            let moved = next != self.chosen;
            self.chosen = next;
            return moved;
        }
        let section = self.here_mut();
        let Some(next) = next_landing(&section.rows, section.cursor, down) else {
            return false;
        };
        let moved = next != section.cursor;
        section.cursor = next;
        moved
    }

    /// A screenful, then the nearest row that can hold the cursor.
    pub fn page(&mut self, rows: usize, down: bool) -> bool {
        self.picked = None;
        self.arming = None;
        self.rewind_doc();
        if self.focus == Focus::Rail {
            return self.step(down);
        }
        // A block of text is read with these keys rather than paged past. The
        // cursor is on it, up and down are still how it is left, and the rows
        // under it do not move while it is being read.
        if matches!(self.at_cursor(), Some(Row::Paper(_))) {
            let at = self.cursor();
            return self.scroll_paper(at, PAPER_LINES, down);
        }
        let by = rows.max(1);
        let section = self.here_mut();
        let reach = match down {
            true => (section.cursor + by).min(section.rows.len().saturating_sub(1)),
            false => section.cursor.saturating_sub(by),
        };
        // From the row a page away, look on in the direction of travel first and
        // then back the other way, so a page that lands among readings does not
        // stop dead and does not jump back past where it came from.
        let next = landing_from(&section.rows, reach, down)
            .or_else(|| landing_from(&section.rows, reach, !down))
            .unwrap_or(section.cursor);
        let moved = next != section.cursor;
        section.cursor = next;
        moved
    }

    /// The first or last row anything can be done to.
    pub fn jump(&mut self, last: bool) -> bool {
        self.picked = None;
        self.arming = None;
        self.rewind_doc();
        if self.focus == Focus::Rail {
            let next = match last {
                true => self.sections.len() - 1,
                false => 0,
            };
            let moved = next != self.chosen;
            self.chosen = next;
            return moved;
        }
        let section = self.here_mut();
        let edge = match last {
            true => section.rows.len().saturating_sub(1),
            false => 0,
        };
        let Some(next) = landing_from(&section.rows, edge, !last) else {
            return false;
        };
        let moved = next != section.cursor;
        section.cursor = next;
        moved
    }

    /// Put the cursor on the row under the pointer, when that row can hold it.
    /// A click in the content is also a click into it, so the keyboard follows
    /// the pointer instead of staying on the rail.
    ///
    /// `side` is which half of a form row was pressed. A press on the half that
    /// is a reading lands on the control beside it rather than on nothing, the
    /// same way the keyboard resolves it.
    pub fn point_at(&mut self, index: usize, side: Side) -> bool {
        let Some(row) = self.row(index) else {
            return false;
        };
        let side = match landable(cell(row, side)) {
            true => side,
            false => side.other(),
        };
        if !landable(cell(row, side)) {
            return false;
        }
        self.picked = None;
        // Only when the pointer moved to another row: the press that deletes is
        // the second one on the same uninstall, and it comes through here first.
        if self.arming.is_some_and(|at| at != index) {
            self.arming = None;
        }
        self.rewind_doc();
        let was = (self.here().cursor, self.side(), self.focus);
        self.focus = Focus::Content;
        let section = self.here_mut();
        section.cursor = index;
        section.side = side;
        (index, side, Focus::Content) != was
    }

    /// How many rows of text each row of the list takes, for the scroll window.
    ///
    /// Not one each any more: a heading is drawn larger than the settings under
    /// it and takes two ([`lines`]). A value too long for the panel is still
    /// clipped rather than wrapped, so a click still cannot resolve to a setting
    /// other than the one under the pointer.
    pub fn heights(&self) -> Vec<usize> {
        text_geometry::heights(self.here().rows.iter().map(lines), 1)
    }

    /// Which rows are on screen in a list `rows` tall: the first one, and how
    /// many fit under it.
    ///
    /// Anchored on a row rather than on a row of text, so the top of the list is
    /// always the top of a row. Half a heading at the top of the list is a
    /// heading nobody can read whose click region starts off the screen, and
    /// every hit region below it would be a row out of step with what is drawn.
    /// A row that does not fit whole is left for the next screenful, except when
    /// it is the only one there is room to start with.
    pub fn window(&self, rows: usize) -> (usize, usize) {
        let heights = self.heights();
        if rows == 0 || heights.is_empty() {
            return (0, 0);
        }
        let first = self.here().first.min(last_top(&heights, rows));
        let mut used = 0;
        let mut count = 0;
        while first + count < heights.len() {
            let height = heights[first + count];
            if count > 0 && used + height > rows {
                break;
            }
            used += height;
            count += 1;
        }
        (first, count)
    }

    /// Bring the cursor on screen, for a `rows` tall list.
    pub fn reveal(&mut self, rows: usize) -> bool {
        if rows == 0 || self.here().rows.is_empty() {
            return false;
        }
        let heights = self.heights();
        let most = last_top(&heights, rows);
        let section = self.here_mut();
        let cursor = section.cursor.min(heights.len() - 1);
        let mut next = section.first.min(cursor);
        // Down one row at a time until the cursor's own row fits under the top,
        // which is the same walk `window` does and cannot disagree with it.
        while next < cursor && heights[next..=cursor].iter().sum::<usize>() > rows {
            next += 1;
        }
        let next = next.min(most);
        let moved = next != section.first;
        section.first = next;
        moved
    }

    /// Move the window without moving the cursor, for the wheel.
    pub fn scroll(&mut self, by: usize, down: bool, rows: usize) -> bool {
        let most = last_top(&self.heights(), rows);
        let section = self.here_mut();
        let next = match down {
            true => (section.first + by).min(most),
            false => section.first.saturating_sub(by),
        };
        let moved = next != section.first;
        section.first = next;
        moved
    }

    /// How much of the list is on screen, for the scrollbar.
    pub fn thumb(&self, rows: usize) -> Option<(f32, f32)> {
        let heights = self.heights();
        let (first, _) = self.window(rows);
        // The scrollbar counts rows of text, so the row the list starts on has
        // to be turned into the row of text it starts on first.
        let above: usize = heights.iter().take(first).sum();
        let back = text_geometry::scrollback_for(&heights, rows, above);
        text_geometry::thumb(&heights, rows, back)
    }
}

/// The furthest down the list can start and still fill the last screenful.
///
/// In rows rather than in rows of text, because the list starts on a row.
fn last_top(heights: &[usize], rows: usize) -> usize {
    let mut used = 0;
    for (index, height) in heights.iter().enumerate().rev() {
        used += height;
        if used > rows {
            return (index + 1).min(heights.len().saturating_sub(1));
        }
    }
    0
}

fn note(text: &str) -> Row {
    Row::Note {
        text: String::from(text),
        bad: false,
    }
}

/// The rows for a group of the window's own settings, read off the config.
///
/// A key on [`OFF_PANEL`] never becomes a row, whatever group it is written
/// into. The list is the rule and not a note about one: putting `show_files`
/// back in a table is not enough to put it back on the panel, which is what
/// stops the removal being undone by a later edit that meant to add something
/// else.
fn settings_rows(config: &Config, group: &[(&'static str, Kind)]) -> Vec<Row> {
    group
        .iter()
        .filter(|(key, _)| !OFF_PANEL.contains(key))
        .map(|(key, kind)| Row::Setting {
            key,
            value: value_of(config, key, *kind),
            kind: *kind,
            file: File::Window,
        })
        .collect()
}

/// What the CLI uses for one of its own settings when the file does not carry
/// it. Read off the CLI rather than chosen here: a row that shows a number the
/// agent is not actually running with is worse than no row.
fn agent_default(key: &str) -> String {
    match key {
        agent::CTX => agent::CTX_DEFAULT.to_string(),
        agent::TASK_CONCURRENCY => agent::TASK_CONCURRENCY_DEFAULT.to_string(),
        // Unreachable through AGENT_SETTINGS, and a number is the honest answer
        // for a row that says it is one.
        _ => String::from("0"),
    }
}


/// What a session row says: when it was, which folder it belongs to, how big
/// the transcript is, how full its context window was, and the opening of the
/// first thing that was said in it.
///
/// The same five things in the same order the picker's session rows use, so the
/// list here and the list a session is resumed from are recognisably the same
/// sessions. The two middle ones are formatted by the picker's own helpers
/// rather than written again here, which is what keeps them saying the same
/// thing. Only the wording for a session with no folder differs, because the
/// picker's row says where pressing it would resume and this panel resumes
/// nothing.
fn session_line(saved: &crate::sessions::Saved, now: std::time::SystemTime) -> String {
    let folder = match (&saved.workspace, saved.gone) {
        (Some(path), true) => format!("{} (gone)", short_folder(path)),
        (Some(path), false) => short_folder(path),
        // Written by the CLI rather than by this window, so nothing ever noted
        // where it was.
        (None, _) => String::from("no folder noted"),
    };
    let said = match saved.opening.is_empty() {
        true => "nothing was said",
        false => saved.opening.as_str(),
    };
    format!(
        "{}  {folder}  {}  {}  {said}",
        crate::sessions::ago(saved.when, now),
        crate::picker::size_label(saved.bytes),
        crate::picker::context_label(saved.context),
    )
}

/// The name of the folder and no more of the path, which is what the picker's
/// session rows say. The two lists have to read the same or they are two lists.
fn short_folder(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.display().to_string())
}

fn skill_line(skill: &agent::Skill) -> String {
    match skill.about.is_empty() {
        true => skill.name.clone(),
        false => format!("{}  {}", skill.name, skill.about),
    }
}

fn server_line(server: &agent::Server) -> String {
    let where_ = match server.project {
        true => "project",
        false => "global",
    };
    format!("{}  {}  ({where_})", server.name, server.how)
}

/// What the column beside the list shows for a server: its entry out of the
/// file, exactly as the file carries it.
///
/// Fenced as JSON so the highlighter reads it the way it reads any other code
/// block: the whole column is Markdown, and a skill's own document is the thing
/// it was built for.
fn server_doc(server: &agent::Server, file: Option<&Path>) -> Vec<String> {
    let mut out = vec![format!("# {}", server.name), String::new()];
    out.push(String::from("```json"));
    out.extend(server.entry.lines().map(str::to_string));
    out.push(String::from("```"));
    if let Some(path) = file {
        out.push(String::new());
        out.push(format!("in {}", path.display()));
    }
    out
}

/// Whether the cursor stops on a row. A cursor that lands where nothing can
/// happen is a dead stop the arrow keys have to be pressed through, so it stops
/// on the settings and the one field and on nothing else. A swatch is a colour
/// to read: the keys cannot change one, and the file is where they are edited.
fn landable(row: &Row) -> bool {
    match row {
        // A block of text holds the cursor because it is read rather than
        // changed: the page keys scroll the one the cursor is on, and a block
        // with nothing in it yet is where the press that writes the file is
        // aimed.
        Row::Setting { .. } | Row::Field { .. } | Row::Entry(_) | Row::Paper(_) => true,
        Row::Pair(left, right) => landable(left) || landable(right),
        _ => false,
    }
}

/// The next row in that direction the cursor can land on, not counting the one
/// it is on.
fn next_landing(rows: &[Row], from: usize, down: bool) -> Option<usize> {
    match down {
        true => landing_from(rows, from + 1, true),
        false => landing_from(rows, from.checked_sub(1)?, false),
    }
    .or_else(|| landing_from(rows, from, down))
}

/// The first row at or beyond `from`, walking in one direction, that can hold
/// the cursor.
fn landing_from(rows: &[Row], from: usize, down: bool) -> Option<usize> {
    let mut range: Box<dyn Iterator<Item = usize>> = match down {
        true => Box::new(from..rows.len()),
        false => Box::new((0..=from.min(rows.len().saturating_sub(1))).rev()),
    };
    range.find(|at| rows.get(*at).is_some_and(landable))
}

/// What the file says for one setting right now.
///
/// Spelled the way the writer would spell it, because this is also what
/// [`Settings::change`] reads to work out the next value: `on`/`off` for a flag
/// and no trailing zeros on a whole number.
fn value_of(config: &Config, key: &str, kind: Kind) -> String {
    match (key, kind) {
        ("theme", _) => theme_name(config).to_string(),
        ("prompt_rows", _) => config.prompt_rows.to_string(),
        (_, Kind::Number { places, .. }) => {
            let value = match key {
                "opacity" => config.opacity,
                "font_size" => config.font_size,
                "pane_font_size" => config.pane_font_size,
                // Unreachable through the groups, and a number is the honest
                // answer for a row that says it is one.
                _ => 0.0,
            };
            format!("{value:.places$}")
        }
        // Same: every key in the groups is answered above.
        _ => String::new(),
    }
}

/// `#rrggbb`, which is what the file wants back.
fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// Write one change to the window's settings file and read the whole file back.
///
/// The value goes in through the settings writer, which keeps every comment and
/// refuses a key the parser does not read, and the Config that comes back is
/// parsed from the file rather than patched in memory, so what the panel shows
/// next is what the next launch will read.
pub fn commit(path: &Path, change: &Change) -> Result<Config, String> {
    config::write_setting(path, change.key, Some(&change.value))?;
    Ok(Config::load_from(path))
}

/// Write one setting into the agent's own file.
///
/// The other half of [`commit`], for the other file. Same shape and same rule:
/// the writer keeps every other line and every comment, and the caller reads the
/// file back rather than trusting what was typed. The endpoint goes through here
/// and so does every [`File::Agent`] change.
pub fn write_endpoint(path: &Path, key: &str, value: &str) -> Result<(), String> {
    agent::write_env(path, key, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn over(config: &Config) -> Settings {
        Settings::open(config, Some(Path::new("/tmp/no0b.conf")), Agent::default())
    }

    /// A scratch settings file of its own per test, since the writer works on a
    /// real path and two tests sharing one would fight over it.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("no0b-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir.join(format!("{name}.conf"))
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("no0b-panel-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    /// Walk the rail to a section and go into it, which is what the arrow keys
    /// do: up to the top, down to the one wanted, right to go in.
    fn go_to(panel: &mut Settings, name: &str) {
        panel.leave();
        panel.jump(false);
        while panel.here().name != name {
            assert!(panel.step(true), "{name} is not on the rail");
        }
        panel.enter();
    }

    fn setting<'a>(panel: &'a Settings, key: &str) -> &'a Row {
        panel
            .all_rows()
            .map(|(_, row)| row)
            .find(|row| matches!(row, Row::Setting { key: k, .. } if *k == key))
            .unwrap_or_else(|| panic!("{key} is not on the panel"))
    }

    fn value(panel: &Settings, key: &str) -> String {
        match setting(panel, key) {
            Row::Setting { value, .. } => value.clone(),
            other => panic!("{other:?}"),
        }
    }

    /// Put the cursor on a setting wherever it lives: section, row and, on a
    /// form row, which half of it.
    fn put_cursor(panel: &mut Settings, key: &str) {
        let section = panel
            .all_rows()
            .find(|(_, row)| matches!(row, Row::Setting { key: k, .. } if *k == key))
            .map(|(section, _)| section)
            .unwrap_or_else(|| panic!("{key} is not on the panel"));
        go_to(panel, section);
        let (at, side) = panel
            .rows()
            .iter()
            .enumerate()
            .find_map(|(at, row)| {
                [Side::Left, Side::Right]
                    .into_iter()
                    .find(|side| {
                        matches!(cell(row, *side), Row::Setting { key: k, .. } if *k == key)
                    })
                    .map(|side| (at, side))
            })
            .expect("the row is in the section it was found in");
        assert!(
            panel.point_at(at, side) || (panel.cursor() == at && panel.side() == side),
            "{key} cannot hold the cursor"
        );
    }

    /// Where one colour sits on the grid of the section that is showing: the row
    /// it is on and the cell along it.
    fn swatch_at(panel: &Settings, key: &str) -> (usize, usize) {
        panel
            .rows()
            .iter()
            .enumerate()
            .find_map(|(at, row)| match row {
                Row::Swatches(cells) => cells
                    .iter()
                    .position(|cell| cell.key == key)
                    .map(|cell| (at, cell)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{key} is not on the grid"))
    }

    /// Everything a section says, as one string, for the tests that care what is
    /// on it rather than which row it is on.
    fn said(panel: &Settings) -> String {
        panel.rows().iter().map(says).collect::<Vec<_>>().join("\n")
    }

    fn says(row: &Row) -> String {
        match row {
                Row::Note { text, .. } | Row::Item(text) => text.clone(),
                Row::Reading { label, value } => format!("{label} {value}"),
                Row::Setting { key, value, .. } | Row::Field { key, value } => {
                    format!("{key} {value}")
                }
                Row::Heading(name) => String::from(*name),
                Row::Swatches(cells) => cells
                    .iter()
                    .map(|cell| format!("{} {} {}", cell.key, cell.about, hex(cell.rgb)))
                    .collect::<Vec<_>>()
                    .join("  "),
                Row::Entry(entry) => format!(
                    "{} {} {}",
                    entry.name,
                    entry.under,
                    match entry.on {
                        true => "on",
                        false => "off",
                    }
                ),
                // Both halves, the way both of them are on the panel.
                Row::Pair(left, right) => format!("{}\n{}", says(left), says(right)),
                Row::Paper(paper) => format!(
                    "{}\n{}\n{}",
                    paper.title,
                    paper.under,
                    paper.body.join("\n")
                ),
        }
    }

    fn a_session(id: &str, ago: u64, folder: Option<&str>, opening: &str) -> crate::sessions::Saved {
        crate::sessions::Saved {
            id: String::from(id),
            when: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 - ago),
            workspace: folder.map(PathBuf::from),
            gone: false,
            bytes: 1_024,
            context: None,
            opening: String::from(opening),
        }
    }

    /// Every section on the rail is reachable with the arrow keys, in both
    /// directions, and every one of them has something on it.
    #[test]
    fn every_section_is_reachable() {
        let mut panel = over(&Config::default());
        assert_eq!(panel.section_names(), SECTIONS.to_vec());
        assert_eq!(panel.focus(), Focus::Rail, "the panel opens on the rail");

        let mut walked = vec![panel.here().name];
        while panel.step(true) {
            walked.push(panel.here().name);
        }
        assert_eq!(walked, SECTIONS.to_vec(), "walking down misses a section");
        assert!(!panel.step(true), "the end of the rail is a stop");

        let mut back = vec![panel.here().name];
        while panel.step(false) {
            back.push(panel.here().name);
        }
        back.reverse();
        assert_eq!(back, SECTIONS.to_vec(), "walking up misses a section");

        // A rail entry that opens on nothing is a section that reads as broken.
        for (at, name) in SECTIONS.iter().enumerate() {
            panel.choose(at);
            assert_eq!(panel.here().name, *name, "the rail cannot reach {name}");
            assert!(!panel.rows().is_empty(), "{name} is empty");
        }

        // In and out, and the pointer does the same thing the keys do.
        assert!(panel.choose(0));
        assert!(panel.enter());
        assert_eq!(panel.focus(), Focus::Content);
        assert!(!panel.enter(), "already in");
        assert!(panel.leave());
        assert_eq!(panel.focus(), Focus::Rail);
        assert!(!panel.leave(), "already out");
    }

    /// Each section keeps its own cursor, so walking away and coming back does
    /// not lose where you were.
    #[test]
    fn a_section_remembers_where_the_cursor_was() {
        let mut panel = over(&Config::default());
        go_to(&mut panel, APPEARANCE);
        assert!(panel.step(true));
        assert!(panel.step(true));
        let was = panel.cursor();
        assert_eq!(was, 2);
        go_to(&mut panel, SESSIONS);
        assert_ne!(panel.cursor(), was, "the two sections share a cursor");
        go_to(&mut panel, APPEARANCE);
        assert_eq!(panel.cursor(), was);
    }

    /// Every key the file understands is on the panel exactly once, in one
    /// section or another, as a row that changes or as a swatch, unless it is
    /// one of the seven [`OFF_PANEL`] names.
    ///
    /// This is the test that stops the panel going stale. A setting added to
    /// `config::keys` and in neither list is a line in everybody's file with no
    /// way to reach it from the window, and nothing else would say so. The list
    /// of exceptions is written out by hand for the same reason: a key drops off
    /// the panel because somebody decided it should, never because a row was
    /// forgotten.
    #[test]
    fn every_key_in_the_file_is_on_the_panel() {
        let config = Config::default();
        let panel = over(&config);
        // The window's own keys only. A row of the agent's file is a setting the
        // same way, nudged the same way, but it is the other file's key and it
        // has no business in this count.
        let mut on_panel: Vec<&str> = panel
            .all_rows()
            .flat_map(|(_, row)| match row {
                Row::Setting {
                    key,
                    file: File::Window,
                    ..
                } => vec![*key],
                // A colour is a cell of a grid row now, not a row of its own.
                Row::Swatches(cells) => cells.iter().map(|cell| cell.key).collect(),
                _ => Vec::new(),
            })
            .collect();
        let mut known: Vec<&str> = config::keys()
            .into_iter()
            .filter(|key| !OFF_PANEL.contains(key))
            .collect();
        on_panel.sort_unstable();
        known.sort_unstable();
        assert_eq!(on_panel, known);
        // And every name in that list is a key the file really carries, so a
        // typo there cannot quietly excuse a setting from the panel.
        for off in OFF_PANEL {
            assert!(config::keys().contains(&off), "{off} is not a key at all");
        }

        // And the agent's own settings are exactly the two the CLI's bounds are
        // known for, so a third one added to the table without a range read off
        // the CLI is a row the agent could refuse.
        let mut of_agent: Vec<&str> = panel
            .all_rows()
            .filter_map(|(_, row)| match row {
                Row::Setting {
                    key,
                    file: File::Agent,
                    ..
                } => Some(*key),
                _ => None,
            })
            .collect();
        of_agent.sort_unstable();
        assert_eq!(of_agent, vec![agent::CTX, agent::TASK_CONCURRENCY]);

        // And nothing retired sneaked in with them: those keys are dead in the
        // file, and the writer refuses them, so a row for one would be a row
        // that can only fail.
        for retired in config::RETIRED {
            assert!(
                !on_panel.contains(&retired),
                "{retired} is retired and on the panel"
            );
        }

        // Every key in the window's file that is on the panel at all is an
        // appearance, colours included: the palette came over when COLOURS was
        // removed. The agent's two are on the agent's section, beside the file
        // they are written into.
        for (section, row) in panel.all_rows() {
            match row {
                Row::Setting { key, file, .. } => {
                    let wanted = match file {
                        File::Window => APPEARANCE,
                        File::Agent => AGENT,
                    };
                    assert_eq!(section, wanted, "{key} is in the wrong section")
                }
                Row::Swatches(cells) => {
                    let keys: Vec<&str> = cells.iter().map(|cell| cell.key).collect();
                    assert_eq!(section, APPEARANCE, "{keys:?} are in the wrong section");
                    assert!(
                        cells.len() <= SWATCH_COLUMNS,
                        "{keys:?} is more than a row of the grid holds"
                    );
                }
                _ => {}
            }
        }
    }

    /// Every colour on the grid says what it colours in words. A block of colour
    /// labelled `gauge_7` is a block of colour.
    #[test]
    fn every_colour_says_what_it_colours() {
        let panel = over(&Config::default());
        let mut said = 0;
        for (_, row) in panel.all_rows() {
            let Row::Swatches(cells) = row else {
                continue;
            };
            for cell in cells {
                assert_ne!(
                    cell.about, "a colour of its own",
                    "{} has no plain words to go with it",
                    cell.key
                );
                assert!(!cell.about.is_empty(), "{} is labelled with nothing", cell.key);
                assert_ne!(cell.about, cell.key, "{} only repeats its key", cell.key);
                said += 1;
            }
        }
        assert_eq!(said, colours(&Config::default()).len());
    }

    /// The cursor only stops where something can happen: not on a heading, not
    /// on a reading, not on a note and not on a colour.
    #[test]
    fn the_cursor_skips_what_it_cannot_change() {
        let config = Config::default();
        let mut panel = over(&config);
        go_to(&mut panel, APPEARANCE);
        assert!(
            matches!(panel.row(panel.cursor()), Some(Row::Setting { key, .. }) if *key == "theme"),
            "the section opens on {:?}",
            panel.row(panel.cursor())
        );

        for name in SECTIONS {
            go_to(&mut panel, name);
            let mut down = vec![panel.cursor()];
            while panel.step(true) {
                down.push(panel.cursor());
            }
            for at in &down {
                let row = panel.row(*at).expect("a row");
                assert!(
                    landable(row) || !panel.on_row(),
                    "{row:?} in {name} cannot hold the cursor"
                );
            }
            let mut up = vec![panel.cursor()];
            while panel.step(false) {
                up.push(panel.cursor());
            }
            up.reverse();
            assert_eq!(down, up, "walking back up {name} visits other rows");
        }

        // APPEARANCE stops on its own settings and nothing else, which is the
        // sizes and the theme: every row of the palette grid under them is
        // stepped over rather than landed on, and that is the whole rest of the
        // section. The panes and the dividers used to be counted here too and
        // are not rows any more.
        go_to(&mut panel, APPEARANCE);
        let mut looks = 1;
        while panel.step(true) {
            looks += 1;
        }
        assert_eq!(looks, LOOKS.len());

        // A section of readings has nothing to land on and says so, rather than
        // drawing a band on a row nothing can be done to.
        go_to(&mut panel, MCP);
        assert!(!panel.on_row());
        go_to(&mut panel, APPEARANCE);
        assert!(panel.on_row());
        // And nothing lands while the keyboard is on the rail.
        panel.leave();
        assert!(!panel.on_row());
    }

    /// No row on the panel is an on and off any more, and nothing changes from
    /// the rail.
    ///
    /// This was `a_flag_flips_and_writes_what_the_file_reads`, which drove the
    /// `show_files` row. The only two flags the window's file carries are the
    /// two panes, and neither is a row now: a closed pane comes back off the
    /// right click menu and the file goes on remembering which are open. What
    /// survives of that test is its last half, which belongs to every row: the
    /// arrow keys on the rail walk the sections and one of them must not also
    /// write a setting.
    #[test]
    fn no_row_is_an_on_and_off_and_the_rail_writes_nothing() {
        let mut panel = over(&Config::default());
        for (section, row) in panel.all_rows() {
            let Row::Setting { key, value, .. } = row else {
                continue;
            };
            assert!(
                !matches!(value.as_str(), "on" | "off"),
                "{key} in {section} is a flag"
            );
        }
        // The file still carries both of them and still reads them, which is
        // what keeps a closed pane closed across a restart.
        assert!(!Config::parse("show_files = off").show_files);
        assert!(!Config::parse("show_activity = off").show_activity);

        put_cursor(&mut panel, "theme");
        assert!(panel.change(true).is_some(), "a row that can change");
        panel.leave();
        assert_eq!(panel.change(true), None);
    }

    /// The theme row names the palette in hand, walks the presets in both
    /// directions, and calls a palette nobody named what it is.
    #[test]
    fn the_theme_row_names_the_palette_the_file_is_carrying() {
        for name in config::THEMES {
            let config = config::theme(name).expect("a preset");
            assert_eq!(theme_name(&config), name);
            assert_eq!(value(&over(&config), "theme"), name);
        }

        let config = Config::default();
        let mut forward = over(&config);
        put_cursor(&mut forward, "theme");
        assert_eq!(forward.change(true).expect("a choice").value, "amber");
        assert_eq!(
            forward.change(false).expect("a choice").value,
            "plum",
            "back from the first preset is the last one"
        );

        // One explicit colour over a preset is not that preset any more, and
        // saying so is the point: the row is read off the colours in hand.
        let tuned = Config::parse("theme = ice\naccent = #ff0000");
        assert_eq!(theme_name(&tuned), CUSTOM);
        let mut panel = over(&tuned);
        put_cursor(&mut panel, "theme");
        assert_eq!(value(&panel, "theme"), CUSTOM);
        assert_eq!(panel.change(true).expect("a choice").value, "noob");
        assert_eq!(panel.change(false).expect("a choice").value, "plum");
    }

    /// A number steps by its own step, stops at both ends, and is written the
    /// way the file spells it.
    #[test]
    fn a_number_steps_and_stops_at_its_ends() {
        let config = Config::default();
        let mut panel = over(&config);
        put_cursor(&mut panel, "font_size");
        assert_eq!(value(&panel, "font_size"), "14", "no trailing zero");
        assert_eq!(panel.change(true).expect("a number").value, "15");
        assert_eq!(panel.change(false).expect("a number").value, "13");

        // At the top of its range the key that was making it bigger does
        // nothing, rather than wrapping round to the smallest size there is.
        let big = Config::parse("font_size = 40");
        let mut panel = over(&big);
        put_cursor(&mut panel, "font_size");
        assert_eq!(value(&panel, "font_size"), "40");
        assert_eq!(panel.change(true), None, "40 is the parser's ceiling");
        assert_eq!(panel.change(false).expect("a number").value, "39");

        let small = Config::parse("opacity = 5%");
        let mut panel = over(&small);
        put_cursor(&mut panel, "opacity");
        assert_eq!(value(&panel, "opacity"), "0.05");
        assert_eq!(panel.change(false), None, "0.05 is the parser's floor");
        assert_eq!(panel.change(true).expect("a number").value, "0.10");
    }

    /// A slider is the same setting the arrow keys are: a position along the
    /// track is a value, that value is back at the position it came from, and
    /// what it writes is one of the values the keys reach.
    #[test]
    fn the_slider_maps_a_position_to_a_value_and_back() {
        for kind in LOOKS
            .iter()
            .chain(AGENT_SETTINGS.iter())
            .map(|(_, kind)| *kind)
            .filter(|kind| matches!(kind, Kind::Number { .. }))
        {
            let Kind::Number {
                step,
                low,
                high,
                places,
            } = kind
            else {
                unreachable!("filtered to the numbers")
            };
            let a_step = step / (high - low);
            for tick in 0..=100 {
                let fraction = tick as f32 / 100.0;
                let value = kind.at(fraction).expect("a number has a track");
                let number = value.parse::<f32>().expect("a number");
                assert!(
                    number >= low - f32::EPSILON && number <= high + f32::EPSILON,
                    "{value} is outside {low}..{high}"
                );
                let back = kind.fraction(number).expect("and back");
                assert!(
                    (back - fraction).abs() <= a_step,
                    "{fraction} became {value}, which is at {back}: more than the one step of {a_step} snapping costs"
                );
            }
            // Both ends are reachable, which is what a track has to be able to
            // say: an opacity slider that cannot reach 1 is a window that can
            // never be solid.
            assert_eq!(kind.at(0.0), Some(format!("{low:.places$}")));
            assert_eq!(kind.at(1.0), Some(format!("{high:.places$}")));
        }
        // Nothing else is a slider. A preset drawn as a track would be a control
        // whose middle means nothing.
        assert_eq!(Kind::Choice(&config::THEMES).at(0.5), None);
        assert_eq!(Kind::Choice(&config::THEMES).fraction(1.0), None);
    }

    /// The row goes on showing the file's value under a preview, and the button
    /// coming up is what says to write. A drag that ended where it started
    /// writes nothing.
    ///
    /// This used to be titled "a drag holds its value until the button comes
    /// up", which is now only half true and was the half that read as a broken
    /// control: the file still waits for the button, and the window does not.
    /// What the drag is holding is on [`Settings::previewed`] from the first
    /// motion event, which is what `main` applies live.
    #[test]
    fn a_drag_writes_once_when_it_ends() {
        let config = Config::parse("opacity = 0.50");
        let mut panel = over(&config);
        put_cursor(&mut panel, "opacity");
        let at = panel.cursor();
        let opacity = Kind::Number {
            step: 0.05,
            low: 0.05,
            high: 1.0,
            places: 2,
        };
        assert_eq!(panel.fraction(at, panel.side()), opacity.fraction(0.5));

        assert!(panel.slide(at, panel.side(), 1.0));
        assert_eq!(panel.preview(at, panel.side()), Some("1.00"));
        assert_eq!(
            value(&panel, "opacity"),
            "0.50",
            "the row said what the file did not"
        );
        assert_eq!(panel.fraction(at, panel.side()), Some(1.0), "the thumb follows the drag");
        assert!(!panel.slide(at, panel.side(), 1.0), "the same place is not a change");

        let change = panel.drop_slider().expect("the drag decided something");
        assert_eq!(change.key, "opacity");
        assert_eq!(change.value, "1.00");
        assert_eq!(panel.preview(at, panel.side()), None, "the preview outlived the drag");

        // A drag that ends on the value the file already has writes nothing: a
        // press on the thumb must not rewrite the file.
        assert!(panel.slide(at, panel.side(), 0.5));
        assert_eq!(panel.drop_slider(), None);
        assert_eq!(panel.drop_slider(), None, "and there is nothing to drop");

        // A row that is not a number has no track at all.
        put_cursor(&mut panel, "theme");
        assert!(!panel.slide(panel.cursor(), panel.side(), 0.5));
        assert_eq!(panel.fraction(panel.cursor(), panel.side()), None);
        assert_eq!(panel.previewed(), None, "a preset is being dragged");
    }

    /// The window takes a dragged value while the pointer is still down. Only
    /// the file waits for the button.
    ///
    /// "the scrolls are not instant, i have to release": a slider you have to
    /// let go of before anything happens is a slider you cannot aim, because the
    /// thing it changes is the thing that would tell you where to stop.
    #[test]
    fn a_drag_moves_the_window_before_the_button_comes_up() {
        let path = scratch("live-drag");
        let _ = std::fs::remove_file(&path);
        let was = Config::load_from(&path);
        let mut panel = Settings::open(&was, Some(&path), Agent::default());
        put_cursor(&mut panel, "opacity");
        let at = panel.cursor();

        assert!(panel.slide(at, panel.side(), 0.0));
        let live = panel.previewed().expect("the drag is holding a value");
        assert_eq!(live.key, "opacity");
        assert_eq!(live.value, "0.05");
        // What `App::preview_setting` does with it: the same setter the file is
        // read with, so the window cannot show a value the file would refuse.
        let mut showing = was.clone();
        assert!(showing.apply(live.key, &live.value));
        assert!((showing.opacity - 0.05).abs() < 0.001, "{}", showing.opacity);
        assert_ne!(showing.opacity, was.opacity, "the window did not move");
        // And nothing has been written: the file still says what it said.
        assert_eq!(Config::load_from(&path).opacity, was.opacity);

        // The pointer keeps moving. Every position is on the panel at once.
        assert!(panel.slide(at, panel.side(), 1.0));
        let live = panel.previewed().expect("still dragging");
        assert_eq!(live.value, "1.00");
        assert_eq!(Config::load_from(&path).opacity, was.opacity, "written mid-drag");

        // The button comes up: one write, and it says what the window already
        // showed.
        let change = panel.drop_slider().expect("the drag decided something");
        let now = commit(&path, &change).expect("the file takes it");
        assert_eq!(now.opacity, 1.0);
        assert_eq!(panel.previewed(), None, "the drag outlived the button");
        let _ = std::fs::remove_file(&path);
    }

    /// Every position on every track is a value the file carries unchanged.
    ///
    /// The live half of the round trip in `a_number_reads_back_as_what_the_panel
    /// _showed`: a drag applies its value to the window without going through
    /// the file, so a track whose ends ran past the parser's clamps would put a
    /// number on screen that the next launch quietly pulls back.
    #[test]
    fn a_drag_cannot_show_a_value_the_file_would_clamp() {
        let was = Config::default();
        let mut panel = over(&was);
        for key in ["opacity", "font_size", "pane_font_size", "prompt_rows"] {
            put_cursor(&mut panel, key);
            let at = panel.cursor();
            // Past both ends as well as along the track: a pointer dragged out
            // of the window is a fraction outside 0..1.
            for fraction in [-3.0, 0.0, 0.25, 0.5, 0.75, 1.0, 4.0] {
                panel.slide(at, panel.side(), fraction);
                let live = panel.previewed().expect("the drag is holding a value");
                assert_eq!(live.key, key);
                let mut showing = was.clone();
                assert!(showing.apply(live.key, &live.value), "{key} is not applied");
                // The panel rebuilt from what the window is now says the same
                // number, which is the check: the setter clamped nothing.
                assert_eq!(
                    value(&over(&showing), key),
                    live.value,
                    "{key} at {fraction} was pulled back by the parser's bounds"
                );
                // And a file line carrying it reads back the same way.
                assert_eq!(
                    Config::parse(&format!("{key} = {}", live.value)),
                    showing,
                    "{key} at {fraction} means one thing live and another in the file"
                );
            }
            panel.drop_slider();
        }
    }

    /// A colour is on the panel to be read, and reading is all.
    ///
    /// This was `a_colour_row_carries_its_swatch_and_cannot_be_changed`, which
    /// asserted the same thing about a `Row::Setting` of its own. A colour is a
    /// cell of a grid row now, so what it is asserted about moved with it: the
    /// grid still cannot hold the cursor and the file is still where a colour is
    /// edited. What is new is the press, which says which key writes it.
    #[test]
    fn a_swatch_carries_its_colour_and_says_which_key_writes_it() {
        let config = Config::parse("accent = #123456");
        let mut panel = over(&config);
        go_to(&mut panel, APPEARANCE);
        let (at, cell) = swatch_at(&panel, "accent");
        assert_eq!(
            *panel.swatch(at, cell).expect("the accent swatch"),
            Swatch {
                key: "accent",
                about: "the accent",
                rgb: [0x12, 0x34, 0x56],
            }
        );
        // More than one to a row, which is what makes it a grid.
        let count = match panel.row(at) {
            Some(Row::Swatches(cells)) => cells.len(),
            other => panic!("the accent is not on a grid row: {other:?}"),
        };
        assert!(count > 1, "one swatch to a row is the list again");

        // The cursor cannot get there, so no change can be aimed at it.
        assert!(!panel.point_at(at, Side::Left));
        assert_ne!(panel.cursor(), at);

        // Pressing one says which line of the file writes it, which is the one
        // thing a block of colour cannot say for itself.
        assert!(panel.pick(at, cell));
        assert_eq!(panel.picked(), Some((at, cell)));
        let says = panel.says();
        assert!(says.contains("accent"), "{says}");
        assert!(says.contains("#123456"), "{says}");
        assert!(says.contains("the accent"), "{says}");
        // And it goes away the moment the keyboard is used again.
        panel.step(true);
        assert_eq!(panel.picked(), None);
        assert_eq!(panel.says(), panel.hint());
        // A cell nobody drew is not a press.
        assert!(!panel.pick(at, count));
        assert!(!panel.pick(0, 0), "the first row of the section is not a grid");

        // And the section says where a colour is edited, with the file to do it
        // in beside it.
        let text = said(&panel);
        assert!(text.contains("edited in the file"), "{text}");
        assert!(text.contains("no0b.conf"), "{text}");
    }

    /// "PANES: remove, has no purpose." Neither group PANES held is anywhere on
    /// the panel: not as a row, not as a heading, not as a reading, and not as a
    /// word in any text the panel writes. The panel still opens on all five
    /// sections and none of them is empty.
    ///
    /// Removed, not moved. An earlier pass deleted the section name and pushed
    /// its rows under APPEARANCE, so every row he was pointing at was still on
    /// screen one heading lower. The keys themselves stay alive: a closed pane
    /// is reopened from the right click menu, a divider is dragged, and both go
    /// on being written and read. See [`OFF_PANEL`].
    #[test]
    fn the_panes_and_the_dividers_are_off_the_panel() {
        let panel = over(&Config::default());
        for (section, row) in panel.all_rows() {
            let named: Vec<&str> = match row {
                Row::Setting { key, .. } | Row::Field { key, .. } => vec![key],
                Row::Swatches(cells) => cells.iter().map(|cell| cell.key).collect(),
                Row::Heading(name) => vec![*name],
                Row::Reading { label, .. } => vec![label.as_str()],
                Row::Note { text, .. } => vec![text.as_str()],
                Row::Item(text) => vec![text.as_str()],
                Row::Entry(entry) => vec![entry.name.as_str()],
                // A form is its halves here: `all_rows` hands those back
                // instead of the pair they are in.
                Row::Pair(_, _) => Vec::new(),
                Row::Paper(paper) => vec![paper.title.as_str(), paper.under.as_str()],
            };
            for said in named {
                for key in OFF_PANEL {
                    assert!(!said.contains(key), "{section} still says {key}: {said:?}");
                }
            }
            // The headings those rows sat under went with them: a heading with
            // nothing under it is worse than the rows were.
            if let Row::Heading(name) = row {
                assert!(
                    !name.contains("PANE") && !name.contains("DIVIDER"),
                    "{name} is still a heading"
                );
            }
        }

        // The panel is unhurt: five sections, all of them with rows, and every
        // one of them still lands the cursor somewhere or says why it cannot.
        let mut panel = panel;
        assert_eq!(panel.section_names(), SECTIONS.to_vec());
        for name in SECTIONS {
            go_to(&mut panel, name);
            assert!(!panel.rows().is_empty(), "{name} has no rows at all");
        }
        // The list is the rule, not a note about one: a group that names one of
        // them builds no row for it.
        let config = Config::default();
        let rows = settings_rows(
            &config,
            &[
                ("font_size", Kind::Number { step: 1.0, low: 8.0, high: 40.0, places: 0 }),
                ("show_files", Kind::Choice(&["on", "off"])),
                ("left_width", Kind::Number { step: 0.05, low: 0.1, high: 0.9, places: 2 }),
            ],
        );
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert!(matches!(&rows[0], Row::Setting { key, .. } if *key == "font_size"));

        // And APPEARANCE did not empty out with them: the sizes, the theme and
        // the whole palette are what it was carrying before PANES ever got
        // pushed into it.
        go_to(&mut panel, APPEARANCE);
        let settings = panel
            .rows()
            .iter()
            .filter(|row| matches!(row, Row::Setting { .. }))
            .count();
        assert_eq!(settings, LOOKS.len());
        assert!(
            panel.rows().iter().any(|row| matches!(row, Row::Swatches(_))),
            "the palette went with them"
        );
    }

    /// The sections that came off are gone from the rail, and so is everything
    /// they carried.
    ///
    /// This was `the_retired_sections_are_off_the_rail_and_their_settings_are
    /// _not`, which asserted the panes and the dividers had landed on
    /// APPEARANCE. That was the wrong half of the answer: PANES was to be
    /// removed, not moved, so the assertion is inverted here and the rows are
    /// gone. `the_panes_and_the_dividers_are_off_the_panel` is where the same
    /// keys are chased through every kind of row.
    #[test]
    fn the_retired_sections_are_off_the_rail_and_so_are_their_settings() {
        let panel = over(&Config::default());
        let names = panel.section_names();
        for gone in ["PANES", "ALL TIME"] {
            assert!(!names.contains(&gone), "{gone} is still a section: {names:?}");
        }
        // Nothing PANES held came over with the section name, and no reading
        // anywhere on the panel is an all-time count.
        for key in OFF_PANEL {
            assert!(
                !panel
                    .all_rows()
                    .any(|(_, row)| matches!(row, Row::Setting { key: name, .. } if *name == key)),
                "{key} is a row again"
            );
        }
        for (_, row) in panel.all_rows() {
            let Row::Reading { label, .. } = row else {
                continue;
            };
            for gone in ["prefilled", "generated", "from cache"] {
                assert_ne!(label, gone, "an all-time reading is still on the panel");
            }
        }

        // No home directory: the panel still opens and says why nothing can be
        // saved rather than pretending there is a file.
        let homeless = Settings::open(&Config::default(), None, Agent::default());
        let where_ = homeless
            .all_rows()
            .find_map(|(_, row)| match row {
                Row::Reading { label, value } if label == "settings" => Some(value.clone()),
                _ => None,
            })
            .expect("the file row");
        assert!(where_.contains("no home directory"), "{where_:?}");
    }

    /// A settings file an older build wrote still opens the panel: the keys it
    /// carries that this build dropped are read off the floor by the parser and
    /// the rail is unchanged by them.
    ///
    /// The retired names are not an error and never were. What is new is that a
    /// section can go too, and a file full of dead keys must not be the thing
    /// that decides which section the panel opens on.
    #[test]
    fn an_older_file_full_of_retired_keys_still_opens_the_panel() {
        let mut text =
            String::from("show_activity = off\nshow_files = off\nleft_width = 0.4\nfont_size = 18\n");
        for key in config::RETIRED {
            text.push_str(&format!("{key} = whatever it used to be\n"));
        }
        let config = Config::parse(&text);
        let panel = over(&config);
        assert_eq!(panel.section_names(), SECTIONS.to_vec());
        assert_eq!(panel.here().name, SECTIONS[0], "the panel opened nowhere");
        assert!(SECTIONS.contains(&panel.here().name));
        for name in SECTIONS {
            assert!(
                panel.all_rows().any(|(section, _)| section == name),
                "{name} opens on nothing"
            );
        }
        // The live rows in that file still read, so a retired neighbour did not
        // take them down with it.
        assert_eq!(value(&panel, "font_size"), "18");
        // And the three keys that are off the panel are still read off the file
        // and still carried, which is what a closed pane and a dragged layout
        // survive a restart on. They are simply not rows.
        assert!(!config.show_activity && !config.show_files);
        assert_eq!(config.left_width, 0.4);
    }

    /// The round trip: what a change writes, the file reads back as, and the
    /// panel then shows are the same value.
    ///
    /// This is what holds [`Kind::Number`]'s bounds to the parser's clamps. A
    /// step past what `Config::parse` accepts would land here as a row showing a
    /// number the file does not carry.
    #[test]
    fn a_number_reads_back_as_what_the_panel_showed() {
        let path = scratch("round-trip");
        let _ = std::fs::remove_file(&path);
        let mut config = Config::load_from(&path);
        let mut panel = Settings::open(&config, Some(&path), Agent::default());

        for key in ["opacity", "font_size", "pane_font_size", "prompt_rows"] {
            // Up first, then down from the top. `prompt_rows` ships at 1,
            // which is the bottom of its range, so a key walked down first has
            // nowhere to go and the walk would prove nothing about its bounds.
            for forward in [true, false] {
                put_cursor(&mut panel, key);
                // Walk to the end of the range, which is where a bound that
                // disagreed with the parser would show up.
                let mut wrote = None;
                while let Some(change) = panel.change(forward) {
                    config = commit(&path, &change).expect("the file takes it");
                    panel.refresh(&config);
                    assert_eq!(
                        value(&panel, key),
                        change.value,
                        "{key} read back as something else"
                    );
                    wrote = Some(change.value);
                }
                assert!(wrote.is_some(), "{key} never moved");
            }
        }
        // The comments the file ships with are still there afterwards, which is
        // the whole reason the writer is used instead of rewriting the file.
        let text = std::fs::read_to_string(&path).expect("the file");
        assert!(text.contains('#'), "{text}");
        let _ = std::fs::remove_file(&path);
    }

    /// A number and a preset go through the same round trip, and the panel keeps
    /// its place across it.
    ///
    /// The first half of this used to be the `show_activity` flag, which is not
    /// a row any more. A size is the same round trip: one nudge, one write, one
    /// reread, and the cursor where it was.
    #[test]
    fn a_change_lands_in_the_file_and_the_cursor_stays_put() {
        let path = scratch("lands");
        let _ = std::fs::remove_file(&path);
        let mut config = Config::load_from(&path);
        let mut panel = Settings::open(&config, Some(&path), Agent::default());
        put_cursor(&mut panel, "pane_font_size");
        let was = (panel.chosen(), panel.cursor());
        let bigger = format!("{}", config.pane_font_size as i32 + 1);

        let change = panel.change(true).expect("a number");
        config = commit(&path, &change).expect("the file takes it");
        assert_eq!(format!("{}", config.pane_font_size as i32), bigger);
        panel.refresh(&config);
        assert_eq!(value(&panel, "pane_font_size"), bigger);
        assert_eq!(
            (panel.chosen(), panel.cursor()),
            was,
            "the cursor moved under the change"
        );

        put_cursor(&mut panel, "theme");
        let change = panel.change(true).expect("a preset");
        config = commit(&path, &change).expect("the file takes it");
        panel.refresh(&config);
        assert_eq!(value(&panel, "theme"), change.value);
        // The preset reached the palette, not just the theme row.
        assert_eq!(
            colours(&config),
            colours(&config::theme(&change.value).expect("a preset"))
        );
        // And the size written before it survived the second write.
        assert_eq!(value(&panel, "pane_font_size"), bigger);
        let _ = std::fs::remove_file(&path);
    }

    /// A write that cannot happen is said out loud and changes nothing.
    #[test]
    fn a_write_that_fails_leaves_the_row_alone() {
        let config = Config::default();
        let mut panel = over(&config);
        put_cursor(&mut panel, "font_size");
        let change = panel.change(true).expect("a number");

        // A directory where the file should be: the write cannot land.
        let path = scratch("refused");
        let _ = std::fs::remove_file(&path);
        std::fs::create_dir_all(&path).expect("a directory in the way");
        assert!(commit(&path, &change).is_err());

        panel.say_trouble(String::from("cannot write it"));
        assert_eq!(panel.trouble(), Some("cannot write it"));
        assert_eq!(value(&panel, "font_size"), "14", "the row moved anyway");
        let _ = std::fs::remove_dir_all(&path);
    }

    /// The footer says what the keys will do where the keyboard is, which
    /// differs by row and differs on the rail.
    #[test]
    fn the_footer_says_what_the_keys_do_here() {
        let config = Config::default();
        let mut panel = over(&config);
        assert!(panel.hint().contains("section"), "{}", panel.hint());
        put_cursor(&mut panel, "theme");
        assert!(panel.hint().contains("presets"), "{}", panel.hint());
        put_cursor(&mut panel, "opacity");
        assert!(panel.hint().contains("slider"), "{}", panel.hint());
    }

    /// A section's list scrolls like every other list in the window, and the
    /// cursor is brought on screen rather than left off the bottom of it.
    #[test]
    fn the_list_scrolls_and_the_cursor_is_kept_on_screen() {
        let config = Config::default();
        let mut panel = over(&config);
        go_to(&mut panel, APPEARANCE);
        let rows = 10;
        assert!(panel.rows().len() > rows, "the longest section is one screenful");
        assert_eq!(panel.first(), 0);
        assert!(panel.thumb(rows).is_some(), "a list this long says so");

        // The wheel moves the window and leaves the cursor where it was.
        let cursor = panel.cursor();
        assert!(panel.scroll(3, true, rows));
        assert_eq!(panel.cursor(), cursor);
        assert!(panel.scroll(3, false, rows));

        // A section short enough to fit does not pretend to scroll.
        go_to(&mut panel, MCP);
        assert!(panel.thumb(rows).is_none(), "three rows do not scroll");

        // Down to the last setting of a longer one, in a window two rows tall:
        // the window follows the cursor to both ends.
        go_to(&mut panel, APPEARANCE);
        assert!(panel.jump(true));
        panel.reveal(2);
        assert!(panel.cursor() < panel.first() + 2, "the cursor is off screen");
        assert!(panel.jump(false));
        panel.reveal(2);
        assert!(
            panel.first() <= panel.cursor(),
            "the cursor is above the window"
        );

        // A page through the palette lands nowhere, because nothing there can
        // hold the cursor, and does not stop dead on a heading either.
        go_to(&mut panel, APPEARANCE);
        let (grid, _) = swatch_at(&panel, "gauge_10");
        while panel.cursor() < grid && panel.page(rows, true) {}
        assert!(
            panel.cursor() < grid,
            "the cursor walked into the palette grid"
        );
    }

    /// The window is counted in rows of text and starts on a row, so a heading
    /// two rows tall never sits half on and half off the top of the list.
    ///
    /// The list was every row exactly one row tall, which the headings are not
    /// any more. If [`Settings::heights`] and the window ever disagree, the rows
    /// the panel draws are not the rows it hit tests.
    #[test]
    fn the_window_counts_a_heading_as_the_two_rows_it_takes() {
        let mut panel = over(&Config::default());
        go_to(&mut panel, APPEARANCE);
        let heights = panel.heights();
        let headings: Vec<usize> = panel
            .rows()
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, Row::Heading(_)))
            .map(|(at, _)| at)
            .collect();
        // The four groups of the palette, which are what is left of the
        // headings now the panes and the dividers are off the section.
        assert!(headings.len() >= 4, "{headings:?}");
        for at in &headings {
            assert_eq!(heights[*at], 2, "a heading is drawn larger than a row");
        }
        for (at, row) in panel.rows().iter().enumerate() {
            assert_eq!(heights[at], lines(row), "the model and the window disagree");
        }

        // Whatever it is scrolled to, the rows on screen start at the top of a
        // row and take no more room than the list has.
        let rows = 12;
        for _ in 0..40 {
            let (first, count) = panel.window(rows);
            let used: usize = heights[first..first + count].iter().sum();
            assert!(count > 0, "the list showed nothing at {first}");
            assert!(
                used <= rows || count == 1,
                "{count} rows from {first} take {used} of {rows}"
            );
            if !panel.scroll(1, true, rows) {
                break;
            }
        }
        // The end of the list is reachable and stops there.
        let (first, count) = panel.window(rows);
        assert_eq!(first + count, panel.rows().len(), "the last row is off screen");
        assert!(!panel.scroll(1, true, rows), "the list scrolled past its end");
    }

    /// The agent section reads the CLI's own file: where it is, what it points
    /// at, and what else is set, with no credential anywhere on it.
    #[test]
    fn the_agent_section_says_what_the_cli_is_pointed_at() {
        let dir = scratch_dir("agent");
        std::fs::write(
            dir.join(".env"),
            "NOOB_BASE_URL=http://localhost:8080/v1\nNOOB_CTX=262144\nNOOB_API_KEY=sk-secret\n",
        )
        .expect("a file");
        let agent = Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel =
            Settings::open(&Config::default(), Some(Path::new("/tmp/no0b.conf")), agent);
        go_to(&mut panel, AGENT);
        let text = said(&panel);
        assert!(
            text.contains(&dir.join(".env").display().to_string()),
            "the panel does not say where the file is: {text}"
        );
        assert!(text.contains("http://localhost:8080/v1"), "{text}");
        assert!(text.contains("NOOB_CTX 262144"), "{text}");
        assert!(!text.contains("sk-secret"), "a credential is on the panel: {text}");
        assert!(text.contains("NOOB_API_KEY set, and not shown here"), "{text}");
        assert_eq!(panel.agent_file(), Some(dir.join(".env").as_path()));

        // The endpoint is the one thing here that is typed into rather than
        // nudged, and it is where the section opens: the left half of the first
        // row of the form.
        assert!(panel.on_row());
        assert_eq!(panel.cursor(), 0, "the section does not open on the form");
        assert_eq!(panel.side(), Side::Left);
        assert!(
            matches!(panel.at_cursor(), Some(Row::Field { key, .. }) if *key == agent::ENDPOINT),
            "{:?}",
            panel.at_cursor()
        );
        assert!(panel.edit());
        assert_eq!(panel.editing(), Some("http://localhost:8080/v1"));
        assert!(!panel.edit(), "already editing");
        assert!(panel.backspace());
        assert!(panel.type_text("2\n\t "), "whitespace is not typed");
        assert_eq!(panel.editing(), Some("http://localhost:8080/v2"));
        assert!(panel.hint().contains("enter saves"), "{}", panel.hint());
        // Nothing has been written: the row still says what the file says.
        assert!(
            said(&panel).contains("http://localhost:8080/v1"),
            "the edit reached the row early"
        );
        assert!(panel.cancel_edit());
        assert!(!panel.cancel_edit());

        // And the whole way through: type it, save it, read it back.
        assert!(panel.edit());
        assert!(panel.type_text("2"));
        let (key, typed) = panel.finish_edit().expect("something was typed");
        assert_eq!(key, agent::ENDPOINT);
        write_endpoint(&dir.join(".env"), key, &typed).expect("the file takes it");
        panel.adopt_agent(
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
            &Config::default(),
        );
        go_to(&mut panel, AGENT);
        assert!(
            panel.all_rows().any(|(_, row)| matches!(
                row,
                Row::Field { value, .. } if value == "http://localhost:8080/v12"
            )),
            "{:?}",
            panel.rows()
        );
        // The rest of the file survived the write, which is the whole point of
        // going through the agent's own writer.
        let after = std::fs::read_to_string(dir.join(".env")).expect("the file");
        assert!(after.contains("NOOB_CTX=262144"), "{after}");
        assert!(after.contains("NOOB_API_KEY=sk-secret"), "{after}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The two numbers that decide what the agent actually gets are controls on
    /// its own section: read off the CLI's file, held to the CLI's own bounds,
    /// nudged and dragged the way every other setting is, and written back into
    /// that file rather than the window's.
    #[test]
    fn the_agent_s_context_and_task_concurrency_are_set_on_the_panel() {
        let dir = scratch_dir("agent-numbers");
        let env = dir.join(".env");
        std::fs::write(
            &env,
            "NOOB_BASE_URL=http://localhost:8080/v1\nNOOB_CTX=262144\nNOOB_TASK_CONCURRENCY=2   # two at a time\n",
        )
        .expect("a file");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), Some(Path::new("/tmp/no0b.conf")), read());

        // What the file says, on the agent's section, once each: a row that is
        // also listed as a reading is the same setting twice with only one of
        // them doing anything.
        put_cursor(&mut panel, agent::CTX);
        assert_eq!(panel.here().name, AGENT);
        assert_eq!(value(&panel, agent::CTX), "262144");
        assert_eq!(value(&panel, agent::TASK_CONCURRENCY), "2");
        let text = said(&panel);
        assert_eq!(text.matches(agent::CTX).count(), 1, "{text}");
        assert_eq!(text.matches(agent::TASK_CONCURRENCY).count(), 1, "{text}");

        // A nudge steps by the CLI's own unit and says which file it belongs in.
        assert_eq!(
            panel.change(true).expect("the context window nudges"),
            Change {
                key: agent::CTX,
                value: String::from("266240"),
                file: File::Agent,
            }
        );

        // Both ends of the concurrency track are the CLI's own: one at the
        // bottom, and at the top the sixteen it caps itself at, so the maximum
        // is somewhere the pointer can be dropped rather than a number to guess.
        put_cursor(&mut panel, agent::TASK_CONCURRENCY);
        let at = panel.cursor();
        assert!(panel.slide(at, panel.side(), 0.0));
        assert_eq!(panel.preview(at, panel.side()), Some("1"));
        assert!(panel.slide(at, panel.side(), 1.0));
        let most = panel.drop_slider().expect("the drag decided something");
        assert_eq!(
            most,
            Change {
                key: agent::TASK_CONCURRENCY,
                value: String::from("16"),
                file: File::Agent,
            }
        );
        // And the context window bottoms out where the CLI stops reading it.
        put_cursor(&mut panel, agent::CTX);
        let at = panel.cursor();
        assert!(panel.slide(at, panel.side(), 0.0));
        assert_eq!(panel.preview(at, panel.side()), Some("4096"));
        panel.drop_slider();

        // Written, it lands in the agent's file, the line keeps its comment and
        // nothing else in the file moves.
        write_endpoint(&env, most.key, &most.value).expect("the file takes it");
        panel.adopt_agent(read(), &Config::default());
        assert_eq!(value(&panel, agent::TASK_CONCURRENCY), "16");
        let after = std::fs::read_to_string(&env).expect("the file");
        assert!(after.contains("NOOB_TASK_CONCURRENCY=16"), "{after}");
        assert!(after.contains("# two at a time"), "the comment is gone: {after}");
        assert!(after.contains("NOOB_CTX=262144"), "{after}");
        assert!(
            after.contains("NOOB_BASE_URL=http://localhost:8080/v1"),
            "{after}"
        );

        // The two files are not interchangeable, which is why a change carries
        // the answer: the window's writer refuses a key of the agent's outright
        // rather than adding a line the window will never read.
        let wrong = commit(
            Path::new("/tmp/no0b.conf"),
            &Change {
                key: agent::CTX,
                value: String::from("8192"),
                file: File::Agent,
            },
        );
        assert!(wrong.is_err(), "the window's file took a setting of the agent's");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With neither of them in the file the rows read what the CLI falls back
    /// to, and the section says that is what they are. A slider showing a number
    /// nobody wrote, with nothing saying so, is a window inventing a setting.
    #[test]
    fn the_agent_s_numbers_read_as_the_cli_s_defaults_until_they_are_written() {
        let dir = scratch_dir("agent-unset");
        let mut panel = Settings::open(
            &Config::default(),
            None,
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
        );
        go_to(&mut panel, AGENT);
        assert_eq!(value(&panel, agent::CTX), agent::CTX_DEFAULT.to_string());
        assert_eq!(
            value(&panel, agent::TASK_CONCURRENCY),
            agent::TASK_CONCURRENCY_DEFAULT.to_string()
        );
        let text = said(&panel);
        assert!(text.contains("not in the file yet"), "{text}");
        assert!(text.contains(agent::CTX), "{text}");
        assert!(text.contains(agent::TASK_CONCURRENCY), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// "actually is awful as is now, unclear because has too many lines
    /// between": the four things this section is for are a form of two columns,
    /// two rows tall, and the keyboard reaches every one of them.
    ///
    /// Left and right are the nudge on a control, so they cannot also be how a
    /// form is crossed; tab is, which is what tab does on every other form. The
    /// heading and the three notes that used to sit between these rows are gone,
    /// so nothing that cannot be set stands between two things that can.
    #[test]
    fn the_agent_s_form_is_two_columns_the_keyboard_can_both_reach() {
        let dir = scratch_dir("agent-form");
        std::fs::write(
            dir.join(".env"),
            "NOOB_BASE_URL=http://localhost:8080/v1\nNOOB_CTX=262144\nNOOB_TASK_CONCURRENCY=2\n",
        )
        .expect("a file");
        let mut panel = Settings::open(
            &Config::default(),
            None,
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
        );
        go_to(&mut panel, AGENT);

        // Two rows of two: where the model is and which file says so on the
        // left, how much the agent gets on the right.
        for (at, left, right) in [
            (0usize, agent::ENDPOINT, agent::CTX),
            (1, "main file", agent::TASK_CONCURRENCY),
        ] {
            let row = panel.row(at).expect("a row of the form");
            assert!(matches!(row, Row::Pair(_, _)), "row {at} is {row:?}");
            assert!(
                says(cell(row, Side::Left)).contains(left),
                "{:?}",
                cell(row, Side::Left)
            );
            assert!(
                says(cell(row, Side::Right)).contains(right),
                "{:?}",
                cell(row, Side::Right)
            );
            // Both halves on the same lines, so the two columns line up.
            assert_eq!(lines(row), 1);
        }
        // Nothing that cannot be set stands between the two rows of the form.
        assert!(
            !panel.rows()[..2]
                .iter()
                .any(|row| matches!(row, Row::Note { .. } | Row::Heading(_))),
            "{:?}",
            panel.rows()
        );

        // It opens on the endpoint, tab crosses to the number beside it, and
        // there the arrow keys still nudge that number rather than moving again.
        assert_eq!((panel.cursor(), panel.side()), (0, Side::Left));
        assert!(matches!(panel.at_cursor(), Some(Row::Field { .. })));
        assert!(panel.hint().contains("tab"), "{}", panel.hint());
        assert!(panel.swap());
        assert_eq!((panel.cursor(), panel.side()), (0, Side::Right));
        assert_eq!(
            panel.change(true).expect("the context window nudges"),
            Change {
                key: agent::CTX,
                value: String::from("266240"),
                file: File::Agent,
            }
        );
        // And down the right hand column: the half is kept while the cursor
        // walks rows, so a form is read a column at a time.
        assert!(panel.step(true));
        assert_eq!((panel.cursor(), panel.side()), (1, Side::Right));
        assert!(
            matches!(panel.at_cursor(), Some(Row::Setting { key, .. }) if *key == agent::TASK_CONCURRENCY)
        );
        // The left half of that row is a reading, so tab has nowhere to go and
        // the cursor stays where something can be done.
        assert!(!panel.swap(), "the cursor crossed onto a reading");
        assert!(panel.step(false));
        assert_eq!((panel.cursor(), panel.side()), (0, Side::Right));
        assert!(panel.swap());
        assert!(matches!(panel.at_cursor(), Some(Row::Field { .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The two blocks under the form: the agent's own global instructions, and
    /// the whole prompt it is one layer of.
    ///
    /// The file is named as the file, because calling it the prompt would be a
    /// lie: the prompt also carries the CLI's base instructions, the environment
    /// block, the project's own AGENTS.md, the skills resolver and the MCP line,
    /// and only `noob debug prompt` returns all of it. Each block is a fixed
    /// height and reads with the page keys, so a prompt a thousand lines long
    /// does not turn the section into a text file.
    #[test]
    fn the_agent_section_carries_the_instructions_and_the_whole_prompt() {
        let dir = scratch_dir("agent-instructions");
        std::fs::write(
            dir.join(agent::AGENTS_MD),
            "# Global instructions\n\nbe brief\n",
        )
        .expect("a file");
        let mut panel = Settings::open(
            &Config::default(),
            None,
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
        );
        go_to(&mut panel, AGENT);

        let block = |panel: &Settings, title: &str| -> Paper {
            panel
                .rows()
                .iter()
                .find_map(|row| match row {
                    Row::Paper(paper) if paper.title.contains(title) => Some(paper.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("there is no {title} block: {:?}", panel.rows()))
        };
        let its = block(&panel, "AGENTS.md");
        assert!(
            its.under.contains(&dir.join(agent::AGENTS_MD).display().to_string()),
            "the block does not name the file: {}",
            its.under
        );
        // Which the panel has to say, because this file is not the `.env`: the
        // prompt is assembled once when a session starts.
        assert!(its.under.contains("session"), "{}", its.under);
        assert_eq!(its.body, ["# Global instructions", "", "be brief"]);
        assert_eq!(its.offer, None, "there is a file to show");

        // Until the CLI answers, the prompt block says it is being read rather
        // than drawing an empty box.
        assert!(
            block(&panel, "PROMPT").under.contains("running"),
            "{}",
            block(&panel, "PROMPT").under
        );
        let body: Vec<String> = (0..PAPER_LINES * 3).map(|at| format!("line {at}")).collect();
        panel.adopt_prompt(
            String::from("/home/hec/workspace/noob-cli"),
            Ok(body.clone()),
            &Config::default(),
        );
        go_to(&mut panel, AGENT);
        let whole = block(&panel, "PROMPT");
        assert_eq!(whole.body, body);
        assert!(whole.under.contains("/home/hec/workspace/noob-cli"), "{}", whole.under);

        // A block is the same height whatever is in it, which is what keeps the
        // rows under it where the clicks below them are tested for.
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Paper(paper) if paper.title.contains("PROMPT")))
            .expect("the prompt block");
        assert_eq!(lines(panel.row(at).expect("the row")), PAPER_LINES + 2);
        assert_eq!(panel.heights()[at], PAPER_LINES + 2, "the model and the window disagree");

        // And it is read with the page keys: the cursor is on it, the block
        // moves and the list under it does not.
        assert!(panel.point_at(at, Side::Left));
        assert!(panel.hint().contains("page"), "{}", panel.hint());
        let was = panel.first();
        assert!(panel.page(20, true));
        assert_eq!(panel.paper(at).expect("the block").first, PAPER_LINES);
        assert_eq!(panel.cursor(), at, "reading the block walked the list");
        assert_eq!(panel.first(), was, "reading the block scrolled the section");
        assert!(panel.page(20, false));
        assert_eq!(panel.paper(at).expect("the block").first, 0);
        assert!(!panel.page(20, false), "it scrolled past its own first line");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With no file there the block says so and offers to write one, and the
    /// press writes it where the agent looks rather than anywhere this window
    /// decided. A prompt the CLI would not print says why instead of showing
    /// nothing.
    #[test]
    fn a_missing_agents_md_is_offered_and_a_failed_prompt_says_why() {
        let dir = scratch_dir("agent-offer");
        let path = dir.join(agent::AGENTS_MD);
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, read());
        go_to(&mut panel, AGENT);
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Paper(paper) if paper.title.contains("AGENTS.md")))
            .expect("the instructions block");
        let paper = panel.paper(at).expect("the block").clone();
        assert_eq!(paper.offer.as_deref(), Some(path.as_path()));
        assert!(paper.under.contains("nothing at"), "{}", paper.under);
        assert!(!paper.body.is_empty(), "an empty box says nothing at all");

        // The press asks for the file the block named, and nothing else on the
        // panel offers one.
        assert!(panel.point_at(at, Side::Left));
        assert!(panel.hint().contains("enter"), "{}", panel.hint());
        assert_eq!(
            panel.make(at),
            Some(Deed::StartInstructions { path: path.clone() })
        );
        agent::start_instructions(&path).expect("the file is written");
        assert!(
            agent::start_instructions(&path).is_err(),
            "a second press wrote over instructions somebody had"
        );
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, AGENT);
        let paper = panel.paper(at).expect("the block");
        assert_eq!(paper.offer, None, "it still offers a file that is there");
        assert!(
            paper.body.iter().any(|line| line.contains("Global instructions")),
            "{:?}",
            paper.body
        );
        assert_eq!(panel.make(at), None);

        // A whitespace-only file is nothing at all to the agent, so it is
        // nothing at all here: it trims the file and a blank one carries no
        // heading into the prompt.
        std::fs::write(&path, "\n   \n").expect("a file");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, AGENT);
        assert!(panel.paper(at).expect("the block").offer.is_some());

        // And the other block says why there is no prompt rather than sitting
        // empty with nothing anywhere saying what happened.
        panel.adopt_prompt(
            String::from("/tmp/work"),
            Err(String::from("noob debug prompt failed: no such subcommand")),
            &Config::default(),
        );
        go_to(&mut panel, AGENT);
        let prompt = panel
            .rows()
            .iter()
            .find_map(|row| match row {
                Row::Paper(paper) if paper.title.contains("PROMPT") => Some(paper),
                _ => None,
            })
            .expect("the prompt block");
        assert!(prompt.bad, "a failure is not marked as one");
        assert!(prompt.under.contains("no such subcommand"), "{}", prompt.under);
        assert!(prompt.under.contains("/tmp/work"), "{}", prompt.under);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An agent with nothing configured says so rather than showing empty
    /// sections that read as broken, and names the files to write.
    #[test]
    fn nothing_configured_is_said_rather_than_shown_as_empty() {
        let dir = scratch_dir("bare");
        let work = dir.join("work");
        let agent = Agent::read(Some(&dir), Some(&work), crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, agent);

        for (section, wanted) in [
            (MCP, "none configured"),
            (SKILLS, "none installed"),
            (SESSIONS, "no saved sessions yet"),
            (AGENT, "probes the usual local ports"),
        ] {
            go_to(&mut panel, section);
            let text = said(&panel);
            assert!(
                text.contains(wanted),
                "{section} does not say {wanted:?}: {text}"
            );
        }

        // Both mcp.json paths are named, so there is somewhere to put one.
        go_to(&mut panel, MCP);
        let text = said(&panel);
        assert!(
            text.contains(&dir.join("mcp.json").display().to_string()),
            "{text}"
        );
        assert!(
            text.contains(&work.join(".noob/mcp.json").display().to_string()),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A malformed mcp.json is a line on the panel, not a panel that will not
    /// open. The servers from the file that did parse are still listed.
    #[test]
    fn a_broken_mcp_file_is_a_line_on_the_panel() {
        let dir = scratch_dir("broken");
        std::fs::create_dir_all(dir.join("work/.noob")).expect("a scratch directory");
        std::fs::write(dir.join("mcp.json"), "{\"servers\": {").expect("a file");
        std::fs::write(
            dir.join("work/.noob/mcp.json"),
            r#"{"servers": {"docs": {"url": "http://localhost:9000/mcp"}}}"#,
        )
        .expect("a file");
        let agent = Agent::read(
            Some(&dir),
            Some(&dir.join("work")),
            crate::sessions::Listing::default(),
        );
        let mut panel = Settings::open(&Config::default(), None, agent);
        go_to(&mut panel, MCP);
        let rows = panel.rows();
        assert!(
            rows.iter().any(|row| matches!(row, Row::Entry(entry)
                if entry.name == "docs"
                    && entry.under.contains("http://localhost:9000/mcp")
                    && entry.under.contains("project")
                    && entry.on)),
            "{rows:?}"
        );
        assert!(
            rows.iter().any(|row| matches!(row, Row::Note { text, bad }
                if *bad && text.contains("not valid JSON"))),
            "the broken file is not reported: {rows:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The sessions on the panel are the sessions the picker offers, read with
    /// the same reader and said the same way.
    #[test]
    fn the_sessions_section_matches_what_the_picker_shows() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let listing = crate::sessions::Listing {
            sessions: vec![
                a_session(
                    "aaa",
                    120,
                    Some("/home/hec/workspace/noob-cli"),
                    "fix the panel",
                ),
                a_session("bbb", 7200, None, ""),
            ],
            skipped: vec![String::from("ccc: no meta line")],
        };
        let agent = Agent {
            now,
            sessions: listing.clone(),
            ..Agent::default()
        };
        let mut panel = Settings::open(&Config::default(), None, agent);
        go_to(&mut panel, SESSIONS);
        let listed: Vec<String> = panel
            .rows()
            .iter()
            .filter_map(|row| match row {
                Row::Item(text) => Some(text.clone()),
                _ => None,
            })
            .collect();

        let mut picker = crate::picker::Picker::open(
            Box::new(crate::picker::Fixed(Vec::new())),
            PathBuf::from("/home/hec/workspace/noob-cli"),
            Vec::new(),
        );
        picker.show_sessions_at(listing, now);
        let offered: Vec<String> = picker
            .rows()
            .iter()
            .filter(|row| matches!(row, crate::picker::Row::Session(_)))
            .map(|row| picker.label(row))
            .collect();
        assert_eq!(offered.len(), 2, "the picker offers both");
        assert_eq!(listed.len(), offered.len(), "{listed:?} against {offered:?}");
        for (mine, theirs) in listed.iter().zip(&offered) {
            // The picker says "this folder" for a session with no folder noted,
            // because that is where pressing it would resume; this panel resumes
            // nothing, so it says what it knows. Everything else is said the
            // same way, in the same order.
            let theirs = theirs.replace("this folder", "no folder noted");
            assert_eq!(*mine, theirs);
        }
        assert!(listed[0].contains("2m ago"), "{listed:?}");
        assert!(listed[0].contains("noob-cli"), "{listed:?}");
        assert!(listed[0].contains("fix the panel"), "{listed:?}");
        assert!(listed[1].contains("nothing was said"), "{listed:?}");

        // A file that could not be read is said rather than quietly missing.
        assert!(
            panel
                .rows()
                .iter()
                .any(|row| matches!(row, Row::Note { text, bad } if *bad && text.contains("ccc"))),
            "{:?}",
            panel.rows()
        );
    }

    /// The skills section lists what is on the disk: a row per skill, the
    /// repository it records or the directory it was found in underneath, and
    /// its own `SKILL.md` in the column beside the list.
    ///
    /// This used to assert two `Row::Item` strings, which is the read-only list
    /// it was. The rows carry an identity now because they can be turned off
    /// and uninstalled, and the section is two columns.
    #[test]
    fn the_skills_section_lists_what_is_on_the_disk() {
        let dir = scratch_dir("skills-section");
        let skills = dir.join("skills");
        std::fs::create_dir_all(skills.join("coding")).expect("a directory");
        std::fs::write(
            skills.join("coding").join("SKILL.md"),
            "---\nname: coding\ndescription: Changing code that already exists.\n---\n\n# Changing code\n\nRead it first.\n",
        )
        .expect("a file");
        std::fs::create_dir_all(skills.join("web-search")).expect("a directory");
        std::fs::write(
            skills.join("web-search").join("SKILL.md"),
            "---\nname: web-search\ndescription: Search the web.\nrepo: https://github.com/someone/web-search\n---\n\n# Searching\n",
        )
        .expect("a file");
        // Installed and turned off, which is a move rather than a delete: it is
        // still on the list and still says what it is.
        std::fs::create_dir_all(agent::skills_off(&skills).join("noisy")).expect("a directory");
        std::fs::write(
            agent::skills_off(&skills).join("noisy").join("SKILL.md"),
            "---\nname: noisy\ndescription: Talks too much.\n---\n",
        )
        .expect("a file");

        let agent = Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, agent);
        go_to(&mut panel, SKILLS);
        let listed: Vec<&Entry> = panel
            .rows()
            .iter()
            .filter_map(|row| match row {
                Row::Entry(entry) => Some(entry),
                _ => None,
            })
            .collect();
        assert_eq!(
            listed
                .iter()
                .map(|entry| (entry.name.as_str(), entry.on))
                .collect::<Vec<_>>(),
            vec![
                ("coding  Changing code that already exists.", true),
                ("noisy  Talks too much.", false),
                ("web-search  Search the web.", true),
            ]
        );
        // Nothing the CLI writes records a repository, so a skill that names one
        // says it and every other one says where it was found.
        assert_eq!(listed[0].under, skills.join("coding").display().to_string());
        assert_eq!(
            listed[1].under,
            agent::skills_off(&skills).join("noisy").display().to_string(),
            "a skill that is off says where it went"
        );
        assert_eq!(listed[2].under, "https://github.com/someone/web-search");
        assert!(listed.iter().all(|entry| entry.removable));
        assert!(
            said(&panel).contains(&skills.display().to_string()),
            "the panel does not say where they live"
        );

        // The column beside the list is the skill under the cursor, and the
        // section opens on the first one rather than on an empty column.
        let showing = panel.showing().expect("something to show");
        assert_eq!(showing.name, "coding  Changing code that already exists.");
        assert_eq!(
            showing.doc,
            vec![
                String::from("# Changing code"),
                String::new(),
                String::from("Read it first."),
            ],
            "the front matter is not the document"
        );
        assert!(panel.step(true), "the cursor walks the entries");
        assert_eq!(panel.showing().expect("the next one").name, "noisy  Talks too much.");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The toggle on a skill's row is a move on the disk and back, and what the
    /// row says next comes from reading the disk again rather than from
    /// remembering what was pressed.
    #[test]
    fn turning_a_skill_off_moves_it_and_the_row_reads_the_disk_again() {
        let dir = scratch_dir("skill-toggle");
        let skills = dir.join("skills");
        std::fs::create_dir_all(skills.join("coding")).expect("a directory");
        std::fs::write(
            skills.join("coding").join("SKILL.md"),
            "---\nname: coding\ndescription: Change code.\n---\n\n# Changing code\n",
        )
        .expect("a file");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, read());
        go_to(&mut panel, SKILLS);
        let at = panel.cursor();
        assert!(matches!(panel.row(at), Some(Row::Entry(entry)) if entry.on));

        let deed = panel.toggle(at).expect("an entry toggles");
        assert_eq!(
            deed,
            Deed::TurnSkill {
                dir: String::from("coding"),
                on: false,
            }
        );
        // What `main` does with it, and then what the panel does with the disk.
        agent::set_skill(panel.skills_at().expect("a skills directory"), "coding", false)
            .expect("it moves");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, SKILLS);
        assert!(
            matches!(panel.row(panel.cursor()), Some(Row::Entry(entry)) if !entry.on),
            "the row still says it is on: {:?}",
            panel.row(panel.cursor())
        );
        assert!(!skills.join("coding").exists());

        let back = panel.toggle(panel.cursor()).expect("and back");
        assert_eq!(
            back,
            Deed::TurnSkill {
                dir: String::from("coding"),
                on: true,
            }
        );
        agent::set_skill(panel.skills_at().expect("a skills directory"), "coding", true)
            .expect("it comes back");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, SKILLS);
        assert!(matches!(panel.row(panel.cursor()), Some(Row::Entry(entry)) if entry.on));
        assert!(skills.join("coding/SKILL.md").is_file());

        // And the uninstall beside it takes two presses and names the same
        // directory: the only thing on this panel that cannot be undone.
        let at = panel.cursor();
        assert_eq!(panel.uninstall(at), None, "one press deleted a skill");
        assert_eq!(panel.arming(), Some(at));
        assert!(
            panel.says().contains("press uninstall again"),
            "the panel does not say what is about to go: {}",
            panel.says()
        );
        assert_eq!(
            panel.uninstall(at),
            Some(Deed::RemoveSkill {
                dir: String::from("coding"),
                on: true,
            })
        );
        assert_eq!(panel.arming(), None, "it stayed armed after it fired");

        // And anything else at all disarms it, so a button pressed once and
        // walked away from cannot be finished off by the next press that lands.
        assert_eq!(panel.uninstall(at), None);
        assert!(panel.step(true) || panel.arming().is_none());
        assert_eq!(panel.arming(), None, "a key left it armed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A server's row toggles the same way, in its own file, and a server that
    /// is off is still a row rather than a server that disappeared.
    #[test]
    fn turning_a_server_off_moves_its_entry_in_its_own_file() {
        let dir = scratch_dir("server-toggle");
        std::fs::write(
            dir.join("mcp.json"),
            r#"{"servers": {"docs": {"url": "http://localhost:9000/mcp"}}}"#,
        )
        .expect("a file");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, read());
        go_to(&mut panel, MCP);
        let at = panel.cursor();
        assert!(
            matches!(panel.row(at), Some(Row::Entry(entry)) if entry.on && entry.removable),
            "{:?}",
            panel.row(at)
        );
        // The entry itself is what the column beside the list shows.
        let showing = panel.showing().expect("something to show");
        assert!(showing.doc.iter().any(|line| line.contains("localhost:9000")), "{:?}", showing.doc);

        let deed = panel.toggle(at).expect("an entry toggles");
        assert_eq!(
            deed,
            Deed::TurnServer {
                name: String::from("docs"),
                project: false,
                on: false,
            }
        );
        let file = panel.mcp_file(false).expect("a global file").to_path_buf();
        assert_eq!(file, dir.join("mcp.json"));
        agent::set_server(&file, "docs", false).expect("it moves");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, MCP);
        assert!(
            matches!(panel.row(panel.cursor()), Some(Row::Entry(entry)) if !entry.on),
            "{:?}",
            panel.row(panel.cursor())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The uninstall a skill has is on a server's row too, takes two presses,
    /// names the file rather than a directory, and the row is gone once the deed
    /// has been done.
    ///
    /// This used to assert the opposite: that a server had no uninstall and
    /// `uninstall` answered nothing on its row. It is written the other way
    /// round now because the button is there, which is the whole of this change.
    #[test]
    fn a_server_is_uninstalled_out_of_its_file_and_leaves_the_list() {
        let dir = scratch_dir("server-remove");
        std::fs::write(
            dir.join("mcp.json"),
            r#"{"servers": {"docs": {"url": "http://localhost:9000/mcp"}, "shell": {"command": "mcp-shell"}}, "timeout_s": 30}"#,
        )
        .expect("a file");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, read());
        go_to(&mut panel, MCP);
        // The rows are sorted by name, so the first entry is "docs".
        let at = panel.cursor();
        assert!(
            matches!(panel.row(at), Some(Row::Entry(entry)) if entry.name == "docs" && entry.removable),
            "{:?}",
            panel.row(at)
        );

        // One press arms it and says what would go, in the words of what would
        // actually happen: nothing about a directory.
        assert_eq!(panel.uninstall(at), None, "one press removed a server");
        assert_eq!(panel.arming(), Some(at));
        let said = panel.says();
        assert!(said.contains("press uninstall again"), "{said}");
        assert!(said.contains("docs") && said.contains("mcp.json"), "{said}");
        assert!(!said.contains("directory"), "a server has no directory: {said}");

        let deed = panel.uninstall(at).expect("the second press is the deed");
        assert_eq!(
            deed,
            Deed::RemoveServer {
                name: String::from("docs"),
                project: false,
            }
        );
        assert_eq!(panel.arming(), None, "it stayed armed after it fired");

        // What `main` does with it, and then what the panel does with the disk.
        let file = panel.mcp_file(false).expect("a global file").to_path_buf();
        agent::remove_server(&file, "docs").expect("it goes");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, MCP);
        let names: Vec<String> = panel
            .here()
            .rows
            .iter()
            .filter_map(|row| match row {
                Row::Entry(entry) => Some(entry.name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec![String::from("shell")], "the row is still listed");
        // And nothing else in the file went with it.
        let text = std::fs::read_to_string(&file).expect("the file");
        let root: serde_json::Value = serde_json::from_str(&text).expect("still JSON");
        assert_eq!(root["timeout_s"], 30, "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A server that was turned off first still uninstalls, out of the key it
    /// was moved to, and the file it was in keeps everything else.
    #[test]
    fn a_server_that_is_off_uninstalls_too() {
        let dir = scratch_dir("server-remove-off");
        std::fs::write(
            dir.join("mcp.json"),
            r#"{"servers": {"docs": {"url": "http://localhost:9000/mcp"}, "shell": {"command": "mcp-shell"}}}"#,
        )
        .expect("a file");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, read());
        let file = panel.mcp_file(false).expect("a global file").to_path_buf();
        agent::set_server(&file, "docs", false).expect("it moves out of the way");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, MCP);
        let at = panel.cursor();
        assert!(
            matches!(panel.row(at), Some(Row::Entry(entry)) if entry.name == "docs" && !entry.on && entry.removable),
            "{:?}",
            panel.row(at)
        );

        assert_eq!(panel.uninstall(at), None);
        assert_eq!(
            panel.uninstall(at),
            Some(Deed::RemoveServer {
                name: String::from("docs"),
                project: false,
            }),
            "a server that is off asks for a different deed"
        );
        agent::remove_server(&file, "docs").expect("an off server goes too");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, MCP);
        assert!(
            !panel
                .here()
                .rows
                .iter()
                .any(|row| matches!(row, Row::Entry(entry) if entry.name == "docs")),
            "the row came back"
        );
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).expect("the file"))
                .expect("still JSON");
        assert!(root["servers"]["shell"]["command"].is_string());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A removal that fails leaves the file and the row exactly as they were,
    /// and says why on the footer.
    #[test]
    fn a_failed_removal_leaves_the_file_and_the_row_where_they_were() {
        let dir = scratch_dir("server-remove-fails");
        let path = dir.join("mcp.json");
        let whole = "{\"servers\": {\"docs\": {\"url\": \"http://localhost:9000/mcp\"}}}";
        std::fs::write(&path, whole).expect("a file");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, read());
        go_to(&mut panel, MCP);
        let at = panel.cursor();
        assert_eq!(panel.uninstall(at), None);
        let deed = panel.uninstall(at).expect("the deed");

        // Somebody edited the file to something that will not parse in between
        // the press and the write, which is the shape every failure here has:
        // the removal refuses and the file it refused on is untouched.
        std::fs::write(&path, "{\"servers\": {\"docs\":").expect("a file");
        let Deed::RemoveServer { name, project } = &deed else {
            panic!("{deed:?}");
        };
        let file = panel.mcp_file(*project).expect("a global file").to_path_buf();
        let why = agent::remove_server(&file, name).expect_err("it cannot be done");
        assert!(why.contains("not valid JSON"), "{why}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("the file"),
            "{\"servers\": {\"docs\":",
            "the refusal rewrote the file"
        );

        // The panel says why and the row it was on is still the row it was on.
        panel.say_trouble(why);
        assert!(
            panel.trouble().is_some_and(|why| why.contains("not valid JSON")),
            "{:?}",
            panel.trouble()
        );
        assert!(
            matches!(panel.row(at), Some(Row::Entry(entry)) if entry.name == "docs"),
            "{:?}",
            panel.row(at)
        );
        assert_eq!(panel.arming(), None, "it stayed armed after it fired");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
