//! The settings panel painter: everything the panel draws, from the places
//! computed beside it.

use noob_draw::{Panel, Run, Scene, Text};

#[allow(unused_imports)]
use crate::design;
use crate::design::icons;
use crate::settings::{Row as SettingRow, Side};
#[allow(clippy::wildcard_imports)]
use crate::settings::places::*;
#[allow(clippy::wildcard_imports)]
use crate::view::*;


/// The settings panel: the whole surface under the title strip while it is up.
///
/// A rail of section names down the left and the chosen section beside it. Each
/// row is two columns: the label says what a setting is called in the file, so
/// the panel doubles as the documentation for editing that file by hand, and the
/// value sits down the right where it can be scanned. Only one thing here is a
/// widget in the usual sense, the slider on a setting with a range; everything
/// else is text, and what makes it a control is that the arrow keys and a click
/// on it change it.
pub(crate) fn settings_panel(scene: &mut Scene, frame: &Frame) {
    let Some(panel) = frame.settings else {
        return;
    };
    let (skin, layout) = (frame.skin, frame.layout);
    let box_ = layout.settings;
    if box_.w < 1.0 || box_.h < 1.0 {
        return;
    }
    scene.rect(panel_fill(box_, skin.panel));
    scene.rect(panel_edge(box_, skin.edge_focus));

    let size = frame.pane_size;
    let line = Text::line_for(size);
    let column = frame.pane_column.max(1.0);
    let content = box_.inset(PAD);
    let cols = cols_of(content, column);
    let say = |scene: &mut Scene, runs: Vec<Run>, at: Panel, tint: [u8; 4]| {
        scene.text(Text::rich(runs, at, size, tint));
    };

    // Two heads over two columns. The rail is headed by the panel's own name,
    // the gear and SETTINGS, aligned with the section names under it; the
    // section's title is written inside the body it titles, over the cards,
    // so MCP names the list beside the rail rather than riding the panel.
    //
    // The section's title rather than the rail's word for it, because on
    // SESSIONS the two are not always the same thing; `Settings::title` is
    // where that mapping lives.
    //
    // Both in the panel title role, which is the largest text in the window: a
    // heading the same size as the settings under it is not a heading.
    let list = layout.settings_list;
    let here = panel.title();
    let title_size = design::panel_title_size(size);
    let title_line = Text::line_for(title_size);
    let rail_head_w = (list.x - PAD - content.x).max(1.0);
    scene.text(Text::rich(
        vec![
            Run::icon(icons::SETTINGS.to_string(), skin.bright),
            Run::tinted(" SETTINGS", skin.bright),
        ],
        Panel::new(content.x + PAD + MARK_W + 3.0, content.y, rail_head_w, title_line),
        title_size,
        skin.bright,
    ));
    if list.w >= 1.0 {
        let title_cols = columns_in(list.w, design::column_for(column, size, design::PANEL_TITLE));
        scene.text(Text::rich(
            vec![Run::tinted(clip(here, title_cols), skin.heading)],
            Panel::new(list.x, content.y, list.w, title_line),
            title_size,
            skin.heading,
        ));
    }

    // The mark and nothing under it. It stood on a filled block while the
    // pointer was on it, which is a button, and the panel is a takeover with no
    // other button on it: the mark is on the panel's own surface the way the
    // heading beside it is. What answers the pointer is the mark itself, in the
    // bad colour, because closing is the one thing here that throws work away
    // and it is the same colour the window uses for that everywhere else.
    let close = layout.settings_close;
    if close.w >= 1.0 {
        let ink = match frame.hot == Some(Hit::SettingsClose) {
            true => skin.bad,
            false => skin.bright,
        };
        say(
            scene,
            vec![Run::icon(icons::CLOSE.to_string(), ink)],
            close,
            ink,
        );
    }

    // The rail. The chosen section carries the band and the mark every list in
    // this window marks its current row with. The mark is always the focus
    // colour now: the rail is pressed rather than walked, so there is no second
    // state for it to be in. Each name is drawn in its own box, which is where
    // the wrap put it: one column when they all fit down the rail, more columns
    // beside it when they do not (see [`settings_rail_cells`]).
    let names = panel.section_names();
    for (index, at) in &layout.settings_rail {
        let Some(name) = names.get(*index) else {
            continue;
        };
        let chosen = *index == panel.chosen();
        if chosen {
            scene.rect(at.fill(skin.strip));
            scene.rect(Panel::new(at.x, at.y, MARK_W, at.h).fill(skin.edge_focus));
        }
        let tint = match (chosen, frame.hot == Some(Hit::SettingsSection(*index))) {
            (true, _) => skin.heading,
            (false, true) => skin.bright,
            (false, false) => skin.dim,
        };
        let text_at = Panel::new(
            at.x + MARK_W + 3.0,
            at.y,
            (at.w - MARK_W - 3.0).max(1.0),
            at.h,
        );
        say(
            scene,
            vec![Run::tinted(
                clip(name, columns_in(text_at.w, column)),
                tint,
            )],
            text_at,
            tint,
        );
    }
    // The hairline between the rail and what it chose, so the two columns read
    // as two columns rather than as a list with a gap in it. Anchored off the
    // rail's edge, not the list's: the list stands its own padding in from
    // this line.
    if list.w >= 1.0 && list.h >= 1.0 {
        scene.rect(
            Panel::new(list.x - PAD - (GAP * 0.5).floor(), list.y, 1.0, list.h).fill(skin.edge),
        );
    }

    // The width a card's body has, in columns, which is what the model counted
    // every card's height in. Read off the list the cards are drawn in, the same
    // way the placement reads it.
    // The columns a card's text wraps in, off the box the rows are really drawn
    // in rather than off the whole list: the gutter down the right of it belongs
    // to the list's own bar, and text measured in a width it is not drawn in is
    // a row measured at one height and drawn at another.
    let cards_box = settings_list_rows(list);
    let list_cols = settings_entry_cols(cards_box.w, column);
    // The band the rows show through and the same window the placement read:
    // the layout hands over the visible part of each row, and the drawing
    // wants the row's true box back, so a card sliding past the top is drawn
    // where it really is and cut where it really ends.
    let keep = ListClip::of(cards_box);
    let window = panel.window(Text::rows_for(size, list.h), list_cols);
    // A text held to that band scrolls what the band cut off out of its own
    // box ([`ListClip::text`]), so the rows still showing keep their pixels.
    let held_text = |text: Text| keep.text(text);
    let hold_say = |scene: &mut Scene, runs: Vec<Run>, at: Panel, tint: [u8; 4]| {
        if let Some(text) = held_text(Text::rich(runs, at, size, tint)) {
            scene.text(text);
        }
    };
    for (index, side, shown) in &layout.settings_rows {
        let Some(whole_row) = panel.row(*index) else {
            continue;
        };
        let entry = whole_row;
        // The row's true box: the layout keeps only what is on screen, so the
        // part the window says is scrolled past the top goes back on, and a
        // row cut by the bottom gets its counted height back. What is drawn
        // from it is still held to the band, piece by piece.
        let mut full = *shown;
        if *index == window.first {
            let over = window.skip as f32 * line;
            full.y -= over;
            full.h += over;
        }
        let natural = crate::settings::lines(entry, list_cols) as f32 * line;
        if full.h + 0.5 < natural {
            full.h = natural;
        }
        let row = &full;
        let label_w = settings_label_w(row.w, column);
        let label_cols = columns_in(label_w, column).saturating_sub(1);
        let on = *index == panel.cursor() && panel.on_row();
        // Not on a block of text: a filled strip fourteen lines tall over a page
        // of prose is a highlight nobody can read through, and the mark down its
        // edge says the same thing.
        let carded = matches!(
            entry,
            SettingRow::Card(_)
                | SettingRow::Entry(_)
                | SettingRow::Paper(_)
                | SettingRow::Table(_)
        );
        if on {
            // A card says the keys are on it with its own border and the mark
            // down its edge, drawn by the card itself. A band nine lines tall
            // over a box is a highlight nobody can read through.
            if !carded {
                // A list being picked from carries the solid band the folder
                // picker's own session list carries; a form being read carries
                // the quiet strip, which is enough to say which row the keys
                // are on without shouting over the values beside it.
                //
                // A skill and a server are picked from too: the row under the
                // cursor is the one the whole column beside the list belongs to,
                // and the strip it used to wear is the panel's own colour at a
                // tenth more alpha, which over a dark desktop is no band at all.
                let band = match entry {
                    SettingRow::Entry(_) => skin.picked,
                    _ => skin.strip,
                };
                scene.rect(row.fill(band));
            }
            if !carded {
                scene.rect(Panel::new(row.x, row.y, MARK_W, row.h).fill(skin.edge_focus));
            }
        }
        // There is no hairline under a row. There was one under every row on
        // the panel, which is a line between every two things on screen and
        // therefore a line that says nothing about which of them belong
        // together. Grouping and space say that instead: a card has a border,
        // its fields have [`design::STEP`] between them, and two cards have
        // [`design::APART`]. The only dividers left on the panel are the one
        // under a card's title and the one under a table's column names.
        let text_x = row.x + MARK_W + 3.0;
        let whole = Panel::new(text_x, row.y, (row.w - MARK_W - 3.0).max(1.0), line);
        // One column short of what the box holds, so the mark saying a line was
        // cut fits inside it: a clipped line whose ellipsis does not fit wraps
        // out of a one line box and reads as a sentence that simply stops.
        let whole_cols = columns_in(whole.w, column).saturating_sub(1);
        let label_room = Panel::new(text_x, row.y, label_w.max(1.0), line);
        let value_at = settings_control(*row, label_w, column);
        match entry {
            // Prose, and the one row that can be trouble. The whole width: a
            // sentence in a label column is two words and three dots.
            SettingRow::Note { text, bad } => {
                let tint = match bad {
                    true => skin.bad,
                    false => skin.dim,
                };
                say(
                    scene,
                    vec![Run::tinted(clip(text, whole_cols), tint)],
                    whole,
                    tint,
                );
            }
            // The saved conversations, as a table inside a card: the count in
            // the header, the banded column names and the rows in the body, and
            // the three buttons in the footer. The rows were rows of the panel
            // with a trash on the end of each one, which said nothing about the
            // list being one thing and made deleting four of them eight presses.
            SettingRow::Table(table) => {
                let card = settings_card(*row, line);
                let parts = settings_card_parts(card, line, size, column, list_cols, true);
                settings_card_shell(
                    scene,
                    card,
                    &parts,
                    &table.title(),
                    skin.heading,
                    on,
                    skin,
                    size,
                    column,
                    keep,
                );
                // How far down a list longer than its body has been read, at the
                // right end of the header: a body showing twelve of forty with
                // nothing saying so reads as the whole list.
                let (names_at, boxes) = settings_table_parts(parts.body, line);
                if table.rows.len() > boxes.len() {
                    let last = (table.first + boxes.len()).min(table.rows.len());
                    let counter = format!("{}-{} of {}", table.first + 1, last, table.rows.len());
                    let wide = (counter.chars().count() as f32 + 1.0) * column;
                    let at = Panel::new(
                        parts.title.x + parts.title.w - wide,
                        parts.title.y,
                        wide,
                        line.min(parts.title.h),
                    );
                    if wide < parts.title.w * 0.5 && keep.holds(at) {
                        say(scene, vec![Run::tinted(counter, skin.dim)], at, skin.dim);
                    }
                }
                // The names stand on a filled band, which is what separates a
                // header from the data under it: rules between the columns and
                // nothing else drew them as one more row of the list.
                if let Some(band) = keep.cut(names_at) {
                    scene.rect(band.fill(skin.strip));
                    let names = settings_session_cells(names_at, column);
                    for (step, (at, name)) in names.iter().zip(&table.names).enumerate() {
                        if at.w < column {
                            continue;
                        }
                        let shown = clip(name, columns_in(at.w, column).saturating_sub(2));
                        let ink = settings_session_ink(*at, step, &shown, column);
                        hold_say(
                            scene,
                            vec![Run::tinted(shown, skin.dim)],
                            Panel::new(ink.x, names_at.y, ink.w, line),
                            skin.dim,
                        );
                    }
                    settings_session_lines(scene, &names, band, skin.edge);
                }
                for (step, at) in boxes.iter().enumerate() {
                    let Some(kept) = table.rows.get(table.first + step) else {
                        break;
                    };
                    let Some(band) = keep.cut(*at) else {
                        continue;
                    };
                    // The row the keys are on wears the solid band the folder
                    // picker's own session list wears, across the whole row:
                    // this is a list being picked from, and a tint on the words
                    // is invisible next to fourteen other tints.
                    let here = on && table.first + step == table.cursor;
                    if here {
                        scene.rect(band.fill(skin.picked));
                    }
                    let ink = match here {
                        true => skin.picked_ink,
                        false => skin.body,
                    };
                    let cells = settings_session_cells(*at, column);
                    for (step, (box_, text)) in cells
                        .iter()
                        .skip(crate::settings::SESSION_FIRST_CELL)
                        .zip(&kept.cells)
                        .enumerate()
                    {
                        if box_.w < column {
                            continue;
                        }
                        let step = step + crate::settings::SESSION_FIRST_CELL;
                        let shown = clip(text, columns_in(box_.w, column).saturating_sub(2));
                        let room = settings_session_ink(*box_, step, &shown, column);
                        hold_say(
                            scene,
                            vec![Run::tinted(shown, ink)],
                            Panel::new(room.x, at.y, room.w, line),
                            ink,
                        );
                    }
                    settings_session_lines(scene, &cells, band, skin.edge);
                    // The mark. A box that is filled when the row is one of the
                    // ones about to go: the whole of what multi selection is is
                    // being able to see which rows are in it without pressing
                    // anything.
                    if let Some(mark) = cells.get(crate::settings::SESSION_MARK) {
                        let hot = frame.hot == Some(Hit::SettingsMark(*index, table.first + step));
                        let tint = match (kept.marked, here) {
                            // On the picked band the mark inverts, or an accent
                            // tick on the accent band is a mark nobody can see.
                            (true, true) => skin.picked_ink,
                            (true, false) => skin.heading,
                            (false, true) => skin.picked_ink,
                            (false, false) => skin.dim,
                        };
                        if hot && let Some(under) = keep.cut(*mark) {
                            match keep.holds(*mark) {
                                true => scene.rect(panel_fill(*mark, skin.hot)),
                                false => scene.rect(under.fill(skin.hot)),
                            };
                        }
                        hold_say(
                            scene,
                            vec![Run::icon(
                                match kept.marked {
                                    true => icons::CHECKED.to_string(),
                                    false => icons::UNCHECKED.to_string(),
                                },
                                tint,
                            )],
                            Panel::new(mark.x + INPUT_PAD, at.y, mark.w, line),
                            tint,
                        );
                    }
                }
                // Its own bar, down the card's right padding and only as far as
                // the rows it counts: the header naming the columns and the
                // buttons under them do not scroll. Nothing at all for a list
                // that already fits in the body, or for a card cut by the edge
                // of the list: a bar over a cut body would count rows that are
                // not on screen.
                if let Some(first) = boxes.first()
                    && let Some(last) = boxes.last()
                {
                    let rows = Panel::new(
                        first.x,
                        first.y,
                        first.w,
                        last.y + last.h - first.y,
                    );
                    let bar = settings_card_bar(card, rows);
                    if keep.holds(bar) {
                        scrollbar(scene, skin, bar, table.thumb(boxes.len()));
                    }
                }
                // The buttons, centred in the footer: they act on the whole
                // list, and a button pinned to one end of a footer reads as
                // belonging to whatever is nearest that end.
                for (at, act, box_) in &layout.settings_acts {
                    if at != index {
                        continue;
                    }
                    let armed = *act == Act::Forget && panel.arming() == Some(*index);
                    let (kind, tint, word) = match act {
                        Act::All => (ButtonKind::Secondary, skin.body, String::from("select all")),
                        Act::None => {
                            (ButtonKind::Secondary, skin.body, String::from("select none"))
                        }
                        Act::Forget => (
                            ButtonKind::Danger,
                            skin.bad,
                            match (armed, table.chosen()) {
                                (true, _) => String::from("sure?"),
                                (false, 0) => String::from("delete"),
                                (false, many) => format!("delete {many}"),
                            },
                        ),
                        // A card's own action, which no table has one of. It
                        // is drawn by the card it stands in.
                        Act::Validate | Act::Install | Act::AddServer | Act::Restore => continue,
                    };
                    settings_button(
                        scene,
                        *box_,
                        kind,
                        vec![Run::tinted(word, tint)],
                        tint,
                        frame.hot == Some(Hit::SettingsAct(*index, *act)),
                        skin,
                        size,
                    );
                }
            }
            // A reading's value starts in the same column a control does and
            // runs to the end of the row rather than stopping where a value
            // would: most of them are paths, and a path in a value column is
            // three dots.
            SettingRow::Reading { label, value } => {
                say(
                    scene,
                    vec![Run::tinted(clip(label, label_cols), skin.dim)],
                    label_room,
                    skin.dim,
                );
                let value_room = Panel::new(
                    value_at.x,
                    row.y,
                    (row.x + row.w - value_at.x).max(1.0),
                    line,
                );
                say(
                    scene,
                    vec![Run::tinted(
                        clip(value, columns_in(value_room.w, column)),
                        skin.body,
                    )],
                    value_room,
                    skin.body,
                );
            }
            // The one field: text, and while it is being typed into, what has
            // been typed with a caret after it.
            SettingRow::Field { key, value } => {
                let tint = if on { skin.bright } else { skin.body };
                say(
                    scene,
                    vec![Run::tinted(clip(key, label_cols), tint)],
                    label_room,
                    tint,
                );
                let typing = on.then(|| panel.editing()).flatten();
                let (shown, ink) = match (typing, value.is_empty()) {
                    (Some(typed), _) => (typed.to_string(), skin.bright),
                    (None, true) => (String::from(crate::settings::UNSET), skin.dim),
                    (None, false) => (value.clone(), skin.bright),
                };
                // The box a field is typed into, drawn as a box: the prompt's
                // own fill and an outline round it. Without one an editable row
                // looked exactly like a reading, and the only way to find out
                // which was which was to press one.
                scene.rect(panel_fill(value_at, skin.input));
                if frame.hot == Some(Hit::SettingsValue(*index, *side)) {
                    scene.rect(panel_fill(value_at, skin.hot));
                }
                scene.rect(panel_edge(
                    value_at,
                    match typing.is_some() || on {
                        true => skin.edge_focus,
                        false => skin.edge,
                    },
                ));
                // The end of it rather than the start: what changes in an
                // endpoint is the port and the path, and a URL clipped from the
                // left keeps the half being typed on screen. Inside the box
                // rather than on its stroke, and two columns short of it, so the
                // caret after the text is inside the box as well.
                let inside = Panel::new(
                    value_at.x + INPUT_PAD,
                    value_at.y,
                    (value_at.w - INPUT_PAD * 2.0).max(1.0),
                    value_at.h,
                );
                let shown = tail(&shown, SETTING_VALUE_COLUMNS.saturating_sub(2));
                say(scene, vec![Run::tinted(shown.clone(), ink)], inside, ink);
                // The same caret the prompt draws, in the same colour: a block
                // character would be a glyph that can be missing, and a missing
                // glyph draws as nothing at all.
                if typing.is_some() {
                    scene.rect(
                        Panel::new(
                            inside.x + shown.chars().count() as f32 * column,
                            inside.y,
                            2.0,
                            line,
                        )
                        .fill(skin.caret),
                    );
                }
            }
            SettingRow::Setting { key, value, .. } => {
                let tint = if on { skin.bright } else { skin.body };
                say(
                    scene,
                    vec![Run::tinted(clip(key, label_cols), tint)],
                    label_room,
                    tint,
                );
                let track = layout
                    .settings_tracks
                    .iter()
                    .find(|(at, half, _)| at == index && half == side)
                    .map(|(_, _, track)| *track);
                let value = panel.preview(*index, *side).unwrap_or(value);
                match track {
                    // A setting with a range is a position on a track, with the
                    // number it is at beside it. Nineteen presses of an arrow
                    // key from one end of opacity to the other is not a control.
                    Some(track) if track.w >= 1.0 => {
                        // Nothing changes on rollover: a slider that lights
                        // up under a passing pointer read as a selection
                        // effect nobody asked for.
                        let thick = (line * 0.3).floor().max(2.0);
                        let up = ((line - thick) * 0.5).floor();
                        let at = panel.fraction(*index, *side).unwrap_or(0.0);
                        scene.rect(
                            Panel::new(track.x, track.y + up, track.w, thick)
                                .fill(skin.gauge_track),
                        );
                        scene.rect(
                            Panel::new(track.x, track.y + up, (track.w * at).floor(), thick)
                                .fill(skin.gauge),
                        );
                        // The grip: a bar at the position, tall enough to
                        // press, brighter while the pointer is on the track.
                        let grip = CARET_W;
                        scene.rect(
                            Panel::new(
                                track.x + ((track.w - grip) * at).floor(),
                                track.y + 1.0,
                                grip,
                                (line - 2.0).max(1.0),
                            )
                            .fill(skin.edge_focus),
                        );
                        let number = Panel::new(
                            track.x + track.w + column,
                            row.y,
                            (value_at.w - track.w - column).max(1.0),
                            line,
                        );
                        say(
                            scene,
                            vec![Run::tinted(
                                clip(value, SETTING_TRACK_VALUE_COLUMNS.saturating_sub(1)),
                                skin.bright,
                            )],
                            number,
                            skin.bright,
                        );
                    }
                    // The value of a setting that can change is drawn as the
                    // control it is: a box with an outline round it, accent
                    // tinted, and lit under the pointer the way a window button
                    // is. The same box the field gets, because they answer the
                    // same press: anything with an outline here can be changed.
                    _ => {
                        scene.rect(panel_fill(value_at, skin.input));
                        if frame.hot == Some(Hit::SettingsValue(*index, *side)) {
                            scene.rect(panel_fill(value_at, skin.hot));
                        }
                        scene.rect(panel_edge(
                            value_at,
                            match on {
                                true => skin.edge_focus,
                                false => skin.edge,
                            },
                        ));
                        say(
                            scene,
                            vec![Run::tinted(
                                clip(value, SETTING_VALUE_COLUMNS.saturating_sub(2)),
                                skin.bright,
                            )],
                            Panel::new(
                                value_at.x + INPUT_PAD,
                                value_at.y,
                                (value_at.w - INPUT_PAD * 2.0).max(1.0),
                                value_at.h,
                            ),
                            skin.bright,
                        );
                    }
                }
            }
            // One group of the palette, as a card: what the group paints in the
            // header, and a block of each colour with what it colours written
            // beside it in the body, with a hairline between the cells so a row
            // reads as several things and not as one sentence.
            SettingRow::Palette(palette) => {
                let card = settings_card(*row, line);
                let parts = settings_card_parts(card, line, size, column, list_cols, false);
                settings_card_shell(
                    scene,
                    card,
                    &parts,
                    palette.title,
                    skin.heading,
                    on,
                    skin,
                    size,
                    column,
                    keep,
                );
                let across = design::swatch_across(design::card_cols(list_cols));
                let slots =
                    settings_palette_slots(parts.body, line, palette.cells.len(), across);
                for (cell, (colour, at)) in palette.cells.iter().zip(&slots).enumerate() {
                    let at = *at;
                    // Only the colours wholly on screen, the same rule the
                    // layout keeps their press regions by: a swatch is a block
                    // with an outline, and half of one reads as another colour.
                    if !keep.holds(at) {
                        continue;
                    }
                    let held = panel.picked() == Some((*index, cell));
                    if held {
                        scene.rect(at.fill(skin.strip));
                    }
                    if frame.hot == Some(Hit::SettingsSwatch(*index, cell)) {
                        scene.rect(at.fill(skin.hot));
                    }
                    // Between two colours on one line, and never down the left
                    // edge of the body: a rule there would be a second border a
                    // column in from the card's own.
                    if cell % across.max(1) > 0 {
                        scene.rect(
                            Panel::new(at.x, at.y + 2.0, 1.0, (at.h - 4.0).max(1.0))
                                .fill(skin.edge),
                        );
                    }
                    let side = (line * 0.6).floor().max(2.0);
                    let up = ((at.h - side) * 0.5).floor().max(0.0);
                    let block = Panel::new(at.x + INPUT_PAD, at.y + up, side, side);
                    scene.rect(block.fill(swatch(colour.rgb)));
                    // An outline round the block as well: a swatch of the
                    // panel's own colour on the panel would have no edges at all.
                    scene.rect(block.fill(skin.edge).stroke(1.0));
                    let words = Panel::new(
                        block.x + side + column,
                        at.y,
                        (at.x + at.w - block.x - side - column).max(1.0),
                        line.min(at.h),
                    );
                    let ink = match held {
                        true => skin.bright,
                        false => skin.body,
                    };
                    // While the pressed swatch is being typed into, its label
                    // gives way to the hex line, with the same caret every
                    // field draws: the edit happens where the colour is rather
                    // than only on the footer.
                    let typing = held.then(|| panel.editing()).flatten();
                    let written = match typing {
                        Some(typed) => tail(typed, columns_in(words.w, column).saturating_sub(2)),
                        None => clip(colour.about, columns_in(words.w, column).saturating_sub(1)),
                    };
                    say(scene, vec![Run::tinted(written.clone(), ink)], words, ink);
                    if typing.is_some() {
                        scene.rect(
                            Panel::new(
                                words.x + written.chars().count() as f32 * column,
                                words.y,
                                2.0,
                                line.min(at.h),
                            )
                            .fill(skin.caret),
                        );
                    }
                }
            }
            // One skill or one server, as a card of its own: its name in the
            // header, what it is for and where it is in the body, and its two
            // buttons in the footer. It was three bare lines with the buttons
            // pinned beside the name, which is the shape the whole panel is
            // being taken out of. A card that is off is drawn in the quiet tint
            // the whole way through, so a list says which of them the agent will
            // actually load without anything having to be read.
            SettingRow::Entry(entry) => {
                let at = settings_card(*row, line);
                let parts = settings_card_parts(at, line, size, column, list_cols, true);
                let title_tint = match (entry.on, on) {
                    (_, true) => skin.bright,
                    (true, false) => skin.heading,
                    (false, false) => skin.dim,
                };
                settings_card_shell(
                    scene,
                    at,
                    &parts,
                    &entry.name,
                    title_tint,
                    on,
                    skin,
                    size,
                    column,
                    keep,
                );
                let about_tint = match entry.on {
                    true => skin.body,
                    false => skin.dim,
                };
                // The description wraps onto as many rows as it needs, in the
                // columns the model counted its height in: one number, so the
                // rows counted, the room left and the rows drawn are the same
                // rows.
                let about_cols = design::card_cols(list_cols);
                let about_rows = crate::settings::about_rows(&entry.about, about_cols);
                let about_at = Panel::new(
                    parts.body.x,
                    parts.body.y,
                    parts.body.w,
                    (about_rows as f32 * line).min(parts.body.h.max(0.0)),
                );
                if !entry.about.is_empty()
                    && about_at.h >= line
                    && let Some(text) = held_text(
                        Text::rich(
                            vec![Run::tinted(entry.about.clone(), about_tint)],
                            about_at,
                            size,
                            about_tint,
                        )
                        .wrap_at(about_cols),
                    )
                {
                    scene.text(text);
                }
                // Where it came from, in the hint role: a repository or a path
                // is the quietest of the three things a row of a list carries,
                // and drawing it at the size of the description made all three
                // of them one run of text.
                let small = design::hint_size(size);
                let small_column = design::column_for(column, size, design::HINT);
                let under_at = Panel::new(
                    parts.body.x,
                    parts.body.y + about_rows as f32 * line + design::tight(line),
                    parts.body.w,
                    Text::line_for(small).min(line),
                );
                if !entry.under.is_empty()
                    && under_at.y + under_at.h <= at.y + at.h
                    && keep.holds(under_at)
                {
                    scene.text(Text::rich(
                        vec![Run::tinted(
                            clip(
                                &entry.under,
                                columns_in(under_at.w, small_column).saturating_sub(1),
                            ),
                            skin.dim,
                        )],
                        under_at,
                        small,
                        skin.dim,
                    ));
                }
                // The toggle: filled while it is on, because a fill is what says
                // a thing is live, and outlined while it is off.
                for (at, box_) in &layout.settings_toggles {
                    if at != index {
                        continue;
                    }
                    let ink = match entry.on {
                        true => skin.bright,
                        false => skin.dim,
                    };
                    settings_button(
                        scene,
                        *box_,
                        match entry.on {
                            true => ButtonKind::Primary,
                            false => ButtonKind::Secondary,
                        },
                        vec![Run::tinted(
                            match entry.on {
                                true => "on",
                                false => "off",
                            },
                            ink,
                        )],
                        ink,
                        frame.hot == Some(Hit::SettingsToggle(*index)),
                        skin,
                        size,
                    );
                }
                // In the colour this window uses for everything that throws
                // work away, which is what a delete is. Pressed once, it says so
                // and waits for the second press; the footer says what would go
                // with it.
                for (at, box_) in &layout.settings_removes {
                    if at != index {
                        continue;
                    }
                    let armed = panel.arming() == Some(*index);
                    settings_button(
                        scene,
                        *box_,
                        ButtonKind::Danger,
                        vec![Run::tinted(
                            match armed {
                                true => "sure?",
                                false => "uninstall",
                            },
                            skin.bad,
                        )],
                        skin.bad,
                        frame.hot == Some(Hit::SettingsRemove(*index)),
                        skin,
                        size,
                    );
                }
            }
            // A group of settings in a box of its own: the title bar, the one
            // divider, and the fields under it. Two fields across while the card
            // is wide enough for both to keep their columns and one across when
            // it is not, which is the whole of what this panel does about a
            // narrow window.
            SettingRow::Card(card) => {
                let at = settings_card(*row, line);
                let parts =
                    settings_card_parts(at, line, size, column, list_cols, card.does.is_some());
                settings_card_shell(
                    scene,
                    at,
                    &parts,
                    &card.title,
                    skin.heading,
                    on,
                    skin,
                    size,
                    column,
                    keep,
                );
                let hints = crate::settings::card_hints(card);
                let across = design::across(card.fields.len(), design::card_cols(list_cols));
                let slots = settings_card_slots(parts.body, line, &hints, across);
                for (at, (field, slot)) in card.fields.iter().zip(&slots).enumerate() {
                    if slot.y + slot.h > parts.body.y + parts.body.h + 0.01 {
                        continue;
                    }
                    // A field cut by either edge of the list is not drawn at
                    // all, the same rule the layout keeps its press region by:
                    // half an input box reads as a whole one further down.
                    if !keep.holds(*slot) {
                        continue;
                    }
                    // Which slot the keys are in, so the field being changed is
                    // the one wearing the focus edge. A card says the keys are
                    // on it with its border; the field says which of its two
                    // they are on.
                    let slot_side = match at {
                        0 => Side::Left,
                        1 => Side::Right,
                        _ => Side::Left,
                    };
                    let here = on && at < 2 && panel.side() == slot_side;
                    settings_card_field(
                        scene,
                        frame,
                        panel,
                        field,
                        *slot,
                        (*index, slot_side),
                        here,
                        line,
                    );
                }
                if let Some(hint) = &card.hint {
                    let small = design::hint_size(size);
                    let small_column = design::column_for(column, size, design::HINT);
                    // Under the last band, at the height the model counted the
                    // bands to rather than at the bottom of whatever was drawn:
                    // a field cut off by the bottom of the list must not pull
                    // the sentence up over the field above it.
                    let hint_at = Panel::new(
                        parts.body.x,
                        parts.body.y
                            + design::fields_lines(&hints, across) * line
                            + design::step(line),
                        parts.body.w,
                        Text::line_for(small).min(line),
                    );
                    if hint_at.y + hint_at.h <= at.y + at.h && keep.holds(hint_at) {
                        scene.text(Text::rich(
                            vec![Run::tinted(
                                clip(
                                    hint,
                                    columns_in(hint_at.w, small_column).saturating_sub(1),
                                ),
                                skin.dim,
                            )],
                            hint_at,
                            small,
                            skin.dim,
                        ));
                    }
                }
                // The card's own action, at the bottom right of its footer: it
                // is the thing the card exists for, which is what a primary is.
                // Unless it loses something, and then it is drawn in the colour
                // this window keeps for exactly that and asks once before it
                // acts.
                if let Some(doing) = card.does {
                    for (at, act, box_) in &layout.settings_acts {
                        if at != index || *act != settings_act_for(doing) {
                            continue;
                        }
                        let armed = doing.dangerous() && panel.arming() == Some(*index);
                        let (kind, tint) = match doing.dangerous() {
                            true => (ButtonKind::Danger, skin.bad),
                            false => (ButtonKind::Primary, skin.bright),
                        };
                        let word = match armed {
                            true => "sure?",
                            false => doing.word(),
                        };
                        settings_button(
                            scene,
                            *box_,
                            kind,
                            vec![Run::tinted(word, tint)],
                            tint,
                            frame.hot == Some(Hit::SettingsAct(*index, *act)),
                            skin,
                            size,
                        );
                    }
                }
            }
            // A block of text under a title of its own: what the agent is really
            // told, and where that came from. Rendered as Markdown, the way the
            // column beside the skills list renders a `SKILL.md`, because both of
            // these are Markdown and printing the marks would be showing the
            // file rather than the instructions.
            SettingRow::Paper(paper) => {
                let box_ = settings_card(*row, line);
                let parts = settings_card_parts(box_, line, size, column, list_cols, false);
                // The text starts under the line saying where it came from, and
                // runs to the bottom of the body. How many lines that really is
                // rather than [`crate::settings::PAPER_LINES`], because a block
                // the bottom of the list cut off shows fewer, and a counter or a
                // bar reading twelve of a box showing four is the one readout
                // there is saying something that is not on the screen.
                let text = settings_paper_text(&parts, line);
                let held = (text.h / line).round() as usize;
                settings_card_shell(
                    scene,
                    box_,
                    &parts,
                    &paper.title,
                    skin.heading,
                    on,
                    skin,
                    size,
                    column,
                    keep,
                );
                let body_cols = columns_in(parts.body.w, column).saturating_sub(1);
                // How far down a block that is longer than its box has been
                // read, at the right end of the header: a page of prose with no
                // way of telling how much of it is left is a box that reads as
                // the whole thing.
                if paper.body.len() > held {
                    let last = (paper.first + held).min(paper.body.len());
                    let counter = format!("{}-{} of {}", paper.first + 1, last, paper.body.len());
                    let wide = (counter.chars().count() as f32 + 1.0) * column;
                    let at = Panel::new(
                        parts.title.x + parts.title.w - wide,
                        parts.title.y,
                        wide,
                        line.min(parts.title.h),
                    );
                    if wide < parts.title.w * 0.5 && keep.holds(at) {
                        say(scene, vec![Run::tinted(counter, skin.dim)], at, skin.dim);
                    }
                }
                // Where the text came from, in the hint role: a path under a
                // title is the quietest thing a block carries, and drawing it at
                // the size of the text made the two one run of words.
                let under = match paper.bad {
                    true => skin.bad,
                    false => skin.dim,
                };
                let small = design::hint_size(size);
                let small_column = design::column_for(column, size, design::HINT);
                if let Some(from) = held_text(Text::rich(
                    vec![Run::tinted(
                        clip(
                            &paper.under,
                            columns_in(parts.body.w, small_column).saturating_sub(1),
                        ),
                        under,
                    )],
                    Panel::new(
                        parts.body.x,
                        parts.body.y,
                        parts.body.w,
                        Text::line_for(small).min(line),
                    ),
                    small,
                    under,
                )) {
                    scene.text(from);
                }
                // Where the fences stand after everything scrolled off, so a
                // block that starts inside a code block is drawn as code.
                let mut fence = crate::markdown::fence_after(
                    paper.body.iter().take(paper.first).map(String::as_str),
                );
                // While the document is being edited its lines are drawn raw:
                // the formatter eats the marks, and a caret counted on the
                // characters would stand beside glyphs that are not them.
                let editing = panel.instructions_caret(*index);
                for (step, at) in paper.body.iter().skip(paper.first).take(held).enumerate() {
                    let box_ = Panel::new(text.x, text.y + step as f32 * line, text.w, line);
                    let mut runs = Vec::new();
                    match editing.is_some() {
                        true => runs.push(Run::tinted(clip(at, body_cols), skin.bright)),
                        false => {
                            crate::markdown::line(&clip(at, body_cols), &mut fence, skin, &mut runs)
                        }
                    }
                    if let Some(text) = held_text(Text::rich(runs, box_, size, skin.body)) {
                        scene.text(text);
                    }
                }
                // The caret, where the typing lands: the same bar every field
                // on this panel draws, at the character it stands before.
                if let Some((on, col)) = editing
                    && on >= paper.first
                    && on < paper.first + held
                {
                    let x = (text.x + col.min(body_cols) as f32 * column)
                        .min(text.x + text.w - 2.0);
                    let at = Panel::new(
                        x,
                        text.y + (on - paper.first) as f32 * line,
                        2.0,
                        line,
                    );
                    if keep.holds(at) {
                        scene.rect(at.fill(skin.caret));
                    }
                }
                // Its own bar, down the card's right padding and only as far as
                // the text it counts. Nothing at all for a block that is already
                // all on screen, or for a card cut by the edge of the list: a
                // bar over a cut body would count lines that are not on screen.
                let bar = settings_card_bar(box_, text);
                if keep.holds(bar) {
                    scrollbar(scene, skin, bar, paper.thumb(held));
                }
            }
        }
    }
    // The column beside that list: the entry under the cursor, rendered the way
    // the transcript renders what the model writes, because a `SKILL.md` is
    // Markdown and showing it with its marks in would be showing the file
    // rather than the skill.
    let doc = layout.settings_doc;
    if doc.w >= 1.0 && doc.h >= 1.0 {
        scene.rect(Panel::new(doc.x - (GAP * 0.5).floor(), doc.y, 1.0, doc.h).fill(skin.edge));
        let entry = panel.showing();
        // The document is a card, the way everything else on this panel is: the
        // name of whatever it belongs to in the header, the one divider under
        // it, and the text in the body. It was a bare line of text over an
        // outlined box, which on a panel of cards is the one thing beside the
        // list that does not read as one.
        let box_ = settings_doc_box(doc, line);
        let inside = layout.settings_doc_text;
        if box_.w >= 1.0 && inside.w >= 1.0 {
            let parts = settings_doc_parts(box_, line, size);
            settings_card_shell(
                scene,
                box_,
                &parts,
                entry.map(|entry| entry.name.as_str()).unwrap_or_default(),
                skin.heading,
                false,
                skin,
                size,
                column,
                ListClip::open(),
            );
            let doc_cols = layout.settings_doc_columns(column);
            let doc_rows = layout.settings_doc_rows(size);
            if let Some(entry) = entry {
                // Wrapped rather than clipped: a line of a `SKILL.md` is a
                // sentence, and a column that ended every one of them in an
                // ellipsis was showing the left edge of a document. The rows
                // come from the same wrap rule the panes use, counted on what
                // the formatter draws, and the renderer breaks the box with that
                // rule rather than with one of its own.
                let window = panel.doc_window(doc_cols, doc_rows);
                // The band under the glyphs, over the pane the panel builds its
                // document into. Before the text and not after it: a highlight
                // painted over the characters it highlights hides them.
                if let Some(selection) = frame.selection
                    && selection.at == crate::select::Where::SettingsDoc
                    && !selection.is_empty()
                {
                    paint_selection(
                        scene,
                        selection,
                        &panel.doc_pane_at(doc_cols, doc_rows),
                        Painted {
                            content: inside,
                            rows: doc_rows,
                            cols: doc_cols,
                            chrome: 0,
                            size,
                            column,
                            tint: skin.select,
                        },
                    );
                }
                let mut fence = crate::markdown::fence_after(
                    entry.doc.iter().take(window.first).map(String::as_str),
                );
                let mut runs = Vec::new();
                for text in entry.doc.iter().skip(window.first).take(window.count) {
                    crate::markdown::line(text, &mut fence, skin, &mut runs);
                    runs.push(Run::plain("\n"));
                }
                scene.text(
                    Text::rich(runs, inside, size, skin.body)
                        .scrolled(window.skip as f32)
                        .wrap_at(doc_cols),
                );
                if entry.doc.is_empty() {
                    say(
                        scene,
                        vec![Run::tinted(
                            clip("nothing to show: this one has no SKILL.md", doc_cols),
                            skin.dim,
                        )],
                        Panel::new(inside.x, inside.y, inside.w, line),
                        skin.dim,
                    );
                }
                // Its own bar, down the card's right padding and only as far as
                // the body it counts: a bar that ran the whole height of the
                // card would have its head beside the header, which is not part
                // of what scrolls. The list's used to be painted against the
                // right edge of the whole panel, which is over this column
                // rather than beside the list it counts.
                scrollbar(
                    scene,
                    skin,
                    settings_card_bar(box_, parts.body),
                    panel.doc_thumb(doc_cols, doc_rows),
                );
            }
        }
    }
    // The list's own bar, in the gutter the cards were kept out of. It used to
    // be drawn on the list itself, which put it through the right edge of every
    // card in it and through the buttons in their footers.
    scrollbar(
        scene,
        skin,
        settings_list_bar(layout.settings_list),
        panel.thumb(
            layout.settings_capacity(size),
            layout.settings_entry_columns(column),
        ),
    );

    // What the keys do to the row under the cursor, which swatch was pressed, or
    // why the last change did not land. A panel that writes a file has to say
    // when the file refused.
    let (foot, tint) = match panel.trouble() {
        Some(why) => (clip(why, cols), skin.bad),
        None => (clip(&panel.says(), cols), skin.dim),
    };
    say(
        scene,
        vec![Run::tinted(foot, tint)],
        Panel::new(content.x, content.y + content.h - line, content.w, line),
        tint,
    );
}

/// A colour from the settings file as the renderer wants it. Fully opaque: the
/// swatch is the colour itself, and the panel's own fill is what carries the
/// window's transparency.
pub(crate) fn swatch(rgb: [u8; 3]) -> [f32; 4] {
    [
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
        1.0,
    ]
}
