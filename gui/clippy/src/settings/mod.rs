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
//! **The rail is pressed, and Tab walks it.** Up and down used to move the rail
//! whenever the keyboard was not inside a section, and in [`SESSIONS`] it never
//! was, because every row of it was a reading nothing could land on. So the two
//! keys anybody reaches for in a list of saved conversations swapped the whole
//! right hand side of the panel out. The arrow keys are always the rows now, and
//! the rail has a key of its own: Tab on to the next section, Shift-Tab back to
//! the one before, wrapping at both ends ([`Settings::walk_section`]). Without
//! it the rail was pointer-only, which made [`APPEARANCE`] unreachable from the
//! keyboard, and [`APPEARANCE`] is where a font size raised too far is put back
//! down. Tab used to cross a two column form row; that is Shift with the arrow
//! that points at the field now ([`Settings::cross`]), since shift is what takes
//! the nudge off left and right. Every line of the footer legend ends with the section
//! keys, because the legend is the only thing that says the arrows changed
//! meaning.
//!
//! **[`SESSIONS`] is a table inside a card.** The table is one row of the panel
//! ([`Row::Table`]): a header saying how many conversations there are and how
//! many are chosen, the banded column names and the rows themselves in the body,
//! and the three buttons in the footer. It scrolls inside its own body the way a
//! [`Row::Paper`] does, because a row's height cannot depend on the height of the
//! window. Each row carries the session's id ([`Kept`]) so what a delete names is
//! what the reader read, not a path parsed back out of the words on screen, and a
//! mark in the first column, because several conversations are deleted in one
//! press. The two columns of numbers are written against their right edge
//! ([`SESSION_COLUMNS`], [`Align`]): sizes and context counts started at the
//! left like the words did, which is a column of digits nobody can compare
//! down. What the section is called is said once, by the panel's own heading
//! ([`SESSION_TITLE`], [`Settings::title`]), because the heading is the line
//! that has to say where you are.
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
//! **The agent's section is cards, and it shows what the agent is really
//! told.** Four things anybody opens it for were seven rows apart, with a
//! heading and three notes standing between them, and half of what was left
//! was a raw environment key over a value with nothing saying what either
//! meant. It is five cards now ([`Row::Card`]): where the model is, which model
//! it asks for, what the agent gets, the file all of it is written in, and
//! whatever else that file carries. Every field is a plain-words label over its
//! value with the key and what it decides in one sentence under it, and the
//! shifted arrow crosses between the two fields of a card because the plain
//! arrow keys are the nudge. Under the cards are two blocks
//! ([`Row::Paper`]), cards themselves: the
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
//! three presets are drawn by name over them.
//!
//! **The palette is a grid, not a list.** Thirty seven colours one to a row was
//! a column of hex strings four screens long, and a hex string does not say what
//! it colours. Each group is one [`Row::Palette`] card now: what it paints in
//! the header, and its colours in the body, as many across as the card is wide
//! enough for, each one a block of the colour with a plain-words label beside
//! it. Pressing one says which key in the file writes it, which is the only
//! thing a hex string was there for.
//!
//! **The palette says where its colours came from, and picking a theme really
//! applies it.** "colors as theme groups i did not saw on the setup, i just
//! sawe many colors, so i dont know", and then "themes are only 2 custom and
//! noob". The grid opened as a wall of swatches with nothing saying what had set
//! them, and the control itself was one box holding one word. All three presets
//! are drawn by name on the first card of the palette now, with the line under
//! them naming the one the colours belong to, and a pick comments out any colour
//! line in the file that would override the preset before it writes the theme:
//! an explicit colour beats the theme it belongs to, so a file carrying eight of
//! them answered every theme change with the same window under a new name.

pub mod paint;
pub mod places;
pub mod sections;

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

/// The sessions section's own vocabulary, re-exported so the frame, the
/// painter and the panel's callers keep one `settings::` path to it.
pub use sections::sessions::{
    table_body_lines, Align, SESSION_COLUMNS, SESSION_FIRST_CELL, SESSION_MARK, SESSION_TITLE,
    TABLE_ROWS,
};

/// The skills section's field key, re-exported for the same reason: `main`
/// branches on it before any write.
pub use sections::skills::SKILL_SOURCE;

/// The MCP add card's two field keys, re-exported the same way.
pub use sections::mcp::{SERVER_HOW, SERVER_NAME};

/// The appearance section's restore set, re-exported for the write-back
/// below; its keep-off list rides along for the tests that honour it.
pub use sections::appearance::restoring;
#[cfg(test)]
pub use sections::appearance::OFF_PANEL;

use sections::appearance::THEME;

/// What the panel's own heading calls a section.
///
/// The rail's word for every section but [`SESSIONS`], whose rail word says the
/// panel's subject twice and the list's subject not at all.
pub fn section_title(name: &'static str) -> &'static str {
    match name {
        SESSIONS => SESSION_TITLE,
        other => other,
    }
}

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
    /// Prose: what a section is, or why it is empty. `bad` when it is something
    /// wrong rather than something explained.
    Note { text: String, bad: bool },
    /// The saved conversations, as one table inside one card: the column names,
    /// the rows under them, and the buttons that act on whatever is marked.
    ///
    /// One row of the panel, like every other card, and the rows of the table
    /// scroll inside its body. They were rows of the panel themselves, with a
    /// trash on the end of each one, which meant deleting four conversations was
    /// eight presses and nothing on the panel said the list was a group at all.
    Table(Table),
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
    /// One group of the palette, as a card: what that group paints in the
    /// header, and its colours in the body, each one a block of the colour with
    /// a plain-words label beside it.
    ///
    /// It was one row of the panel per three colours, under a bare heading, so
    /// a group of fourteen was five rows nothing tied together and the grid
    /// stopped at three across however wide the window was. The card is the
    /// group, and the colours in it reflow with the card
    /// ([`crate::design::swatch_across`]).
    Palette(Palette),
    /// One installed skill or one configured server: three lines of text, a
    /// toggle that really turns it off, and an uninstall beside it. The row the
    /// column on the right belongs to.
    Entry(Entry),
    /// A block of text under a title of its own: the agent's own instructions
    /// file, and the prompt it is a layer of. The one row that is more than a
    /// line or two of text.
    Paper(Paper),
    /// A group of related settings inside a box of its own: a title bar, a
    /// divider, and the fields under it.
    ///
    /// The grouping device the panel had none of. Everything was a full width
    /// row at one text size with a hairline under it, so a group title, a field
    /// label and a value all read alike and nothing said where one group ended.
    ///
    /// One card is one row, because the scroll window counts rows and a card
    /// that spanned several of them could not be counted. Never nested: a card holds fields, not cards.
    Card(Card),
}

/// A group of related settings, drawn in a box of its own.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    /// What the group is, in the title bar. Upper case, the way the rail's
    /// names are.
    pub title: String,
    /// The fields in its body, in the order they are read. Two across when the
    /// card is wide enough for both to keep their columns, one across when it
    /// is not ([`crate::design::across`]).
    pub fields: Vec<CardField>,
    /// The sentence under the body: what the fields above it mean, or why
    /// there is nothing in them. Nothing at all when the fields say it
    /// themselves, rather than a row of prose padding out every card.
    pub hint: Option<String>,
    /// The one action the card exists for, in a footer of its own, or nothing
    /// on a card that is only read and nudged.
    ///
    /// One, and at the bottom right where a card's own action belongs. A card
    /// with several would be a form with a toolbar in it.
    pub does: Option<Doing>,
}

/// What the button in a card's footer does.
///
/// Named here rather than in the drawing, because it is what the card is for:
/// the layout only has to know there is a footer with one button in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Doing {
    /// Say what the typed source would install, without installing it. The
    /// card's button starts here and turns into Install once the source
    /// checks out; typing something else turns it back.
    Validate,
    /// Take what was typed into the card's first field and install it as a
    /// skill.
    Install,
    /// Write the typed server into the global `mcp.json`.
    AddServer,
    /// Take every line the appearance settings own out of the file, so all of
    /// them go back to what the window ships with.
    Restore,
}

impl Doing {
    /// The word on the button.
    pub fn word(self) -> &'static str {
        match self {
            Doing::Validate => "validate",
            Doing::Install => "install",
            Doing::AddServer => "add",
            Doing::Restore => "restore",
        }
    }

    /// Whether pressing it loses something. A destructive button is drawn in
    /// the danger kind and takes two presses, the way every other delete on
    /// this panel does.
    pub fn dangerous(self) -> bool {
        matches!(self, Doing::Restore)
    }
}

/// One field of a card: its name, what it holds, and the sentence that says
/// what it is for.
#[derive(Clone, Debug, PartialEq)]
pub struct CardField {
    /// The name, on its own line above the value. Never beside it: a label and
    /// a value on one line read as one sentence, which is what made every value
    /// on this panel look like part of its label.
    ///
    /// Plain words rather than the key the file writes: `NOOB_TASK_CONCURRENCY`
    /// says nothing to somebody who has not read the CLI. The key is in
    /// [`CardField::hint`], where it is the answer to "which line do I edit".
    pub label: String,
    /// The sentence under the input: what this field decides, and what happens
    /// when it is not set. Nothing on a field whose label and value say it
    /// themselves, rather than a line of prose under every one of them.
    pub hint: Option<String>,
    /// What the field holds, as the row that kind of value has always been:
    /// [`Row::Reading`] for something read out, [`Row::Field`] for something
    /// typed into, [`Row::Setting`] for a number on a track or a name from a
    /// list.
    ///
    /// The same row rather than a second shape for the same thing, so a nudge, a
    /// drag and an edit are the one code path they were when these were rows of
    /// their own ([`control`] is what hands this to the keys).
    pub holds: Box<Row>,
}

impl CardField {
    /// A field that is read out rather than set.
    pub fn reading(label: &str, value: String) -> CardField {
        CardField::of(
            label,
            Row::Reading {
                label: String::from(label),
                value,
            },
        )
    }

    /// A field that is typed into. The one on this panel is the endpoint.
    pub fn text(label: &str, key: &'static str, value: String) -> CardField {
        CardField::of(label, Row::Field { key, value })
    }

    /// A field that is a setting of a file: a number on a track, held to the
    /// bounds of whoever reads that file, or one name out of a list.
    pub fn setting(
        label: &str,
        key: &'static str,
        value: String,
        kind: Kind,
        file: File,
    ) -> CardField {
        CardField::of(
            label,
            Row::Setting {
                key,
                value,
                kind,
                file,
            },
        )
    }

    fn of(label: &str, holds: Row) -> CardField {
        CardField {
            label: String::from(label),
            hint: None,
            holds: Box::new(holds),
        }
    }

    /// The sentence under it. Written at the call site so the field and what it
    /// means are one expression rather than two lists to keep in step.
    pub fn saying(mut self, hint: &str) -> CardField {
        self.hint = Some(String::from(hint));
        self
    }

    /// What the field says right now, for whoever draws it.
    pub fn value(&self) -> &str {
        match self.holds.as_ref() {
            Row::Reading { value, .. } | Row::Field { value, .. } | Row::Setting { value, .. } => {
                value
            }
            // Nothing else is ever built into a field, and a field with nothing
            // in it is drawn as the empty line it is rather than panicking a
            // window that is already open.
            _ => "",
        }
    }

    /// Whether the value is changed here rather than read out. A reading is
    /// drawn in the same shape with no border and no fill, so what can be typed
    /// into is obvious without pressing anything.
    pub fn editable(&self) -> bool {
        landable(&self.holds)
    }
}

/// Which field of a card one side names.
///
/// A press carries a [`Side`] and a side is one of two, so the fields a card
/// lets anybody change are its first two. Everything after them is read out,
/// and `a_cards_editable_fields_are_the_two_a_press_can_name` fails on a card
/// built the other way round.
pub fn card_slot(side: Side) -> usize {
    match side {
        Side::Left => 0,
        Side::Right => 1,
    }
}

/// The one field of a card a side names, or nothing when the card has none
/// there.
pub fn card_field(card: &Card, side: Side) -> Option<&CardField> {
    card.fields.get(card_slot(side))
}

/// Whether every field of a card that can be changed is one a press can name.
///
/// Read by the test that walks every section: a card with a third field that
/// can be changed is a control the pointer cannot reach and the keyboard cannot
/// cross to.
#[cfg(test)]
pub fn card_is_reachable(card: &Card) -> bool {
    !card.fields.iter().skip(2).any(CardField::editable)
}

/// One flag per field of a card, saying whether it carries a sentence.
///
/// The one list the height arithmetic runs on: [`card_body_lines`] counts with
/// it and `view::settings_card_slots` places with it.
pub fn card_hints(card: &Card) -> Vec<bool> {
    card.fields
        .iter()
        .map(|field| field.hint.is_some())
        .collect()
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

    /// How much of the block is on screen, for its own scrollbar, in a box
    /// showing `rows` of it.
    ///
    /// `None` when the whole of it is already on screen, which is what draws no
    /// bar at all: a bar that is always there and always full says nothing about
    /// whether there is more to read.
    ///
    /// `rows` is what the layout really drew rather than [`PAPER_LINES`],
    /// because a block cut off by the bottom of the list shows fewer, and a bar
    /// counting lines that are not on screen is the readout lying about the one
    /// thing it is for.
    pub fn thumb(&self, rows: usize) -> Option<(f32, f32)> {
        let heights = vec![1usize; self.body.len()];
        let back = self.body.len().saturating_sub(rows).saturating_sub(self.first);
        text_geometry::thumb(&heights, rows, back)
    }

    /// Take it to its first or its last screenful, for Home and End.
    fn jump_to(&mut self, last: bool) -> bool {
        let next = match last {
            true => self.most(),
            false => 0,
        };
        let moved = next != self.first;
        self.first = next;
        moved
    }
}

/// How many lines of a [`Paper`] are on screen at once.
///
/// A number rather than what fits, because a row's height cannot depend on the
/// width or the height of the window: [`lines`] is what the scroll window counts
/// in and what the layout places with, and a block that grew when the window did
/// would put every click under it on another row.
pub const PAPER_LINES: usize = 12;

/// The saved conversations, as a table in the body of one card.
///
/// The rows scroll inside the body rather than down the panel, so the header
/// naming the columns and the buttons under them stay where they are however far
/// down the list you are: a header that scrolls away is a table of five unnamed
/// columns, and buttons that scroll away are buttons nobody can reach without
/// first scrolling back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    /// What each column is called, in order, out of [`SESSION_COLUMNS`].
    pub names: Vec<&'static str>,
    /// One conversation per entry, newest first, the way the reader read them.
    pub rows: Vec<Kept>,
    /// Which row the body starts on.
    pub first: usize,
    /// Which row the keys are on. Its own number rather than the section's
    /// cursor, because the section's cursor is on the card: the card is one row
    /// of the panel and this is one row of the card.
    pub cursor: usize,
}

/// One saved conversation on the table: the cells that are drawn, the id a
/// delete needs, and whether it is one of the ones marked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    /// The name of its transcript on the disk. What a delete names, so what goes
    /// is what the reader read rather than a path parsed back out of the words
    /// on screen.
    pub id: String,
    /// [`SESSION_CELLS`] of them, in [`SESSION_COLUMNS`] order after the mark.
    pub cells: Vec<String>,
    pub marked: bool,
}

impl Table {
    /// What the card's header says: how many conversations there are, and how
    /// many of them are marked once any of them is.
    ///
    /// The count rather than the section's own name, which the panel's heading
    /// already says: a card headed with the words above it says nothing, and how
    /// many are about to be deleted is the one thing the header can say that
    /// nothing else on the panel does.
    pub fn title(&self) -> String {
        let all = match self.rows.len() {
            1 => String::from("1 SESSION"),
            many => format!("{many} SESSIONS"),
        };
        match self.chosen() {
            0 => all,
            some => format!("{all}, {some} CHOSEN"),
        }
    }

    /// How many rows are marked.
    pub fn chosen(&self) -> usize {
        self.rows.iter().filter(|row| row.marked).count()
    }

    /// The ids a delete would take: every marked row, or the row the keys are on
    /// when none of them is marked.
    ///
    /// So the single row path is still one press on one row, and marking is what
    /// makes it several. In row order, which is the order they are read in.
    pub fn taking(&self) -> Vec<String> {
        match self.chosen() {
            0 => self
                .rows
                .get(self.cursor)
                .map(|row| vec![row.id.clone()])
                .unwrap_or_default(),
            _ => self
                .rows
                .iter()
                .filter(|row| row.marked)
                .map(|row| row.id.clone())
                .collect(),
        }
    }

    /// The row the keys are on.
    pub fn at_cursor(&self) -> Option<&Kept> {
        self.rows.get(self.cursor)
    }

    /// The furthest down the body can start and still be full.
    pub fn most(&self) -> usize {
        self.rows.len().saturating_sub(TABLE_ROWS)
    }

    /// How much of the list is on screen, for the table's own scrollbar, in a
    /// body showing `rows` of it. `None` when the whole list already fits,
    /// which is what draws no bar.
    pub fn thumb(&self, rows: usize) -> Option<(f32, f32)> {
        let heights = vec![1usize; self.rows.len()];
        let back = self.rows.len().saturating_sub(rows).saturating_sub(self.first);
        text_geometry::thumb(&heights, rows, back)
    }

    /// Bring the row the keys are on back inside the body.
    ///
    /// Called by everything that moves the cursor, so a cursor walked off the
    /// bottom of the body scrolls the body rather than leaving the band drawn
    /// somewhere nobody can see it.
    fn reveal(&mut self) {
        self.first = self.first.min(self.most());
        if self.cursor < self.first {
            self.first = self.cursor;
        }
        if self.cursor >= self.first + TABLE_ROWS {
            self.first = self.cursor + 1 - TABLE_ROWS;
        }
    }
}

/// Which field of a card something is in. Left for every row that is not one, so
/// a press on an ordinary row is still a press on the row.
///
/// It was which half of a two column form row a press landed in. The form is
/// gone: a card is what groups two settings side by side now, and this is which
/// of its fields the press or the keys are on.
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

/// What the keys and the pointer act on in one slot of a row: the field a card
/// keeps there, or the row itself.
///
/// Asked by the model, the layout and the drawing, so what a key changes, what a
/// click lands on and what is drawn are the same thing. Nothing when a card has
/// no field in that slot, which is what stops the right hand side of a one field
/// card reading as somewhere the cursor can go. The row it hands back is a
/// [`Row::Setting`], a [`Row::Field`] or a [`Row::Reading`] whether it came out
/// of a card or off the section, so a nudge, a drag and an edit are the one code
/// path for both.
pub fn control(row: &Row, side: Side) -> Option<&Row> {
    match row {
        Row::Card(card) => card_field(card, side).map(|field| field.holds.as_ref()),
        row => Some(row),
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
    /// What it is called, and nothing else. The description used to be glued to
    /// the end of this with two spaces, which made the name and what it is one
    /// run of text competing for one line with two buttons on the end of it.
    pub name: String,
    /// What it is for, on its own lines under the name: a skill's own
    /// description, or the address or command line a server is started with.
    /// Wrapped rather than clipped, so a long one is as many rows as it takes
    /// and the row grows with it. Empty when there is none to read.
    pub about: String,
    /// The line under that: the repository a skill records, or the directory it
    /// was found in when it records none; for a server, the file its entry
    /// lives in.
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
    /// Write a new server into the global file, from the add card's fields.
    AddServer { name: String, how: String },
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
    /// Delete saved conversations: each transcript and the line about it in the
    /// note beside them. Named by the ids the rows carry, which came off the
    /// reader rather than off anything drawn.
    ///
    /// A set rather than one id, because the table is marked and deleted in one
    /// press. One marked row and one row with nothing marked are the same deed
    /// with one id in it, so there is one delete path and not two.
    ForgetSessions { ids: Vec<String> },
    /// Take every appearance line out of the window's own settings file, so the
    /// sizes, the transparency and the whole palette go back to what the window
    /// ships with ([`restoring`] is the list).
    ///
    /// Commented out rather than written back as values: a key with no live
    /// line falls back to the default on the next read, and spelling the
    /// defaults out would put thirty seven colours in the file and make `theme`
    /// mean nothing ever again.
    RestoreLooks,
}

/// One colour on the grid: the key the file writes it under, what it actually
/// colours said in words, and the colour itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swatch {
    pub key: &'static str,
    pub about: &'static str,
    pub rgb: [u8; 3],
}

/// One group of the palette, in a card of its own.
///
/// The card with a grid in it, the way [`Table`] is the card with a list in it:
/// a colour is read and never set here, so the body is cells rather than fields
/// and nothing in it holds the cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    /// What this group paints, in the header. Not the keys it holds: `THE
    /// METERS` says what the ten colours under it are for and `gauge_1` does
    /// not.
    pub title: &'static str,
    /// The colours, in file order.
    pub cells: Vec<Swatch>,
}

/// How tall a group of the palette is inside its card, in lines: one line per
/// row of colours, as many across as the card is wide enough for.
///
/// The width is the list's own, which [`lines`] is handed and the layout reads
/// off the same list, so the rows counted here and the rows drawn are the same
/// rows.
pub fn palette_body_lines(cells: usize, cols: usize) -> f32 {
    let across = crate::design::swatch_across(crate::design::card_cols(cols));
    cells.div_ceil(across).max(1) as f32 * crate::design::TEXT_LINES
}

/// How many rows of text one row of the panel takes, in a list `cols` wide.
///
/// A heading is two, because it is drawn larger than the settings under it: a
/// heading measured at one height and drawn at another puts every click below it
/// on the wrong row. [`Settings::heights`] and `view::place_settings` both read
/// this, which is what keeps the two agreeing.
///
/// `cols` is the width the list has, because an entry's description wraps in it
/// and a row that wrapped to four rows is four rows tall. Every other kind of
/// row ignores it: a value too long for the panel is still clipped rather than
/// wrapped, so a click cannot resolve to a setting other than the one under the
/// pointer.
pub fn lines(row: &Row, cols: usize) -> usize {
    match row {
        // A card: its header, its body, the room around the body and the space
        // under the card itself, all counted by the one function the layout
        // places it with. A footer only when it has an action, since a card
        // with no button in it is a strip of empty space at the bottom.
        Row::Card(card) => {
            crate::design::card_row_lines(card_body_lines(card, cols), card.does.is_some())
        }
        // One card per entry: the name in the header, what it is for and where
        // it is in the body, and its two buttons in the footer. The description
        // wraps, so an entry is as tall as its description needs: it was three
        // rows whatever the description said, and a description longer than the
        // column ended in an ellipsis with the rest of it unreadable.
        Row::Entry(entry) => crate::design::card_row_lines(entry_body_lines(entry, cols), true),
        // A block of text is a card too: its title in the header, where the
        // text came from and the text itself in the body. It was a bare title,
        // a bare line and twelve lines of prose with nothing round any of it,
        // which on a panel of cards is the one thing on screen that reads as
        // loose text.
        Row::Paper(_) => crate::design::card_row_lines(paper_body_lines(), false),
        // A table is a card as well: the column names and the rows in the body,
        // and the buttons that act on what is marked in the footer. A fixed
        // number of rows, scrolled inside itself, for the same reason a block of
        // text is: the height of a row cannot depend on the height of the
        // window.
        Row::Table(_) => crate::design::card_row_lines(table_body_lines(), true),
        // A group of the palette is a card as well: what it paints in the
        // header and its colours in the body, as many across as the width
        // leaves room for.
        Row::Palette(palette) => crate::design::card_row_lines(
            palette_body_lines(palette.cells.len(), cols),
            false,
        ),
        _ => 1,
    }
}

/// How tall a block of text is inside its card, in lines: where it came from,
/// and the [`PAPER_LINES`] of the text itself under that.
pub fn paper_body_lines() -> f32 {
    crate::design::TEXT_LINES
        + crate::design::TIGHT
        + PAPER_LINES as f32 * crate::design::TEXT_LINES
}

/// How tall a card's body is, in lines, inside a list `cols` wide.
///
/// The fields, in as many bands as the width leaves room for, with [`STEP`] of
/// [`crate::design`] between one band and the next, and the hint under the last
/// of them. Read by [`lines`] and by `view::place_settings` through the same
/// tokens, which is what keeps the counted height and the drawn one the same.
///
/// [`STEP`]: crate::design::STEP
pub fn card_body_lines(card: &Card, cols: usize) -> f32 {
    let cols = crate::design::card_cols(cols);
    let across = crate::design::across(card.fields.len(), cols);
    let tall = crate::design::fields_lines(&card_hints(card), across);
    match card.hint.is_some() {
        true => tall + crate::design::STEP + crate::design::TEXT_LINES,
        false => tall,
    }
}

/// How tall an entry's card body is, in lines: what it is for, wrapped in the
/// card's own columns, and where it is on the line under that.
pub fn entry_body_lines(entry: &Entry, cols: usize) -> f32 {
    let wrapped = about_rows(&entry.about, crate::design::card_cols(cols));
    wrapped as f32 * crate::design::TEXT_LINES
        + crate::design::TIGHT
        + crate::design::TEXT_LINES
}

/// How many rows an entry's description takes in a column `cols` wide.
///
/// The panes' own wrap rule, through the one function that owns it, so the
/// height counted here, the room the layout leaves the row and the rows the
/// renderer breaks the text into are the same rows. Always at least one, which
/// is what an entry with no description to read still spends on it.
pub fn about_rows(about: &str, cols: usize) -> usize {
    text_geometry::rows_in(about, cols, crate::state::PANE_WRAP).len()
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

/// What a field with nothing in it reads as. Drawn instead of an empty row,
/// which reads as a value that failed to load rather than one nobody set.
pub const UNSET: &str = "not set";

/// What a credential reads as. Never the value: the panel says whether the key
/// is there and nothing more.
pub const SECRET: &str = "set, and not shown here";

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
    /// Where the column beside the list is scrolled to, as a wrapped row of that
    /// document. Its own number because the two columns scroll separately: the
    /// wheel over a skill's own text must not walk the list of skills.
    doc_first: usize,
    /// Which field of a card the keyboard is in. Kept while the cursor walks
    /// rows, so going down one column of cards stays in that column.
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
    /// Where each table was left, by the row it was on: what it was scrolled to,
    /// which row the keys were on, and which conversations were marked.
    ///
    /// The marks are ids and not row numbers. The rows are rebuilt here and a
    /// delete takes one out from under the ones below it, so a mark kept by
    /// number would come back on the wrong conversation; an id that is no longer
    /// on the disk simply drops out.
    tables: Vec<(usize, usize, usize, Vec<String>)>,
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
    ///
    /// Changed by a press on the rail and by Tab, and by nothing else. The
    /// arrow keys used to walk it, which meant up and down in a list of saved
    /// sessions swapped the whole right hand side out from under whoever was
    /// reading it. They are always the rows now, and the rail's own key is Tab
    /// ([`Settings::walk_section`]).
    chosen: usize,
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
    /// The skills section's own state: the install field, the validate
    /// verdict and the install cycle ([`sections::skills::SkillsSection`]).
    skills: sections::skills::SkillsSection,
    /// The MCP section's own state: the add card's two fields
    /// ([`sections::mcp::McpSection`]).
    mcp: sections::mcp::McpSection,
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
            editing: None,
            dragging: None,
            picked: None,
            arming: None,
            file: file.map(PathBuf::from),
            trouble: None,
            prompt: Assembled::Waiting,
            // The skills box decides its own opening state, the web-search
            // suggestion included.
            skills: sections::skills::SkillsSection::new(&agent.skills),
            mcp: sections::mcp::McpSection::default(),
            agent,
        };
        panel.sections = panel.build(config);
        panel
    }

    /// What is in the install field right now. Only the tests want it on its
    /// own: the window reads it off the field the row builder put it in.
    #[cfg(test)]
    pub fn source(&self) -> &str {
        self.skills.source()
    }

    /// Take what the install field holds, ending the edit if one is running.
    ///
    /// One function for the two ways an install is asked for: Enter while
    /// typing, and a press on the button with the caret still in the field. A
    /// button that read the row instead would install the last thing that was
    /// saved rather than the address on screen.
    pub fn take_source(&mut self) -> String {
        let typing = matches!(
            self.at_cursor(),
            Some(Row::Field { key, .. }) if *key == SKILL_SOURCE
        );
        let typed = match typing {
            true => self.editing.take(),
            false => None,
        };
        self.skills.take_source(typed)
    }

    /// The validate button's answer, shown under the card and voided the
    /// moment the field says something else. The rows are rebuilt rather than
    /// patched, the same as everything else that arrives here.
    pub fn note_check(&mut self, source: String, verdict: Result<String, String>, config: &Config) {
        self.skills.note_check(source, verdict);
        self.sections = self.build(config);
    }

    /// Whether the field's current source is the one the validate button
    /// approved. What turns the card's button from validate into install.
    pub fn checked_ok(&self) -> bool {
        self.skills.checked_ok()
    }

    /// Keep a running edit on the add card's two fields: the text goes into
    /// the panel, never into any file, and the rows are rebuilt so the field
    /// shows what it now holds. Answers whether the edit was one of them.
    pub fn keep_server_edit(&mut self, config: &Config) -> bool {
        let key = match self.at_cursor() {
            Some(Row::Field { key, .. }) if *key == SERVER_NAME || *key == SERVER_HOW => *key,
            _ => return false,
        };
        let Some(typed) = self.editing.take() else {
            return false;
        };
        self.mcp.keep_edit(key, typed);
        self.sections = self.build(config);
        true
    }

    /// Drop an edit running anywhere but the two named fields, so a button
    /// press acts on what its own card shows and never on half of somebody
    /// else's endpoint.
    pub fn cancel_edit_elsewhere(&mut self, one: &str, other: &str) {
        let here = matches!(
            self.at_cursor(),
            Some(Row::Field { key, .. }) if *key == one || *key == other
        );
        if !here {
            self.cancel_edit();
        }
    }

    /// The add card's press, as the deed that writes the global file, or the
    /// refusal said on the panel. A running edit on either field is taken in
    /// first, so the deed carries what is on screen.
    pub fn add_server_deed(&mut self, config: &Config) -> Option<Deed> {
        self.keep_server_edit(config);
        match self.mcp.add_deed() {
            Ok(deed) => Some(deed),
            Err(why) => {
                self.say_trouble(why);
                None
            }
        }
    }

    /// After the deed landed: the card goes back to empty, ready for the next
    /// one. On a refusal the fields keep what was typed.
    pub fn clear_server_fields(&mut self) {
        self.mcp.clear();
    }

    /// Say that an install has started, so the section says so while it runs.
    ///
    /// The rows are rebuilt rather than patched, the same as everything else
    /// that arrives here.
    pub fn begin_install(&mut self, source: String, config: &Config) {
        self.skills.begin_install(source);
        self.refresh(config);
    }

    /// Take what the install answered, with a fresh reading of the disk beside
    /// it.
    ///
    /// The list comes back off that reading and not out of what the install
    /// said: a skill is on the panel because its directory is there. The
    /// message goes on the block above the list, whichever way it went, because
    /// a git failure is several lines and a footer holds one.
    pub fn adopt_install(
        &mut self,
        source: String,
        answer: Result<String, String>,
        agent: Agent,
        config: &Config,
    ) {
        self.skills.end_install(source, answer);
        self.agent = agent;
        self.refresh(config);
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
                tables: section
                    .rows
                    .iter()
                    .enumerate()
                    .filter_map(|(at, row)| match row {
                        Row::Table(table) => Some((
                            at,
                            table.first,
                            table.cursor,
                            table
                                .rows
                                .iter()
                                .filter(|row| row.marked)
                                .map(|row| row.id.clone())
                                .collect(),
                        )),
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
                tables,
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
            // The same for a table, marks and all. Pruned by the rebuild itself:
            // a mark whose conversation is no longer on the disk has no row to
            // land on, which is what makes a delete of four marked rows leave
            // nothing marked behind it.
            for (at, was_first, was_cursor, marked) in tables {
                if let Some(Row::Table(table)) = section.rows.get_mut(at) {
                    table.cursor = was_cursor.min(table.rows.len().saturating_sub(1));
                    table.first = was_first.min(table.most());
                    for row in table.rows.iter_mut() {
                        row.marked = marked.contains(&row.id);
                    }
                    table.reveal();
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
                    AGENT => sections::agent::rows(&self.agent, &self.prompt),
                    SESSIONS => sections::sessions::rows(&self.agent),
                    SKILLS => self.skills.rows(&self.agent),
                    MCP => self.mcp.rows(&self.agent),
                    APPEARANCE => sections::appearance::rows(config, self.file.as_deref()),
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

    /// Every section's name, in rail order.
    pub fn section_names(&self) -> Vec<&'static str> {
        self.sections.iter().map(|section| section.name).collect()
    }

    /// What the heading at the top of the panel calls the section it is
    /// showing, which is not always the word the rail marks it with
    /// ([`section_title`]).
    pub fn title(&self) -> &'static str {
        section_title(self.here().name)
    }

    pub fn chosen(&self) -> usize {
        self.chosen
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
            section.rows.iter().flat_map(move |row| {
                let mut out = vec![(section.name, row)];
                if let Row::Card(card) = row {
                    out.extend(
                        card.fields
                            .iter()
                            .map(|field| (section.name, field.holds.as_ref())),
                    );
                }
                out
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
        if !two_sided(row) {
            return Side::Left;
        }
        match landable_at(row, here.side) {
            true => here.side,
            false => here.side.other(),
        }
    }

    /// The half of the row under the cursor the keys act on, which is the row
    /// itself everywhere but in a form or a card.
    pub fn at_cursor(&self) -> Option<&Row> {
        control(self.row(self.cursor())?, self.side())
    }

    /// One slot of one row, for the layout and the drawing.
    pub fn cell(&self, index: usize, side: Side) -> Option<&Row> {
        control(self.row(index)?, side)
    }

    /// Move to one half of a form row, which is the one thing on this panel the
    /// plain arrow keys cannot do: left and right are the nudge. False when the
    /// cursor is not on a form, when it is already in that half, or when that
    /// half is a reading.
    ///
    /// This was `swap`, a toggle on Tab. Tab is how the keyboard walks the rail
    /// now ([`Settings::walk_section`]), so the crossing is the shifted arrow
    /// instead: it points at the half it lands on, and the shift is what takes
    /// the nudge off the key.
    pub fn cross(&mut self, to: Side) -> bool {
        if self.editing.is_some() {
            return false;
        }
        if self.side() == to {
            return false;
        }
        let here = self.here();
        let Some(row) = here.rows.get(here.cursor).filter(|row| two_sided(row)) else {
            return false;
        };
        if !landable_at(row, to) {
            return false;
        }
        self.picked = None;
        self.arming = None;
        self.here_mut().side = to;
        true
    }

    /// Walk the rail one section on, wrapping at both ends.
    ///
    /// The keyboard's only route between sections. The rail is a column of
    /// names that is pressed, and at a big font in a small window it is also a
    /// column that wraps, so the section anybody is looking for may be in the
    /// second column of it: a key that reaches every name whatever the window is
    /// doing is what keeps APPEARANCE, and with it the font size, reachable.
    /// It wraps rather than stopping, so the section before the first one is the
    /// last one instead of nothing at all.
    pub fn walk_section(&mut self, forward: bool) -> bool {
        let count = self.sections.len();
        if count == 0 {
            return false;
        }
        let next = match forward {
            true => (self.chosen + 1) % count,
            false => (self.chosen + count - 1) % count,
        };
        self.choose(next)
    }

    /// Whether the cursor is on a row at all: a section of readings has nothing
    /// to land on, and a band drawn on a row nothing can be done to is a lie.
    pub fn on_row(&self) -> bool {
        self.row(self.cursor()).is_some_and(landable)
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
    /// palette card or has no such cell.
    pub fn swatch(&self, row: usize, cell: usize) -> Option<&Swatch> {
        let Row::Palette(palette) = self.row(row)? else {
            return None;
        };
        palette.cells.get(cell)
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
        // A saved conversation says which one, by the folder it belongs to and
        // how long ago it was: the id is a name nobody typed and nobody would
        // recognise on a footer.
        // Several at once says how many. "sure?" over a marked list is a
        // question that does not say what is about to go, and the whole of what
        // a confirmation is for is saying that.
        if let Some(Row::Table(table)) = self.arming.and_then(|at| self.row(at)) {
            let taking = table.taking();
            if taking.len() > 1 {
                return format!(
                    "press delete again to remove {} conversations; anything else leaves them alone",
                    taking.len()
                );
            }
            let row = match table.chosen() {
                0 => table.at_cursor(),
                _ => table.rows.iter().find(|row| row.marked),
            };
            let cells = row.map(|row| row.cells.clone()).unwrap_or_default();
            let when = cells.first().map(String::as_str).unwrap_or("that session");
            let folder = cells.get(1).map(String::as_str).unwrap_or_default();
            return format!(
                "press delete again to remove the {folder} conversation from {when}; anything else leaves it alone"
            );
        }
        // The restore under the palette. It says what would go, which is lines
        // of a file rather than a directory: somebody who has tuned a colour by
        // hand is about to lose that line, and nothing else on screen says so.
        if let Some(Row::Card(card)) = self.arming.and_then(|at| self.row(at))
            && card.does == Some(Doing::Restore)
        {
            return String::from(
                "press restore again to comment out every size, transparency and colour line in the settings file; anything else leaves them alone",
            );
        }
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

    /// What the keys do here, ending in what moves between sections.
    ///
    /// Every line ends the same way, because the arrow keys changed meaning and
    /// the footer is the only thing that says so: they are the rows of one
    /// section now, and the rail is Tab. A legend that named the arrows and left
    /// the section key to be guessed is how somebody who has raised the font too
    /// far ends up with no way back to APPEARANCE.
    pub fn hint(&self) -> &'static str {
        if self.editing.is_some() {
            // The one field on the panel Enter does not write a file with.
            return match self.at_cursor() {
                Some(Row::Field { key, .. }) if *key == SKILL_SOURCE => {
                    "type an address or a folder \u{2022} enter installs it \u{2022} esc leaves it alone"
                }
                _ => "type it \u{2022} enter saves it \u{2022} esc leaves it alone",
            };
        }
        // A card the keys are on for its button rather than for a field: the
        // arrows do nothing there, so the line names the key that does.
        if let Some(Row::Card(card)) = self.row(self.cursor())
            && card.fields.iter().all(|field| !field.editable())
            && let Some(doing) = card.does
        {
            return match doing.dangerous() {
                true => {
                    "up and down move \u{2022} delete puts every size, transparency and colour back to its default \u{2022} tab and shift-tab change section"
                }
                false => {
                    "up and down move \u{2022} press what this card is for \u{2022} tab and shift-tab change section"
                }
            };
        }
        // On a card of two fields, the one thing the plain arrow keys cannot
        // say is how to get to the other one, because left and right are the
        // nudge.
        let across = self.row(self.cursor()).is_some_and(|row| {
            two_sided(row)
                && landable_at(row, Side::Left)
                && landable_at(row, Side::Right)
        });
        match self.at_cursor() {
            Some(Row::Setting { kind, .. }) => match (kind, across) {
                // Every theme is drawn, so pressing one is the shortest way
                // there; the arrows are still what the keyboard walks them with.
                (Kind::Choice(_), _) => {
                    "press a theme to wear it \u{2022} left and right walk them \u{2022} tab and shift-tab change section"
                }
                (Kind::Number { .. }, false) => {
                    "up and down move \u{2022} left and right nudge it, or drag the slider \u{2022} tab and shift-tab change section"
                }
                (Kind::Number { .. }, true) => {
                    "left and right nudge it \u{2022} shift left and right cross the card \u{2022} tab and shift-tab change section"
                }
            },
            Some(Row::Paper(paper)) => match paper.offer.is_some() {
                true => {
                    "enter writes a starter AGENTS.md there \u{2022} tab and shift-tab change section"
                }
                false => {
                    "page up and page down read it \u{2022} up and down leave it \u{2022} tab and shift-tab change section"
                }
            },
            // The install field. Enter on it starts typing, and the sentence
            // under the field already says what a source is, so the legend
            // says what happens when the typing ends.
            Some(Row::Field { key, .. }) if *key == SKILL_SOURCE => {
                "up and down move \u{2022} enter types an address in, and enter again installs it \u{2022} tab and shift-tab change section"
            }
            Some(Row::Field { .. }) if across => {
                "enter edits it \u{2022} shift left and right cross the card \u{2022} tab and shift-tab change section"
            }
            // Naming the arrows on every row the cursor can land on is the
            // legend's whole job: they are the rows now, and this row is the one
            // the panel opens on.
            Some(Row::Field { .. }) => {
                "up and down move \u{2022} enter edits it \u{2022} tab and shift-tab change section"
            }
            Some(Row::Entry(entry)) => match (entry.removable, &entry.what) {
                (false, _) => {
                    "up and down move \u{2022} enter turns it on and off \u{2022} tab and shift-tab change section"
                }
                (true, Which::Skill { .. }) => {
                    "enter turns it on and off \u{2022} uninstall deletes its directory \u{2022} tab and shift-tab change section"
                }
                (true, Which::Server { .. }) => {
                    "enter turns it on and off in its file \u{2022} uninstall takes it out \u{2022} tab and shift-tab change section"
                }
            },
            // The one row on the panel whose keys are not the panel's own, so
            // the legend names all four of them: nothing else says that space
            // marks a conversation or that delete takes every marked one.
            Some(Row::Table(_)) => {
                "up and down pick a session \u{2022} space marks it \u{2022} delete removes what is marked \u{2022} tab and shift-tab change section"
            }
            _ => "up and down move \u{2022} tab and shift-tab change section \u{2022} esc closes",
        }
    }

    /// Put the rail on one section, which is what a click on it does and what
    /// [`Settings::walk_section`] does for the keyboard.
    ///
    /// The cursor stays on the rows of whatever is chosen: picking a section
    /// does not put the keyboard on the rail, and the arrow keys go on walking
    /// the list beside it, so up and down never swap the right hand side out.
    pub fn choose(&mut self, index: usize) -> bool {
        if index >= self.sections.len() {
            return false;
        }
        let moved = index != self.chosen;
        self.chosen = index;
        self.editing = None;
        self.dragging = None;
        self.picked = None;
        self.arming = None;
        // A refusal belongs to the section it was said on. Left standing, it
        // burns at the foot of every other section as a warning about nothing
        // on screen.
        self.trouble = None;
        self.rewind_doc();
        moved
    }

    /// What the row under the cursor becomes when it is nudged, or nothing when
    /// the cursor is on a row that cannot change.
    ///
    /// Takes `&self`: the row is not touched here. What the panel shows comes
    /// back from the file, so a write that fails leaves the row reading what the
    /// file still says instead of the value it was asked for.
    pub fn change(&self, forward: bool) -> Option<Change> {
        if self.editing.is_some() {
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

    /// Press one option of a choice by name: what that writes, or nothing when
    /// the field is not a choice or has no such option.
    ///
    /// The options are all drawn, so all of them can be pressed. Left and right
    /// still walk them ([`Settings::change`]), and both land in the same writer:
    /// this is the same [`Change`] the arrow keys make, with the option that was
    /// pressed instead of the next one along.
    ///
    /// The one already set is not refused. A file carrying colours of its own
    /// reads as [`CUSTOM`] however many times the theme line says otherwise, and
    /// pressing the theme it claims to be is then the one press that puts the
    /// window back on it.
    pub fn choose_option(&self, index: usize, side: Side, at: usize) -> Option<Change> {
        let Row::Setting {
            key,
            kind: Kind::Choice(names),
            file,
            ..
        } = self.cell(index, side)?
        else {
            return None;
        };
        Some(Change {
            key,
            value: String::from(*names.get(at)?),
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
        sections::mcp::file(&self.agent, project)
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
        let deed = match self.row(index) {
            Some(Row::Entry(entry)) if entry.removable => match &entry.what {
                Which::Skill { dir } => Deed::RemoveSkill {
                    dir: dir.clone(),
                    on: entry.on,
                },
                Which::Server { project } => Deed::RemoveServer {
                    name: entry.name.clone(),
                    project: *project,
                },
            },
            // The delete under the table of saved conversations. The same two
            // presses, because it is the same kind of act: the transcripts are
            // gone and nothing here can put them back. What it takes is whatever
            // is marked, or the row the keys are on when nothing is.
            Some(Row::Table(table)) => match table.taking() {
                ids if ids.is_empty() => return None,
                ids => Deed::ForgetSessions { ids },
            },
            // A card whose own action loses something: the restore under the
            // palette, which takes lines out of a file somebody may have edited
            // by hand. Same two presses for the same reason.
            Some(Row::Card(card)) => match card.does {
                Some(Doing::Restore) => Deed::RestoreLooks,
                _ => return None,
            },
            _ => return None,
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

    /// The table on one row, for whoever draws it.
    pub fn table(&self, index: usize) -> Option<&Table> {
        match self.row(index)? {
            Row::Table(table) => Some(table),
            _ => None,
        }
    }

    /// The row the panel's cursor is on, when that row is a table.
    pub fn table_at_cursor(&self) -> Option<(usize, &Table)> {
        let at = self.cursor();
        self.table(at).map(|table| (at, table))
    }

    fn table_mut(&mut self, index: usize) -> Option<&mut Table> {
        match self.here_mut().rows.get_mut(index)? {
            Row::Table(table) => Some(table),
            _ => None,
        }
    }

    /// Put the keys on one row of a table, which is what a press on it does.
    ///
    /// The panel's own cursor goes on the card at the same time, so the card
    /// wears the focus border while one of its rows wears the band.
    pub fn point_at_row(&mut self, index: usize, at: usize) -> bool {
        let moved = self.point_at(index, Side::Left);
        let Some(table) = self.table_mut(index) else {
            return moved;
        };
        if at >= table.rows.len() {
            return moved;
        }
        let walked = table.cursor != at;
        table.cursor = at;
        table.reveal();
        // An armed delete with nothing marked names the row the keys are on, so
        // moving them is the question changing: leaving it armed would make the
        // next press take a conversation nothing had asked about.
        if walked {
            self.arming = None;
        }
        moved || walked
    }

    /// Mark one row of a table, or take the mark off it.
    ///
    /// Never cleared by a keypress the way an armed delete is: marking three
    /// rows means moving between them, and a set the arrow keys emptied could
    /// only ever hold one.
    pub fn mark(&mut self, index: usize, at: usize) -> bool {
        let Some(table) = self.table_mut(index) else {
            return false;
        };
        let Some(row) = table.rows.get_mut(at) else {
            return false;
        };
        row.marked = !row.marked;
        // An armed delete names a set, and this is that set changing: the
        // question on the footer would be about a list that is no longer the one
        // the second press would take.
        self.arming = None;
        true
    }

    /// Mark every row of a table, or none of them.
    ///
    /// Every row on the list and not only the ones on screen: the body holds
    /// [`TABLE_ROWS`] of them and "all" meaning "the twelve you can see" is a
    /// button that says one thing and does another. What it took is on the
    /// header and on the button that would delete them.
    pub fn mark_all(&mut self, index: usize, on: bool) -> bool {
        let Some(table) = self.table_mut(index) else {
            return false;
        };
        let mut moved = false;
        for row in table.rows.iter_mut() {
            moved |= row.marked != on;
            row.marked = on;
        }
        let armed = self.arming.is_some();
        self.arming = None;
        moved || armed
    }

    /// Move a table inside its own body, without moving the list. Same rule as a
    /// block of text: the pointer is on the thing being scrolled.
    pub fn scroll_table(&mut self, index: usize, by: usize, down: bool) -> bool {
        let Some(table) = self.table_mut(index) else {
            return false;
        };
        let most = table.most();
        let next = match down {
            true => (table.first + by).min(most),
            false => table.first.saturating_sub(by),
        };
        let moved = next != table.first;
        table.first = next;
        moved
    }

    /// Walk the rows inside the table the cursor is on. Nothing at all when the
    /// cursor is not on one, which is what leaves the arrow keys to the list.
    fn step_table(&mut self, down: bool, by: usize) -> Option<bool> {
        let at = self.cursor();
        let table = self.table_mut(at)?;
        let last = table.rows.len().saturating_sub(1);
        let next = match down {
            true => (table.cursor + by).min(last),
            false => table.cursor.saturating_sub(by),
        };
        let moved = next != table.cursor;
        table.cursor = next;
        table.reveal();
        Some(moved)
    }

    /// The first or last row of the table the cursor is on.
    fn jump_table(&mut self, last: bool) -> Option<bool> {
        let at = self.cursor();
        let table = self.table_mut(at)?;
        let edge = match last {
            true => table.rows.len().saturating_sub(1),
            false => 0,
        };
        let moved = edge != table.cursor;
        table.cursor = edge;
        table.reveal();
        Some(moved)
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

    /// How many rows of a `cols` wide column each line of the showing document
    /// takes, which is what everything about that column is counted in.
    ///
    /// Measured on what is drawn rather than on the source: the formatter eats
    /// the marks and puts structure in front of a line (`▌ `, `• `, `│ `), so a
    /// line counted as its own text and drawn as the rendered one drifts by a
    /// row as soon as it is near the width of the column.
    pub fn doc_heights(&self, cols: usize) -> Vec<usize> {
        let Some(entry) = self.showing() else {
            return Vec::new();
        };
        let mut fence = crate::markdown::Fence::default();
        entry
            .doc
            .iter()
            .map(|line| {
                let shown = crate::markdown::shown(line, &mut fence);
                text_geometry::rows_in(&shown, cols, crate::state::PANE_WRAP).len()
            })
            .collect()
    }

    /// The showing document as a pane, which is what selecting over it is
    /// measured in.
    ///
    /// A pane rather than a second arithmetic of its own: the panes already
    /// know how a Markdown line reaches the screen, how many rows it takes
    /// there, which character a row holds and what a run of them copies, and
    /// every one of those answers has to be the same answer the column was
    /// drawn with. Built here rather than kept, because the document is a
    /// property of whichever entry the cursor is on and a stored one would go
    /// stale the moment the cursor moved.
    ///
    /// Its capacity is the whole document, so nothing falls off the front and a
    /// line's number in the pane is its number in the file.
    pub fn doc_pane(&self) -> crate::state::Pane {
        let empty: Vec<String> = Vec::new();
        let doc = self.showing().map_or(&empty, |entry| &entry.doc);
        let mut pane = crate::state::Pane::new(doc.len().max(1)).rendered();
        for text in doc {
            pane.push(crate::state::Line::new(
                text.clone(),
                crate::state::Tone::Body,
            ));
        }
        pane
    }

    /// The same, scrolled to where the column is drawn, so the row a pointer is
    /// over is the row the pane resolves.
    ///
    /// The column keeps its place as a row counted from the top and a pane
    /// keeps its place as rows held back from the bottom, so the two are the
    /// same place said the two ways round.
    pub fn doc_pane_at(&self, cols: usize, rows: usize) -> crate::state::Pane {
        let mut pane = self.doc_pane();
        let heights = self.doc_heights(cols);
        pane.scrollback = text_geometry::scrollback_for(&heights, rows, self.doc_at(&heights, rows));
        pane
    }

    /// Which row of that document the column starts on, given its heights.
    ///
    /// A wrapped row, not a line of the file: a line of a `SKILL.md` is as many
    /// rows as the column is narrow, and a scroll counted in lines would step
    /// over a paragraph at a time and run off the end of a document that fits.
    /// Clamped here rather than where it is set, so a column that got narrower
    /// or an entry whose document is shorter cannot be left showing nothing.
    fn doc_at(&self, heights: &[usize], rows: usize) -> usize {
        self.here()
            .doc_first
            .min(text_geometry::max_scrollback(heights, rows))
    }

    /// The same, in a box `cols` wide and `rows` tall. Only the tests want the
    /// number on its own: what the drawing asks for is [`Settings::doc_window`].
    #[cfg(test)]
    pub fn doc_first(&self, cols: usize, rows: usize) -> usize {
        self.doc_at(&self.doc_heights(cols), rows)
    }

    /// Which lines of it to draw, and how much of the first one is above the
    /// box, for a column `cols` wide and `rows` tall.
    pub fn doc_window(&self, cols: usize, rows: usize) -> text_geometry::Window {
        let heights = self.doc_heights(cols);
        let back = text_geometry::scrollback_for(&heights, rows, self.doc_at(&heights, rows));
        text_geometry::window(&heights, rows, back)
    }

    /// How much of that document is on screen, for its own scrollbar.
    pub fn doc_thumb(&self, cols: usize, rows: usize) -> Option<(f32, f32)> {
        let heights = self.doc_heights(cols);
        let back = text_geometry::scrollback_for(&heights, rows, self.doc_at(&heights, rows));
        text_geometry::thumb(&heights, rows, back)
    }

    /// Move that column, for a wheel with the pointer over it.
    pub fn scroll_doc(&mut self, by: usize, down: bool, cols: usize, rows: usize) -> bool {
        let most = text_geometry::max_scrollback(&self.doc_heights(cols), rows);
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

    /// Move the cursor one row of the chosen section, over anything it cannot
    /// land on. Never the rail: which section is showing is a press on it or a
    /// Tab, and no arrow key. Clamped at both ends, since a list that wraps under an
    /// arrow key held down is a cursor that arrives somewhere nobody was
    /// looking.
    pub fn step(&mut self, down: bool) -> bool {
        // The footer goes back to saying what the keys do the moment a key is
        // pressed: a swatch that was pressed a screen ago is not what the
        // keyboard is on. The column beside the list goes back to the top of
        // its document for the same reason: the cursor is about to be on
        // another entry, and that entry's text starts at its own first line.
        self.picked = None;
        self.arming = None;
        self.rewind_doc();
        // Inside a table these walk its rows rather than the panel's: the card
        // is one row of the panel and the conversation being picked is one row
        // of the card. The marks are left alone, unlike the armed delete above:
        // a set the arrow keys emptied could only ever hold one row.
        if let Some(moved) = self.step_table(down, 1)
            && moved
        {
            return true;
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
        // A block of text is read with these keys rather than paged past. The
        // cursor is on it, up and down are still how it is left, and the rows
        // under it do not move while it is being read.
        if matches!(self.at_cursor(), Some(Row::Paper(_))) {
            let at = self.cursor();
            return self.scroll_paper(at, PAPER_LINES, down);
        }
        // A table is paged the same way, by the rows its body holds: the list
        // under the pointer is the one being paged.
        if let Some(moved) = self.step_table(down, TABLE_ROWS) {
            return moved;
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
        // The two ends of a block of text, when the cursor is on one. Page moves
        // it a screenful and this takes it the whole way, which is the pair of
        // keys every other scrolling thing in this window answers: a block whose
        // only route was the wheel was a block nobody reading with the keyboard
        // could reach the end of.
        if matches!(self.at_cursor(), Some(Row::Paper(_))) {
            let at = self.cursor();
            if let Some(Row::Paper(paper)) = self.here_mut().rows.get_mut(at) {
                return paper.jump_to(last);
            }
        }
        // The ends of the table, when the cursor is in one: the section has
        // nothing else the cursor can reach, and Home in a list of two hundred
        // conversations means the first conversation.
        if let Some(moved) = self.jump_table(last) {
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

    /// Put the cursor on the row under the pointer, when that row can hold it,
    /// so the keyboard follows the pointer.
    ///
    /// `side` is which half of a form row was pressed. A press on the half that
    /// is a reading lands on the control beside it rather than on nothing, the
    /// same way the keyboard resolves it.
    pub fn point_at(&mut self, index: usize, side: Side) -> bool {
        let Some(row) = self.row(index) else {
            return false;
        };
        let side = match landable_at(row, side) {
            true => side,
            false => side.other(),
        };
        if !landable_at(row, side) {
            return false;
        }
        self.picked = None;
        // Only when the pointer moved to another row: the press that deletes is
        // the second one on the same uninstall, and it comes through here first.
        if self.arming.is_some_and(|at| at != index) {
            self.arming = None;
        }
        self.rewind_doc();
        let was = (self.here().cursor, self.side());
        let section = self.here_mut();
        section.cursor = index;
        section.side = side;
        (index, side) != was
    }

    /// How many rows of text each row of the list takes, for the scroll window,
    /// in a list `cols` characters wide.
    ///
    /// Not one each any more: a heading is drawn larger than the settings under
    /// it and takes two, and an entry takes what its description wrapped to
    /// ([`lines`]). A value too long for the panel is still clipped rather than
    /// wrapped, so a click still cannot resolve to a setting other than the one
    /// under the pointer.
    pub fn heights(&self, cols: usize) -> Vec<usize> {
        text_geometry::heights(self.here().rows.iter().map(|row| lines(row, cols)), 1)
    }

    /// Which rows are on screen in a list `rows` tall and `cols` wide: the first
    /// one, and how many fit under it.
    ///
    /// Anchored on a row rather than on a row of text, so the top of the list is
    /// always the top of a row. Half a heading at the top of the list is a
    /// heading nobody can read whose click region starts off the screen, and
    /// every hit region below it would be a row out of step with what is drawn.
    /// A row that does not fit whole is left for the next screenful, except when
    /// it is the only one there is room to start with.
    pub fn window(&self, rows: usize, cols: usize) -> (usize, usize) {
        let heights = self.heights(cols);
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

    /// Bring the cursor on screen, for a `rows` tall and `cols` wide list.
    pub fn reveal(&mut self, rows: usize, cols: usize) -> bool {
        if rows == 0 || self.here().rows.is_empty() {
            return false;
        }
        let heights = self.heights(cols);
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
    pub fn scroll(&mut self, by: usize, down: bool, rows: usize, cols: usize) -> bool {
        let most = last_top(&self.heights(cols), rows);
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
    pub fn thumb(&self, rows: usize, cols: usize) -> Option<(f32, f32)> {
        let heights = self.heights(cols);
        let (first, _) = self.window(rows, cols);
        // The scrollbar counts rows of text, so the row the list starts on has
        // to be turned into the row of text it starts on first.
        let above: usize = heights.iter().take(first).sum();
        let back = text_geometry::scrollback_for(&heights, rows, above);
        text_geometry::thumb(&heights, rows, back)
    }

    /// The wheel, with the pointer over row `over` of a list `rows` tall and
    /// `cols` wide. `pages` is signed the way the window's wheel is: negative is
    /// down the list.
    ///
    /// A block of text and a table scroll inside themselves, so the wheel over
    /// one of them moves that and not the list behind it, each by its own body
    /// rather than by a screenful of the panel. At either end of it the wheel
    /// goes on to the list: a block claims nineteen rows of the panel, so over
    /// the end of one the wheel used to do nothing at all, which reads as a
    /// window that has stopped answering rather than as a block that is
    /// finished.
    ///
    /// Here rather than in the window because it is the model that knows which
    /// rows scroll inside themselves. The window knows only which row the
    /// pointer is on.
    pub fn wheel(&mut self, over: Option<usize>, pages: f32, rows: usize, cols: usize) -> bool {
        let down = pages < 0.0;
        let by = |lines: usize| ((lines as f32 * pages.abs()).round() as usize).max(1);
        if let Some(index) = over {
            if self.paper(index).is_some() && self.scroll_paper(index, by(PAPER_LINES), down) {
                return true;
            }
            if self.table(index).is_some() && self.scroll_table(index, by(TABLE_ROWS), down) {
                return true;
            }
        }
        self.scroll(by(rows), down, rows, cols)
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
        // A table holds it because there is something to do to a conversation:
        // the arrow keys pick one inside it, space marks it, and the buttons in
        // its footer act on what is marked. A table with nothing in it is never
        // built, and would be a card the cursor stopped on for nothing.
        Row::Table(table) => !table.rows.is_empty(),
        // A card of readings is read and never landed on; a card with something
        // to set holds the cursor on whichever of its first two fields that is.
        // Only those two, because a press carries a [`Side`]: a card that kept
        // the cursor for a field nothing can name is a row the arrow keys stop
        // on and no key changes.
        //
        // A card whose own button does something holds it too, fields or not:
        // the restore under the palette has no fields at all, and a card the
        // keys cannot reach is a button only a pointer can press.
        Row::Card(card) => {
            card.does.is_some()
                || [Side::Left, Side::Right]
                    .into_iter()
                    .any(|side| card_field(card, side).is_some_and(CardField::editable))
        }
        _ => false,
    }
}

/// Whether one slot of a row can hold the cursor: the field a card keeps there,
/// or the half of a form.
fn landable_at(row: &Row, side: Side) -> bool {
    control(row, side).is_some_and(landable)
}

/// Whether a row is read a slot at a time, which is what makes the shifted
/// arrow keys mean something on it: a form of two halves, or a card whose
/// fields are both set here.
fn two_sided(row: &Row) -> bool {
    matches!(row, Row::Card(_))
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
    match change.key == THEME {
        // A theme is not one line. An explicit colour beats the preset it
        // belongs to, so a file carrying eight of them answered every theme
        // change with the same window under a different name and the panel then
        // read the palette back as custom: "themes are only 2 custom and noob".
        // Picking one takes those lines out of the way as it writes.
        true => config::pick_theme(path, &change.value)?,
        false => config::write_setting(path, change.key, Some(&change.value))?,
    }
    Ok(Config::load_from(path))
}

/// Take every appearance line out of the window's settings file and read the
/// whole file back.
///
/// The other half of [`commit`], for the one press that writes no value at all:
/// [`restoring`] names the keys, the writer comments each of them out where it
/// stands, and what comes back is the file parsed again, so the window and the
/// next launch agree the way they do after any other change here.
pub fn restore(path: &Path) -> Result<Config, String> {
    config::clear_settings(path, &restoring())?;
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

/// Test helpers shared by the frame's tests and the section boxes' tests: a
/// panel over a config, scratch files and directories, and readers over the
/// rows. Test-only, and never part of the box's public surface.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::time::{Duration, SystemTime};

    /// How wide the list is taken to be where a test does not care. Only a row
    /// carrying an entry reads it, since that is the only kind of row whose
    /// height depends on the width it wraps in.
    pub const COLS: usize = 60;

    pub fn over(config: &Config) -> Settings {
        Settings::open(config, Some(Path::new("/tmp/no0b.conf")), Agent::default())
    }

    /// A scratch settings file of its own per test, since the writer works on a
    /// real path and two tests sharing one would fight over it.
    pub fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("no0b-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir.join(format!("{name}.conf"))
    }

    pub fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("no0b-panel-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    /// Press a section on the rail, which is what a pointer does; Tab is the
    /// other way in, and both land here.
    ///
    /// This used to walk the rail with the arrow keys. They move the cursor
    /// down the rows of the chosen section now and never touch the rail, which
    /// is the whole point of this round: up and down in a list of saved
    /// conversations must not swap the list out.
    pub fn go_to(panel: &mut Settings, name: &str) {
        let at = panel
            .section_names()
            .iter()
            .position(|section| *section == name)
            .unwrap_or_else(|| panic!("{name} is not on the rail"));
        panel.choose(at);
        assert_eq!(panel.here().name, name);
    }

    pub fn setting<'a>(panel: &'a Settings, key: &str) -> &'a Row {
        panel
            .all_rows()
            .map(|(_, row)| row)
            .find(|row| matches!(row, Row::Setting { key: k, .. } if *k == key))
            .unwrap_or_else(|| panic!("{key} is not on the panel"))
    }

    pub fn value(panel: &Settings, key: &str) -> String {
        match setting(panel, key) {
            Row::Setting { value, .. } => value.clone(),
            other => panic!("{other:?}"),
        }
    }

    /// Put the cursor on a setting wherever it lives: section, row and, on a
    /// form row, which half of it.
    pub fn put_cursor(panel: &mut Settings, key: &str) {
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
                        matches!(control(row, *side), Some(Row::Setting { key: k, .. }) if *k == key)
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
    pub fn swatch_at(panel: &Settings, key: &str) -> (usize, usize) {
        panel
            .rows()
            .iter()
            .enumerate()
            .find_map(|(at, row)| match row {
                Row::Palette(palette) => palette
                    .cells
                    .iter()
                    .position(|cell| cell.key == key)
                    .map(|cell| (at, cell)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{key} is not on the grid"))
    }

    /// Everything a section says, as one string, for the tests that care what is
    /// on it rather than which row it is on.
    pub fn said(panel: &Settings) -> String {
        panel.rows().iter().map(says).collect::<Vec<_>>().join("\n")
    }

    pub fn says(row: &Row) -> String {
        match row {
                Row::Note { text, .. } => text.clone(),
                Row::Table(table) => {
                    let mut out = vec![table.title(), table.names.join(" ")];
                    out.extend(table.rows.iter().map(|row| row.cells.join("  ")));
                    out.join("\n")
                }
                Row::Reading { label, value } => format!("{label} {value}"),
                Row::Setting { key, value, .. } | Row::Field { key, value } => {
                    format!("{key} {value}")
                }
                Row::Palette(palette) => {
                    let mut out = vec![String::from(palette.title)];
                    out.extend(
                        palette
                            .cells
                            .iter()
                            .map(|cell| format!("{} {} {}", cell.key, cell.about, hex(cell.rgb))),
                    );
                    out.join("\n")
                }
                Row::Entry(entry) => format!(
                    "{} {} {} {}",
                    entry.name,
                    entry.about,
                    entry.under,
                    match entry.on {
                        true => "on",
                        false => "off",
                    }
                ),
                Row::Paper(paper) => format!(
                    "{}\n{}\n{}",
                    paper.title,
                    paper.under,
                    paper.body.join("\n")
                ),
                // The title, then every field as its label, its value and the
                // sentence under it, then the sentence under the card.
                Row::Card(card) => {
                    let mut out = vec![card.title.clone()];
                    for field in &card.fields {
                        out.push(format!("{} {}", field.label, field.value()));
                        out.extend(field.hint.clone());
                    }
                    out.extend(card.hint.clone());
                    out.join("\n")
                }
        }
    }

    pub fn a_session(
        id: &str,
        ago: u64,
        folder: Option<&str>,
        opening: &str,
    ) -> crate::sessions::Saved {
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

    /// The skills section with one skill on it whose document is whatever the
    /// test needs. No disk: what is being tested is the arithmetic over the
    /// document, not where it was read from.
    pub fn a_panel_showing(doc: Vec<String>) -> Settings {
        let agent = Agent {
            skills_at: Some(PathBuf::from("/home/hec/.config/noob/skills")),
            skills: vec![agent::Skill {
                dir: String::from("coding"),
                name: String::from("coding"),
                about: String::from("Changing code that already exists."),
                repo: None,
                path: PathBuf::from("/home/hec/.config/noob/skills/coding"),
                on: true,
                doc,
            }],
            ..Agent::default()
        };
        let mut panel = Settings::open(&Config::default(), None, agent);
        go_to(&mut panel, SKILLS);
        panel
    }
}

#[cfg(test)]
mod tests {
    use super::sections::agent::AGENT_SETTINGS;
    use super::sections::appearance::{colours, LOOKS};
    use super::testing::*;
    use super::*;
    use std::time::{Duration, SystemTime};

    /// Every card on the panel is a card a press can work: each field named,
    /// and nothing that can be changed sitting where no press can name it.
    ///
    /// A press carries a [`Side`] and a side is one of two, so the two fields a
    /// card can be changed through are its first two. A third one would draw as
    /// a control and answer nothing.
    #[test]
    fn every_card_on_the_panel_is_named_and_reachable() {
        let mut panel = over(&Config::default());
        let dir = scratch_dir("cards");
        std::fs::write(
            dir.join(".env"),
            "NOOB_BASE_URL=http://localhost:8080/v1\nNOOB_MODEL=laguna-s\nNOOB_SOMETHING=1\n",
        )
        .expect("a file");
        panel.adopt_agent(
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
            &Config::default(),
        );
        let mut cards = 0;
        for (section, row) in panel.all_rows() {
            let Row::Card(card) = row else {
                continue;
            };
            cards += 1;
            assert!(!card.title.is_empty(), "{section}: a card with no title");
            assert!(
                card_is_reachable(card),
                "{section}: {} keeps a field that can be set where no press can name it",
                card.title
            );
            for field in &card.fields {
                assert!(
                    !field.label.is_empty(),
                    "{section}: {} has a field with no name",
                    card.title
                );
                assert!(
                    field.hint.is_some() || card.hint.is_some(),
                    "{section}: {} says nothing about {}",
                    card.title,
                    field.label
                );
            }
        }
        assert!(cards >= 6, "only {cards} cards on the whole panel");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every section on the rail is reached by pressing it, and every one of
    /// them has something on it.
    ///
    /// This used to walk the rail with `step`, and asserted the panel opened on
    /// the rail and went in and out of it with `enter` and `leave`. None of
    /// that exists any more: up and down are the rows of whatever is showing,
    /// the rail is pressed, and the two keys can no longer take a list out from
    /// under whoever is reading it.
    #[test]
    fn every_section_is_reachable() {
        let mut panel = over(&Config::default());
        assert_eq!(panel.section_names(), SECTIONS.to_vec());
        assert_eq!(panel.chosen(), 0, "the panel opens on the first section");

        // A rail entry that opens on nothing is a section that reads as broken.
        for (at, name) in SECTIONS.iter().enumerate() {
            panel.choose(at);
            assert_eq!(panel.here().name, *name, "the rail cannot reach {name}");
            assert!(!panel.rows().is_empty(), "{name} is empty");
        }

        // Pressing the section that is already showing changes nothing, and a
        // press past the end of the rail is not a press.
        assert!(!panel.choose(SECTIONS.len() - 1), "already there");
        assert!(!panel.choose(SECTIONS.len()), "off the end of the rail");
        assert_eq!(panel.here().name, SECTIONS[SECTIONS.len() - 1]);

        // And the arrow keys never move it, in any section, in either
        // direction, however long they are held down.
        for (at, name) in SECTIONS.iter().enumerate() {
            panel.choose(at);
            for down in [true, false] {
                for _ in 0..40 {
                    panel.step(down);
                    panel.page(8, down);
                    panel.jump(down);
                    assert_eq!(panel.chosen(), at, "a key walked off {name}");
                }
            }
        }
    }

    /// Item F1: Tab walks the sections and the arrow keys go on walking the
    /// rows.
    ///
    /// The rail was pointer-only, so a font size raised far enough to push
    /// APPEARANCE off the bottom of the rail left no way at all to lower it
    /// again. Tab is the way in from the keyboard: on to the next section,
    /// Shift-Tab back to the one before, wrapping at both ends so every section
    /// is reachable from every other one whatever the window is doing. The
    /// window binds it in `walk_in_settings`, which is tested beside it.
    #[test]
    fn tab_walks_the_sections_and_the_arrows_walk_the_rows() {
        let mut panel = over(&Config::default());
        assert_eq!(panel.chosen(), 0);

        // Forward through every section and round to the front again.
        for name in SECTIONS.iter().skip(1) {
            assert!(panel.walk_section(true), "stuck on {}", panel.here().name);
            assert_eq!(panel.here().name, *name);
        }
        assert!(panel.walk_section(true), "the last section is a dead end");
        assert_eq!(panel.here().name, SECTIONS[0], "forward does not wrap");

        // And backward, which from the first section is the last one.
        assert!(panel.walk_section(false));
        assert_eq!(
            panel.here().name,
            SECTIONS[SECTIONS.len() - 1],
            "back does not wrap"
        );
        for at in (0..SECTIONS.len() - 1).rev() {
            assert!(panel.walk_section(false));
            assert_eq!(panel.here().name, SECTIONS[at]);
        }
        assert_eq!(panel.chosen(), 0);

        // APPEARANCE, which is the section this key exists for, is a few
        // presses away in both directions and lands on the same place.
        let looks = SECTIONS
            .iter()
            .position(|name| *name == APPEARANCE)
            .expect("appearance is a section");
        let mut forward = over(&Config::default());
        for _ in 0..looks {
            forward.walk_section(true);
        }
        assert_eq!(forward.here().name, APPEARANCE);
        let mut back = over(&Config::default());
        for _ in 0..SECTIONS.len() - looks {
            back.walk_section(false);
        }
        assert_eq!(back.here().name, APPEARANCE);

        // The arrows are still the rows: walking a section with them moves the
        // cursor and leaves the rail exactly where Tab put it.
        let was = forward.chosen();
        assert!(forward.step(true), "up and down stopped moving the cursor");
        assert_eq!(forward.cursor(), 1);
        assert_eq!(forward.chosen(), was, "an arrow key walked the rail");
        assert!(forward.step(false));
        assert_eq!((forward.cursor(), forward.chosen()), (0, was));

        // And the footer names both, in every section, on every row the cursor
        // can land on: the legend is the only thing that says the arrows are
        // the rows and Tab is the rail.
        for (at, name) in SECTIONS.iter().enumerate() {
            panel.choose(at);
            for _ in 0..panel.rows().len() {
                let hint = panel.hint();
                assert!(
                    hint.contains("tab and shift-tab change section"),
                    "{name} does not name the section keys: {hint}"
                );
                assert!(
                    hint.contains("up and down")
                        || hint.contains("left and right")
                        || hint.contains("page up"),
                    "{name} does not name the arrows: {hint}"
                );
                panel.step(true);
            }
        }
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

    /// The cursor only stops where something can happen: not on a card of
    /// readings, not on a note and not on a colour.
    ///
    /// The section used to open on `theme`, because `theme` was its first row.
    /// It is the first field of the palette's own card now, over the colours it
    /// writes, so what the section opens on is the first of the sizes and the
    /// assertion says that instead. `the_theme_field_is_the_top_of_the_palette`
    /// is where the move itself is held.
    #[test]
    fn the_cursor_skips_what_it_cannot_change() {
        let config = Config::default();
        let mut panel = over(&config);
        go_to(&mut panel, APPEARANCE);
        assert!(
            matches!(panel.at_cursor(), Some(Row::Setting { key, .. }) if *key == "font_size"),
            "the section opens on {:?}",
            panel.at_cursor()
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

        // APPEARANCE stops on the cards that carry a setting and on the one
        // whose own button does something: the palette's own cards and the one
        // naming the file are stepped over rather than landed on. Four cards
        // carry the six settings, since two fields share a card twice, and the
        // fifth stop is the restore, which has no fields at all and would
        // otherwise be a button only a pointer could reach.
        go_to(&mut panel, APPEARANCE);
        let mut cards = 1;
        while panel.step(true) {
            cards += 1;
        }
        assert_eq!(cards, 5);
        assert!(
            matches!(panel.row(panel.cursor()), Some(Row::Card(card))
                if card.does == Some(Doing::Restore)),
            "the last stop is {:?}",
            panel.row(panel.cursor())
        );
        let held: Vec<&str> = panel
            .rows()
            .iter()
            .filter(|row| landable(row))
            .flat_map(|row| match row {
                Row::Card(card) => card
                    .fields
                    .iter()
                    .filter_map(|field| match field.holds.as_ref() {
                        Row::Setting { key, .. } => Some(*key),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(held.len(), LOOKS.len(), "{held:?}");

        // Every section now carries something to land on; MCP's is the add
        // card's own fields.
        go_to(&mut panel, MCP);
        assert!(panel.on_row());
        go_to(&mut panel, APPEARANCE);
        assert!(panel.on_row());
    }

    /// No row on the panel is an on and off any more, and a row that is not a
    /// setting writes nothing.
    ///
    /// This was `a_flag_flips_and_writes_what_the_file_reads`, which drove the
    /// `show_files` row. The only two flags the window's file carries are the
    /// two panes, and neither is a row now: a closed pane comes back off the
    /// right click menu and the file goes on remembering which are open. What
    /// survives of that test is its last half, which belongs to every row: a
    /// nudge aimed at a row with nothing to nudge must not write a setting the
    /// cursor happens to have been on before.
    #[test]
    fn no_row_is_an_on_and_off_and_a_list_writes_nothing() {
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
        go_to(&mut panel, MCP);
        assert_eq!(panel.change(true), None, "a list is not a setting");
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

        // And the value the window opens at is one of the steps: 88% was
        // between two of them, so the first arrow key left it for good.
        let mut panel = over(&Config::default());
        put_cursor(&mut panel, "opacity");
        assert_eq!(value(&panel, "opacity"), "0.90");
        assert_eq!(panel.change(true).expect("a number").value, "0.95");
        assert_eq!(panel.change(false).expect("a number").value, "0.85");
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
        for key in [
            "opacity",
            "window_opacity",
            "font_size",
            "pane_font_size",
            "prompt_rows",
        ] {
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
                Row::Reading { label, value } if label == "the settings file" => {
                    Some(value.clone())
                }
                _ => None,
            })
            .expect("the file field");
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

    /// The footer says what the keys will do where the keyboard is, and never
    /// says the arrow keys choose a section.
    ///
    /// It used to open with "up and down choose a section", which was the line
    /// that told him the two keys did the one thing he did not want them to do.
    /// They walk the rows now, so nothing on the panel may still say that.
    #[test]
    fn the_footer_says_what_the_keys_do_here() {
        let config = Config::default();
        let mut panel = over(&config);
        put_cursor(&mut panel, "theme");
        assert!(panel.hint().contains("press a theme"), "{}", panel.hint());
        // On a card of two, the one thing the arrow keys cannot say for
        // themselves is how to reach the other field, so that is what the line
        // spends itself on; on a card of one it names the slider.
        put_cursor(&mut panel, "opacity");
        assert!(panel.hint().contains("nudge it"), "{}", panel.hint());
        assert!(panel.hint().contains("cross the card"), "{}", panel.hint());
        put_cursor(&mut panel, "prompt_rows");
        assert!(panel.hint().contains("slider"), "{}", panel.hint());

        // Every footer the panel can say, walking every row of every section.
        for (at, name) in SECTIONS.iter().enumerate() {
            panel.choose(at);
            loop {
                let said = panel.says();
                for wrong in ["choose a section", "back to the sections", "right goes in"] {
                    assert!(!said.contains(wrong), "{name}: {said}");
                }
                if !panel.step(true) {
                    break;
                }
            }
        }

        // On the table of saved conversations it names all four of its keys:
        // nothing else on the panel says that space marks a conversation or
        // that delete takes every marked one.
        let mut panel = Settings::open(
            &config,
            None,
            Agent {
                now: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000),
                sessions: crate::sessions::Listing {
                    sessions: vec![a_session(
                        "aaa",
                        60,
                        Some("/home/hec/workspace/noob-cli"),
                        "fix the panel",
                    )],
                    skipped: Vec::new(),
                },
                ..Agent::default()
            },
        );
        go_to(&mut panel, SESSIONS);
        let said = panel.says();
        assert!(said.contains("up and down pick a session"), "{said}");
        assert!(said.contains("space marks it"), "{said}");
        assert!(said.contains("delete removes what is marked"), "{said}");
    }

    /// A section's list scrolls like every other list in the window, and the
    /// cursor is brought on screen rather than left off the bottom of it.
    #[test]
    fn the_list_scrolls_and_the_cursor_is_kept_on_screen() {
        let config = Config::default();
        let mut panel = over(&config);
        go_to(&mut panel, APPEARANCE);
        let rows = 10;
        // Counted in rows of text rather than in rows of the list: a card is
        // nine or ten of them, so a section of ten cards is many screenfuls
        // even though it is ten rows.
        let tall: usize = panel.heights(COLS).iter().sum();
        assert!(tall > rows, "the longest section is one screenful");
        assert_eq!(panel.first(), 0);
        assert!(panel.thumb(rows, COLS).is_some(), "a list this long says so");

        // The wheel moves the window and leaves the cursor where it was.
        let cursor = panel.cursor();
        assert!(panel.scroll(3, true, rows, COLS));
        assert_eq!(panel.cursor(), cursor);
        assert!(panel.scroll(3, false, rows, COLS));

        // A section short enough for its window does not pretend to scroll:
        // the empty MCP section is one add card.
        go_to(&mut panel, MCP);
        let tall: usize = panel.heights(COLS).iter().sum();
        assert!(
            panel.thumb(tall + 1, COLS).is_none(),
            "one card does not scroll in a window that holds it"
        );

        // Down to the last setting of a longer one, in a window two rows tall:
        // the window follows the cursor to both ends.
        go_to(&mut panel, APPEARANCE);
        assert!(panel.jump(true));
        panel.reveal(2, COLS);
        assert!(panel.cursor() < panel.first() + 2, "the cursor is off screen");
        assert!(panel.jump(false));
        panel.reveal(2, COLS);
        assert!(
            panel.first() <= panel.cursor(),
            "the cursor is above the window"
        );

        // A page through the palette stops on nothing inside it: the colours
        // are read here and edited in the file, so the cursor goes over the
        // grid to the card under it rather than landing on a swatch.
        go_to(&mut panel, APPEARANCE);
        let mut stops = vec![panel.cursor()];
        while panel.page(rows, true) {
            stops.push(panel.cursor());
        }
        for at in &stops {
            assert!(
                !matches!(panel.row(*at), Some(Row::Palette(_))),
                "the cursor stopped on row {at} of the palette"
            );
        }
        let (grid, _) = swatch_at(&panel, "gauge_10");
        assert!(stops.iter().any(|at| *at > grid), "the page never got past the grid");
    }

    /// The window is counted in rows of text and starts on a row, so a card
    /// nine rows tall never sits half on and half off the top of the list.
    ///
    /// This counted headings, which were two rows of text and the only thing on
    /// the panel drawn larger than a row. There are no headings: every group is
    /// a card, and a card's height is its header, its body and the space under
    /// it. If [`Settings::heights`] and the window ever disagree, the rows the
    /// panel draws are not the rows it hit tests.
    #[test]
    fn the_window_counts_a_card_as_the_rows_it_takes() {
        let mut panel = over(&Config::default());
        go_to(&mut panel, APPEARANCE);
        let heights = panel.heights(COLS);
        let cards: Vec<usize> = panel
            .rows()
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, Row::Card(_) | Row::Palette(_)))
            .map(|(at, _)| at)
            .collect();
        // Every row of this section is one of the two: there is nothing loose
        // on it any more.
        assert_eq!(cards.len(), panel.rows().len(), "{cards:?}");
        for at in &cards {
            assert!(heights[*at] > 2, "a card is drawn taller than two rows");
        }
        for (at, row) in panel.rows().iter().enumerate() {
            assert_eq!(heights[at], lines(row, COLS), "the model and the window disagree");
        }
        // A palette card is taller in a narrow list than in a wide one, because
        // its colours reflow: the model is handed the width, so the height it
        // counts follows it.
        let (grid, _) = swatch_at(&panel, "tool_bash");
        let narrow = panel.heights(30)[grid];
        let wide = panel.heights(160)[grid];
        assert!(narrow > wide, "the palette does not reflow: {narrow} and {wide}");

        // Whatever it is scrolled to, the rows on screen start at the top of a
        // row and take no more room than the list has.
        let rows = 12;
        for _ in 0..40 {
            let (first, count) = panel.window(rows, COLS);
            let used: usize = heights[first..first + count].iter().sum();
            assert!(count > 0, "the list showed nothing at {first}");
            assert!(
                used <= rows || count == 1,
                "{count} rows from {first} take {used} of {rows}"
            );
            if !panel.scroll(1, true, rows, COLS) {
                break;
            }
        }
        // The end of the list is reachable and stops there.
        let (first, count) = panel.window(rows, COLS);
        assert_eq!(first + count, panel.rows().len(), "the last row is off screen");
        assert!(!panel.scroll(1, true, rows, COLS), "the list scrolled past its end");
    }

    /// A description longer than one row of a narrow column, so the height of
    /// the row that carries it depends on the width it is read in.
    const A_WRAPPING_ABOUT: &str = "reads the file before it writes one and says which file it read";

    /// An entry is as tall as its description wrapped to, and the window counts
    /// it that way.
    ///
    /// The row was three lines whatever the description said, so a long one was
    /// cut off at the width of the column. Now that it wraps, a row measured at
    /// three lines and drawn at five would put every press below it on the wrong
    /// row and let the wheel run off the end of the list.
    #[test]
    fn the_window_counts_an_entry_as_the_rows_its_description_wrapped_to() {
        let dir = scratch_dir("wrapping-skill");
        let skills = dir.join("skills");
        std::fs::create_dir_all(skills.join("coding")).expect("a directory");
        std::fs::write(
            skills.join("coding").join("SKILL.md"),
            format!("---\nname: coding\ndescription: {A_WRAPPING_ABOUT}\n---\n\n# Changing code\n"),
        )
        .expect("a file");
        let agent = Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, agent);
        go_to(&mut panel, SKILLS);
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Entry(_)))
            .expect("the skill is on the list");

        // Wide enough for the description on one row of the card's own body,
        // which is narrower than the list by the border and the padding.
        let wide = A_WRAPPING_ABOUT.chars().count() + 1 + crate::design::CARD_COLUMNS;
        let body = crate::design::card_cols(wide);
        assert_eq!(about_rows(A_WRAPPING_ABOUT, body), 1);
        let one = panel.heights(wide)[at];
        assert_eq!(one, lines(panel.row(at).expect("the row"), wide));
        assert_eq!(
            one,
            crate::design::card_row_lines(
                1.0 + crate::design::TIGHT + crate::design::TEXT_LINES,
                true
            ),
            "an entry is a card of one row of words, the line under it and a footer"
        );

        // Half of it, and the description is more than one row, so the card is
        // taller by exactly what it wrapped to.
        let half = wide / 2;
        let wrapped = about_rows(A_WRAPPING_ABOUT, crate::design::card_cols(half));
        assert!(wrapped > 1, "{wrapped} rows in {half} columns");
        let many = panel.heights(half)[at];
        assert_eq!(many, lines(panel.row(at).expect("the row"), half));
        assert_eq!(
            many,
            one + wrapped - 1,
            "the wrapped card did not grow by the rows it wrapped to"
        );

        // And the window counts it: the section is the card naming the skills
        // directory and the entry under it, and a list one row short of both of
        // them stops at the first.
        assert_eq!(panel.rows().len(), 2, "{:?}", panel.rows());
        let both: usize = panel.heights(wide).iter().sum();
        assert_eq!(panel.window(both, wide), (0, 2), "the section fits");
        assert_eq!(
            panel.window(both - 1, wide),
            (0, 1),
            "a row that does not fit whole was drawn anyway"
        );
    }

    /// The wheel over a block reads the block, leaves the section where it was,
    /// and goes on to the section once the block has nothing left to show.
    ///
    /// The last part is the fix: a block claims nineteen rows of the panel, so a
    /// wheel that stopped dead over one was most of the window answering
    /// nothing once the block was at its end.
    #[test]
    fn the_wheel_over_a_block_reads_it_and_goes_on_to_the_list_at_its_end() {
        let dir = scratch_dir("agent-block-wheel");
        let mut panel = Settings::open(
            &Config::default(),
            None,
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
        );
        go_to(&mut panel, AGENT);
        panel.adopt_prompt(
            String::from("/home/hec/workspace/noob-cli"),
            Ok((0..PAPER_LINES * 3).map(|at| format!("line {at}")).collect()),
            &Config::default(),
        );
        go_to(&mut panel, AGENT);
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Paper(paper) if paper.title.contains("PROMPT")))
            .expect("the prompt block");
        let (rows, cols) = (12, 80);

        // Over the block: the block moves and the section does not.
        let was = panel.first();
        assert!(panel.wheel(Some(at), -1.0, rows, cols));
        assert_eq!(panel.paper(at).expect("the block").first, PAPER_LINES);
        assert_eq!(panel.first(), was, "the section moved with the block");

        // Wheeled to the end of the block, the next notch moves the section
        // instead of doing nothing at all.
        while panel.paper(at).expect("the block").first
            < panel.paper(at).expect("the block").body.len() - PAPER_LINES
        {
            assert!(panel.wheel(Some(at), -1.0, rows, cols));
        }
        assert!(panel.wheel(Some(at), -1.0, rows, cols), "the wheel went dead");
        assert!(panel.first() > was, "the section did not take the wheel on");

        // And nowhere near a block, the wheel is the section's, as it always
        // was.
        let was = panel.first();
        assert!(panel.wheel(None, 1.0, rows, cols));
        assert!(panel.first() < was);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A block longer than its box reports how much of it is off screen, and one
    /// that fits reports nothing at all: a bar that is always there says nothing
    /// about whether there is more to read.
    ///
    /// The block had no bar of any kind. The nearest one belonged to the section
    /// list, counted the block as the nineteen rows its card claims rather than
    /// the hundreds of lines in it, and did not move when the wheel over the
    /// block did.
    #[test]
    fn a_block_says_how_much_of_it_is_off_screen_and_a_short_one_says_nothing() {
        let dir = scratch_dir("agent-block-extent");
        let mut panel = Settings::open(
            &Config::default(),
            None,
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
        );
        go_to(&mut panel, AGENT);
        let body: Vec<String> = (0..PAPER_LINES * 4).map(|at| format!("line {at}")).collect();
        panel.adopt_prompt(
            String::from("/home/hec/workspace/noob-cli"),
            Ok(body.clone()),
            &Config::default(),
        );
        go_to(&mut panel, AGENT);
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Paper(paper) if paper.title.contains("PROMPT")))
            .expect("the prompt block");

        // A quarter of it is on screen, so the thumb is a quarter of the track
        // and it starts at the top.
        let (top, size) = panel
            .paper(at)
            .expect("the block")
            .thumb(PAPER_LINES)
            .expect("a block four screenfuls long draws a bar");
        assert!((size - 0.25).abs() < 0.01, "the thumb says {size} of it fits");
        assert!(top.abs() < 0.01, "it is not at the top of its own track");

        // Scrolled to the end, the thumb is at the foot of the track.
        assert!(panel.scroll_paper(at, 9_999, true));
        let (top, size) = panel
            .paper(at)
            .expect("the block")
            .thumb(PAPER_LINES)
            .expect("still a bar");
        assert!(
            (top + size - 1.0).abs() < 0.01,
            "the thumb is at {top} rather than the foot of its track"
        );

        // A box showing fewer lines than the block was measured for, which is
        // what the bottom of the list cuts a card down to, reports the lines it
        // is really showing rather than twelve of them.
        let (_, cut) = panel
            .paper(at)
            .expect("the block")
            .thumb(PAPER_LINES / 2)
            .expect("still a bar");
        assert!(cut < size, "a shorter box says as much fits as a tall one");

        // And a block that is already all on screen has no bar at all.
        let short: Vec<String> = (0..PAPER_LINES - 2).map(|at| format!("line {at}")).collect();
        panel.adopt_prompt(
            String::from("/home/hec/workspace/noob-cli"),
            Ok(short),
            &Config::default(),
        );
        go_to(&mut panel, AGENT);
        assert_eq!(
            panel.paper(at).expect("the block").thumb(PAPER_LINES),
            None,
            "a block that fits its box drew a bar anyway"
        );
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
            (SESSIONS, "none saved yet"),
            (AGENT, "probes the usual local ports"),
        ] {
            go_to(&mut panel, section);
            let text = said(&panel);
            assert!(
                text.contains(wanted),
                "{section} does not say {wanted:?}: {text}"
            );
        }

        // The add card names the file it writes, so there is somewhere to
        // put one, and the empty state is said rather than shown as nothing.
        go_to(&mut panel, MCP);
        let text = said(&panel);
        assert!(
            text.contains(&dir.join("mcp.json").display().to_string()),
            "{text}"
        );
        assert!(text.contains("none configured yet"), "{text}");
        assert!(text.contains("ADD A SERVER"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The column beside the list is counted in the rows it is drawn as, not in
    /// the lines the file has: one long paragraph is many rows in a narrow
    /// column, and it used to be one line that was cut off at the edge.
    #[test]
    fn a_long_document_line_is_as_many_rows_as_it_wraps_to() {
        let long = "the whole point of a document column is that a sentence in it can be read to the end of itself";
        let panel = a_panel_showing(vec![String::from(long), String::from("short")]);
        let heights = panel.doc_heights(20);
        assert_eq!(
            heights.len(),
            2,
            "one height per line of the file, whatever it wraps to"
        );
        assert!(heights[0] >= 5, "a 95 character line in 20 columns: {heights:?}");
        assert_eq!(heights[1], 1);
        // Wider is fewer rows, which is what makes the column worth widening.
        assert!(panel.doc_heights(80)[0] < heights[0]);

        // Counted on what is drawn rather than on the source: the formatter puts
        // a bullet in front of an item and eats the marker that asked for it, so
        // a line measured raw is measured two characters short.
        let marked = a_panel_showing(vec![format!("- **{long}**")]);
        assert_eq!(
            marked.doc_heights(20),
            vec![text_geometry::rows_in(
                &format!("• {long}"),
                20,
                crate::state::PANE_WRAP
            )
            .len()]
        );
    }

    /// The two columns of a two-column section scroll apart: the wheel over the
    /// document moves the document and leaves the list where it was, and the
    /// wheel over the list moves the list and leaves the document where it was.
    #[test]
    fn the_list_and_the_document_scroll_apart() {
        let doc: Vec<String> = (0..40).map(|n| format!("line {n} of it")).collect();
        let mut panel = a_panel_showing(doc);
        let (cols, rows) = (30, 8);
        // The list is three rows long here, so it is asked for a window it does
        // not all fit in; the document is asked for the eight the column holds.
        let list_rows = 2;
        assert_eq!(panel.doc_first(cols, rows), 0);
        let list_was = panel.window(list_rows, COLS);

        assert!(panel.scroll_doc(5, true, cols, rows), "the document moves");
        assert_eq!(panel.doc_first(cols, rows), 5);
        assert_eq!(panel.doc_window(cols, rows).first, 5);
        assert_eq!(panel.window(list_rows, COLS), list_was, "the list moved with it");

        assert!(panel.scroll(1, true, list_rows, COLS), "the list moves");
        assert_ne!(panel.window(list_rows, COLS), list_was);
        assert_eq!(
            panel.doc_first(cols, rows),
            5,
            "the list took the document with it"
        );

        // And neither runs off its own end: the last screenful is the last one.
        assert!(panel.scroll_doc(500, true, cols, rows));
        assert_eq!(
            panel.doc_first(cols, rows),
            40 - rows,
            "40 lines that each fit on one row, 8 at a time"
        );
        assert!(!panel.scroll_doc(500, true, cols, rows), "already at the end");

        // In rows of the column rather than lines of the file: the same document
        // in half the width is twice as far to scroll.
        assert!(panel.scroll_doc(500, true, 8, rows));
        assert!(
            panel.doc_first(8, rows) > 40 - rows,
            "a wrapped document scrolls further than it has lines: {}",
            panel.doc_first(8, rows)
        );
    }

    /// The pane the document is selected in is the document: the same lines,
    /// rendered the same way, wrapped into the same rows, and scrolled to the
    /// same place the column is drawn at.
    ///
    /// One rule rather than two. A pane measured on the source while the column
    /// was drawn on the rendering is a band over glyphs the clipboard has never
    /// heard of, which is the bug this whole arrangement exists to make
    /// impossible.
    #[test]
    fn the_document_pane_is_the_document_the_column_draws() {
        let long = "the whole point of a document column is that a sentence in it can be read to the end of itself";
        let panel = a_panel_showing(vec![
            String::from("- **read** a file"),
            String::from(long),
            String::from("last"),
        ]);
        let (cols, rows) = (20, 6);
        let pane = panel.doc_pane_at(cols, rows);

        // The marks are eaten here exactly as they are on screen.
        assert_eq!(pane.line(0).expect("the line").shown(), "• read a file");
        assert_eq!(pane.last(), 3, "one pane line per line of the file");

        // The rows agree line for line with what the column is scrolled by.
        let heights = panel.doc_heights(cols);
        assert!(heights[1] > 1, "the long line has to wrap");
        for (line, tall) in heights.iter().enumerate() {
            assert_eq!(pane.rows_of_line(line, cols).len(), *tall, "line {line}");
        }
        assert_eq!(pane.window(rows, cols), panel.doc_window(cols, rows));

        // And it follows the column when the column is scrolled.
        let mut panel = panel;
        assert!(panel.scroll_doc(2, true, cols, rows));
        assert_eq!(
            panel.doc_pane_at(cols, rows).window(rows, cols),
            panel.doc_window(cols, rows)
        );

        // A section with no entry at all has no document and no pane lines,
        // rather than a pane of one empty line nothing can point at.
        let mut bare = Settings::open(&Config::default(), None, Agent::default());
        go_to(&mut bare, APPEARANCE);
        assert_eq!(bare.doc_pane().last(), 0);
    }

    /// Moving the cursor to another entry takes the column back to the top of
    /// that entry's own document.
    #[test]
    fn the_document_rewinds_when_the_cursor_leaves_the_entry() {
        let doc: Vec<String> = (0..40).map(|n| format!("line {n}")).collect();
        let mut panel = a_panel_showing(doc);
        assert!(panel.scroll_doc(4, true, 30, 8));
        assert_eq!(panel.doc_first(30, 8), 4);
        panel.step(true);
        assert_eq!(panel.doc_first(30, 8), 0);
    }

}
