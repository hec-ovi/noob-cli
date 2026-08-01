//! The APPEARANCE section: everything about what the window looks like, as
//! cards, with the palette under its own theme card.
//!
//! One of the settings panel's nested section boxes. It owns the window-file
//! vocabulary: which keys are fields, their plain-words names, the keys that
//! are deliberately off the panel, what a restore takes out, and the naming of
//! the palette. The frame owns the cursor, the nudge and the writes; `commit`
//! and `restore` there read [`THEME`] and [`restoring`] from here.

use std::path::Path;

use crate::config::{self, Config};
use crate::settings::{Card, CardField, Doing, File, Kind, Palette, Row, Swatch};

/// The key that picks a preset, named here because two places want it: the
/// field is built out of [`LOOKS`] with the sizes and then stands at the top of
/// the palette instead, since it is what writes every colour under it.
pub(crate) const THEME: &str = "theme";

/// What the `theme` row says when the palette in the file is not one of the
/// presets. Not a value that can be written: the writer refuses it, and the row
/// only ever hands it back to itself.
const CUSTOM: &str = "custom";

/// What to call each of the window's own settings in plain words, and the
/// sentence under it.
///
/// The same rule the agent's fields follow: the key is in the sentence, not in
/// the label. `window_opacity` over a slider says nothing to somebody who has
/// not read the file, and two sliders both labelled "opacity" say nothing to
/// anybody at all.
const LOOK_FIELDS: [(&str, &str, &str); 6] = [
    (
        THEME,
        "theme",
        "theme. Picking one clears the colour lines in your file that were overriding it",
    ),
    (
        "opacity",
        "widget windows",
        "opacity. The widget windows: the panes, the bars and the menus, everything with words on it",
    ),
    (
        "window_opacity",
        "base application",
        "window_opacity. The base application: the empty space between the panes, where your desktop shows through",
    ),
    (
        "font_size",
        "the conversation",
        "font_size. The messages, and everything else in the transcript",
    ),
    (
        "pane_font_size",
        "the panes",
        "pane_font_size. The activity, plan, agents and file panes, and this panel",
    ),
    (
        "prompt_rows",
        "the box you type in",
        "prompt_rows. How many lines the input prompt has, full or empty",
    ),
];

/// One of the window's own settings, as a field of a card: its plain-words
/// name, what the file says it is now, and the sentence that says what it
/// decides.
///
/// Built off [`LOOKS`] rather than spelled out here, so a field and the bounds
/// it is nudged inside cannot come apart. A key on neither list is not a field
/// anybody can reach, which `every_key_in_the_file_is_on_the_panel` fails on.
fn look_field(config: &Config, key: &str) -> CardField {
    let (key, kind) = LOOKS
        .iter()
        .find(|(known, _)| *known == key)
        .unwrap_or_else(|| panic!("{key} is not one of the window's settings"));
    let (_, label, hint) = LOOK_FIELDS
        .iter()
        .find(|(known, ..)| known == key)
        .unwrap_or_else(|| panic!("{key} has nothing to call it"));
    CardField::setting(label, key, value_of(config, key, *kind), *kind, File::Window).saying(hint)
}

/// Every key a restore takes out of the window's own file: the settings this
/// section carries, and the whole palette under them.
///
/// The dividers and the pane flags are not on it. They are set by using the
/// window rather than by this panel ([`OFF_PANEL`]), and putting the panes back
/// where they started is not what somebody pressing a button under the colours
/// is asking for.
pub fn restoring() -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = LOOKS
        .iter()
        .map(|(key, _)| *key)
        .filter(|key| !OFF_PANEL.contains(key))
        .collect();
    keys.extend(config::colour_keys());
    keys
}

/// The settings of the window's own file that two arrow keys can cover, grouped
/// the way their sections list them.
///
/// Every other key in that file is a colour and is listed from the live
/// [`Config`] instead, so a colour added to the palette appears here with no
/// edit at all. `every_key_in_the_file_is_on_the_panel` fails if a key ends up
/// in neither list, which is what stops a setting being added to the file and
/// forgotten here.
pub(crate) const LOOKS: [(&str, Kind); 6] = [
    (THEME, Kind::Choice(&config::THEMES)),
    (
        "opacity",
        Kind::Number {
            step: 0.05,
            low: 0.05,
            high: 1.0,
            places: 2,
            stops: &[],
        },
    ),
    // The window's own, which goes all the way to nothing: the empty space has
    // no text in it, so a window whose gaps are glass is still readable.
    (
        "window_opacity",
        Kind::Number {
            step: 0.05,
            low: 0.0,
            high: 1.0,
            places: 2,
            stops: &[],
        },
    ),
    (
        "font_size",
        Kind::Number {
            step: 1.0,
            low: 8.0,
            high: 40.0,
            places: 0,
            stops: &[],
        },
    ),
    (
        "pane_font_size",
        Kind::Number {
            step: 1.0,
            low: 8.0,
            high: 40.0,
            places: 0,
            stops: &[],
        },
    ),
    (
        "prompt_rows",
        Kind::Number {
            step: 1.0,
            low: 1.0,
            high: 24.0,
            places: 0,
            stops: &[],
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
/// `every_colour_says_what_it_colours` fails on one that is not, and on a label
/// that is the key said again: `gauge_7` reading "reading 7" is the key with a
/// word in front of it, which is what these ten were before they were the
/// readings that actually wear them.
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
        // Which readings wear which slot is `monitor`'s to say: its `HUE_*`
        // constants are these ten in this order, and a slot no reading wears is
        // spare rather than given a name it does not have.
        // `every_meter_says_which_readings_wear_it` fails on a label that has
        // come apart from the readings.
        "gauge_1" => "the gpu meter",
        "gauge_2" => "the vram meter",
        "gauge_3" => "gtt and generated",
        "gauge_4" => "the decode rate",
        "gauge_5" => "the context meter",
        "gauge_6" => "cpu and cached",
        "gauge_7" => "ram and prefilled",
        "gauge_8" => "the prefill rate",
        "gauge_9" => "spare meter",
        "gauge_10" => "the other spare",
        // A colour added to the file and not to this list. Said rather than
        // left blank, and `every_colour_says_what_it_colours` fails on it so it
        // is not a label anybody sees.
        _ => "a colour of its own",
    }
}

/// The one line under the theme row: whose colours these are, and what a colour
/// written in the file does to that.
///
/// It names the theme the row above it is showing, [`CUSTOM`] included, because
/// the complaint this answers was a grid of colours with nothing on it saying
/// where they came from. Short enough to be read on the row it is drawn on: a
/// note is clipped rather than wrapped, so a second sentence would be the half
/// nobody sees.
fn palette_line(config: &Config) -> String {
    match theme_name(config) {
        CUSTOM => {
            String::from("these colours are custom: a line in the file is overriding the theme")
        }
        name => format!("the {name} theme set these; a colour written in the file overrides its key"),
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

/// What the file says for one setting right now.
///
/// Spelled the way the writer would spell it, because this is also what
/// [`crate::settings::Settings::change`] reads to work out the next value:
/// `on`/`off` for a flag and no trailing zeros on a whole number.
fn value_of(config: &Config, key: &str, kind: Kind) -> String {
    match (key, kind) {
        ("theme", _) => theme_name(config).to_string(),
        ("prompt_rows", _) => config.prompt_rows.to_string(),
        (_, Kind::Number { places, .. }) => {
            let value = match key {
                "opacity" => config.opacity,
                "window_opacity" => config.window_opacity,
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

/// Everything about what the window looks like, as cards: how big the text
/// is, how solid the window is, the box you type in, then the palette.
///
/// This was the last flat section: bare rows at one text size, the key of
/// the file in the label column, the two transparencies that were really one
/// number, and the palette under four headings that grouped nothing. Each
/// group is a card now, every field is a plain-words name over its value
/// with the key in the sentence under it, and the two groups PANES held are
/// still nowhere (see [`OFF_PANEL`] for which keys those were and what sets
/// them instead).
pub fn rows(config: &Config, file: Option<&Path>) -> Vec<Row> {
    let mut rows = vec![
        Row::Card(Card {
            title: String::from("HOW BIG THE TEXT IS"),
            fields: vec![
                look_field(config, "font_size"),
                look_field(config, "pane_font_size"),
            ],
            hint: None,
            does: None,
        }),
        // The two transparencies together, because the whole of what makes
        // either of them readable is which surface it moves: one setting
        // called "opacity" with a backdrop pinned to 55% of it is what made
        // "the empty spaces of noob" unreachable. The base application first,
        // the widget windows second, which is the order he asked to read them
        // in.
        Row::Card(Card {
            title: String::from("TRANSPARENCY"),
            fields: vec![
                look_field(config, "window_opacity"),
                look_field(config, "opacity"),
            ],
            hint: None,
            does: None,
        }),
        Row::Card(Card {
            title: String::from("INPUT PROMPT"),
            fields: vec![look_field(config, "prompt_rows")],
            hint: None,
            does: None,
        }),
    ];
    rows.extend(colour_rows(config, file));
    rows
}

/// The palette: the theme that set it, then one card per group of colours,
/// then where the file is and the way back to the defaults.
///
/// It was one colour per row, thirty seven rows of hex string. That is four
/// screens of a column half of which is empty, and no row said what it
/// coloured. Grouped blocks of labelled blocks read as a palette, which is
/// what this is.
///
/// The theme stands over the colours it writes, as the first card, because
/// with it several rows up among the sizes the grid opened as swatches
/// nobody could act on: nothing on it said the colours were a preset's or
/// what would change them. Each group's card is headed with what that group
/// paints rather than with the keys it holds, for the same reason the
/// swatches are labelled in words.
fn colour_rows(config: &Config, file: Option<&Path>) -> Vec<Row> {
    let all = colours(config);
    // The control that writes every colour under it, first, out of the same
    // table the sizes are built from so the field is the field: same
    // choices, same nudge, same writer.
    let mut rows = vec![Row::Card(Card {
        title: String::from("THE PALETTE"),
        fields: vec![look_field(config, THEME)],
        hint: Some(palette_line(config)),
        does: None,
    })];
    let mut at = 0;
    for (title, count) in [
        ("THE WINDOW'S OWN TONES", WINDOW_TONES),
        ("THE CODE COLOURS", SYNTAX_TONES),
        ("THE TOOL MARKS", config::TOOL_KEYS.len()),
        ("THE METERS", config::GAUGE_KEYS.len()),
    ] {
        // One card per group rather than one row per three colours, so a
        // row never carries the end of one group and the start of the next:
        // a grid whose blocks run into each other is the list again.
        rows.push(Row::Palette(Palette {
            title,
            cells: all[at..at + count]
                .iter()
                .map(|(key, rgb)| Swatch {
                    key,
                    about: about(key),
                    rgb: *rgb,
                })
                .collect(),
        }));
        at += count;
    }
    rows.push(Row::Card(Card {
        title: String::from("WHERE ALL THIS IS WRITTEN"),
        fields: vec![
            CardField::reading(
                "the settings file",
                match file {
                    Some(path) => path.display().to_string(),
                    // Not a failure worth refusing to open the panel over:
                    // every reading is still true and the presets still
                    // apply for as long as the window is up. It is why
                    // nothing can be saved.
                    None => String::from("nowhere: no home directory to write one in"),
                },
            )
            .saying("a colour typed on a swatch lands here, under the key the footer names"),
        ],
        hint: Some(String::from(
            "press a colour above and type a hex value (#rrggbb) to change it",
        )),
        does: None,
    }));
    // The way out, on a card of its own and at the end, because it takes
    // back everything above it rather than anything on one card.
    rows.push(Row::Card(Card {
        title: String::from("BACK TO THE DEFAULTS"),
        fields: Vec::new(),
        hint: Some(String::from(
            "comments out the sizes, the transparency and the palette in that file, and leaves every other line of it alone",
        )),
        does: Some(Doing::Restore),
    }));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{self, Agent};
    use crate::settings::testing::*;
    use crate::settings::{
        commit, restore, Change, Deed, Settings, Side, AGENT, APPEARANCE, SECTIONS,
    };

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
                // A colour is a cell of a palette card now, not a row of its
                // own.
                Row::Palette(palette) => palette.cells.iter().map(|cell| cell.key).collect(),
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
                Row::Palette(palette) => {
                    let keys: Vec<&str> = palette.cells.iter().map(|cell| cell.key).collect();
                    assert_eq!(section, APPEARANCE, "{keys:?} are in the wrong section");
                    assert!(!palette.title.is_empty(), "{keys:?} are under no title");
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
            let Row::Palette(Palette { cells, .. }) = row else {
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
                assert_ne!(
                    cell.about,
                    cell.key.replace('_', " "),
                    "{} is its own key with the underscore taken out",
                    cell.key
                );
                // And not the key's number with a word in front of it either:
                // `gauge_7` labelled "reading 7" is `gauge_7` again, which is
                // what he could not read anything out of.
                let digits: String = cell
                    .key
                    .chars()
                    .skip_while(|letter| !letter.is_ascii_digit())
                    .collect();
                assert!(
                    digits.is_empty() || !cell.about.ends_with(&digits),
                    "{} is labelled {:?}, which is its own number",
                    cell.key,
                    cell.about
                );
                said += 1;
            }
        }
        assert_eq!(said, colours(&Config::default()).len());
    }

    /// The theme stands over the colours it writes, on a card of its own, and
    /// that card says whose colours these are.
    ///
    /// "colors as theme groups i did not saw on the setup... i just sawe many
    /// colors... so i dont know". The grid was a wall of swatches: `theme` sat
    /// several rows above it with the sizes, and nothing anywhere on the block
    /// said the colours were a preset's or what would change them. It is the
    /// first field of the first card of the palette now, with the sentence
    /// naming the theme those colours came from under it, and the groups
    /// underneath.
    ///
    /// Written when the block was a heading, a bare row and a note. There is no
    /// heading and no note in this section any more: every group is a card, so
    /// the assertions are about which card carries what.
    #[test]
    fn the_theme_field_is_the_top_of_the_palette() {
        for name in config::THEMES {
            let config = Config::parse(&format!("theme = {name}"));
            let mut panel = over(&config);
            go_to(&mut panel, APPEARANCE);
            let rows = panel.rows().to_vec();
            let at = rows
                .iter()
                .position(|row| {
                    matches!(row, Row::Card(card)
                        if card.fields.iter().any(|field| {
                            matches!(field.holds.as_ref(), Row::Setting { key, .. } if *key == THEME)
                        }))
                })
                .expect("the theme card");
            let first = rows
                .iter()
                .position(|row| matches!(row, Row::Palette(_)))
                .expect("the grid");
            assert_eq!(at + 1, first, "{:?} came between them", rows[at + 1]);
            let Row::Card(card) = &rows[at] else {
                panic!("{:?}", rows[at]);
            };
            assert_eq!(card.title, "THE PALETTE");
            // The theme is the card's first field, so it is the one the keys
            // land on and the one a press can name.
            assert!(
                matches!(card.fields[0].holds.as_ref(), Row::Setting { key, .. } if *key == THEME)
            );
            let hint = card.hint.clone().expect("the line under the field");
            assert!(
                hint.contains(name),
                "the line does not name the theme that is set: {hint}"
            );
            assert!(
                hint.contains("a colour written in the file overrides its key"),
                "the line does not say what changing one colour does: {hint}"
            );
            // And the field's own sentence says what picking one will do to the
            // file, which is the whole of why picking one works now.
            let said = card.fields[0].hint.clone().expect("the field's sentence");
            assert!(said.contains("clears the colour lines"), "{said}");
            assert_eq!(value(&panel, THEME), name);
            // The sizes are still their own cards and still above the palette:
            // the theme did not take them with it.
            assert!(
                rows[..at]
                    .iter()
                    .any(|row| matches!(row, Row::Card(card)
                        if card.fields.iter().any(|field| {
                            matches!(field.holds.as_ref(), Row::Setting { key, .. } if *key == "opacity")
                        }))),
                "the sizes are not above the palette any more"
            );
        }

        // A file carrying a preset and then one colour of its own is not that
        // preset, and the line says so rather than naming a theme the window is
        // not wearing.
        let mut panel = over(&Config::parse("theme = noob-cool\naccent = #123456"));
        go_to(&mut panel, APPEARANCE);
        assert_eq!(value(&panel, THEME), CUSTOM);
        let text = said(&panel);
        assert!(text.contains("these colours are custom"), "{text}");
        assert!(!text.contains("the noob-cool theme set these"), "{text}");
    }

    /// Every group of the palette is a card of its own, titled with what it
    /// paints rather than with the keys it holds.
    ///
    /// `ONE PER TOOL` and `ONE PER GAUGE` were the file's own words for those
    /// two lists: they say how many colours there are and nothing about what
    /// they colour. The groups were bare headings over rows of three swatches
    /// until this round, which is the shape the whole panel is being taken out
    /// of: a heading is not a group, a card is.
    #[test]
    fn every_group_of_the_palette_says_what_it_paints() {
        let mut panel = over(&Config::default());
        go_to(&mut panel, APPEARANCE);
        let titles: Vec<&str> = panel
            .rows()
            .iter()
            .filter_map(|row| match row {
                Row::Palette(palette) => Some(palette.title),
                _ => None,
            })
            .collect();
        assert_eq!(
            titles,
            vec![
                "THE WINDOW'S OWN TONES",
                "THE CODE COLOURS",
                "THE TOOL MARKS",
                "THE METERS",
            ]
        );
        // And every colour in the file is in exactly one of them, in file
        // order: a group that dropped one is a colour nobody can find.
        let cells: Vec<&str> = panel
            .rows()
            .iter()
            .flat_map(|row| match row {
                Row::Palette(palette) => {
                    palette.cells.iter().map(|cell| cell.key).collect::<Vec<_>>()
                }
                _ => Vec::new(),
            })
            .collect();
        let all: Vec<&str> = colours(&Config::default())
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(cells, all);
    }

    /// A meter's colour is labelled with the readings that actually wear that
    /// slot, out of the monitor rather than out of a guess.
    ///
    /// They were "reading 1" to "reading 10", which is `gauge_1` to `gauge_10`
    /// with a word in front of it. Which slot a reading wears is `monitor`'s to
    /// say, so this walks what the monitor really produced and checks the label
    /// of the slot each one asked for names it.
    #[test]
    fn every_meter_says_which_readings_wear_it() {
        let mut monitor = crate::monitor::Monitor::new();
        let state = crate::state::State::default();
        // Twice: the processor reading is a difference between two samples and
        // there is no reading for it after the first one.
        monitor.sample(&state);
        monitor.sample(&state);
        let readings: Vec<crate::monitor::Gauge> = monitor
            .hardware()
            .into_iter()
            .chain(monitor.context())
            .chain(monitor.session())
            .collect();
        assert!(
            readings.len() >= 6,
            "the monitor read almost nothing to check against: {}",
            readings.len()
        );
        for gauge in &readings {
            let key = config::GAUGE_KEYS[gauge.hue];
            let label = about(key);
            assert!(
                gauge
                    .label
                    .split_whitespace()
                    .any(|word| label.contains(&word.to_lowercase())),
                "{} wears {key}, which is labelled {label:?}",
                gauge.label
            );
        }
        // The last two slots are worn by nothing and say so: a name for a meter
        // that is not on screen is worse than admitting the slot is spare.
        for spare in ["gauge_9", "gauge_10"] {
            let at = config::GAUGE_KEYS
                .iter()
                .position(|key| *key == spare)
                .expect("a slot of the palette");
            assert!(
                !readings.iter().any(|gauge| gauge.hue == at),
                "{spare} is worn by a reading now and is not spare"
            );
            assert!(about(spare).contains("spare"), "{spare} is {:?}", about(spare));
        }
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
        assert_eq!(forward.change(true).expect("a choice").value, "noob-cool");
        assert_eq!(
            forward.change(false).expect("a choice").value,
            "noob-red",
            "back from the first preset is the last one"
        );

        // A name an older build wrote is a palette the panel can name, since it
        // loads as one of the three rather than as a palette of its own.
        assert_eq!(theme_name(&Config::parse("theme = plum")), "noob-cool");

        // One explicit colour over a preset is not that preset any more, and
        // saying so is the point: the row is read off the colours in hand.
        let tuned = Config::parse("theme = noob-cool\naccent = #ff0000");
        assert_eq!(theme_name(&tuned), CUSTOM);
        let mut panel = over(&tuned);
        put_cursor(&mut panel, "theme");
        assert_eq!(value(&panel, "theme"), CUSTOM);
        assert_eq!(panel.change(true).expect("a choice").value, "noob-matrix");
        assert_eq!(panel.change(false).expect("a choice").value, "noob-red");
    }

    /// A swatch carries its colour, and a press on it says which key writes it
    /// and opens the hex line on the value in hand.
    ///
    /// This was `a_colour_row_carries_its_swatch_and_cannot_be_changed`, which
    /// asserted the same thing about a `Row::Setting` of its own. A colour is a
    /// cell of a grid row now: the grid still cannot hold the cursor, and the
    /// pointer press is how a colour is read and edited.
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
        // A whole group in one card, which is what makes it a grid.
        let count = match panel.row(at) {
            Some(Row::Palette(palette)) => palette.cells.len(),
            other => panic!("the accent is not on a palette card: {other:?}"),
        };
        assert!(count > 1, "one swatch to a card is the list again");

        // The cursor cannot get there: the pointer is how a swatch is reached.
        assert!(!panel.point_at(at, Side::Left));
        assert_ne!(panel.cursor(), at);

        // Pressing one says which line of the file writes it, and opens the
        // hex line on what that line says now.
        assert!(panel.pick(at, cell));
        assert_eq!(panel.picked(), Some((at, cell)));
        assert_eq!(panel.editing(), Some("#123456"));
        let says = panel.says();
        assert!(says.contains("accent"), "{says}");
        assert!(says.contains("#123456"), "{says}");
        assert!(says.contains("the accent"), "{says}");
        // And it goes away the moment the keyboard is used again, the typed
        // line with it.
        panel.step(true);
        assert_eq!(panel.picked(), None);
        assert_eq!(panel.editing(), None);
        assert_eq!(panel.says(), panel.hint());
        // A cell nobody drew is not a press.
        assert!(!panel.pick(at, count));
        assert!(!panel.pick(0, 0), "the first row of the section is not a grid");

        // And the section says what a press does, with the file it writes
        // beside it.
        let text = said(&panel);
        assert!(text.contains("type a hex value"), "{text}");
        assert!(text.contains("a colour typed on a swatch lands here"), "{text}");
        assert!(text.contains("no0b.conf"), "{text}");
    }

    /// "where is the setup i asked for individual colors and palettes?" A
    /// pressed swatch takes a typed hex value: a good one lands in the file
    /// under the swatch's own key and overrides the theme, a bad one is
    /// refused on the footer with nothing written, and Escape keeps what the
    /// file says.
    #[test]
    fn a_pressed_swatch_takes_a_hex_value_and_writes_it_under_its_key() {
        let path = scratch("swatch-edit");
        let _ = std::fs::remove_file(&path);
        let config = Config::load_from(&path);
        let mut panel = Settings::open(&config, Some(&path), Agent::default());
        go_to(&mut panel, APPEARANCE);
        let (at, cell) = swatch_at(&panel, "accent");

        // The press opens the hex line on the value in hand; retyping it is
        // the same buffer every field edits with.
        assert!(panel.pick(at, cell));
        assert_eq!(panel.editing(), Some("#7cd894"));
        while panel.backspace() {}
        assert!(panel.type_text("#12ff34"));
        let says = panel.says();
        assert!(says.contains("accent = #12ff34"), "{says}");
        let change = panel.finish_swatch_edit().expect("a colour the file can read");
        assert_eq!(
            change,
            Change {
                key: "accent",
                value: String::from("#12ff34"),
                file: File::Window,
            }
        );
        let config = commit(&path, &change).expect("the file takes it");
        assert_eq!(config.accent, [0x12, 0xff, 0x34]);
        panel.refresh(&config);
        go_to(&mut panel, APPEARANCE);
        // The swatch shows the value it really has, and the palette is no
        // preset's any more: an explicit colour overrides its theme.
        let (at, cell) = swatch_at(&panel, "accent");
        assert_eq!(panel.swatch(at, cell).expect("the accent").rgb, [0x12, 0xff, 0x34]);
        assert_eq!(value(&panel, THEME), CUSTOM);
        assert!(panel.editing().is_none(), "the write left the line open");
        let text = std::fs::read_to_string(&path).expect("the file");
        assert!(text.contains("accent = #12ff34"), "{text}");

        // A value that is not a colour is refused on the footer and nothing
        // lands: the file still says what it said.
        assert!(panel.pick(at, cell));
        while panel.backspace() {}
        panel.type_text("#nothex");
        assert!(panel.finish_swatch_edit().is_none());
        assert!(
            panel.trouble().is_some_and(|why| why.contains("#nothex")),
            "{:?}",
            panel.trouble()
        );
        assert_eq!(Config::load_from(&path).accent, [0x12, 0xff, 0x34]);

        // Escape keeps what the file says, and the pressed swatch stays
        // readable on the footer.
        assert!(panel.pick(at, cell));
        panel.type_text("ff");
        assert!(panel.cancel_edit());
        assert_eq!(panel.picked(), Some((at, cell)));
        assert!(panel.says().contains("#12ff34"), "{}", panel.says());
        assert_eq!(Config::load_from(&path).accent, [0x12, 0xff, 0x34]);
        let _ = std::fs::remove_file(&path);
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
                Row::Palette(palette) => {
                    let mut said = vec![palette.title];
                    said.extend(palette.cells.iter().map(|cell| cell.key));
                    said
                }
                Row::Reading { label, .. } => vec![label.as_str()],
                Row::Note { text, .. } => vec![text.as_str()],
                Row::Table(table) => {
                    let mut said = table.names.clone();
                    said.extend(
                        table
                            .rows
                            .iter()
                            .flat_map(|row| row.cells.iter().map(String::as_str)),
                    );
                    said
                }
                Row::Entry(entry) => vec![entry.name.as_str()],
                Row::Paper(paper) => vec![paper.title.as_str(), paper.under.as_str()],
                Row::Card(card) => {
                    let mut said = vec![card.title.as_str()];
                    said.extend(card.fields.iter().map(|field| field.label.as_str()));
                    said.extend(card.hint.as_deref());
                    said
                }
            };
            for said in named {
                for key in OFF_PANEL {
                    assert!(!said.contains(key), "{section} still says {key}: {said:?}");
                }
            }
            // The headings those rows sat under went with them, and so did
            // headings altogether: every group is a card with a title now, and
            // no card is titled for either of them.
            if let Row::Card(card) = row {
                assert!(
                    !card.title.contains("PANE") && !card.title.contains("DIVIDER"),
                    "{} is still a group",
                    card.title
                );
            }
        }

        // The panel is unhurt: every section on the rail with rows, and every
        // one of them still lands the cursor somewhere or says why it cannot.
        let mut panel = panel;
        assert_eq!(panel.section_names(), SECTIONS.to_vec());
        for name in SECTIONS {
            go_to(&mut panel, name);
            assert!(!panel.rows().is_empty(), "{name} has no rows at all");
        }
        // The list is the rule, not a note about one: none of those keys is a
        // field of any card, and a restore never writes one either.
        for key in OFF_PANEL {
            assert!(!restoring().contains(&key), "{key} is put back by a restore");
        }

        // And APPEARANCE did not empty out with them: the sizes, the theme and
        // the whole palette are what it was carrying before PANES ever got
        // pushed into it.
        go_to(&mut panel, APPEARANCE);
        let settings = panel
            .rows()
            .iter()
            .flat_map(|row| match row {
                Row::Card(card) => card.fields.iter().map(|field| field.holds.as_ref()).collect(),
                _ => Vec::new(),
            })
            .filter(|held| matches!(held, Row::Setting { file: File::Window, .. }))
            .count();
        assert_eq!(settings, LOOKS.len());
        assert!(
            panel.rows().iter().any(|row| matches!(row, Row::Palette(_))),
            "the palette went with them"
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
        let mut panel = Settings::open(&config, Some(&path), Agent::default());

        for key in [
            "opacity",
            "window_opacity",
            "font_size",
            "pane_font_size",
            "prompt_rows",
        ] {
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

    /// Item H5: picking a theme puts the window on that theme, on the file that
    /// made him say there were only two of them.
    ///
    /// "themes are only 2 custom and noob", "i only see noob matrix the others i
    /// asked are absent". The control was never broken: his file is the one
    /// adopted from CLIppy and it carries eight live colour lines, which land on
    /// top of whatever preset the theme line names. So a pick wrote
    /// `theme = noob-cool`, the eight lines put the green back on reload, the
    /// palette then matched no preset and the field read `custom`. Picking one
    /// takes those lines out of the way now, so the window really wears it.
    #[test]
    fn picking_a_theme_puts_the_window_on_it_over_a_file_that_was_overriding_it() {
        let path = scratch("theme-applies");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            "\
# CLIppy settings
accent = #7cd894
text   = #9ad6ac
dim    = #58966e
bright = #cefadb
good   = #74d194
bad    = #e87a6c
panel  = #000000
bar    = #0e2e1e
theme = noob-matrix
font_size = 15   # mine
",
        )
        .expect("his file");
        let mut config = Config::load_from(&path);
        let mut panel = Settings::open(&config, Some(&path), Agent::default());
        put_cursor(&mut panel, THEME);

        // Every one of the three, pressed by name the way the panel draws them.
        let (at, side) = (panel.cursor(), panel.side());
        for (option, name) in config::THEMES.iter().enumerate() {
            let change = panel.choose_option(at, side, option).expect("an option");
            assert_eq!(change.value, *name);
            config = commit(&path, &change).expect("the file takes it");
            panel.refresh(&config);
            // The window is that theme, not the name of it over the old
            // colours: every colour in hand is the preset's.
            assert_eq!(
                colours(&config),
                colours(&config::theme(name).expect("a preset")),
                "{name} did not reach the palette"
            );
            assert_eq!(value(&panel, THEME), *name, "the field still reads custom");
        }

        // And the file is still his: the size he set, the comment on it, the
        // header, and the eight colour lines kept as comments rather than
        // deleted.
        let text = std::fs::read_to_string(&path).expect("the file");
        assert!(text.starts_with("# CLIppy settings\n"), "{text}");
        assert!(text.contains("font_size = 15   # mine"), "{text}");
        assert!(text.contains("# accent = #7cd894"), "{text}");
        assert!(text.contains("# bar    = #0e2e1e"), "{text}");
        assert_eq!(text.matches("theme = ").count(), 1, "{text}");
        assert_eq!(config.font_size, 15.0);
        // An option nobody drew is not a press.
        assert!(panel.choose_option(at, side, config::THEMES.len()).is_none());
        assert!(panel.choose_option(0, side, 0).is_none(), "the sizes are not a choice");
        let _ = std::fs::remove_file(&path);
    }

    /// The window's own transparency is a setting of its own, beside the
    /// panels', and each one says which surface it moves.
    ///
    /// "another opacity should be present in settings which is the window itself
    /// the empty spaces of noob". There was one key, and the empty space was
    /// pinned at 55% of it, so the only way to see more desktop through the gaps
    /// was to make the text harder to read.
    #[test]
    fn the_window_has_a_transparency_of_its_own_beside_the_panels() {
        let path = scratch("window-opacity");
        let _ = std::fs::remove_file(&path);
        let config = Config::load_from(&path);
        let mut panel = Settings::open(&config, Some(&path), Agent::default());
        go_to(&mut panel, APPEARANCE);

        // Both on one card called TRANSPARENCY, base application first and the
        // widget windows second, each sentence naming its own key: two sliders
        // labelled "opacity" say nothing to anybody. "put TRANSPARENCY ...
        // switch so first base and second widgets".
        let card = panel
            .rows()
            .iter()
            .find_map(|row| match row {
                Row::Card(card)
                    if card.fields.iter().any(|field| {
                        matches!(field.holds.as_ref(), Row::Setting { key, .. } if *key == "window_opacity")
                    }) =>
                {
                    Some(card.clone())
                }
                _ => None,
            })
            .expect("the card the two of them stand on");
        assert_eq!(card.title, "TRANSPARENCY");
        assert_eq!(card.fields.len(), 2);
        assert_ne!(card.fields[0].label, card.fields[1].label);
        assert_eq!(card.fields[0].label, "base application");
        assert_eq!(card.fields[1].label, "widget windows");
        assert!(
            card.fields[0]
                .hint
                .as_deref()
                .is_some_and(|said| said.starts_with("window_opacity."))
        );
        assert!(card.fields[1].hint.as_deref().is_some_and(|said| said.starts_with("opacity.")));

        // The prompt card is INPUT PROMPT and its row says what the number is:
        // "how many lines the input prompt has".
        let prompt = panel
            .rows()
            .iter()
            .find_map(|row| match row {
                Row::Card(card)
                    if card.fields.iter().any(|field| {
                        matches!(field.holds.as_ref(), Row::Setting { key, .. } if *key == "prompt_rows")
                    }) =>
                {
                    Some(card.clone())
                }
                _ => None,
            })
            .expect("the input prompt card");
        assert_eq!(prompt.title, "INPUT PROMPT");
        assert!(
            prompt.fields[0]
                .hint
                .as_deref()
                .is_some_and(|said| said.contains("how many lines the input prompt has")
                    || said.contains("How many lines the input prompt has"))
        );

        // Nudging it writes that key and nothing else, and the surface the
        // window paints its empty space with follows it.
        put_cursor(&mut panel, "window_opacity");
        let change = panel.change(false).expect("a number");
        assert_eq!(change.key, "window_opacity");
        let next = commit(&path, &change).expect("the file takes it");
        assert!(next.window_opacity < config.window_opacity);
        assert_eq!(next.opacity, config.opacity, "the panels moved with it");
        assert_eq!(
            crate::skin::Skin::from(&next).backdrop[3],
            next.window_opacity
        );
        assert_ne!(
            crate::skin::Skin::from(&next).backdrop[3],
            crate::skin::Skin::from(&config).backdrop[3]
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Restoring puts every appearance key back to what the window ships with,
    /// in one press, and leaves the file a file.
    ///
    /// "put also in appearance restore to default". Two presses, because it
    /// throws away lines somebody may have written by hand, and the footer says
    /// what is about to go in between them.
    #[test]
    fn restoring_puts_every_appearance_key_back_and_leaves_the_file_readable() {
        let path = scratch("restore");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            "\
# mine
opacity = 20%
window_opacity = 100%
font_size = 30
pane_font_size = 9
prompt_rows = 6
theme = noob-red
accent = #123456
gauge_3 = #654321
left_width = 0.31
show_files = off
something_else = keep me
",
        )
        .expect("a file");
        let config = Config::load_from(&path);
        let mut panel = Settings::open(&config, Some(&path), Agent::default());
        go_to(&mut panel, APPEARANCE);
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Card(card) if card.does == Some(Doing::Restore)))
            .expect("the card that puts it back");

        // The keys reach it, fields or not, and the footer names the one key
        // that acts on it: a button only a pointer can press is half a control.
        while panel.cursor() != at && panel.step(true) {}
        assert_eq!(panel.cursor(), at, "the keys cannot reach the restore");
        assert!(panel.on_row());
        assert!(panel.hint().contains("delete puts every size"), "{}", panel.hint());

        // The first press arms it and writes nothing; the footer says what
        // would go.
        assert!(panel.uninstall(at).is_none(), "one press restored it");
        assert_eq!(panel.arming(), Some(at));
        let says = panel.says();
        assert!(says.contains("press restore again"), "{says}");
        assert!(says.contains("colour"), "{says}");
        assert_eq!(Config::load_from(&path), config, "the file changed anyway");

        // The second one does it.
        assert_eq!(panel.uninstall(at), Some(Deed::RestoreLooks));
        let after = restore(&path).expect("the file takes it");
        let fresh = Config::default();
        for key in restoring() {
            assert!(config::keys().contains(&key), "{key} is not a key at all");
        }
        assert_eq!(after.opacity, fresh.opacity);
        assert_eq!(after.window_opacity, fresh.window_opacity);
        assert_eq!(after.font_size, fresh.font_size);
        assert_eq!(after.pane_font_size, fresh.pane_font_size);
        assert_eq!(after.prompt_rows, fresh.prompt_rows);
        assert_eq!(colours(&after), colours(&fresh), "the palette stayed red");

        // What it did not take: the divider it was never asked about, the pane
        // flag, and a line this build has never heard of.
        assert_eq!(after.left_width, 0.31);
        assert!(!after.show_files);
        let text = std::fs::read_to_string(&path).expect("the file");
        assert!(text.contains("something_else = keep me"), "{text}");
        assert!(text.starts_with("# mine\n"), "{text}");
        // Commented out rather than deleted, so the lines are still there to
        // read and the file still parses clean.
        assert!(text.contains("# opacity = 20%"), "{text}");
        assert!(text.contains("# theme = noob-red"), "{text}");
        assert!(text.contains("# accent = #123456"), "{text}");
        assert!(after.unknown.is_empty() || after.unknown == ["something_else"], "{:?}", after.unknown);

        // And the panel reads the restored file as the defaults.
        panel.refresh(&after);
        go_to(&mut panel, APPEARANCE);
        assert_eq!(value(&panel, "opacity"), "0.90");
        assert_eq!(value(&panel, THEME), "noob-matrix");
        let _ = std::fs::remove_file(&path);
    }
}
