//! The settings panel: what the file carries, what it says now, and what
//! changing a row writes back.
//!
//! A full screen takeover rather than a popup or a second OS window. A second
//! window means a second wgpu surface with its own renderer and its own event
//! routing, for a list of rows; a popup over the panes means a scroll region
//! floating over another scroll region. While the panel is up it is the whole
//! window, the way the folder picker is before a folder has been chosen.
//!
//! Nothing here draws and only [`commit`] reads or writes a file.
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
//! The colours are listed and not editable here. Changing one means typing a hex
//! value, which needs a text field with the keyboard focus, and this window has
//! no focus model to give one: `App::key` is its only text sink. So the palette
//! is on the panel as swatches you can read, with the path of the file to edit
//! at the bottom of the list, and the four presets are one row away in `theme`.

use std::path::{Path, PathBuf};

use crate::config::{self, Config};
use crate::totals::Totals;

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
    /// The bounds are the parser's own (`Config::parse` clamps every one of
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
}

/// One row of the panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A group's name.
    Heading(&'static str),
    /// Something read out rather than set: the all-time totals, and where the
    /// files behind all this live.
    Reading { label: &'static str, value: String },
    /// A setting, its value spelled the way the file spells it, and its kind.
    Setting {
        key: &'static str,
        value: String,
        kind: Kind,
    },
}

/// What a nudge on the row under the cursor should write.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub key: &'static str,
    pub value: String,
}

/// What the `theme` row says when the palette in the file is not one of the
/// presets. Not a value that can be written: the writer refuses it, and the row
/// only ever hands it back to itself.
const CUSTOM: &str = "custom";

/// The settings a row can change, grouped the way the panel lists them.
///
/// Only the keys whose shape two arrow keys can cover. Every other key in the
/// file is a colour and is listed from the live [`Config`] instead, so a colour
/// added to the palette appears here with no edit at all.
/// `every_key_in_the_file_is_on_the_panel` fails if a key ends up in neither
/// list, which is what stops a setting being added to the file and forgotten
/// here.
const GROUPS: [(&str, &[(&str, Kind)]); 3] = [
    (
        "WHAT IT LOOKS LIKE",
        &[
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
        ],
    ),
    (
        "WHICH PANES OPEN",
        &[("show_activity", Kind::Flag), ("show_files", Kind::Flag)],
    ),
    (
        // The same two numbers the dividers write when they are dragged. Here
        // as well because a pointer is not the only way anyone works, and
        // because a value that only a drag can reach is a value nobody can read
        // off the window.
        "WHERE THE DIVIDERS SIT",
        &[
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
                "top_height",
                Kind::Number {
                    step: 0.05,
                    low: config::SPLIT_LOW,
                    high: config::SPLIT_HIGH,
                    places: 2,
                },
            ),
        ],
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

pub struct Settings {
    rows: Vec<Row>,
    cursor: usize,
    /// The top row on screen. Top anchored, like the picker and the explorer.
    first: usize,
    /// The settings file, or nothing when there is no home directory to put one
    /// in. Kept so [`Settings::refresh`] does not have to be handed it again.
    file: Option<PathBuf>,
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
    /// written to avoid.
    pub fn open(config: &Config, totals: &Totals, file: Option<&Path>) -> Settings {
        let mut panel = Settings {
            rows: Vec::new(),
            cursor: 0,
            first: 0,
            file: file.map(PathBuf::from),
            trouble: None,
        };
        panel.rows = panel.build(config, totals);
        // The first row anything can be done to, so the panel opens with the
        // cursor on a setting rather than on the heading above it.
        panel.cursor = panel.next_landing(0, true).unwrap_or(0);
        panel
    }

    /// Rebuild the rows from the file as it now reads, keeping the cursor where
    /// it was. Called after a change has been written and the file read back.
    pub fn refresh(&mut self, config: &Config, totals: &Totals) {
        self.rows = self.build(config, totals);
        self.trouble = None;
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        if !self.rows.get(self.cursor).is_some_and(landable) {
            self.cursor = self.next_landing(self.cursor, false).unwrap_or(0);
        }
    }

    fn build(&self, config: &Config, totals: &Totals) -> Vec<Row> {
        let reading = |label: &'static str, value: String| Row::Reading { label, value };
        let mut rows = vec![Row::Heading("ALL TIME")];
        // The one place these numbers are shown. They had a pane of their own,
        // which read as this session's spend because nothing on it said
        // otherwise; under a heading that says ALL TIME they mean what they are.
        rows.push(reading("prefilled", grouped(totals.prefilled)));
        rows.push(reading("generated", grouped(totals.generated)));
        rows.push(reading("from cache", grouped(totals.cached)));
        rows.push(reading(
            "prefill",
            rates(totals.average_prefill(), totals.median_prefill()),
        ));
        rows.push(reading(
            "decode",
            rates(totals.average_decode(), totals.median_decode()),
        ));

        for (heading, settings) in GROUPS {
            rows.push(Row::Heading(heading));
            for (key, kind) in settings {
                rows.push(Row::Setting {
                    key,
                    value: value_of(config, key, *kind),
                    kind: *kind,
                });
            }
        }

        rows.push(Row::Heading("COLOURS"));
        for (key, rgb) in colours(config) {
            rows.push(Row::Setting {
                key,
                value: hex(rgb),
                kind: Kind::Colour(rgb),
            });
        }

        rows.push(Row::Heading("WHERE THIS LIVES"));
        rows.push(reading(
            "settings",
            match &self.file {
                Some(path) => path.display().to_string(),
                // Not a failure worth refusing to open the panel over: every
                // reading above is still true and the presets still apply for
                // as long as the window is up. It is why nothing can be saved.
                None => String::from("nowhere: no home directory to write one in"),
            },
        ));
        rows
    }

    /// Every row at once, which only the tests want: the panel is a takeover, so
    /// the layout sizes itself to the surface rather than to the list, and the
    /// drawing asks for the rows the scroll window actually reaches through
    /// [`Settings::row`]. A whole-list accessor in the window would be a second
    /// way to draw rows nobody clamped.
    #[cfg(test)]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn row(&self, index: usize) -> Option<&Row> {
        self.rows.get(index)
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn first(&self) -> usize {
        self.first
    }

    pub fn trouble(&self) -> Option<&str> {
        self.trouble.as_deref()
    }

    /// Say why a change did not land. Shown on the panel rather than dropped: a
    /// row that stays where it was with nothing said reads as a dead panel.
    pub fn say_trouble(&mut self, why: String) {
        self.trouble = Some(why);
    }

    /// What the keys under the cursor do, spelled out for the footer. The panel
    /// is the one surface in this window where there is nothing to experiment on
    /// safely, so it says what a key will do before it is pressed.
    pub fn hint(&self) -> &'static str {
        match self.rows.get(self.cursor) {
            Some(Row::Setting { kind, .. }) => match kind {
                Kind::Flag => "enter or left and right turn it on and off",
                Kind::Choice(_) => "left and right walk the presets",
                Kind::Number { .. } => "left and right nudge it",
                Kind::Colour(_) => "colours are edited in the file",
            },
            _ => "up and down move \u{2022} esc closes",
        }
    }

    /// What the row under the cursor becomes when it is nudged, or nothing when
    /// the cursor is on a row that cannot change.
    ///
    /// Takes `&self`: the row is not touched here. What the panel shows comes
    /// back from the file, so a write that fails leaves the row reading what the
    /// file still says instead of the value it was asked for.
    pub fn change(&self, forward: bool) -> Option<Change> {
        let Row::Setting { key, value, kind } = self.rows.get(self.cursor)? else {
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

    /// Move the cursor one row, over anything it cannot land on. Clamped at both
    /// ends: a list that wraps under an arrow key held down is a cursor that
    /// arrives somewhere nobody was looking.
    pub fn step(&mut self, down: bool) -> bool {
        let Some(next) = self.next_landing(self.cursor, down) else {
            return false;
        };
        let moved = next != self.cursor;
        self.cursor = next;
        moved
    }

    /// A screenful, then the nearest row that can hold the cursor.
    pub fn page(&mut self, rows: usize, down: bool) -> bool {
        let by = rows.max(1);
        let reach = match down {
            true => (self.cursor + by).min(self.rows.len().saturating_sub(1)),
            false => self.cursor.saturating_sub(by),
        };
        // From the row a page away, look on in the direction of travel first and
        // then back the other way, so a page that lands in the colours does not
        // stop dead and does not jump back past where it came from.
        let next = self
            .landing_from(reach, down)
            .or_else(|| self.landing_from(reach, !down))
            .unwrap_or(self.cursor);
        let moved = next != self.cursor;
        self.cursor = next;
        moved
    }

    /// The first or last row anything can be done to.
    pub fn jump(&mut self, last: bool) -> bool {
        let edge = match last {
            true => self.rows.len().saturating_sub(1),
            false => 0,
        };
        let Some(next) = self.landing_from(edge, !last) else {
            return false;
        };
        let moved = next != self.cursor;
        self.cursor = next;
        moved
    }

    /// Put the cursor on the row under the pointer, when that row can hold it.
    pub fn point_at(&mut self, index: usize) -> bool {
        if !self.rows.get(index).is_some_and(landable) {
            return false;
        }
        let moved = index != self.cursor;
        self.cursor = index;
        moved
    }

    /// The next row in that direction the cursor can land on, not counting the
    /// one it is on.
    fn next_landing(&self, from: usize, down: bool) -> Option<usize> {
        match down {
            true => self.landing_from(from + 1, true),
            false => self.landing_from(from.checked_sub(1)?, false),
        }
        .or_else(|| self.landing_from(from, down))
    }

    /// The first row at or beyond `from`, walking in one direction, that can
    /// hold the cursor.
    fn landing_from(&self, from: usize, down: bool) -> Option<usize> {
        let mut range: Box<dyn Iterator<Item = usize>> = match down {
            true => Box::new(from..self.rows.len()),
            false => Box::new((0..=from.min(self.rows.len().saturating_sub(1))).rev()),
        };
        range.find(|at| self.rows.get(*at).is_some_and(landable))
    }

    /// One row per entry, for the scroll window. Every row is one row: a value
    /// too long for the panel is clipped rather than wrapped, so a click cannot
    /// resolve to a setting other than the one under the pointer.
    pub fn heights(&self) -> Vec<usize> {
        text_geometry::heights(self.rows.iter().map(|_| 0), 1)
    }

    /// Bring the cursor on screen, for a `rows` tall list.
    pub fn reveal(&mut self, rows: usize) -> bool {
        if rows == 0 || self.rows.is_empty() {
            return false;
        }
        let most = text_geometry::max_scrollback(&self.heights(), rows);
        let mut next = self.first.min(self.cursor);
        if self.cursor + 1 > next + rows {
            next = self.cursor + 1 - rows;
        }
        let next = next.min(most);
        let moved = next != self.first;
        self.first = next;
        moved
    }

    /// Move the window without moving the cursor, for the wheel.
    pub fn scroll(&mut self, by: usize, down: bool, rows: usize) -> bool {
        let most = text_geometry::max_scrollback(&self.heights(), rows);
        let next = match down {
            true => (self.first + by).min(most),
            false => self.first.saturating_sub(by),
        };
        let moved = next != self.first;
        self.first = next;
        moved
    }

    /// How much of the list is on screen, for the scrollbar.
    pub fn thumb(&self, rows: usize) -> Option<(f32, f32)> {
        let heights = self.heights();
        let back = text_geometry::scrollback_for(&heights, rows, self.first);
        text_geometry::thumb(&heights, rows, back)
    }
}

fn landable(row: &Row) -> bool {
    matches!(row, Row::Setting { kind, .. } if kind.changes())
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
                "top_height" => config.top_height,
                // Unreachable through GROUPS, and a number is the honest answer
                // for a row that says it is one.
                _ => 0.0,
            };
            format!("{value:.places$}")
        }
        (_, Kind::Colour(rgb)) => hex(rgb),
        // Same: every key in GROUPS is answered above.
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

/// Write one change to the settings file and read the whole file back.
///
/// The only part of this module that touches a disk. The value goes in through
/// the settings writer, which keeps every comment and refuses a key the parser
/// does not read, and the Config that comes back is parsed from the file rather
/// than patched in memory, so what the panel shows next is what the next launch
/// will read.
pub fn commit(path: &Path, change: &Change) -> Result<Config, String> {
    config::write_setting(path, change.key, Some(&change.value))?;
    Ok(Config::load_from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn over(config: &Config) -> Settings {
        Settings::open(config, &Totals::default(), Some(Path::new("/tmp/no0b.conf")))
    }

    /// A scratch settings file of its own per test, since the writer works on a
    /// real path and two tests sharing one would fight over it.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("no0b-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir.join(format!("{name}.conf"))
    }

    fn setting<'a>(panel: &'a Settings, key: &str) -> &'a Row {
        panel
            .rows()
            .iter()
            .find(|row| matches!(row, Row::Setting { key: k, .. } if *k == key))
            .unwrap_or_else(|| panic!("{key} is not on the panel"))
    }

    fn value(panel: &Settings, key: &str) -> String {
        match setting(panel, key) {
            Row::Setting { value, .. } => value.clone(),
            other => panic!("{other:?}"),
        }
    }

    fn put_cursor(panel: &mut Settings, key: &str) {
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Setting { key: k, .. } if *k == key))
            .unwrap_or_else(|| panic!("{key} is not on the panel"));
        assert!(
            panel.point_at(at) || panel.cursor() == at,
            "{key} cannot hold the cursor"
        );
    }

    /// Every key the file understands is on the panel exactly once, as a row
    /// that changes or as a swatch.
    ///
    /// This is the test that stops the panel going stale. A setting added to
    /// `config::keys` and not to [`GROUPS`] is a line in everybody's file with
    /// no way to reach it from the window, and nothing else would say so.
    #[test]
    fn every_key_in_the_file_is_on_the_panel() {
        let config = Config::default();
        let panel = over(&config);
        let mut on_panel: Vec<&str> = panel
            .rows()
            .iter()
            .filter_map(|row| match row {
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
    }

    /// The cursor only stops where something can happen: not on a heading, not
    /// on a reading, and not on a colour.
    #[test]
    fn the_cursor_skips_what_it_cannot_change() {
        let config = Config::default();
        let mut panel = over(&config);
        assert!(
            matches!(panel.row(panel.cursor()), Some(Row::Setting { key, .. }) if *key == "theme"),
            "the panel opens on {:?}",
            panel.row(panel.cursor())
        );

        // Every row it stops on, walking the whole panel down and back up.
        let mut down = vec![panel.cursor()];
        while panel.step(true) {
            down.push(panel.cursor());
        }
        for at in &down {
            assert!(
                landable(panel.row(*at).expect("a row")),
                "{:?} cannot hold the cursor",
                panel.row(*at)
            );
        }
        let mut up = vec![panel.cursor()];
        while panel.step(false) {
            up.push(panel.cursor());
        }
        up.reverse();
        assert_eq!(down, up, "walking back up visits other rows");
        // Nine changeable settings, and the walk ends on the last of them
        // rather than in the colours below it. Seven before the two dividers
        // were given rows of their own.
        assert_eq!(down.len(), 9, "{down:?}");
        assert!(!panel.step(false), "the top of the list is a stop");
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
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Setting { key, .. } if *key == "accent"))
            .expect("the accent row");
        assert!(!panel.point_at(at));
        assert_ne!(panel.cursor(), at);
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
        let panel = Settings::open(&Config::default(), &totals, None);
        let readings: Vec<(&str, String)> = panel
            .rows()
            .iter()
            .filter_map(|row| match row {
                Row::Reading { label, value } => Some((*label, value.clone())),
                _ => None,
            })
            .collect();
        assert!(readings.contains(&("prefilled", String::from("12 345 678"))), "{readings:?}");
        assert!(readings.contains(&("generated", String::from("9 012"))), "{readings:?}");
        // 300 tokens in 10 seconds is 30 a second on average; the middle of the
        // three samples is 40.
        assert!(
            readings.contains(&("decode", String::from("30 mean, 40 median tok/s"))),
            "{readings:?}"
        );
        // No home directory: the panel still opens and says why nothing can be
        // saved rather than pretending there is a file.
        let where_ = readings
            .iter()
            .find(|(label, _)| *label == "settings")
            .expect("the file row");
        assert!(where_.1.contains("no home directory"), "{where_:?}");

        let empty = Settings::open(&Config::default(), &Totals::default(), None);
        assert!(
            empty.rows().contains(&Row::Reading {
                label: "prefill",
                value: String::from("nothing measured yet")
            }),
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
        let mut panel = Settings::open(&config, &Totals::default(), Some(&path));

        for key in [
            "opacity",
            "font_size",
            "pane_font_size",
            "max_input_rows",
            "left_width",
            "top_height",
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
        let mut panel = Settings::open(&config, &Totals::default(), Some(&path));
        put_cursor(&mut panel, "show_activity");
        let was = panel.cursor();

        let change = panel.change(true).expect("a flag");
        config = commit(&path, &change).expect("the file takes it");
        assert!(!config.show_activity);
        panel.refresh(&config, &Totals::default());
        assert_eq!(value(&panel, "show_activity"), "off");
        assert_eq!(panel.cursor(), was, "the cursor moved under the change");

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

    /// The footer says what the two keys will do to the row under the cursor,
    /// which differs by kind.
    #[test]
    fn the_footer_says_what_the_keys_do_here() {
        let config = Config::default();
        let mut panel = over(&config);
        put_cursor(&mut panel, "theme");
        assert!(panel.hint().contains("presets"), "{}", panel.hint());
        put_cursor(&mut panel, "opacity");
        assert!(panel.hint().contains("nudge"), "{}", panel.hint());
        put_cursor(&mut panel, "show_files");
        assert!(panel.hint().contains("on and off"), "{}", panel.hint());
    }

    /// The list scrolls like every other list in the window, and the cursor is
    /// brought on screen rather than left off the bottom of it.
    #[test]
    fn the_list_scrolls_and_the_cursor_is_kept_on_screen() {
        let config = Config::default();
        let mut panel = over(&config);
        let rows = 10;
        assert!(panel.rows().len() > rows, "the panel is one screenful");
        assert_eq!(panel.first(), 0);
        assert!(panel.thumb(rows).is_some(), "a list this long says so");

        // Down to the last setting: the window follows it.
        assert!(panel.jump(true));
        assert!(panel.reveal(rows), "the cursor is off the bottom");
        assert!(panel.first() > 0);
        assert!(panel.cursor() < panel.first() + rows, "the cursor is off screen");

        // The wheel moves the window and leaves the cursor where it was.
        let cursor = panel.cursor();
        assert!(panel.scroll(3, false, rows));
        assert_eq!(panel.cursor(), cursor);

        // Back to the first setting. Reveal only moves the window when the
        // cursor is off it, so what it owes here is a window the cursor is
        // inside, not a window scrolled back to the top.
        assert!(panel.jump(false));
        panel.reveal(rows);
        assert!(panel.first() <= panel.cursor(), "the cursor is above the window");
        assert!(panel.cursor() < panel.first() + rows);

        // A page down lands on a row that can hold the cursor, from the top and
        // from inside the colours.
        assert!(panel.page(rows, true));
        assert!(landable(panel.row(panel.cursor()).expect("a row")));
        panel.page(rows * 4, true);
        assert!(landable(panel.row(panel.cursor()).expect("a row")));
    }
}
