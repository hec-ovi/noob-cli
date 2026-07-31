//! The settings panel: eight sections, what each one carries, and what changing
//! a row writes back.
//!
//! A full screen takeover rather than a popup or a second OS window. A second
//! window means a second wgpu surface with its own renderer and its own event
//! routing, for a list of rows; a popup over the panes means a scroll region
//! floating over another scroll region. While the panel is up it is the whole
//! window, the way the folder picker is before a folder has been chosen.
//!
//! **Sections, not one scroll.** The panel was a single list sixty rows long:
//! the all-time totals, four unlabelled groups of settings, then forty six
//! colours, with nothing above it saying where you were and nothing on it about
//! the agent the window is a front end for. It is a rail of section names now,
//! with the chosen section's rows beside it, and each section is short enough to
//! read at a glance. Four of them ([`AGENT`], [`SESSIONS`], [`SKILLS`], [`MCP`])
//! are the agent's own files rather than the window's, read through
//! [`crate::agent`]; the other four are the window's settings file.
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
//! quietly disagreeing. A slider being dragged is the one exception and says so:
//! it carries a preview until the button comes up, because writing the file on
//! every motion event is hundreds of writes for one decision.
//!
//! The colours are listed and not editable here. Changing one means typing a hex
//! value into a field, and the one field this window has is the agent's
//! endpoint; forty six of them is a form. So the palette is on the panel as
//! swatches you can read, with the path of the file to edit beside them, and the
//! four presets are one row away in `theme`.

use std::path::{Path, PathBuf};

use crate::agent::{self, Agent};
use crate::config::{self, Config};
use crate::totals::Totals;

/// The sections, in the order the rail lists them: the agent first, because
/// what the window is a front end for matters more than what colour it is.
pub const AGENT: &str = "AGENT";
pub const SESSIONS: &str = "SESSIONS";
pub const SKILLS: &str = "SKILLS";
pub const MCP: &str = "MCP";
pub const APPEARANCE: &str = "APPEARANCE";
pub const PANES: &str = "PANES";
pub const COLOURS: &str = "COLOURS";
pub const ALL_TIME: &str = "ALL TIME";

/// Every section name, in rail order.
pub const SECTIONS: [&str; 8] = [
    AGENT,
    SESSIONS,
    SKILLS,
    MCP,
    APPEARANCE,
    PANES,
    COLOURS,
    ALL_TIME,
];

/// What a setting holds, which is what decides how its row changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// On or off. Enter flips it, and so does a nudge either way: there is
    /// nowhere else for a flag to go.
    Flag,
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
    /// A colour, carried so the row can draw a swatch of it. Read only here.
    Colour([u8; 3]),
}

impl Kind {
    /// Whether a row of this kind can be changed from the panel, which is also
    /// whether the cursor stops on it. A cursor that lands where nothing can
    /// happen is a dead stop the arrow keys have to be pressed through.
    pub fn changes(self) -> bool {
        !matches!(self, Kind::Colour(_))
    }

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
    /// Something read out rather than set: the all-time totals, what the agent's
    /// own file says, and where the files behind all this live.
    Reading { label: String, value: String },
    /// A setting in the window's own file, spelled the way that file spells it.
    Setting {
        key: &'static str,
        value: String,
        kind: Kind,
    },
    /// A line of text in the agent's file, edited by typing. The endpoint, and
    /// nothing else: it is the one setting here whose value is not a number, a
    /// flag or a name from a list.
    Field { key: &'static str, value: String },
}

/// What a nudge on the row under the cursor should write.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub key: &'static str,
    pub value: String,
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
        "max_input_rows",
        Kind::Number {
            step: 1.0,
            low: 1.0,
            high: 24.0,
            places: 0,
        },
    ),
];

/// Which panes open.
const PANE_SETTINGS: [(&str, Kind); 2] =
    [("show_activity", Kind::Flag), ("show_files", Kind::Flag)];

/// The same four numbers the dividers write when they are dragged. Here as well
/// because a pointer is not the only way anyone works, and because a value that
/// only a drag can reach is a value nobody can read off the window.
///
/// All four whichever way the grid is reading. One line always runs the whole
/// way across, so one of these is not on screen as a line at any given moment;
/// it is still the number that line will take when the grid turns round, and a
/// row that disappeared and came back would read as a setting that comes and
/// goes.
const DIVIDERS: [(&str, Kind); 4] = [
    (
        "left_width",
        Kind::Number {
            step: 0.05,
            low: config::SPLIT_LOW,
            high: config::SPLIT_HIGH,
            places: 2,
        },
    ),
    (
        "left_width_bottom",
        Kind::Number {
            step: 0.05,
            low: config::SPLIT_LOW,
            high: config::SPLIT_HIGH,
            places: 2,
        },
    ),
    (
        "top_height",
        Kind::Number {
            step: 0.05,
            low: config::SPLIT_LOW,
            high: config::SPLIT_HIGH,
            places: 2,
        },
    ),
    (
        "top_height_right",
        Kind::Number {
            step: 0.05,
            low: config::SPLIT_LOW,
            high: config::SPLIT_HIGH,
            places: 2,
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
}

impl Section {
    fn new(name: &'static str, rows: Vec<Row>) -> Section {
        let cursor = rows.iter().position(landable).unwrap_or(0);
        Section {
            name,
            rows,
            cursor,
            first: 0,
        }
    }
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
    /// The row a slider is being dragged on and the value it is being dragged
    /// to. Nothing is written until the button comes up.
    dragging: Option<(usize, String)>,
    /// The settings file, or nothing when there is no home directory to put one
    /// in. Kept so [`Settings::refresh`] does not have to be handed it again.
    file: Option<PathBuf>,
    /// What the agent's own files said when the panel opened.
    agent: Agent,
    /// Why the last change did not land. Cleared by the next refresh, since a
    /// refresh only happens after a write that worked.
    trouble: Option<String>,
}

impl Settings {
    /// Open the panel over the settings as they are now.
    ///
    /// `totals` is the all-time file with this session already added in, which
    /// is the caller's job: the file on disk holds the sessions that came before
    /// and adding the live one twice is exactly the bug the totals module is
    /// written to avoid. `agent` is a snapshot of the CLI's own files, read once
    /// here rather than on every frame.
    pub fn open(config: &Config, totals: &Totals, file: Option<&Path>, agent: Agent) -> Settings {
        let mut panel = Settings {
            sections: Vec::new(),
            chosen: 0,
            focus: Focus::Rail,
            editing: None,
            dragging: None,
            file: file.map(PathBuf::from),
            agent,
            trouble: None,
        };
        panel.sections = panel.build(config, totals);
        panel
    }

    /// Rebuild the rows from the files as they now read, keeping the cursor
    /// where it was. Called after a change has been written and read back.
    pub fn refresh(&mut self, config: &Config, totals: &Totals) {
        let places: Vec<(usize, usize)> = self
            .sections
            .iter()
            .map(|section| (section.cursor, section.first))
            .collect();
        self.sections = self.build(config, totals);
        self.trouble = None;
        self.dragging = None;
        for (section, (cursor, first)) in self.sections.iter_mut().zip(places) {
            let last = section.rows.len().saturating_sub(1);
            section.first = first.min(last);
            section.cursor = cursor.min(last);
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
    pub fn adopt_agent(&mut self, agent: Agent, config: &Config, totals: &Totals) {
        self.agent = agent;
        self.refresh(config, totals);
    }

    /// One section per name on the rail, in rail order. Driven off [`SECTIONS`]
    /// so the rail and what is behind it cannot come apart: a name with no rows
    /// would be a section that opens on nothing.
    fn build(&self, config: &Config, totals: &Totals) -> Vec<Section> {
        SECTIONS
            .into_iter()
            .map(|name| {
                let rows = match name {
                    AGENT => self.agent_rows(),
                    SESSIONS => self.session_rows(),
                    SKILLS => self.skill_rows(),
                    MCP => self.mcp_rows(),
                    APPEARANCE => settings_rows(config, &LOOKS),
                    PANES => pane_rows(config),
                    COLOURS => self.colour_rows(config),
                    _ => all_time_rows(totals),
                };
                Section::new(name, rows)
            })
            .collect()
    }

    /// What the agent is pointed at, out of the file the CLI owns.
    fn agent_rows(&self) -> Vec<Row> {
        let mut rows = vec![note(
            "the CLI reads this file on every request, so a change here lands on the next one",
        )];
        rows.push(Row::Reading {
            label: String::from("file"),
            value: match (&self.agent.env_path, self.agent.env_exists) {
                // Not there yet is worth saying: an agent configured entirely by
                // environment has no file at all, and the first save writes one.
                (Some(path), false) => format!("{} (not there yet)", path.display()),
                (Some(path), true) => path.display().to_string(),
                (None, _) => String::from("nowhere: no config directory to read one in"),
            },
        });
        rows.push(Row::Field {
            key: agent::ENDPOINT,
            value: self.agent.endpoint().unwrap_or_default().to_string(),
        });
        if self.agent.endpoint().is_none() {
            rows.push(note(
                "with no endpoint set, noob probes the usual local ports and takes the first that answers",
            ));
        }
        for (key, value) in &self.agent.env {
            if key == agent::ENDPOINT {
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
        rows.push(note(
            "api keys are not shown or written here: edit the file to change one",
        ));
        rows
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

    /// What is installed under the agent's `skills/`.
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
        }
        for skill in &self.agent.skills {
            rows.push(Row::Item(skill_line(skill)));
        }
        rows
    }

    /// The MCP servers, out of the two files the CLI merges.
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
            rows.push(Row::Item(server_line(server)));
        }
        for why in &mcp.trouble {
            rows.push(Row::Note {
                text: why.clone(),
                bad: true,
            });
        }
        if !mcp.servers.is_empty() {
            rows.push(note("the project file wins for a server named in both"));
        }
        rows
    }

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
            for (key, rgb) in all.iter().skip(at).take(count) {
                rows.push(Row::Setting {
                    key,
                    value: hex(*rgb),
                    kind: Kind::Colour(*rgb),
                });
            }
            at += count;
        }
        rows.push(note(
            "colours are read here and edited in the file: a hex value needs a keyboard this window has nowhere to put",
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
    #[cfg(test)]
    pub fn all_rows(&self) -> impl Iterator<Item = (&'static str, &Row)> {
        self.sections
            .iter()
            .flat_map(|section| section.rows.iter().map(move |row| (section.name, row)))
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

    /// Whether the cursor is on a row at all: a section of readings has nothing
    /// to land on, and a band drawn on a row nothing can be done to is a lie.
    pub fn on_row(&self) -> bool {
        self.focus == Focus::Content && self.row(self.cursor()).is_some_and(landable)
    }

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
    pub fn preview(&self, index: usize) -> Option<&str> {
        self.dragging
            .as_ref()
            .filter(|(at, _)| *at == index)
            .map(|(_, value)| value.as_str())
    }

    /// What the keys under the cursor do, spelled out for the footer. The panel
    /// is the one surface in this window where there is nothing to experiment on
    /// safely, so it says what a key will do before it is pressed.
    pub fn hint(&self) -> &'static str {
        if self.editing.is_some() {
            return "type it \u{2022} enter saves it \u{2022} esc leaves it alone";
        }
        if self.focus == Focus::Rail {
            return "up and down choose a section \u{2022} right goes in \u{2022} esc closes";
        }
        match self.row(self.cursor()) {
            Some(Row::Setting { kind, .. }) => match kind {
                Kind::Flag => "enter or left and right turn it on and off",
                Kind::Choice(_) => "left and right walk the presets",
                Kind::Number { .. } => "left and right nudge it, or drag the slider",
                Kind::Colour(_) => "colours are edited in the file",
            },
            Some(Row::Field { .. }) => "enter edits it \u{2022} left goes back to the sections",
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
        let Row::Setting { key, value, kind } = self.row(self.cursor())? else {
            return None;
        };
        let next = match kind {
            Kind::Colour(_) => return None,
            // Either direction, and Enter comes through here too. A flag has two
            // states, so "the next one" and "the one before" are the same one.
            Kind::Flag => match value.as_str() {
                "on" => "off",
                _ => "on",
            }
            .to_string(),
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
        Some(Change { key, value: next })
    }

    /// Where along its track the value of a row sits, for drawing the thumb.
    /// Nothing for a row that is not a slider.
    pub fn fraction(&self, index: usize) -> Option<f32> {
        let Row::Setting { value, kind, .. } = self.row(index)? else {
            return None;
        };
        let value = self.preview(index).unwrap_or(value);
        kind.fraction(value.parse::<f32>().ok()?)
    }

    /// Drag the slider on one row to a position along its track.
    ///
    /// The value is held here and not written: a drag across a window is
    /// hundreds of motion events, and writing the settings file at each one is
    /// hundreds of rename-over-the-file writes for one decision. The cursor
    /// follows the drag, so letting go and pressing an arrow key carries on from
    /// where the slider was left.
    pub fn slide(&mut self, index: usize, fraction: f32) -> bool {
        let Some(Row::Setting { kind, .. }) = self.row(index) else {
            return false;
        };
        let Some(next) = kind.at(fraction) else {
            return false;
        };
        self.focus = Focus::Content;
        self.here_mut().cursor = index;
        let moved = self.preview(index) != Some(next.as_str());
        self.dragging = Some((index, next));
        moved
    }

    /// The button came up: what the drag decided, or nothing when it decided
    /// what the file already said.
    pub fn drop_slider(&mut self) -> Option<Change> {
        let (index, value) = self.dragging.take()?;
        let Some(Row::Setting { key, value: was, .. }) = self.row(index) else {
            return None;
        };
        if *was == value {
            return None;
        }
        Some(Change { key, value })
    }

    /// Start typing into the field under the cursor, from what it says now.
    pub fn edit(&mut self) -> bool {
        if self.editing.is_some() {
            return false;
        }
        let Some(Row::Field { value, .. }) = self.row(self.cursor()) else {
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
        let Some(Row::Field { key, .. }) = self.row(self.cursor()) else {
            return None;
        };
        Some((key, typed))
    }

    /// Where the agent's own file is, for the write the field asks for.
    pub fn agent_file(&self) -> Option<&Path> {
        self.agent.env_path.as_deref()
    }

    /// Move the cursor one row, over anything it cannot land on, or the rail one
    /// section. Clamped at both ends: a list that wraps under an arrow key held
    /// down is a cursor that arrives somewhere nobody was looking.
    pub fn step(&mut self, down: bool) -> bool {
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
        if self.focus == Focus::Rail {
            return self.step(down);
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
    pub fn point_at(&mut self, index: usize) -> bool {
        if !self.row(index).is_some_and(landable) {
            return false;
        }
        let was = (self.here().cursor, self.focus);
        self.focus = Focus::Content;
        self.here_mut().cursor = index;
        (index, Focus::Content) != was
    }

    /// One row per entry, for the scroll window. Every row is one row: a value
    /// too long for the panel is clipped rather than wrapped, so a click cannot
    /// resolve to a setting other than the one under the pointer.
    pub fn heights(&self) -> Vec<usize> {
        text_geometry::heights(self.here().rows.iter().map(|_| 0), 1)
    }

    /// Bring the cursor on screen, for a `rows` tall list.
    pub fn reveal(&mut self, rows: usize) -> bool {
        if rows == 0 || self.here().rows.is_empty() {
            return false;
        }
        let most = text_geometry::max_scrollback(&self.heights(), rows);
        let section = self.here_mut();
        let mut next = section.first.min(section.cursor);
        if section.cursor + 1 > next + rows {
            next = section.cursor + 1 - rows;
        }
        let next = next.min(most);
        let moved = next != section.first;
        section.first = next;
        moved
    }

    /// Move the window without moving the cursor, for the wheel.
    pub fn scroll(&mut self, by: usize, down: bool, rows: usize) -> bool {
        let most = text_geometry::max_scrollback(&self.heights(), rows);
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
        let back = text_geometry::scrollback_for(&heights, rows, self.here().first);
        text_geometry::thumb(&heights, rows, back)
    }
}

fn note(text: &str) -> Row {
    Row::Note {
        text: String::from(text),
        bad: false,
    }
}

/// The rows for a group of the window's own settings, read off the config.
fn settings_rows(config: &Config, group: &[(&'static str, Kind)]) -> Vec<Row> {
    group
        .iter()
        .map(|(key, kind)| Row::Setting {
            key,
            value: value_of(config, key, *kind),
            kind: *kind,
        })
        .collect()
}

/// Which panes open, and where the dividers between them sit.
fn pane_rows(config: &Config) -> Vec<Row> {
    let mut rows = vec![Row::Heading("WHICH PANES OPEN")];
    rows.extend(settings_rows(config, &PANE_SETTINGS));
    rows.push(Row::Heading("WHERE THE DIVIDERS SIT"));
    rows.extend(settings_rows(config, &DIVIDERS));
    rows
}

/// The all-time block: the one place these numbers are shown.
///
/// They had a pane of their own, which read as this session's spend because
/// nothing on it said otherwise; under a section called ALL TIME they mean what
/// they are.
fn all_time_rows(totals: &Totals) -> Vec<Row> {
    let reading = |label: &str, value: String| Row::Reading {
        label: String::from(label),
        value,
    };
    vec![
        reading("prefilled", grouped(totals.prefilled)),
        reading("generated", grouped(totals.generated)),
        reading("from cache", grouped(totals.cached)),
        reading(
            "prefill",
            rates(totals.average_prefill(), totals.median_prefill()),
        ),
        reading(
            "decode",
            rates(totals.average_decode(), totals.median_decode()),
        ),
    ]
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

fn landable(row: &Row) -> bool {
    match row {
        Row::Setting { kind, .. } => kind.changes(),
        Row::Field { .. } => true,
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
    let flag = |on: bool| String::from(if on { "on" } else { "off" });
    match (key, kind) {
        ("theme", _) => theme_name(config).to_string(),
        ("show_activity", _) => flag(config.show_activity),
        ("show_files", _) => flag(config.show_files),
        ("max_input_rows", _) => config.max_input_rows.to_string(),
        (_, Kind::Number { places, .. }) => {
            let value = match key {
                "opacity" => config.opacity,
                "font_size" => config.font_size,
                "pane_font_size" => config.pane_font_size,
                "left_width" => config.left_width,
                "left_width_bottom" => config.left_width_bottom,
                "top_height" => config.top_height,
                "top_height_right" => config.top_height_right,
                // Unreachable through the groups, and a number is the honest
                // answer for a row that says it is one.
                _ => 0.0,
            };
            format!("{value:.places$}")
        }
        (_, Kind::Colour(rgb)) => hex(rgb),
        // Same: every key in the groups is answered above.
        _ => String::new(),
    }
}

/// `#rrggbb`, which is what the file wants back.
fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// A count with its thousands split up. Eight digits of tokens in a row is a
/// number nobody reads, and this panel is the only place the all-time counts
/// are shown.
fn grouped(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(digit);
    }
    out
}

/// The two readings a rate has, said in one row.
///
/// Both, because they answer different questions: one cold start with a huge
/// transcript drags the average down for the rest of the day, and the median is
/// what a typical request actually did. A run with no requests in it yet has
/// neither, and a row of zeros would read as a machine doing nothing.
fn rates(average: f64, median: f64) -> String {
    if average <= 0.0 && median <= 0.0 {
        return String::from("nothing measured yet");
    }
    format!("{average:.0} mean, {median:.0} median tok/s")
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
/// file back rather than trusting what was typed.
pub fn write_endpoint(path: &Path, key: &str, value: &str) -> Result<(), String> {
    agent::write_env(path, key, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn over(config: &Config) -> Settings {
        Settings::open(
            config,
            &Totals::default(),
            Some(Path::new("/tmp/no0b.conf")),
            Agent::default(),
        )
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

    /// Put the cursor on a setting wherever it lives, section and all.
    fn put_cursor(panel: &mut Settings, key: &str) {
        let section = panel
            .all_rows()
            .find(|(_, row)| matches!(row, Row::Setting { key: k, .. } if *k == key))
            .map(|(section, _)| section)
            .unwrap_or_else(|| panic!("{key} is not on the panel"));
        go_to(panel, section);
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Setting { key: k, .. } if *k == key))
            .expect("the row is in the section it was found in");
        assert!(
            panel.point_at(at) || panel.cursor() == at,
            "{key} cannot hold the cursor"
        );
    }

    /// Everything a section says, as one string, for the tests that care what is
    /// on it rather than which row it is on.
    fn said(panel: &Settings) -> String {
        panel
            .rows()
            .iter()
            .map(|row| match row {
                Row::Note { text, .. } | Row::Item(text) => text.clone(),
                Row::Reading { label, value } => format!("{label} {value}"),
                Row::Setting { key, value, .. } | Row::Field { key, value } => {
                    format!("{key} {value}")
                }
                Row::Heading(name) => String::from(*name),
            })
            .collect::<Vec<_>>()
            .join("\n")
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
        go_to(&mut panel, PANES);
        assert_ne!(panel.cursor(), was, "the two sections share a cursor");
        go_to(&mut panel, APPEARANCE);
        assert_eq!(panel.cursor(), was);
    }

    /// Every key the file understands is on the panel exactly once, in one
    /// section or another, as a row that changes or as a swatch.
    ///
    /// This is the test that stops the panel going stale. A setting added to
    /// `config::keys` and in none of the sections is a line in everybody's file
    /// with no way to reach it from the window, and nothing else would say so.
    #[test]
    fn every_key_in_the_file_is_on_the_panel() {
        let config = Config::default();
        let panel = over(&config);
        let mut on_panel: Vec<&str> = panel
            .all_rows()
            .filter_map(|(_, row)| match row {
                Row::Setting { key, .. } => Some(*key),
                _ => None,
            })
            .collect();
        let mut known = config::keys();
        on_panel.sort_unstable();
        known.sort_unstable();
        assert_eq!(on_panel, known);

        // And nothing retired sneaked in with them: those keys are dead in the
        // file, and the writer refuses them, so a row for one would be a row
        // that can only fail.
        for retired in config::RETIRED {
            assert!(
                !on_panel.contains(&retired),
                "{retired} is retired and on the panel"
            );
        }

        // The colours are all in one section instead of in the middle of
        // everything else, and each setting is in the section that names it.
        for (section, row) in panel.all_rows() {
            let Row::Setting { key, kind, .. } = row else {
                continue;
            };
            let wanted = match kind {
                Kind::Colour(_) => COLOURS,
                _ if LOOKS.iter().any(|(name, _)| name == key) => APPEARANCE,
                _ => PANES,
            };
            assert_eq!(section, wanted, "{key} is in the wrong section");
        }
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

        // The settings sections stop on their own settings and nothing else.
        go_to(&mut panel, APPEARANCE);
        let mut looks = 1;
        while panel.step(true) {
            looks += 1;
        }
        assert_eq!(looks, LOOKS.len());
        go_to(&mut panel, PANES);
        let mut panes = 1;
        while panel.step(true) {
            panes += 1;
        }
        assert_eq!(panes, PANE_SETTINGS.len() + DIVIDERS.len());

        // A section of readings has nothing to land on and says so, rather than
        // drawing a band on a row nothing can be done to.
        go_to(&mut panel, ALL_TIME);
        assert!(!panel.on_row());
        go_to(&mut panel, APPEARANCE);
        assert!(panel.on_row());
        // And nothing lands while the keyboard is on the rail.
        panel.leave();
        assert!(!panel.on_row());
    }

    /// A flag says on or off, flips either way, and Enter is a nudge forward.
    #[test]
    fn a_flag_flips_and_writes_what_the_file_reads() {
        let config = Config::default();
        let mut panel = over(&config);
        put_cursor(&mut panel, "show_files");
        assert_eq!(value(&panel, "show_files"), "on");

        let change = panel.change(true).expect("a flag changes");
        assert_eq!(change.key, "show_files");
        assert_eq!(change.value, "off");
        // Both directions, because a flag has nowhere else to be.
        assert_eq!(panel.change(false).expect("either way").value, "off");
        // And the file agrees the value means what the row says.
        assert!(!Config::parse("show_files = off").show_files);

        // The row still reads what the file says: nothing was applied here.
        assert_eq!(value(&panel, "show_files"), "on");

        // Nothing changes from the rail: the arrow keys there walk the sections,
        // and one of them must not also write a setting.
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
            .chain(DIVIDERS.iter())
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
        // Nothing else is a slider. A flag or a preset drawn as a track would be
        // a control whose middle means nothing.
        assert_eq!(Kind::Flag.at(0.5), None);
        assert_eq!(Kind::Choice(&config::THEMES).fraction(1.0), None);
        assert_eq!(Kind::Colour([1, 2, 3]).at(0.5), None);
    }

    /// A drag holds its value until the button comes up, and only then says what
    /// to write. A drag that ended where it started writes nothing.
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
        assert_eq!(panel.fraction(at), opacity.fraction(0.5));

        assert!(panel.slide(at, 1.0));
        assert_eq!(panel.preview(at), Some("1.00"));
        assert_eq!(
            value(&panel, "opacity"),
            "0.50",
            "the row said what the file did not"
        );
        assert_eq!(panel.fraction(at), Some(1.0), "the thumb follows the drag");
        assert!(!panel.slide(at, 1.0), "the same place is not a change");

        let change = panel.drop_slider().expect("the drag decided something");
        assert_eq!(change.key, "opacity");
        assert_eq!(change.value, "1.00");
        assert_eq!(panel.preview(at), None, "the preview outlived the drag");

        // A drag that ends on the value the file already has writes nothing: a
        // press on the thumb must not rewrite the file.
        assert!(panel.slide(at, 0.5));
        assert_eq!(panel.drop_slider(), None);
        assert_eq!(panel.drop_slider(), None, "and there is nothing to drop");

        // A row that is not a number has no track at all.
        put_cursor(&mut panel, "show_files");
        assert!(!panel.slide(panel.cursor(), 0.5));
        assert_eq!(panel.fraction(panel.cursor()), None);
    }

    /// A colour is on the panel to be read, and reading is all.
    #[test]
    fn a_colour_row_carries_its_swatch_and_cannot_be_changed() {
        let config = Config::parse("accent = #123456");
        let mut panel = over(&config);
        assert_eq!(
            *setting(&panel, "accent"),
            Row::Setting {
                key: "accent",
                value: String::from("#123456"),
                kind: Kind::Colour([0x12, 0x34, 0x56]),
            }
        );
        // The cursor cannot get there, so no change can be aimed at it.
        go_to(&mut panel, COLOURS);
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Setting { key, .. } if *key == "accent"))
            .expect("the accent row");
        assert!(!panel.point_at(at));
        assert_ne!(panel.cursor(), at);
        // And the section says why, with the file to edit instead.
        let text = said(&panel);
        assert!(text.contains("edited in the file"), "{text}");
        assert!(text.contains("no0b.conf"), "{text}");
    }

    /// The all-time block is the totals file, with the thousands split so the
    /// counts can be read, and says so plainly before there is anything to say.
    #[test]
    fn the_all_time_block_reads_the_totals_it_was_handed() {
        let mut totals = Totals {
            prefilled: 12_345_678,
            generated: 9_012,
            cached: 500,
            decode_tokens: 300,
            decode_seconds: 10.0,
            decode_rates: vec![10.0, 40.0, 90.0],
            ..Totals::default()
        };
        totals.prefill_rates = vec![100.0];
        totals.prefill_tokens = 100;
        totals.prefill_seconds = 1.0;
        let panel = Settings::open(&Config::default(), &totals, None, Agent::default());
        let readings: Vec<(String, String)> = panel
            .all_rows()
            .filter_map(|(_, row)| match row {
                Row::Reading { label, value } => Some((label.clone(), value.clone())),
                _ => None,
            })
            .collect();
        let has = |label: &str, value: &str| readings.iter().any(|(l, v)| l == label && v == value);
        assert!(has("prefilled", "12 345 678"), "{readings:?}");
        assert!(has("generated", "9 012"), "{readings:?}");
        // 300 tokens in 10 seconds is 30 a second on average; the middle of the
        // three samples is 40.
        assert!(has("decode", "30 mean, 40 median tok/s"), "{readings:?}");
        // No home directory: the panel still opens and says why nothing can be
        // saved rather than pretending there is a file.
        let where_ = readings
            .iter()
            .find(|(label, _)| label == "settings")
            .expect("the file row");
        assert!(where_.1.contains("no home directory"), "{where_:?}");

        let empty = Settings::open(&Config::default(), &Totals::default(), None, Agent::default());
        assert!(
            empty.all_rows().any(|(_, row)| matches!(
                row,
                Row::Reading { label, value }
                    if label == "prefill" && value == "nothing measured yet"
            )),
            "a machine with no requests behind it reads as zeros"
        );
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
        let mut panel = Settings::open(&config, &Totals::default(), Some(&path), Agent::default());

        for key in [
            "opacity",
            "font_size",
            "pane_font_size",
            "max_input_rows",
            "left_width",
            "left_width_bottom",
            "top_height",
            "top_height_right",
        ] {
            for forward in [false, true] {
                put_cursor(&mut panel, key);
                // Walk to the end of the range, which is where a bound that
                // disagreed with the parser would show up.
                let mut wrote = None;
                while let Some(change) = panel.change(forward) {
                    config = commit(&path, &change).expect("the file takes it");
                    panel.refresh(&config, &Totals::default());
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

    /// A flag and a preset go through the same round trip, and the panel keeps
    /// its place across it.
    #[test]
    fn a_change_lands_in_the_file_and_the_cursor_stays_put() {
        let path = scratch("lands");
        let _ = std::fs::remove_file(&path);
        let mut config = Config::load_from(&path);
        let mut panel = Settings::open(&config, &Totals::default(), Some(&path), Agent::default());
        put_cursor(&mut panel, "show_activity");
        let was = (panel.chosen(), panel.cursor());

        let change = panel.change(true).expect("a flag");
        config = commit(&path, &change).expect("the file takes it");
        assert!(!config.show_activity);
        panel.refresh(&config, &Totals::default());
        assert_eq!(value(&panel, "show_activity"), "off");
        assert_eq!(
            (panel.chosen(), panel.cursor()),
            was,
            "the cursor moved under the change"
        );

        put_cursor(&mut panel, "theme");
        let change = panel.change(true).expect("a preset");
        config = commit(&path, &change).expect("the file takes it");
        panel.refresh(&config, &Totals::default());
        assert_eq!(value(&panel, "theme"), change.value);
        // The preset reached the palette, not just the theme row.
        assert_eq!(
            colours(&config),
            colours(&config::theme(&change.value).expect("a preset"))
        );
        // And the flag written before it survived the second write.
        assert_eq!(value(&panel, "show_activity"), "off");
        let _ = std::fs::remove_file(&path);
    }

    /// A write that cannot happen is said out loud and changes nothing.
    #[test]
    fn a_write_that_fails_leaves_the_row_alone() {
        let config = Config::default();
        let mut panel = over(&config);
        put_cursor(&mut panel, "show_files");
        let change = panel.change(true).expect("a flag");

        // A directory where the file should be: the write cannot land.
        let path = scratch("refused");
        let _ = std::fs::remove_file(&path);
        std::fs::create_dir_all(&path).expect("a directory in the way");
        assert!(commit(&path, &change).is_err());

        panel.say_trouble(String::from("cannot write it"));
        assert_eq!(panel.trouble(), Some("cannot write it"));
        assert_eq!(value(&panel, "show_files"), "on", "the row moved anyway");
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
        put_cursor(&mut panel, "show_files");
        assert!(panel.hint().contains("on and off"), "{}", panel.hint());
    }

    /// A section's list scrolls like every other list in the window, and the
    /// cursor is brought on screen rather than left off the bottom of it.
    #[test]
    fn the_list_scrolls_and_the_cursor_is_kept_on_screen() {
        let config = Config::default();
        let mut panel = over(&config);
        go_to(&mut panel, COLOURS);
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
        go_to(&mut panel, ALL_TIME);
        assert!(panel.thumb(rows).is_none(), "five rows do not scroll");

        // Down to the last setting of a longer one, in a window two rows tall:
        // the window follows the cursor to both ends.
        go_to(&mut panel, PANES);
        assert!(panel.jump(true));
        panel.reveal(2);
        assert!(panel.cursor() < panel.first() + 2, "the cursor is off screen");
        assert!(panel.jump(false));
        panel.reveal(2);
        assert!(
            panel.first() <= panel.cursor(),
            "the cursor is above the window"
        );

        // A page through the colours lands nowhere, because nothing there can
        // hold the cursor, and does not stop dead on a heading either.
        go_to(&mut panel, COLOURS);
        panel.page(rows, true);
        assert!(!panel.on_row(), "the colours hold no cursor at all");
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
        let mut panel = Settings::open(
            &Config::default(),
            &Totals::default(),
            Some(Path::new("/tmp/no0b.conf")),
            agent,
        );
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

        // The endpoint is the one row here anything can be done to, and what it
        // does is take typing.
        assert!(panel.on_row());
        assert!(
            matches!(panel.row(panel.cursor()), Some(Row::Field { key, .. }) if *key == agent::ENDPOINT)
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
            &Totals::default(),
        );
        go_to(&mut panel, AGENT);
        assert!(
            panel.rows().iter().any(|row| matches!(
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

    /// An agent with nothing configured says so rather than showing empty
    /// sections that read as broken, and names the files to write.
    #[test]
    fn nothing_configured_is_said_rather_than_shown_as_empty() {
        let dir = scratch_dir("bare");
        let work = dir.join("work");
        let agent = Agent::read(Some(&dir), Some(&work), crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), &Totals::default(), None, agent);

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
        let mut panel = Settings::open(&Config::default(), &Totals::default(), None, agent);
        go_to(&mut panel, MCP);
        let rows = panel.rows();
        assert!(
            rows.iter().any(|row| matches!(row, Row::Item(text)
                if text.contains("docs")
                    && text.contains("http://localhost:9000/mcp")
                    && text.contains("project"))),
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
        let mut panel = Settings::open(&Config::default(), &Totals::default(), None, agent);
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

    /// The skills section lists the directories under the agent's `skills/`,
    /// named and described by their own front matter.
    #[test]
    fn the_skills_section_lists_what_is_installed() {
        let agent = Agent {
            skills: vec![
                agent::Skill {
                    dir: String::from("coding"),
                    name: String::from("coding"),
                    about: String::from("Changing code that already exists."),
                },
                agent::Skill {
                    dir: String::from("web-search"),
                    name: String::from("web-search"),
                    about: String::new(),
                },
            ],
            skills_at: Some(PathBuf::from("/home/hec/.config/noob/skills")),
            ..Agent::default()
        };
        let mut panel = Settings::open(&Config::default(), &Totals::default(), None, agent);
        go_to(&mut panel, SKILLS);
        let listed: Vec<String> = panel
            .rows()
            .iter()
            .filter_map(|row| match row {
                Row::Item(text) => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            listed,
            vec![
                String::from("coding  Changing code that already exists."),
                String::from("web-search"),
            ]
        );
        assert!(
            said(&panel).contains(".config/noob/skills"),
            "the panel does not say where they live"
        );
    }
}
