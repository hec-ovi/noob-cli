//! The settings panel painter: everything the panel draws, from the places
//! computed beside it.

use noob_draw::{Panel, Run, Scene, Text};

#[allow(unused_imports)]
use crate::design;
use crate::design::icons;
use crate::settings::{Row as SettingRow, Side};
#[allow(clippy::wildcard_imports)]
use crate::settings::places::*;
use crate::settings::Act;
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
        let text_at = Panel::new(
            at.x + MARK_W + 3.0,
            at.y,
            (at.w - MARK_W - 3.0).max(1.0),
            at.h,
        );
        let shown = clip(name, columns_in(text_at.w, column));
        if chosen {
            // The band hugs the name: the mark, the text and a breath of
            // padding, never the whole cell. "the line on each menu is too
            // long, exceeding the size of the text". The press region stays
            // the full cell; only the paint tightened.
            let band = (MARK_W + 3.0 + (shown.chars().count() as f32 + 1.0) * column).min(at.w);
            scene.rect(Panel::new(at.x, at.y, band, at.h).fill(skin.strip));
            scene.rect(
                Panel::new(
                    at.x,
                    design::mark_top(at.y, at.h, line),
                    MARK_W,
                    design::mark_height(line),
                )
                .fill(skin.edge_focus),
            );
        }
        let tint = match (chosen, frame.hot == Some(Hit::SettingsSection(*index))) {
            (true, _) => skin.heading,
            (false, true) => skin.bright,
            (false, false) => skin.dim,
        };
        say(scene, vec![Run::tinted(shown, tint)], text_at, tint);
    }
    // The hairline between the rail and what it chose, so the two columns read
    // as two columns rather than as a list with a gap in it. Anchored off the
    // rail's edge, not the list's: the list stands its own padding in from this
    // line, and it runs the full height of the panel, which is what the two
    // columns are.
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
    for (index, _, shown) in &layout.settings_rows {
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
        // The band this row stands in: its own height, or the pair's, so two
        // cards side by side are drawn as one row at one height.
        let natural = crate::settings::band_lines(panel.rows(), *index, list_cols) as f32 * line;
        if full.h + 0.5 < natural {
            full.h = natural;
        }
        let row = &full;
        // The columns this row wraps in: its own width, which is half the list
        // for either card of a pair.
        let list_cols = settings_entry_cols(row.w, column);
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
                    let names = settings_session_cells(names_at, column, table.columns);
                    for (step, (at, (name, ..))) in
                        names.iter().zip(table.columns).enumerate()
                    {
                        if at.w < column {
                            continue;
                        }
                        let shown = clip(name, columns_in(at.w, column).saturating_sub(2));
                        let ink = settings_session_ink(*at, step, &shown, column, table.columns);
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
                    let cells = settings_session_cells(*at, column, table.columns);
                    let first_cell = table.of.first_cell();
                    for (step, (box_, text)) in
                        cells.iter().skip(first_cell).zip(&kept.cells).enumerate()
                    {
                        if box_.w < column {
                            continue;
                        }
                        let step = step + first_cell;
                        let shown = clip(text, columns_in(box_.w, column).saturating_sub(2));
                        let room =
                            settings_session_ink(*box_, step, &shown, column, table.columns);
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
                    if let Some(mark) = table.of.mark().and_then(|at| cells.get(at)) {
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
                    let armed = matches!(act, Act::Forget | Act::Uninstall)
                        && panel.arming() == Some(*index);
                    // What the two skill buttons act on: the row the keys are
                    // on, when that row is a skill installed here. The shipped
                    // web search is neither turned off nor uninstalled, so both
                    // buttons are dim while the cursor is on it.
                    let acting = table.at_cursor().and_then(|row| row.on);
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
                        Act::Turn => (
                            ButtonKind::Secondary,
                            match acting.is_some() {
                                true => skin.body,
                                false => skin.dim,
                            },
                            String::from(match acting {
                                Some(true) => "turn off",
                                _ => "turn on",
                            }),
                        ),
                        Act::Uninstall => (
                            match acting.is_some() {
                                true => ButtonKind::Danger,
                                false => ButtonKind::Secondary,
                            },
                            match acting.is_some() {
                                true => skin.bad,
                                false => skin.dim,
                            },
                            String::from(match armed {
                                true => "sure?",
                                false => "uninstall",
                            }),
                        ),
                        // A card's or a document's own action, which no table
                        // has one of. Each is drawn by the row it stands in.
                        Act::Validate
                        | Act::Install
                        | Act::AddServer
                        | Act::Restore
                        | Act::EditPrompt
                        | Act::SavePrompt
                        | Act::RestorePrompt
                        | Act::LoadPrompt
                        | Act::Check
                        | Act::Reveal
                        | Act::DefaultEndpoint => continue,
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
                    // The pointer lights the block of colour and nothing else.
                    // It used to wash the whole row, which on a list of colours
                    // is a band over the two things being compared.
                    let hot = frame.hot == Some(Hit::SettingsSwatch(*index, cell));
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
                    // It is the focus colour under the pointer, which is the
                    // whole of the rollover.
                    scene.rect(
                        block
                            .fill(match hot {
                                true => skin.edge_focus,
                                false => skin.edge,
                            })
                            .stroke(1.0),
                    );
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
                let group = card.group.as_ref().map(|group| group.at);
                let slots = settings_card_slots(parts.body, line, &hints, across, group);
                // A second group inside the card: a rule the width of the body
                // with its title over the band it opens, so two batches of
                // settings read as two batches under one heading.
                if let (Some(group), Some(slot)) =
                    (card.group.as_ref(), slots.get(group.unwrap_or(0)))
                {
                    let big = design::card_title_size(size);
                    let big_column = design::column_for(column, size, design::CARD_TITLE);
                    let tall = design::TITLE_LINES * line;
                    let title = Panel::new(
                        parts.body.x,
                        slot.y - design::tight(line) - tall,
                        parts.body.w,
                        tall,
                    );
                    let rule = Panel::new(
                        parts.body.x,
                        (title.y - design::tight(line)).floor(),
                        parts.body.w,
                        1.0,
                    );
                    if rule.y >= parts.body.y && keep.holds(rule) {
                        scene.rect(rule.fill(skin.edge));
                    }
                    if let Some(text) = keep.text(Text::rich(
                        vec![Run::tinted(
                            clip(
                                &group.title,
                                columns_in(title.w, big_column).saturating_sub(1),
                            ),
                            skin.heading,
                        )],
                        title,
                        big,
                        skin.heading,
                    )) {
                        scene.text(text);
                    }
                }
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
                    // on it with its border; the field says which of them they
                    // are on.
                    let slot_side = Side::of(at).unwrap_or(Side::Left);
                    let here = on && Side::of(at).is_some() && panel.side() == slot_side;
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
                            + design::fields_lines(&hints, across, group) * line
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
                        // The one button that says what it does with a mark as
                        // well as a word: an eye, open or struck through, for
                        // the press that puts a credential on the screen.
                        let runs = match (armed, doing) {
                            (false, crate::settings::Doing::Reveal) => vec![
                                Run::icon(icons::EYE.to_string(), tint),
                                Run::tinted(format!(" {word}"), tint),
                            ],
                            (false, crate::settings::Doing::Hide) => vec![
                                Run::icon(icons::EYE_OFF.to_string(), tint),
                                Run::tinted(format!(" {word}"), tint),
                            ],
                            _ => vec![Run::tinted(word, tint)],
                        };
                        settings_button(
                            scene,
                            *box_,
                            kind,
                            runs,
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
                let parts =
                    settings_card_parts(box_, line, size, column, list_cols, paper.does);
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
                // While the load line is open on this block it stands where the
                // path does, drawn as the box every other typed value on this
                // panel is drawn as: fill, outline, caret. It was a line of
                // hint-sized prose, which is why pressing load read as a
                // button that did nothing.
                let loading_here = panel.loading() && panel.cursor() == *index;
                let small = design::hint_size(size);
                let small_column = design::column_for(column, size, design::HINT);
                let under_at = Panel::new(
                    parts.body.x,
                    parts.body.y,
                    parts.body.w,
                    Text::line_for(small).min(line),
                );
                if loading_here {
                    let field = Panel::new(parts.body.x, parts.body.y, parts.body.w, line);
                    scene.rect(panel_fill(field, skin.input));
                    scene.rect(panel_edge(field, skin.edge_focus));
                    let inside = Panel::new(
                        field.x + INPUT_PAD,
                        field.y,
                        (field.w - INPUT_PAD * 2.0).max(1.0),
                        field.h,
                    );
                    let room = columns_in(inside.w, column).saturating_sub(1);
                    let typed = tail(panel.editing().unwrap_or_default(), room);
                    say(
                        scene,
                        vec![Run::tinted(format!("load {typed}"), skin.bright)],
                        inside,
                        skin.bright,
                    );
                    scene.rect(
                        Panel::new(
                            inside.x + (typed.chars().count() + 5) as f32 * column,
                            inside.y,
                            2.0,
                            line,
                        )
                        .fill(skin.caret),
                    );
                } else {
                    let under = match paper.bad {
                        true => skin.bad,
                        false => skin.dim,
                    };
                    if let Some(from) = held_text(Text::rich(
                        vec![Run::tinted(
                            clip(
                                &paper.under,
                                columns_in(parts.body.w, small_column).saturating_sub(1),
                            ),
                            under,
                        )],
                        under_at,
                        small,
                        under,
                    )) {
                        scene.text(from);
                    }
                }
                // Where the fences stand after everything scrolled off, so a
                // block that starts inside a code block is drawn as code.
                let mut fence = crate::markdown::fence_after(
                    paper.body.iter().take(paper.first).map(String::as_str),
                );
                // The band under the glyphs of a drag over this block, before
                // the text and not after it: a highlight painted over the
                // characters it covers hides them.
                if let Some(selection) = frame.selection
                    && selection.at == crate::select::Where::SettingsPaper(*index)
                    && !selection.is_empty()
                {
                    paint_selection(
                        scene,
                        selection,
                        &panel.paper_pane_at(*index, held),
                        Painted {
                            content: text,
                            rows: held,
                            cols: body_cols,
                            chrome: 0,
                            size,
                            column,
                            tint: skin.select,
                        },
                    );
                }
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
                // The footer of a document that is edited here: the
                // enable-edition checkbox, and the actions the layout placed.
                // Drawn off the same boxes the presses are tested in.
                if paper.does {
                    let ticked = panel.edition_on(*index);
                    for (at, act, box2) in &layout.settings_acts {
                        if at != index {
                            continue;
                        }
                        match act {
                            Act::EditPrompt => {
                                // No band under the pointer: a checkbox that
                                // fills in as the pointer crosses it reads as
                                // a row being selected, and this one is not a
                                // row. The mark is the state and the only
                                // state it has.
                                let ink = match ticked {
                                    true => skin.heading,
                                    false => skin.dim,
                                };
                                hold_say(
                                    scene,
                                    vec![
                                        Run::icon(
                                            match ticked {
                                                true => icons::CHECKED.to_string(),
                                                false => icons::UNCHECKED.to_string(),
                                            },
                                            ink,
                                        ),
                                        Run::tinted(" enable edition", ink),
                                    ],
                                    Panel::new(
                                        box2.x + INPUT_PAD,
                                        box2.y,
                                        (box2.w - INPUT_PAD).max(1.0),
                                        box2.h,
                                    ),
                                    ink,
                                );
                            }
                            // The save and the restore stand in the footer
                            // whether or not edition is on, dim while it is
                            // off: a button that appears when a checkbox is
                            // ticked is a button nobody knew was there.
                            Act::SavePrompt => {
                                let ink = match ticked {
                                    true => skin.bright,
                                    false => skin.dim,
                                };
                                settings_button(
                                    scene,
                                    *box2,
                                    ButtonKind::Primary,
                                    vec![Run::tinted("save", ink)],
                                    ink,
                                    ticked
                                        && frame.hot
                                            == Some(Hit::SettingsAct(*index, Act::SavePrompt)),
                                    skin,
                                    size,
                                );
                            }
                            Act::RestorePrompt => {
                                let armed = panel.arming() == Some(*index);
                                let ink = match ticked {
                                    true => skin.bad,
                                    false => skin.dim,
                                };
                                // The red outline only while the word in it is
                                // red: a red box around a dim word says the
                                // button is live and the word says it is not.
                                let kind = match ticked {
                                    true => ButtonKind::Danger,
                                    false => ButtonKind::Secondary,
                                };
                                settings_button(
                                    scene,
                                    *box2,
                                    kind,
                                    vec![Run::tinted(
                                        match armed {
                                            true => "sure?",
                                            false => "restore",
                                        },
                                        ink,
                                    )],
                                    ink,
                                    ticked
                                        && frame.hot
                                            == Some(Hit::SettingsAct(*index, Act::RestorePrompt)),
                                    skin,
                                    size,
                                );
                            }
                            Act::LoadPrompt => settings_button(
                                scene,
                                *box2,
                                ButtonKind::Secondary,
                                vec![Run::tinted("load", skin.body)],
                                skin.body,
                                frame.hot == Some(Hit::SettingsAct(*index, Act::LoadPrompt)),
                                skin,
                                size,
                            ),
                            _ => {}
                        }
                    }
                }
            }
            // A setting, a typed line and a reading are fields of a card:
            // nothing builds one as a row of the list, so the list draws none.
            _ => {}
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
                entry.map(|shown| shown.name).unwrap_or_default(),
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

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::wildcard_imports)]
    use crate::view::testkit::*;
    #[allow(clippy::wildcard_imports)]
    use crate::settings::testkit::*;
    use crate::config::Config;
    use crate::dock::{Dock, Space};
    
    use crate::settings::Row as SettingRow;
    
    
    



    /// The panel is a takeover: while it is up there are no panes, no tabs and
    /// no prompt, and it answers for every point under the title strip. The
    /// strip itself still works, so the window can be moved and closed from it.
    #[test]
    fn the_settings_panel_takes_the_whole_window() {
        let panel = a_settings_panel(&Config::default());
        let out = render_settings(&panel, 1205.0, 1600.0, None);
        let layout = &out.layout;
        assert!(layout.in_settings);
        assert!(!layout.picking, "the two takeovers are different shapes");
        for space in Space::ALL {
            assert!(layout.placed(space).tabs.is_empty(), "{space:?}");
            assert_eq!(layout.placed(space).body.w, 0.0, "{space:?}");
            assert_eq!(layout.placed(space).arrow_left.w, 0.0, "{space:?}");
            assert_eq!(layout.placed(space).arrow_right.w, 0.0, "{space:?}");
        }
        assert_eq!(layout.input.w, 0.0, "the prompt is behind the panel");
        assert_eq!(layout.cell(600.0, 400.0, 13.0, 8.0), None);

        // The whole surface under the strip, rather than a box in the middle of
        // it: sixty rows in a picker-sized box is six rows and a lot of margin.
        let box_ = layout.settings;
        assert!(box_.y >= TITLE_H, "it starts below the strip: {box_:?}");
        assert!(box_.y + box_.h <= 1600.0 && box_.x + box_.w <= 1205.0, "{box_:?}");
        assert!(box_.w >= 1205.0 - 4.0 * GAP, "not a takeover: {box_:?}");
        assert!(box_.h >= 1600.0 - TITLE_H - 4.0 * GAP, "not a takeover: {box_:?}");

        assert_eq!(
            layout.hit(box_.x + 1.0, box_.y + box_.h - 1.0),
            Some(Hit::Settings),
            "its own margin swallows a press rather than passing it on"
        );
        assert_eq!(layout.hit(400.0, 8.0), Some(Hit::TitleBar));
        let (x, y) = middle(layout.close);
        assert_eq!(layout.hit(x, y), Some(Hit::Close));
    }
    /// Every row of every section is hit where it is drawn, the control at the
    /// end of a row that can change is its own region, and a row that cannot
    /// change has none.
    #[test]
    fn every_settings_row_lands_where_it_is_drawn() {
        for section in crate::settings::SECTIONS {
            let panel = a_panel_on(&Config::default(), section);
            let out = render_settings(&panel, 1400.0, 1600.0, None);
            let layout = &out.layout;
            assert!(!layout.settings_rows.is_empty(), "{section} draws no rows");
            for (index, side, row) in &layout.settings_rows {
                assert!(
                    layout.settings_list.contains(row.x + 1.0, row.y + 1.0),
                    "row {index} of {section} is outside the list: {row:?}"
                );
                // The left of the row, which is the label, puts the cursor there.
                assert_eq!(
                    layout.hit(row.x + 2.0, row.y + row.h * 0.5),
                    Some(Hit::SettingsRow(*index, *side)),
                    "{section}"
                );
                // What a row carries: a track when its setting has a range, a
                // value when it is a flag, a preset or the endpoint, and nothing
                // at all when there is nothing to press.
                // A palette card is all controls: one cell per colour in its
                // body, each one hit where its block is drawn.
                if let Some(crate::settings::Row::Palette(palette)) = panel.row(*index) {
                    let cells: Vec<(usize, Panel)> = layout
                        .settings_cells
                        .iter()
                        .filter(|(at, ..)| at == index)
                        .map(|(_, cell, panel)| (*cell, *panel))
                        .collect();
                    assert_eq!(cells.len(), palette.cells.len(), "row {index} of {section}");
                    for (cell, at) in cells {
                        let (x, y) = middle(at);
                        assert_eq!(
                            layout.hit(x, y),
                            Some(Hit::SettingsSwatch(*index, cell)),
                            "{section}"
                        );
                        assert!(row.contains(x, y), "cell {cell} is outside its row");
                    }
                    continue;
                }
                // A choice is drawn as all of its options, so it is those
                // presses rather than one over the field: every option is hit
                // where its own box is.
                if let Some(crate::settings::Row::Setting {
                    kind: crate::settings::Kind::Choice(names),
                    ..
                }) = panel.cell(*index, *side)
                {
                    let boxes: Vec<(usize, Panel)> = layout
                        .settings_choices
                        .iter()
                        .filter(|(at, half, ..)| at == index && half == side)
                        .map(|(_, _, option, panel)| (*option, *panel))
                        .collect();
                    assert_eq!(boxes.len(), names.len(), "row {index} of {section}");
                    for (option, at) in boxes {
                        let (x, y) = middle(at);
                        assert_eq!(
                            layout.hit(x, y),
                            Some(Hit::SettingsChoice(*index, *side, option)),
                            "{section}"
                        );
                        assert!(row.contains(x, y), "option {option} is outside its row");
                    }
                    continue;
                }
                let wanted = match panel.cell(*index, *side) {
                    Some(crate::settings::Row::Setting { kind, .. })
                        if kind.fraction(0.0).is_some() =>
                    {
                        Some(Hit::SettingsSlider(*index, *side))
                    }
                    Some(crate::settings::Row::Setting { .. })
                    | Some(crate::settings::Row::Field { .. }) => {
                        Some(Hit::SettingsValue(*index, *side))
                    }
                    _ => None,
                };
                let control = layout
                    .settings_values
                    .iter()
                    .chain(layout.settings_tracks.iter())
                    .find(|(at, half, _)| at == index && half == side)
                    .map(|(_, _, panel)| *panel);
                match (wanted, control) {
                    (Some(hit), Some(control)) => {
                        let (x, y) = middle(control);
                        assert_eq!(layout.hit(x, y), Some(hit), "{section}");
                        assert!(row.contains(x, y), "the control is outside its row");
                    }
                    // The table of saved conversations is a card whose body is a
                    // list, so the middle of it is a row of that list rather
                    // than more of the panel row it stands in. The card itself
                    // still answers where its own border and header are.
                    (None, None)
                        if matches!(panel.row(*index), Some(crate::settings::Row::Table(_))) =>
                    {
                        assert_eq!(
                            layout.hit(row.x + 1.0, row.y + 1.0),
                            Some(Hit::SettingsRow(*index, *side)),
                            "{section}: the card's own edge is not the card"
                        );
                        let picks = layout
                            .settings_picks
                            .iter()
                            .filter(|(at, _, _)| at == index)
                            .count();
                        assert!(picks > 0, "{section}: the table has no rows to press");
                    }
                    // A heading, a note, a column name or a reading: the whole
                    // row is the row, and a press on its right hand end changes
                    // nothing.
                    (None, None) => assert_eq!(
                        layout.hit(row.x + row.w - 2.0, row.y + row.h * 0.5),
                        Some(Hit::SettingsRow(*index, *side)),
                        "{section}"
                    ),
                    other => panic!("row {index} of {section} carries {other:?}"),
                }
            }
            // The controls of a column all start in the same place, which is
            // what makes a screen of settings scannable rather than a wall of
            // words. Per column rather than across the panel, because a form row
            // has two of them: the right hand column lines up with itself.
            // Values with values and tracks with tracks: a slider stands a
            // column of air in from where an input box starts, on purpose.
            for want in [crate::settings::Side::Left, crate::settings::Side::Right] {
                for group in [&layout.settings_values, &layout.settings_tracks] {
                    let lefts: Vec<f32> = group
                        .iter()
                        .filter(|(_, side, _)| *side == want)
                        .map(|(_, _, p)| p.x)
                        .collect();
                    assert!(
                        lefts.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 0.01),
                        "{section}: {lefts:?}"
                    );
                }
            }
        }
    }
    /// Item H6: one bar per region and no two of them in the same pixels.
    ///
    /// The list's bar was painted at the right edge of the list, which is the
    /// right edge of every card in it. It ran through the border of every card,
    /// through the trash button at the end of every saved conversation, and
    /// through the last glyph column of every wrapped description; a rectangle
    /// cannot cover a glyph on this layer, so the letters were drawn on top of
    /// the bar. Now the cards stop short of a gutter that belongs to the bar
    /// and to nothing else.
    #[test]
    fn no_two_scrollbars_on_the_settings_panel_share_a_pixel() {
        let panels = [
            ("INSTALL", a_wordy_install_panel()),
            ("SESSIONS", a_long_sessions_panel()),
            ("SKILLS", a_wordy_servers_panel()),
            (
                "APPEARANCE",
                a_panel_on(&Config::default(), crate::settings::APPEARANCE),
            ),
        ];
        for (name, panel) in &panels {
            for (w, h) in [(1400.0, 900.0), (980.0, 620.0), (760.0, 460.0)] {
                let out = render_settings(panel, w, h, None);
                let (tracks, thumbs) = bars_of(&out);
                for (at, one) in tracks.iter().enumerate() {
                    assert!(
                        within(*one, out.layout.settings),
                        "{name} at {w}x{h}: a bar outside the panel: {one:?}"
                    );
                    for other in tracks.iter().skip(at + 1) {
                        assert!(
                            !overlap(*one, *other),
                            "{name} at {w}x{h}: {one:?} and {other:?} share pixels"
                        );
                    }
                }
                // Every thumb stands in exactly one track: a thumb outside one
                // is a bar drawn somewhere its own track is not.
                for thumb in &thumbs {
                    let held = tracks.iter().filter(|track| within(*thumb, **track)).count();
                    assert_eq!(held, 1, "{name} at {w}x{h}: {thumb:?} is in {held} tracks");
                }
                // And no bar is drawn over a card. The gutter is the list's, and
                // a card's own bar stands in the card's padding, clear of the
                // body its text is written in.
                let line = Text::line_for(PANE_TEXT.0);
                let cols = out.layout.settings_entry_columns(PANE_TEXT.1);
                for (index, _, row) in &out.layout.settings_rows {
                    let card = settings_card(*row, line);
                    let footer = matches!(
                        panel.row(*index),
                        Some(SettingRow::Entry(_) | SettingRow::Table(_))
                    );
                    let parts =
                        settings_card_parts(card, line, PANE_TEXT.0, PANE_TEXT.1, cols, footer);
                    for track in &tracks {
                        assert!(
                            !overlap(*track, parts.body),
                            "{name} at {w}x{h}: {track:?} is over the body of row {index}"
                        );
                    }
                }
            }
        }
    }
    /// The list's own bar stands in the gutter beside the cards rather than in
    /// the last four pixels of them, and it is still the bar that counts the
    /// list: what it reports is the whole section, not one card of it.
    #[test]
    fn the_list_s_bar_stands_in_a_gutter_the_cards_are_kept_out_of() {
        let panel = a_wordy_install_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let list = out.layout.settings_list;
        let cards = settings_list_rows(list);
        let gutter = settings_list_bar(list);
        assert!(cards.w < list.w, "the list reserved no gutter");
        assert!(
            (cards.x + cards.w + GAP - gutter.x).abs() < 0.01,
            "the gutter does not start a gap after the cards: {cards:?} {gutter:?}"
        );
        for (index, _, row) in &out.layout.settings_rows {
            assert!(
                row.x + row.w <= gutter.x - GAP + 0.01,
                "row {index} runs into the gutter: {row:?}"
            );
        }

        // The list is longer than the panel, so it draws a bar, and that bar is
        // in the gutter. It used to start eight pixels below the first row, on a
        // chamfer the list does not have.
        let (tracks, _) = bars_of(&out);
        let track = tracks
            .iter()
            .find(|track| within(**track, gutter))
            .unwrap_or_else(|| panic!("nothing in the gutter: {tracks:?} in {gutter:?}"));
        assert!(
            track.y - list.y <= 3.01,
            "the bar starts {} pixels below the first row",
            track.y - list.y
        );
    }
    /// A block of text scrolls inside its own card, says so with a bar of its
    /// own, and a block that is already all on screen draws none.
    ///
    /// The block had no bar at all. The nearest one belonged to the list, was
    /// drawn immediately to its right, counted the block as the rows its card
    /// claims rather than the hundreds of lines in it, and did not move when the
    /// wheel over the block did. That is the "scroll overlaps the scroll" this
    /// item exists for.
    #[test]
    fn a_block_of_text_carries_its_own_bar_and_a_short_one_carries_none() {
        let mut panel = a_wordy_install_panel();
        // The block is the second row of the section now, and what is on
        // screen is wherever the cursor is: put it on the block this test is
        // about rather than rendering whatever the install left behind.
        while panel.scroll(4, false, 8, 80) {}
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let cols = out.layout.settings_entry_columns(PANE_TEXT.1);
        let (index, _, row) = *out
            .layout
            .settings_rows
            .iter()
            .find(|(index, _, _)| {
                matches!(panel.row(*index), Some(SettingRow::Paper(paper)) if paper.title.contains("INSTALL"))
            })
            .expect("the install block is on screen");
        let card = settings_card(row, line);
        let parts = settings_card_parts(card, line, PANE_TEXT.0, PANE_TEXT.1, cols, false);
        let text = settings_paper_text(&parts, line);
        assert_eq!(
            (text.h / line).round() as usize,
            crate::settings::PAPER_LINES,
            "the block shows fewer lines than the model counted it at"
        );

        // Its bar: inside the card, down the padding right of the body, and only
        // as tall as the text, so its head is not beside a header that does not
        // scroll.
        let (tracks, thumbs) = bars_of(&out);
        let track = *tracks
            .iter()
            .find(|track| within(**track, card))
            .unwrap_or_else(|| panic!("the block at row {index} drew no bar"));
        assert!(track.x >= parts.body.x + parts.body.w, "the bar is over the text");
        assert!((track.y - text.y).abs() < 3.01, "it reaches past the text");
        let thumb = *thumbs
            .iter()
            .find(|thumb| within(**thumb, track))
            .expect("the block's bar has no thumb");
        assert!(
            thumb.h < track.h * 0.5,
            "the thumb says most of a 200 line block is on screen: {thumb:?} in {track:?}"
        );

        // The wheel over the block moves the block and leaves the panel behind
        // it exactly where it was: two scrolls, two bars.
        let was = out.layout.settings_rows[0];
        let list_track = *tracks
            .iter()
            .find(|track| within(**track, settings_list_bar(out.layout.settings_list)))
            .expect("the list drew no bar");
        let list_thumb = *thumbs
            .iter()
            .find(|thumb| within(**thumb, list_track))
            .expect("the list's bar has no thumb");
        assert!(panel.scroll_paper(index, crate::settings::PAPER_LINES, true));
        let after = render_settings(&panel, 1400.0, 900.0, None);
        assert_eq!(after.layout.settings_rows[0], was, "the list moved with it");
        let (after_tracks, after_thumbs) = bars_of(&after);
        let moved = after_thumbs
            .iter()
            .find(|thumb| within(**thumb, track))
            .expect("the block's bar went away");
        assert!(moved.y > thumb.y, "the block's thumb did not move");
        let still = after_thumbs
            .iter()
            .find(|thumb| within(**thumb, list_track))
            .expect("the list's bar went away");
        assert_eq!(still.y, list_thumb.y, "the list's thumb moved with the block");
        assert_eq!(after_tracks.len(), tracks.len(), "a bar came or went");

        // A block that fits its box draws nothing, so a bar here means there is
        // more of it.
        panel.begin_install(String::from("owner/skill"), &Config::default());
        let short = render_settings(&panel, 1400.0, 900.0, None);
        let (tracks, _) = bars_of(&short);
        let (_, row) = *short
            .layout
            .settings_rows
            .iter()
            .find(|(index, _, _)| {
                matches!(panel.row(*index), Some(SettingRow::Paper(paper)) if paper.title.contains("INSTALL"))
            })
            .map(|(index, _, row)| (*index, *row))
            .as_ref()
            .expect("the block is still there");
        let card = settings_card(row, line);
        assert!(
            !tracks.iter().any(|track| within(*track, card)),
            "a block that fits its box drew a bar: {tracks:?}"
        );
    }
    /// The table of saved conversations scrolls inside its own body, so it
    /// carries its own bar too, and the bar stops at the rows: neither the
    /// header naming the columns nor the buttons under them scroll.
    #[test]
    fn the_table_of_conversations_carries_its_own_bar_over_its_rows_only() {
        let panel = a_long_sessions_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let cols = out.layout.settings_entry_columns(PANE_TEXT.1);
        let (index, _, row) = *out
            .layout
            .settings_rows
            .iter()
            .find(|(index, _, _)| matches!(panel.row(*index), Some(SettingRow::Table(_))))
            .expect("the section carries a table");
        let card = settings_card(row, line);
        let parts = settings_card_parts(card, line, PANE_TEXT.0, PANE_TEXT.1, cols, true);
        let (names, boxes) = settings_table_parts(parts.body, line);
        let (tracks, thumbs) = bars_of(&out);
        let track = *tracks
            .iter()
            .find(|track| within(**track, card))
            .unwrap_or_else(|| panic!("the table at row {index} drew no bar"));
        assert!(track.y >= names.y + names.h - 0.01, "the bar counts the header");
        assert!(
            track.y + track.h <= parts.footer.y + 0.01,
            "the bar runs into the footer"
        );
        assert!(track.x >= parts.body.x + parts.body.w, "the bar is over the rows");
        let thumb = *thumbs
            .iter()
            .find(|thumb| within(**thumb, track))
            .expect("the table's bar has no thumb");
        // A third of the list is on screen, so the thumb is about a third of the
        // track: the bar reports the real extent.
        let want = boxes.len() as f32 / (crate::settings::TABLE_ROWS * 3) as f32;
        assert!(
            (thumb.h / track.h - want).abs() < 0.06,
            "the thumb is {} of its track and {want} of the list is on screen",
            thumb.h / track.h
        );

        // Three conversations fit in a body that holds twelve, so that table
        // draws no bar at all.
        let few = render_settings(&a_sessions_panel(), 1400.0, 900.0, None);
        let (_, row) = *few
            .layout
            .settings_rows
            .iter()
            .find(|(index, _, _)| matches!(panel.row(*index), Some(SettingRow::Table(_))))
            .map(|(index, _, row)| (*index, *row))
            .as_ref()
            .expect("the short table is placed");
        let (tracks, _) = bars_of(&few);
        assert!(
            !tracks.iter().any(|track| within(*track, settings_card(row, line))),
            "a table that fits its body drew a bar: {tracks:?}"
        );
    }
    /// A drag across the document selects the characters under the pointer, and
    /// what comes off it is what was highlighted: the glyphs, with the Markdown
    /// marks gone the same way they are gone from the screen.
    #[test]
    fn a_drag_across_the_document_selects_the_characters_under_it() {
        let panel = a_selectable_skills_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        let cols = layout.settings_doc_columns(8.0);
        assert!(
            cols > A_DRAWN_DOC_LINE.chars().count(),
            "the line has to fit on one row for the columns to be the characters"
        );

        // Two columns into the drawn first line, and six: `read`.
        let word = doc_drag(layout, &panel, (0, 2), (0, 6));
        assert_eq!(word.range().0, crate::select::Spot::new(0, 2));
        assert_eq!(word.range().1, crate::select::Spot::new(0, 6));
        assert_eq!(word.text(&panel.doc_pane()), "read");

        // The whole of that line is the rendering, not the source: no stars, no
        // backticks, and every column of it is reachable.
        let whole = doc_drag(layout, &panel, (0, 0), (0, A_DRAWN_DOC_LINE.chars().count()));
        let copied = whole.text(&panel.doc_pane());
        assert_eq!(copied, A_DRAWN_DOC_LINE);
        assert!(
            !copied.contains('*') && !copied.contains('`'),
            "a marker that is nowhere on screen came back: {copied:?}"
        );

        // Down two rows: one break per line and not one more, and nothing of the
        // last line past where the drag stopped.
        let block = doc_drag(layout, &panel, (0, 0), (2, 5));
        let copied = block.text(&panel.doc_pane());
        assert_eq!(
            copied,
            format!("{A_DRAWN_DOC_LINE}\nsecond line of the document\nthird")
        );
        assert_eq!(copied.matches('\n').count(), 2, "a break was doubled: {copied:?}");

        // A press that never moved is not a selection, so it cannot swallow the
        // next copy.
        let click =
            crate::select::Selection::new(crate::select::Where::SettingsDoc, doc_spot(layout, &panel, 1, 3));
        assert!(click.is_empty());
        assert_eq!(click.text(&panel.doc_pane()), "");
    }
    /// The band covers what is selected and nothing else: it starts at the
    /// column the drag started on and is as wide as the run.
    #[test]
    fn the_band_covers_what_the_document_drag_selected() {
        let panel = a_selectable_skills_panel();
        let layout = render_settings(&panel, 1400.0, 900.0, None).layout;
        let word = doc_drag(&layout, &panel, (0, 2), (0, 6));
        let out = render_settings_selecting(&panel, 1400.0, 900.0, word);
        let inside = out.layout.settings_doc_text;
        let line = Text::line_for(13.0);

        let bands = doc_bands(&out);
        assert_eq!(bands.len(), 1, "one run is one rectangle: {bands:?}");
        let [x, y, w, h] = bands[0];
        assert!((x - (inside.x + 2.0 * 8.0)).abs() < 0.01, "{bands:?}");
        assert!((w - 4.0 * 8.0).abs() < 0.01, "the band is not four columns: {bands:?}");
        assert!((y - inside.y).abs() < 0.01, "{bands:?}");
        assert!((h - line).abs() < 0.01, "{bands:?}");

        // Nothing highlighted paints nothing at all.
        let none = render_settings(&panel, 1400.0, 900.0, None);
        assert!(doc_bands(&none).is_empty());

        // Three lines are three rectangles, one per line, because the first and
        // the last stop partway along.
        let block = doc_drag(&layout, &panel, (0, 4), (2, 5));
        let out = render_settings_selecting(&panel, 1400.0, 900.0, block);
        let bands = doc_bands(&out);
        assert_eq!(bands.len(), 3, "{bands:?}");
        assert!((bands[0][0] - (inside.x + 4.0 * 8.0)).abs() < 0.01, "{bands:?}");
        assert!((bands[2][0] - inside.x).abs() < 0.01, "the last line starts at the left");
        assert!((bands[2][2] - 5.0 * 8.0).abs() < 0.01, "{bands:?}");
        for (step, band) in bands.iter().enumerate() {
            assert!(
                (band[1] - (inside.y + step as f32 * line)).abs() < 0.01,
                "row {step} is not where it is drawn: {bands:?}"
            );
        }
    }
    /// A selection is made of line numbers, so scrolling the column moves the
    /// band with the text and copies the same characters.
    #[test]
    fn a_document_selection_survives_a_scroll_of_the_column() {
        let mut panel = a_selectable_skills_panel();
        let mut agent = an_agent();
        agent.skills[0].doc = (0..200).map(|n| format!("line {n} of it")).collect();
        panel.adopt_agent(agent, &Config::default());
        on_the_installed_skill(&mut panel);
        let layout = render_settings(&panel, 1400.0, 900.0, None).layout;
        let cols = layout.settings_doc_columns(8.0);
        let rows = layout.settings_doc_rows(13.0);
        assert!(rows > 6, "the box has to hold the rows this drags across");

        // `line 5` on the sixth row, which is where the sixth line is drawn.
        let selection = doc_drag(&layout, &panel, (5, 0), (5, 6));
        assert_eq!(selection.text(&panel.doc_pane()), "line 5");

        // Three rows up. The same characters come off it, and the band is three
        // rows higher up the box.
        let before = doc_bands(&render_settings_selecting(&panel, 1400.0, 900.0, selection));
        assert!(panel.scroll_doc(3, true, cols, rows), "the wheel moves it");
        let after_out = render_settings_selecting(&panel, 1400.0, 900.0, selection);
        assert_eq!(
            selection.text(&panel.doc_pane()),
            "line 5",
            "the selection came to mean another line"
        );
        let after = doc_bands(&after_out);
        assert_eq!(before.len(), 1, "{before:?}");
        assert_eq!(after.len(), 1, "{after:?}");
        assert!(
            (before[0][1] - after[0][1] - 3.0 * Text::line_for(13.0)).abs() < 0.01,
            "the band did not move with the text: {before:?} then {after:?}"
        );
        assert!((before[0][0] - after[0][0]).abs() < 0.01);
        assert!((before[0][2] - after[0][2]).abs() < 0.01);

        // And the pointer over that row now lands on the line that is drawn
        // there, which is three further down the document.
        assert_eq!(doc_spot(&layout, &panel, 5, 0), crate::select::Spot::new(8, 0));
    }
    /// The pane the selection is resolved in is the text the column draws: the
    /// same lines, wrapped into the same rows, or a band would be over glyphs
    /// the clipboard does not have.
    #[test]
    fn the_document_pane_holds_what_the_column_draws() {
        let panel = a_wordy_servers_panel();
        let layout = render_settings(&panel, 1400.0, 900.0, None).layout;
        let cols = layout.settings_doc_columns(8.0);
        let rows = layout.settings_doc_rows(13.0);
        let pane = panel.doc_pane_at(cols, rows);
        assert_eq!(pane.window(rows, cols), panel.doc_window(cols, rows));
        let heights = panel.doc_heights(cols);
        assert!(
            heights.iter().any(|tall| *tall > 1),
            "no line wraps, so this proves nothing"
        );
        for (line, tall) in heights.iter().enumerate() {
            assert_eq!(pane.rows_of_line(line, cols).len(), *tall, "line {line}");
        }
    }
    /// The document is the one thing on the panel a menu can act on, and the
    /// row it offers is greyed until something is highlighted.
    #[test]
    fn the_document_offers_a_copy_on_the_right_button() {
        let panel = a_selectable_skills_panel();
        let layout = render_settings(&panel, 1400.0, 900.0, None).layout;
        let (x, y) = doc_cell(&layout, 0, 3);
        assert_eq!(layout.hit(x, y), Some(Hit::SettingsDoc));

        let empty = crate::menu::Menu::for_settings_doc((x, y), false);
        assert_eq!(empty.target, crate::menu::Target::SettingsDoc);
        assert_eq!(empty.rows.len(), 1);
        assert_eq!(empty.rows[0].item, crate::menu::Item::CopySelection);
        assert!(!empty.rows[0].enabled, "it copies nothing and says so");

        let held = crate::menu::Menu::for_settings_doc((x, y), true);
        assert_eq!(held.rows.len(), empty.rows.len(), "the menu is the same shape");
        assert!(held.rows[0].enabled);
    }
    /// Every section is on the rail, is hit where its name is drawn, and picking
    /// one swaps what is beside it.
    #[test]
    fn every_section_is_on_the_rail_and_can_be_pressed() {
        let mut panel = a_settings_panel(&Config::default());
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        assert_eq!(
            layout.settings_rail.len(),
            panel.section_names().len(),
            "the rail is short of a section"
        );
        for (index, at) in &layout.settings_rail {
            let (x, y) = middle(*at);
            assert_eq!(layout.hit(x, y), Some(Hit::SettingsSection(*index)));
            assert!(
                at.x + at.w <= layout.settings_list.x,
                "the rail runs into the list: {at:?}"
            );
        }
        // Every name is written, and the chosen one is in the accent green while
        // the rest are not.
        let text = text_of(&out.scene);
        for name in panel.section_names() {
            assert!(text.contains(name), "{name} is not on the rail: {text}");
        }
        let tint_of = |out: &Rendered, name: &str| {
            out.scene
                .texts
                .iter()
                .flat_map(|text| text.runs.iter())
                .filter(|run| run.text.trim() == name)
                .filter_map(|run| run.color)
                .next_back()
                .unwrap_or_else(|| panic!("{name} is not drawn"))
        };
        assert_eq!(tint_of(&out, "AGENT"), out.skin.heading);
        assert_ne!(tint_of(&out, "APPEARANCE"), out.skin.heading);

        // Pressing one changes what the list shows, which is the whole point of
        // a rail: the same panel, a different screen.
        let looks = layout
            .settings_rail
            .iter()
            .find(|(index, _)| panel.section_names()[*index] == crate::settings::APPEARANCE)
            .map(|(index, _)| *index)
            .expect("the appearance is on the rail");
        panel.choose(looks);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let text = text_of(&out.scene);
        assert!(text.contains("BACKGROUND COLORS"), "{text}");
        assert!(!text.contains("api keys"), "the agent section is still up");
        assert_eq!(tint_of(&out, "APPEARANCE"), out.skin.heading);
    }
    /// "the line on each menu is too long, exceeding the size of the text."
    /// The chosen section's band hugs its name: the mark, the text and a
    /// breath of padding, never the whole cell. The press region is still the
    /// full cell, so nothing got harder to hit.
    #[test]
    fn the_rail_band_hugs_the_chosen_name() {
        // AGENT, the shortest name on the rail, so the gap between the name
        // and the cell is the widest there is to prove the band lets it go.
        let panel = a_panel_on(&Config::default(), crate::settings::AGENT);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let (at, cell) = out
            .layout
            .settings_rail
            .iter()
            .find(|(index, _)| *index == panel.chosen())
            .map(|(index, cell)| (*index, *cell))
            .expect("the chosen section is on the rail");
        let name = panel.section_names()[at];
        let band = out
            .scene
            .rects
            .iter()
            .find(|rect| {
                let [x, y, ..] = rect.xywh();
                rect.rgba() == out.skin.strip
                    && (x - cell.x).abs() < 0.01
                    && (y - cell.y).abs() < 0.01
            })
            .unwrap_or_else(|| panic!("{name} has no band"));
        // Wide enough for the mark and the name, with a breath of padding, and
        // well short of the cell the old band filled.
        let column = 8.0; // what render_settings hands the frame
        let text = name.chars().count() as f32 * column;
        let [_, _, w, _] = band.xywh();
        assert!(w >= MARK_W + 3.0 + text, "the band does not cover {name}: {w}");
        assert!(
            w <= MARK_W + 3.0 + text + column * 2.0,
            "the band runs past the text: {w} for {text} of name"
        );
        assert!(w < cell.w - column, "the band still fills the cell: {w} of {}", cell.w);
        // The cell past the band is still the press (the cell's far edge
        // belongs to the rail divider's grab, which is not the band's doing).
        let past_band = cell.x + cell.w * 0.85;
        assert!(past_band > cell.x + w, "nowhere past the band to probe");
        assert_eq!(
            out.layout.hit(past_band, cell.y + cell.h * 0.5),
            Some(Hit::SettingsSection(at))
        );
    }
    /// Item F1: the rail hides no section, at the smallest window the window
    /// will open at and the largest text the settings file will carry.
    ///
    /// The names were one column that stopped at the last one that fitted, so
    /// 40 point in a 680 by 380 window drew three of the five: MCP and
    /// APPEARANCE had no box, answered no click, and APPEARANCE is the only
    /// place a font size raised that far can be lowered again. The column wraps
    /// now, so every section keeps a box it can be pressed on whatever the
    /// window and the font are doing.
    #[test]
    fn every_section_keeps_a_box_at_the_smallest_window_and_the_biggest_text() {
        let panel = a_settings_panel(&Config::default());
        let names = panel.section_names();
        let (w, h) = (crate::MIN_SIZE.width as f32, crate::MIN_SIZE.height as f32);
        for font in [PANE_TEXT, BIGGEST_TEXT] {
            let out = render_settings_at_font(&panel, w, h, font);
            let layout = &out.layout;
            assert_eq!(
                layout.settings_rail.len(),
                names.len(),
                "{font:?} lost a section"
            );
            let foot = layout.settings.y + layout.settings.h;
            for (index, at) in &layout.settings_rail {
                let (x, y) = middle(*at);
                assert_eq!(
                    layout.hit(x, y),
                    Some(Hit::SettingsSection(*index)),
                    "{} answers nothing at {font:?}",
                    names[*index]
                );
                assert!(
                    at.x + at.w <= layout.settings_list.x,
                    "{} runs into the list at {font:?}: {at:?}",
                    names[*index]
                );
                assert!(
                    at.y + at.h <= foot,
                    "{} is drawn past the bottom of the panel at {font:?}: {at:?}",
                    names[*index]
                );
            }
            // No two of them are the same box, and none of them is on top of
            // another: a box under another box is a name nothing can be aimed
            // at even though the layout carries it.
            for (index, at) in &layout.settings_rail {
                for (other, was) in &layout.settings_rail {
                    if index == other {
                        continue;
                    }
                    let apart = at.x + at.w <= was.x
                        || was.x + was.w <= at.x
                        || at.y + at.h <= was.y
                        || was.y + was.h <= at.y;
                    assert!(apart, "{at:?} and {was:?} are on top of each other");
                }
            }
            // And every one of them is written where its box is, in as much of
            // the name as the box holds. A narrow box clips, the way every name
            // in this window clips, so what is asserted is the front of the
            // name rather than the whole of it.
            let text = text_of(&out.scene);
            for name in &names {
                let front: String = name.chars().take(3).collect();
                assert!(
                    text.contains(&front),
                    "{name} is not written at {font:?}: {text}"
                );
            }
        }
    }
    /// The line between the rail and the settings is a divider like any other:
    /// it is grabbed by a band wider than the gap it stands in, and it takes
    /// nothing from either side that either side needs.
    #[test]
    fn the_settings_rail_is_grabbed_by_the_line_beside_it() {
        let panel = a_settings_panel(&Config::default());
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        let divider = layout.settings_rail_divider;
        assert!(divider.live(), "there is nothing to drag");
        assert!(divider.band.w > GAP, "the band is no wider than the gap");

        let y = layout.settings_list.y + 30.0;
        // The hairline the eye reads as the line, and both ends of the band
        // around it. The list stands its own padding in from the line.
        let drawn = layout.settings_list.x - PAD - (GAP * 0.5).floor();
        for x in [
            drawn,
            divider.band.x + 0.5,
            divider.band.x + divider.band.w - 0.5,
        ] {
            assert_eq!(layout.hit(x, y), Some(Hit::SettingsRailDivider), "at {x}");
        }

        // A name is still pressed where it is drawn, and the left hand end of a
        // row is still that row: the band reaches into the rail, where there is
        // room for it, and stops at the list, where the labels start.
        let (rx, ry) = middle(layout.settings_rail[1].1);
        assert_eq!(layout.hit(rx, ry), Some(Hit::SettingsSection(1)));
        let (index, side, row) = layout.settings_rows[0];
        assert_eq!(
            layout.hit(row.x + 2.0, row.y + row.h * 0.5),
            Some(Hit::SettingsRow(index, side))
        );

        // And it is not there when the panel is not: a band left behind by a
        // shape change is a press that lands on something nobody can see.
        let dock = Dock::new();
        let plain = Layout::compute(1400.0, 900.0, &shape(&dock, &[]));
        assert!(!plain.settings_rail_divider.live());
        assert_ne!(plain.hit(drawn, y), Some(Hit::SettingsRailDivider));
    }
    /// Dragging it puts the line under the pointer and the settings beside it
    /// move with it: the rail ends where the list begins, and nothing is drawn
    /// across the two.
    #[test]
    fn dragging_the_settings_rail_moves_the_settings_with_it() {
        let panel = a_panel_on(&Config::default(), crate::settings::APPEARANCE);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let was = out.layout.settings_list.x;
        let mut seen = Vec::new();
        for x in [200.0, 420.0, 170.0] {
            let ratio = out.layout.settings_rail_ratio_at(x);
            let moved = render_settings_at_rail(&panel, 1400.0, 900.0, None, ratio);
            let layout = &moved.layout;
            let list = layout.settings_list;
            seen.push(list.x);
            // Under the pointer rather than near it, to the pixel the width was
            // floored to.
            let drawn = list.x - PAD - GAP * 0.5;
            assert!((drawn - x).abs() <= 1.5, "{x}: the line landed at {drawn}");

            // Every name ends where the gap begins, and every row starts a
            // padding past the far side of it.
            for (index, at) in &layout.settings_rail {
                assert!(
                    (at.x + at.w + GAP + PAD - list.x).abs() <= 0.01,
                    "name {index} at {at:?} against a list at {}",
                    list.x
                );
            }
            for (index, _, row) in &layout.settings_rows {
                assert!(row.x >= list.x - 0.01, "row {index} at {row:?}");
                assert!(row.x + row.w <= list.x + list.w + 0.01, "row {index}");
            }

            // And nothing straddles the line: a text box in the panel's body is
            // either a name in the rail or a setting in the list.
            for text in &moved.scene.texts {
                let at = text.at;
                if at.y + at.h <= list.y + 0.01 || at.y >= list.y + list.h - 0.01 {
                    continue;
                }
                let in_rail = at.x + at.w <= list.x - GAP + 0.01;
                let in_list = at.x >= list.x - 0.01;
                assert!(in_rail || in_list, "{at:?} is drawn across the line at {x}");
            }
        }
        assert!(
            seen.iter().any(|at| (at - was).abs() > 1.0),
            "the drag moved nothing: {seen:?} against {was}"
        );
    }
    /// Thrown past either end it stops where the names still fit, and so does
    /// the list beside them. Neither side is ever squeezed to nothing.
    #[test]
    fn the_settings_rail_dragged_past_the_end_stops_at_the_floor() {
        let panel = a_settings_panel(&Config::default());
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let floor = out.layout.settings_rail_divider.floor;
        assert!(floor > 0.0);
        for x in [-9000.0, -1.0, 0.0, 700.0, 1401.0, 9000.0] {
            let ratio = out.layout.settings_rail_ratio_at(x);
            let moved = render_settings_at_rail(&panel, 1400.0, 900.0, None, ratio);
            let layout = &moved.layout;
            // The cells stand PAD in from the rail's left edge, so the rail
            // itself is a padding wider than its first cell.
            let rail_w = layout.settings_rail[0].1.w + PAD;
            assert!(rail_w >= floor, "{x}: the rail is {rail_w}");
            assert!(
                layout.settings_list.w >= floor,
                "{x}: the settings are {}",
                layout.settings_list.w
            );
            assert!(!layout.settings_rows.is_empty(), "{x}: the list emptied");
        }
        // A fraction out of a settings file nobody clamped is held the same way.
        for ratio in [0.0, 1.0, -5.0, 12.0] {
            let moved = render_settings_at_rail(&panel, 1400.0, 900.0, None, ratio);
            assert!(
                moved.layout.settings_rail[0].1.w + PAD >= floor,
                "{ratio}"
            );
            assert!(moved.layout.settings_list.w >= floor, "{ratio}");
        }
    }
    /// The slider is a track a pointer can be anywhere along, and where it is
    /// along it is the value that would be written.
    #[test]
    fn the_slider_reads_a_pointer_as_a_value() {
        let panel = a_panel_on(&Config::parse("opacity = 0.50"), crate::settings::APPEARANCE);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        let (index, side, track) = layout
            .settings_tracks
            .first()
            .copied()
            .expect("a number is a slider");
        // The first track of the section is the first field of its first card,
        // which is the size of the text: the sliders are fields now.
        assert!(
            matches!(panel.cell(index, side), Some(crate::settings::Row::Setting { key, .. }) if *key == "font_size")
        );

        // Both ends and the middle, off the geometry the row is drawn with.
        assert_eq!(layout.slider_at(index, side, track.x), Some(0.0));
        assert_eq!(layout.slider_at(index, side, track.x + track.w), Some(1.0));
        let half = layout
            .slider_at(index, side, track.x + track.w * 0.5)
            .expect("the middle");
        assert!((half - 0.5).abs() < 0.01, "{half}");
        // A pointer that ran off the end holds the end rather than going dead.
        assert_eq!(layout.slider_at(index, side, track.x - 500.0), Some(0.0));
        assert_eq!(
            layout.slider_at(index, side, track.x + track.w + 500.0),
            Some(1.0)
        );
        // A row with no track has no position along one, and neither has the
        // side of a card that keeps no field there.
        assert_eq!(layout.slider_at(index + 500, side, track.x), None);
        let (alone, half, box_) = layout
            .settings_tracks
            .iter()
            .copied()
            .find(|(at, half, _)| {
                half.step(true)
                    .is_none_or(|next| panel.cell(*at, next).is_none())
            })
            .expect("a card of one field");
        let past = half.step(true).unwrap_or(crate::settings::Side::RightBelow);
        assert_eq!(layout.slider_at(alone, past, box_.x), None);

        // The track is drawn where it is pressed: an unlit bar the width of the
        // track and a lit one as far along it as the value.
        // Shorter than the line it stands in, or the card's own focus border
        // answers first: it is drawn in the accent too, and the accent is what
        // a lit track is.
        let on_the_track = |rgba: [f32; 4]| {
            out.scene
                .rects
                .iter()
                .filter(|rect| rect.rgba() == rgba)
                .map(|rect| rect.xywh())
                .find(|[x, y, _, h]| track.contains(*x + 0.5, *y + 0.5) && *h < track.h * 0.5)
        };
        let thumb = on_the_track(out.skin.gauge).expect("nothing is lit");
        assert!((thumb[0] - track.x).abs() < 0.01, "{thumb:?}");
        let full = on_the_track(out.skin.gauge_track).expect("there is no track");
        assert!((full[2] - track.w).abs() < 0.01, "{full:?}");
        // Where the value sits in its range, which for the text size the
        // window opens at is a fifth along.
        let at = (14.0 - 8.0) / (40.0 - 8.0);
        assert!(
            (thumb[2] / full[2] - at).abs() < 0.02,
            "the lit part is {thumb:?} of {full:?}"
        );
    }
    /// The mark that closes it is reachable, and clear of the corner the panel's
    /// own cut takes away.
    #[test]
    fn the_close_mark_clears_the_cut_corner() {
        let panel = a_settings_panel(&Config::default());
        for (w, h) in [(1400.0, 900.0), (700.0, 460.0), (2200.0, 1400.0)] {
            let out = render_settings(&panel, w, h, None);
            let layout = &out.layout;
            let (close, box_) = (layout.settings_close, layout.settings);
            let (x, y) = middle(close);
            assert_eq!(layout.hit(x, y), Some(Hit::SettingsClose), "{w}x{h}");
            assert!(
                close.x + close.w <= box_.x + box_.w - cut_of(box_),
                "the mark is drawn in the cut: {close:?} in {box_:?}"
            );
            // The mark stands on the panel, not on a block of its own: nothing
            // smaller than the panel is drawn behind it, whether or not the
            // pointer is on it.
            let mark = |hot: Option<Hit>| {
                let out = render_settings(&panel, w, h, hot);
                let close = out.layout.settings_close;
                let box_ = out.layout.settings;
                for rect in &out.scene.rects {
                    let [rx, ry, rw, rh] = rect.xywh();
                    let overlaps = rx < close.x + close.w
                        && rx + rw > close.x
                        && ry < close.y + close.h
                        && ry + rh > close.y;
                    // The panel's own surface and outline are the surface the
                    // mark is written on; anything smaller is a block.
                    let panel_itself = rw >= box_.w - 0.01 && rh >= box_.h - 0.01;
                    assert!(
                        !overlaps || panel_itself,
                        "{rect:?} is a block behind the close mark at {w}x{h}, hot {hot:?}"
                    );
                }
                // In the panel's own mark, not the window's: the title strip
                // draws the same glyph and is on screen behind a takeover.
                out.scene
                    .texts
                    .iter()
                    .filter(|text| close.contains(text.at.x + 1.0, text.at.y + 1.0))
                    .flat_map(|text| text.runs.iter())
                    .find(|run| run.icon && run.text == icons::CLOSE.to_string())
                    .and_then(|run| run.color)
                    .unwrap_or_else(|| panic!("no close mark at {w}x{h}"))
            };
            // What answers the pointer is the mark, in the colour this window
            // uses for losing something. A close with no answer at all would be
            // worse than the block it replaced.
            assert_eq!(mark(None), out.skin.bright, "{w}x{h}");
            assert_eq!(
                mark(Some(Hit::SettingsClose)),
                out.skin.bad,
                "the close mark does not answer the pointer at {w}x{h}"
            );
            assert_ne!(out.skin.bright, out.skin.bad);
        }
    }
    /// A card is drawn in exactly the room the model counted for it, and
    /// everything it draws is inside itself.
    ///
    /// This is the invariant the whole panel stands on: the model measures rows
    /// in whole lines and the layout draws them in pixels, so a card measured at
    /// one height and drawn at another puts every press below it on another
    /// card.
    #[test]
    fn a_card_measures_the_height_it_draws() {
        let panel = a_panel_on(&Config::default(), crate::settings::MCP);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let cols = out.layout.settings_entry_columns(PANE_TEXT.1);
        let (index, row) = the_card_row(&out, &panel);
        let card = match panel.row(index) {
            Some(crate::settings::Row::Card(card)) => card,
            other => panic!("row {index} is {other:?}"),
        };
        let counted = crate::settings::lines(
            panel.row(index).expect("the row"),
            cols,
        );
        assert_eq!(
            counted,
            design::card_row_lines(
                crate::settings::card_body_lines(card, cols),
                card.does.is_some()
            )
        );
        assert!(
            (row.h - counted as f32 * line).abs() < 0.01,
            "the row is {row:?} and the model counted {counted} lines of {line}"
        );

        // The card itself, and the space under it that keeps two cards apart.
        let (box_, parts) = the_card(&out, row, false);
        assert!(
            (row.h - box_.h - design::apart(line)).abs() < 0.01,
            "the space under the card is not APART: {box_:?} in {row:?}"
        );
        // Its body sits ROOM inside the border on every side, and its last
        // field and its hint are inside it.
        assert!(parts.body.x >= box_.x + design::room(line) - 0.01);
        assert!(parts.body.y >= parts.rule.y + design::room(line) - 0.01);
        assert!(parts.body.y + parts.body.h <= box_.y + box_.h - design::room(line) + 0.01);
        for text in out.scene.texts.iter().filter(|text| {
            text.at.y >= box_.y - 0.01 && text.at.y < box_.y + box_.h && text.at.x >= box_.x
        }) {
            assert!(
                text.at.y + text.at.h <= box_.y + box_.h + 0.01,
                "a line of the card is drawn out of the bottom of it: {:?} in {box_:?}",
                text.at
            );
            assert!(
                text.at.x + text.at.w <= box_.x + box_.w + 0.01,
                "a line of the card runs out of its right edge: {:?}",
                text.at
            );
        }
        // And the press at the very bottom of the row is still that row, which
        // is what a card measured short takes away.
        let (x, _) = middle(row);
        assert_eq!(
            out.layout.hit(x, row.y + row.h - 1.0),
            Some(Hit::SettingsRow(index, crate::settings::Side::Left))
        );
    }
    /// A field is a label with its value under it, never beside it, and the
    /// press inside a card lands on what is under the pointer rather than on
    /// whatever row happens to be nearest.
    ///
    /// "all text looks the same name, description, repo": a label and a value on
    /// one line read as one sentence, and every value on the panel looked like
    /// part of its own key.
    #[test]
    fn a_field_is_its_label_over_its_value_and_a_press_in_a_card_lands_in_it() {
        let panel = a_panel_on(&Config::default(), crate::settings::MCP);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let cols = out.layout.settings_entry_columns(PANE_TEXT.1);
        let (index, row) = the_card_row(&out, &panel);
        let card = match panel.row(index) {
            Some(crate::settings::Row::Card(card)) => card,
            other => panic!("row {index} is {other:?}"),
        };
        let (_, parts) = the_card(&out, row, false);
        let across = design::across(card.fields.len(), design::card_cols(cols));
        let slots = settings_card_slots(
            parts.body,
            line,
            &crate::settings::card_hints(card),
            across,
            card.group.as_ref().map(|group| group.at),
        );
        for (field, slot) in card.fields.iter().zip(&slots) {
            let (label_at, input_at) = settings_field_boxes(*slot, line);
            assert_eq!(
                line_of(&out, label_at.x, label_at.y),
                field.label,
                "the label is not on its own line"
            );
            assert!(
                input_at.y >= label_at.y + label_at.h - 0.01,
                "the value is beside its label rather than under it"
            );
            assert!(
                (input_at.y - label_at.y - line - design::tight(line)).abs() < 0.01,
                "the gap between a label and its value is not TIGHT"
            );
            assert_eq!(
                line_of(&out, input_at.x, input_at.y),
                field.value(),
                "the value is not under its own label"
            );
            // A reading has no border and no fill, which is the whole of what
            // says it cannot be typed into; a field that can be typed into
            // wears the input box.
            if field.editable() {
                continue;
            }
            for rgba in [out.skin.input, out.skin.edge] {
                assert!(
                    !covered(&out, input_at, input_at.h, rgba),
                    "a reading is drawn as a box that can be typed into"
                );
            }
            let (x, y) = middle(input_at);
            assert_eq!(
                out.layout.hit(x, y),
                Some(Hit::SettingsRow(index, crate::settings::Side::Left)),
                "a press inside the card answers for another row"
            );
        }
        // Only what can be typed into claims a press region: one per
        // editable field, none for a reading.
        let editable = card
            .fields
            .iter()
            .filter(|field| field.editable())
            .count();
        assert_eq!(
            out.layout
                .settings_values
                .iter()
                .filter(|(at, _, _)| *at == index)
                .count(),
            editable,
            "the card's press regions do not match its editable fields"
        );

        // And the field that can be typed into is the same shape with the box
        // round it, in the section that has one.
        let out = render_settings(
            &a_panel_on(&Config::default(), crate::settings::AGENT),
            1400.0,
            900.0,
            None,
        );
        let (index, side, at) = *out
            .layout
            .settings_values
            .first()
            .expect("a value that can be changed");
        let (x, y) = middle(at);
        assert_eq!(out.layout.hit(x, y), Some(Hit::SettingsValue(index, side)));
        assert!(covered(&out, at, at.h, out.skin.input), "no box round it");
    }
    /// The two fields of a card go side by side while the card is wide enough
    /// for both to keep their columns, and stack when it is not.
    ///
    /// "aware flexible design on resize as well". Cards stay full width and it
    /// is their contents that answer a narrow window, so nothing is ever laid
    /// out for one width.
    #[test]
    fn a_card_puts_two_fields_across_until_it_is_too_narrow_for_both() {
        let panel = a_panel_on(&Config::default(), crate::settings::MCP);
        let line = Text::line_for(PANE_TEXT.0);
        let flip = design::reflow_columns();

        let wide = render_settings(&panel, 1400.0, 900.0, None);
        let (index, wide_row) = the_card_row(&wide, &panel);
        let wide_cols = wide.layout.settings_entry_columns(PANE_TEXT.1);
        assert!(wide_cols >= flip, "{wide_cols} columns is not wide enough");

        let narrow = render_settings(&panel, 470.0, 900.0, None);
        let (_, narrow_row) = the_card_row(&narrow, &panel);
        let narrow_cols = narrow.layout.settings_entry_columns(PANE_TEXT.1);
        assert!(narrow_cols < flip, "{narrow_cols} columns is still wide");

        let card = match panel.row(index) {
            Some(crate::settings::Row::Card(card)) => card,
            other => panic!("row {index} is {other:?}"),
        };
        assert_eq!(card.fields.len(), 2, "the card has two fields to reflow");
        let places = |out: &Rendered, row: Panel, cols: usize| -> Vec<Panel> {
            let (_, parts) = the_card(out, row, false);
            let across = design::across(card.fields.len(), design::card_cols(cols));
            settings_card_slots(
                parts.body,
                line,
                &crate::settings::card_hints(card),
                across,
                card.group.as_ref().map(|group| group.at),
            )
        };
        let side_by_side = places(&wide, wide_row, wide_cols);
        assert_eq!(side_by_side[0].y, side_by_side[1].y, "not on one band");
        assert!(
            side_by_side[1].x >= side_by_side[0].x + side_by_side[0].w,
            "the two fields overlap: {side_by_side:?}"
        );
        let stacked = places(&narrow, narrow_row, narrow_cols);
        assert_eq!(stacked[0].x, stacked[1].x, "not in one column");
        assert!(
            stacked[1].y >= stacked[0].y + stacked[0].h,
            "the two fields overlap: {stacked:?}"
        );

        // Both labels are really drawn where the reflow put them, and the card
        // grew by the band it gained: the model counted the same flip the layout
        // drew, or every press under the card would be a row out.
        for (out, slots) in [(&wide, &side_by_side), (&narrow, &stacked)] {
            for (field, slot) in card.fields.iter().zip(slots) {
                assert_eq!(line_of(out, slot.x, slot.y), field.label);
            }
        }
        assert!(
            narrow_row.h > wide_row.h,
            "the stacked card is not taller: {narrow_row:?} against {wide_row:?}"
        );
    }
    /// The panel's own title is drawn in the panel title role and a card's title
    /// in the smaller one under it, so what you are looking at and what group it
    /// is in are two different sizes.
    ///
    /// "title totally unclear on each panel section increase font size on each".
    /// Everything on the panel was one size, so nothing on it was a heading.
    #[test]
    fn the_panel_title_is_larger_than_a_card_title_which_is_larger_than_a_value() {
        let panel = a_panel_on(&Config::default(), crate::settings::MCP);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let size = PANE_TEXT.0;
        let title = out
            .scene
            .texts
            .iter()
            .find(|text| text.runs.iter().any(|run| run.text.contains("SETTINGS")))
            .expect("the panel says what it is");
        assert_eq!(title.size, design::panel_title_size(size));

        let (_, row) = the_card_row(&out, &panel);
        let (_, parts) = the_card(&out, row, true);
        let card_title = out
            .scene
            .texts
            .iter()
            .find(|text| (text.at.y - parts.title.y).abs() < 0.51)
            .expect("the card says what it holds");
        assert_eq!(card_title.size, design::card_title_size(size));
        assert!(
            card_title
                .runs
                .iter()
                .any(|run| run.text.contains("ADD A SERVER")),
            "{:?}",
            card_title.runs
        );
        assert!(title.size > card_title.size, "the two headings are one size");
        assert!(card_title.size > size, "a card title is the size of its fields");

        // And the whole scale really is on screen: the hint under the fields is
        // smaller than the value over it.
        let hint = out
            .scene
            .texts
            .iter()
            .filter(|text| text.at.y > parts.body.y)
            .map(|text| text.size)
            .fold(f32::INFINITY, f32::min);
        assert_eq!(hint, design::hint_size(size));
        assert!(hint < size, "a hint is not quieter than a value");
    }
    /// Every button a card carries stays inside its own card, at every size the
    /// panel is used at.
    ///
    /// The pane text goes to forty points and the window goes down to nothing,
    /// and a card cut off by the bottom of the list has less room than it asked
    /// for. A button drawn above its own card would be a press that answers for
    /// the card over it.
    #[test]
    fn a_card_keeps_its_buttons_inside_itself_at_every_size() {
        for (w, h) in [(1400.0, 900.0), (700.0, 420.0), (420.0, 260.0), (300.0, 180.0)] {
            for font in [PANE_TEXT, BIGGEST_TEXT] {
                for section in [crate::settings::SKILLS, crate::settings::MCP] {
                    let mut panel = a_wrapping_skills_panel();
                    let at = panel
                        .section_names()
                        .iter()
                        .position(|name| *name == section)
                        .expect("a section");
                    panel.choose(at);
                    let out = render_settings_at_font(&panel, w, h, font);
                    let line = Text::line_for(font.0);
                    for (index, box_) in out
                        .layout
                        .settings_toggles
                        .iter()
                        .chain(out.layout.settings_removes.iter())
                    {
                        let row = out
                            .layout
                            .settings_rows
                            .iter()
                            .find(|(at, _, _)| at == index)
                            .map(|(_, _, row)| *row)
                            .expect("the row the button stands in");
                        let card = settings_card(row, line);
                        assert!(
                            box_.y >= card.y - 0.01
                                && box_.y + box_.h <= card.y + card.h + 0.01
                                && box_.x >= card.x - 0.01
                                && box_.x + box_.w <= card.x + card.w + 0.01,
                            "{section} at {w}x{h}, {font:?}: {box_:?} is outside {card:?}"
                        );
                        assert!(
                            out.layout.settings_list.contains(box_.x + 1.0, box_.y + 1.0),
                            "{section} at {w}x{h}, {font:?}: {box_:?} is outside the list"
                        );
                    }
                }
            }
        }
    }
    /// Every group's title is the same green the showing tab's line is, is
    /// drawn larger than the settings under it, and is given the room for that
    /// by the row it was laid out in.
    ///
    /// A list is unreadable if its groups do not separate from their contents.
    /// This was written about the bare headings the palette stood under: those
    /// are gone, and the same assertions are made about the card titles that
    /// replaced them, since a title measured at one height and drawn at another
    /// would put every click below it on the wrong row.
    #[test]
    fn the_settings_card_titles_are_the_heading_accent_and_the_size_they_were_measured_at() {
        let mut found = 0;
        let mut panel = a_panel_on(&Config::default(), crate::settings::APPEARANCE);
        // A section of cards is taller than any window, so each title is looked
        // for on the screenful its own card is on rather than on the first one.
        // Tall on purpose: the last card here is ten swatches, and what this
        // test is about is the title's tint and the row it was measured in.
        let shape = render_settings(&panel, 1400.0, 1800.0, None);
        let rows = shape.layout.settings_capacity(13.0);
        let cols = shape.layout.settings_entry_columns(PANE_TEXT.1);
        for heading in [
            "DEFAULT THEMES",
            "THE WINDOW'S OWN TONES",
            "THE CODE COLOURS",
            "THE TOOL MARKS",
            "THE METERS",
        ] {
            let at = panel
                .rows()
                .iter()
                .position(|row| match row {
                    crate::settings::Row::Palette(palette) => palette.title == heading,
                    crate::settings::Row::Card(card) => card.title == heading,
                    _ => false,
                })
                .unwrap_or_else(|| panic!("{heading} is not a group of the section"));
            // From the top of the section every time, so where one heading was
            // found does not decide where the next one is looked for.
            while panel.scroll(4, false, rows, cols) {}
            while panel.first() < at && panel.scroll(1, true, rows, cols) {}
            let out = render_settings(&panel, 1400.0, 1800.0, None);
            let text = out
                .scene
                .texts
                .iter()
                .find(|text| text.runs.iter().any(|run| run.text.trim() == heading))
                .unwrap_or_else(|| panic!("{heading} is not on the panel"));
            let run = text
                .runs
                .iter()
                .find(|run| run.text.trim() == heading)
                .expect("the run that was just found");
            assert_eq!(run.color, Some(out.skin.heading), "{heading}");
            // Larger than the rows under it, and inside the row it was measured
            // into: 13pt is what everything else on the panel is drawn at.
            assert!(text.size > 13.0, "{heading} is drawn at {}", text.size);
            let row = out
                .layout
                .settings_rows
                .iter()
                .find(|(_, _, row)| row.contains(text.at.x + 1.0, text.at.y + 1.0))
                .map(|(index, _, row)| (*index, *row))
                .unwrap_or_else(|| panic!("{heading} is drawn on no row at all"));
            let named = match panel.row(row.0) {
                Some(crate::settings::Row::Palette(palette)) => palette.title == heading,
                Some(crate::settings::Row::Card(card)) => card.title == heading,
                _ => false,
            };
            assert!(
                named,
                "{heading} is drawn on row {}, which is {:?}",
                row.0,
                panel.row(row.0)
            );
            assert!(
                text.at.y + noob_draw::Text::line_for(text.size) <= row.1.y + row.1.h + 0.01,
                "{heading} is taller than the row it was laid out in: {:?} in {:?}",
                text.at,
                row.1
            );
            found += 1;
        }
        assert_eq!(found, 5);
        // Not the tint a field's value is written in, or a title is another
        // line of the card.
        let skin = shape.skin;
        assert_ne!(skin.heading, skin.body);
        assert_ne!(skin.heading, skin.title);
    }
    /// No row has a hairline under it any more, and a card carries a border
    /// and exactly one divider instead.
    ///
    /// There was a line under every row on the panel. A line between every two
    /// things on screen says nothing about which of them belong together, which
    /// is the whole of "lines everywhere, unclear what each thing is". Grouping
    /// and space say it now: a card is a bordered box, its fields have space
    /// between them, and two cards have more.
    #[test]
    fn no_row_is_ruled_off_and_a_card_is_bordered_instead() {
        let out = render_settings(
            &a_panel_on(&Config::default(), crate::settings::APPEARANCE),
            1400.0,
            900.0,
            None,
        );
        let rows = &out.layout.settings_rows;
        assert!(rows.len() > 3, "not enough rows to prove it: {}", rows.len());
        for (index, _, row) in rows {
            let ruled = out.scene.rects.iter().any(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == out.skin.edge
                    && h <= 1.01
                    && (x - row.x).abs() < 0.01
                    && (w - row.w).abs() < 0.01
                    && (y - (row.y + row.h - 1.0)).abs() < 0.01
            });
            assert!(!ruled, "row {index} still has a hairline under it");
        }

        // And the container that replaced it: the card's own border, cut on the
        // same corner every surface in this window is cut on, with the one
        // divider under its title and nothing else drawn between its fields.
        let panel = a_panel_on(&Config::default(), crate::settings::MCP);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let (_, row) = the_card_row(&out, &panel);
        let (card, parts) = the_card(&out, row, true);
        // The cursor opens on this card, so its border may wear the focus
        // colour; either ink, the container is a border and not a rule.
        let border = out
            .scene
            .rects
            .iter()
            .find(|rect| {
                let [x, y, w, h] = rect.xywh();
                (rect.rgba() == out.skin.edge || rect.rgba() == out.skin.edge_focus)
                    && (x - card.x).abs() < 0.01
                    && (y - card.y).abs() < 0.01
                    && (w - card.w).abs() < 0.01
                    && (h - card.h).abs() < 0.01
            })
            .expect("the card has no border");
        assert!(border.extra()[1] > 0.0, "the border is filled, not stroked");
        assert_eq!(
            (border.extra()[2] as u32) & noob_draw::Rect::TOP_RIGHT,
            noob_draw::Rect::TOP_RIGHT,
            "the card is not cut on the window's own corner"
        );
        let hairlines: Vec<[f32; 4]> = out
            .scene
            .rects
            .iter()
            .filter(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == out.skin.edge
                    && h <= 1.01
                    && w > card.w * 0.5
                    && x >= card.x - 0.01
                    && y >= card.y
                    && y <= card.y + card.h
            })
            .map(|rect| rect.xywh())
            .collect();
        assert_eq!(
            hairlines.len(),
            1,
            "a card gets one divider and no more: {hairlines:?}"
        );
        assert!(
            (hairlines[0][1] - parts.rule.y).abs() < 1.01,
            "the divider is not under the title: {:?} against {:?}",
            hairlines[0],
            parts.rule
        );
        // Under the title and above the first field, which is what makes it a
        // header rather than a line through the card.
        assert!(parts.rule.y > parts.title.y + parts.title.h - 0.01);
        assert!(parts.rule.y < parts.body.y);
    }
    /// Anything that can be typed into or pressed to change is drawn as a box
    /// with an outline round it. Without one an editable row looked exactly like
    /// a reading, and the only way to tell one from the other was to press it.
    ///
    /// APPEARANCE was on this list for the theme, which was one box holding one
    /// word. It is every theme drawn as its own box now, which is asserted at
    /// the end: the section carries no plain value box left.
    #[test]
    fn an_editable_row_is_drawn_as_a_box_with_an_edge() {
        // The endpoint, which is the one row on the panel that is typed into.
        for section in [crate::settings::AGENT] {
            let panel = a_panel_on(&Config::default(), section);
            let out = render_settings(&panel, 1400.0, 900.0, None);
            let boxes = &out.layout.settings_values;
            assert!(!boxes.is_empty(), "{section} has no control on it");
            for (index, _, at) in boxes {
                let over = |rect: &noob_draw::Rect| {
                    let [x, y, w, h] = rect.xywh();
                    (x - at.x).abs() < 0.01
                        && (y - at.y).abs() < 0.01
                        && (w - at.w).abs() < 0.01
                        && (h - at.h).abs() < 0.01
                };
                assert!(
                    out.scene
                        .rects
                        .iter()
                        .any(|rect| over(rect) && rect.extra()[3] >= 1.0),
                    "the control on row {index} of {section} has no outline"
                );
                assert!(
                    out.scene
                        .rects
                        .iter()
                        .any(|rect| over(rect) && rect.rgba() == out.skin.input),
                    "the control on row {index} of {section} has no box under it"
                );
            }
        }

        // Every option of the theme, each one its own box: the one that is set
        // is filled the way a primary button is, and the rest carry the outline
        // a secondary does.
        let panel = a_panel_on(&Config::default(), crate::settings::APPEARANCE);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let options = &out.layout.settings_choices;
        // The three presets on the left column and custom on the right.
        assert_eq!(options.len(), crate::config::THEMES.len() + 1, "{options:?}");
        for (index, side, option, at) in options {
            let over = |rect: &&noob_draw::Rect| {
                let [x, y, w, h] = rect.xywh();
                (x - at.x).abs() < 0.01
                    && (y - at.y).abs() < 0.01
                    && (w - at.w).abs() < 0.01
                    && (h - at.h).abs() < 0.01
            };
            let name = option_name(*side, *option);
            let set = matches!(
                panel.cell(*index, *side),
                Some(crate::settings::Row::Setting { value, .. })
                    if value == name
            );
            let wanted = match set {
                true => out.skin.button,
                false => out.skin.input,
            };
            assert!(
                out.scene.rects.iter().filter(over).any(|rect| rect.rgba() == wanted),
                "option {option} is not drawn as the box it is"
            );
        }
        // And the value box the theme used to be is gone with it.
        assert!(
            out.layout.settings_values.is_empty(),
            "{:?} is still a one word control",
            out.layout.settings_values
        );

        // The endpoint's text sits inside its box rather than on the stroke, and
        // so does the caret while it is being typed into.
        let mut panel = a_panel_on(&Config::default(), crate::settings::AGENT);
        while !matches!(
            panel.at_cursor(),
            Some(crate::settings::Row::Field { .. })
        ) {
            assert!(panel.step(true), "the agent section has no field on it");
        }
        assert!(panel.edit());
        assert!(panel.type_text("http://localhost:9/v1"));
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let (_, _, at) = out
            .layout
            .settings_values
            .iter()
            .find(|(index, side, _)| *index == panel.cursor() && *side == panel.side())
            .expect("the endpoint's box");
        let caret = out
            .scene
            .rects
            .iter()
            .filter(|rect| rect.rgba() == out.skin.caret && rect.extra()[3] == 0.0)
            .find(|rect| {
                let [x, y, w, _] = rect.xywh();
                w <= 3.0 && (y - at.y).abs() < 0.01 && x >= at.x && x <= at.x + at.w
            })
            .unwrap_or_else(|| panic!("no caret inside {at:?} while typing"));
        let [x, _, w, _] = caret.xywh();
        assert!(
            x > at.x && x + w <= at.x + at.w,
            "the caret is on the border: {caret:?} in {at:?}"
        );
    }
    /// The cut corner belongs to the control, not to the pointer.
    ///
    /// The fill and the outline of every one of these boxes carried the
    /// diagonal and the fill drawn over them under the pointer did not, so the
    /// theme button squared its corner off the moment the pointer arrived and
    /// painted into the cut its own outline draws. Every box drawn that way had
    /// it: the theme and the flags, the endpoint field, a skill's on/off, its
    /// uninstall, and a session's delete.
    #[test]
    fn a_control_under_the_pointer_keeps_its_cut_corner() {
        // Every kind of control the panel has, in the section that carries it.
        let sections = [
            crate::settings::APPEARANCE,
            crate::settings::AGENT,
            crate::settings::MCP,
            crate::settings::SESSIONS,
        ];
        let (mut values, mut toggles, mut removes) = (0, 0, 0);
        for section in sections {
            let panel = a_panel_on(&Config::default(), section);
            let plain = render_settings(&panel, 1400.0, 900.0, None);
            let controls: Vec<(Panel, Hit)> = plain
                .layout
                .settings_values
                .iter()
                .map(|(index, side, at)| (*at, Hit::SettingsValue(*index, *side)))
                .chain(
                    plain
                        .layout
                        .settings_toggles
                        .iter()
                        .map(|(index, at)| (*at, Hit::SettingsToggle(*index))),
                )
                .chain(
                    plain
                        .layout
                        .settings_removes
                        .iter()
                        .map(|(index, at)| (*at, Hit::SettingsRemove(*index))),
                )
                .chain(
                    plain
                        .layout
                        .settings_acts
                        .iter()
                        .map(|(index, act, at)| (*at, Hit::SettingsAct(*index, *act))),
                )
                .chain(plain.layout.settings_choices.iter().map(
                    |(index, side, option, at)| {
                        (*at, Hit::SettingsChoice(*index, *side, *option))
                    },
                ))
                .collect();
            assert!(!controls.is_empty(), "{section} has no control on it");
            for (at, hit) in controls {
                let out = render_settings(&panel, 1400.0, 900.0, Some(hit));
                let over = |rect: &&noob_draw::Rect| {
                    let [x, y, w, h] = rect.xywh();
                    (x - at.x).abs() < 0.01
                        && (y - at.y).abs() < 0.01
                        && (w - at.w).abs() < 0.01
                        && (h - at.h).abs() < 0.01
                };
                // A control is a box of some kind: the input fill a field and
                // a value wear, the accent a primary button is filled with, or
                // the outline a secondary and a danger button carry.
                let boxed = [
                    out.skin.input,
                    out.skin.button,
                    out.skin.edge,
                    out.skin.close_hot,
                ];
                // Off the cold render: a primary button under the pointer is
                // drawn in the hot accent instead of its own, so its idle fill
                // is only there while nothing is over it.
                let base = plain
                    .scene
                    .rects
                    .iter()
                    .find(|rect| over(rect) && boxed.contains(&rect.rgba()))
                    .unwrap_or_else(|| panic!("{section}: {hit:?} has no box under it"));
                // And each kind lights with its own hot colour: a filled button
                // in the brighter accent, everything else in the window's own.
                let lit = out
                    .scene
                    .rects
                    .iter()
                    .find(|rect| {
                        over(rect)
                            && (rect.rgba() == out.skin.hot || rect.rgba() == out.skin.button_hot)
                    })
                    .unwrap_or_else(|| panic!("{section}: {hit:?} does not light up"));
                assert!(
                    lit.extra()[1] > 0.0,
                    "{section}: {hit:?} lights up as a square: {lit:?}"
                );
                assert_eq!(
                    lit.extra()[1..3],
                    base.extra()[1..3],
                    "{section}: {hit:?} is not the shape of the box under it"
                );
                assert_eq!(
                    (lit.extra()[2] as u32) & noob_draw::Rect::TOP_RIGHT,
                    noob_draw::Rect::TOP_RIGHT,
                    "{section}: {hit:?} cuts a corner other than the top right"
                );
                match hit {
                    Hit::SettingsValue(..) | Hit::SettingsChoice(..) => values += 1,
                    Hit::SettingsToggle(_) => toggles += 1,
                    _ => removes += 1,
                }
            }
        }
        // One of each kind at the least, or the pass proves it about one box.
        assert!(values > 0 && toggles > 0 && removes > 0, "{values} {toggles} {removes}");
    }
    /// Nothing the panel draws leaves it, at any size. A rectangle outside a
    /// takeover is a rectangle over the desktop.
    #[test]
    fn nothing_the_settings_panel_draws_escapes_it() {
        let panel = a_settings_panel(&Config::default());
        for (w, h) in [(1400.0, 900.0), (680.0, 380.0), (2200.0, 1400.0)] {
            let out = render_settings(&panel, w, h, Some(Hit::SettingsValue(7, Side::Left)));
            let box_ = out.layout.settings;
            let inside = |x: f32, y: f32, rw: f32, rh: f32| {
                x >= box_.x - 0.01
                    && y >= box_.y - 0.01
                    && x + rw <= box_.x + box_.w + 0.01
                    && y + rh <= box_.y + box_.h + 0.01
            };
            for rect in &out.scene.rects {
                let [x, y, rw, rh] = rect.xywh();
                // The backdrop and the title strip are the window's, not the
                // panel's; everything else here belongs to the panel.
                let backdrop = rw >= w - 0.01 && rh >= h - 0.01;
                assert!(
                    backdrop || y + rh <= TITLE_H + 0.01 || inside(x, y, rw, rh),
                    "{rect:?} escapes the panel at {w}x{h}"
                );
            }
            for text in &out.scene.texts {
                let at = text.at;
                assert!(
                    at.y + at.h <= TITLE_H + 0.01 || inside(at.x, at.y, at.w, at.h),
                    "{at:?} escapes the panel at {w}x{h}"
                );
            }
            assert!(out.scene.over_rects.is_empty(), "nothing floats over a takeover");
        }
    }
    /// The footer's buttons are inside the card, at the bottom of it, and in the
    /// three kinds this window has and no others.
    ///
    /// "buttons, default bottom always, they are messy": they were pinned to the
    /// right of whichever line of a row they happened to belong to, so no two of
    /// them were at the same height and each one sat wherever its own row left
    /// space.
    #[test]
    fn the_buttons_of_a_card_stand_in_its_footer() {
        let panel = a_wordy_servers_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let (index, row) = the_entry_row(&out, &panel);
        let (card, parts) = the_card(&out, row, true);
        let toggle = out
            .layout
            .settings_toggles
            .iter()
            .find(|(at, _)| *at == index)
            .map(|(_, at)| *at)
            .expect("a toggle");
        let remove = out
            .layout
            .settings_removes
            .iter()
            .find(|(at, _)| *at == index)
            .map(|(_, at)| *at)
            .expect("an uninstall");
        for (name, at) in [("the toggle", toggle), ("the uninstall", remove)] {
            assert!(
                at.y >= parts.footer.y - 0.01 && at.y + at.h <= card.y + card.h - 0.01,
                "{name} is not in the footer: {at:?} against {:?}",
                parts.footer
            );
            assert!(
                at.y + at.h >= parts.body.y + parts.body.h - 0.01,
                "{name} is not under the body: {at:?}"
            );
            assert!(
                at.x >= card.x && at.x + at.w <= card.x + card.w + 0.01,
                "{name} is outside the card: {at:?} in {card:?}"
            );
            // And pressed where it is drawn.
            let (x, y) = middle(at);
            assert!(card.contains(x, y), "{name} is drawn off its own card");
        }
        assert!(
            toggle.x + toggle.w <= remove.x,
            "the two buttons overlap: {toggle:?} and {remove:?}"
        );
        // Room under the last line of the body, so the buttons are not standing
        // on the words.
        assert!(
            parts.footer.y >= parts.body.y + parts.body.h + design::room(line) - 0.01,
            "the footer sits on the body: {:?} under {:?}",
            parts.footer,
            parts.body
        );

        // The primary is filled, the danger is outlined in the bad colour, and
        // neither of them is the other.
        let over = |at: Panel, rgba: [f32; 4]| {
            out.scene.rects.iter().any(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == rgba
                    && (x - at.x).abs() < 0.01
                    && (y - at.y).abs() < 0.01
                    && (w - at.w).abs() < 0.01
                    && (h - at.h).abs() < 0.01
            })
        };
        assert!(
            over(toggle, out.skin.button),
            "the skill is on, so its toggle is the filled button"
        );
        assert!(
            over(remove, out.skin.close_hot),
            "the uninstall is not outlined in the colour this window loses work in"
        );
        assert!(!over(remove, out.skin.button), "the uninstall is filled like a primary");
        assert!(text_of(&out.scene).contains("uninstall"));
    }
    /// The footer says what the keys will do to the row under the cursor, and
    /// says a refused write instead when there is one. A panel that writes a
    /// file has to say when the file said no.
    #[test]
    fn the_footer_carries_the_keys_and_then_the_trouble() {
        let config = Config::default();
        let mut panel = a_settings_panel(&config);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        assert!(text_of(&out.scene).contains(&panel.says()), "{}", panel.says());

        panel.say_trouble(String::from("cannot write it"));
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let text = text_of(&out.scene);
        assert!(text.contains("cannot write it"), "{text}");
        assert!(!text.contains(panel.hint()), "the trouble and the keys share a row");
        let said = out
            .scene
            .texts
            .iter()
            .flat_map(|t| t.runs.iter())
            .find(|run| run.text.contains("cannot write it"))
            .expect("the trouble is drawn");
        assert_eq!(said.color, Some(out.skin.bad), "trouble is not marked as trouble");
    }
    /// A slider is a bare track: no input box behind it at rest, and neither
    /// rollover nor the cursor adds one rectangle to it.
    ///
    /// The flat rows kept this rule already; the card fields did not. A card
    /// slider stood in the filled, edged, cut-cornered box a typed value
    /// wears, and the track lit under the pointer: an input's costume on a
    /// control that is dragged, and a rollover effect nobody asked for.
    #[test]
    fn a_card_slider_is_a_bare_track_that_rollover_does_not_change() {
        let panel = a_panel_on(&Config::default(), crate::settings::APPEARANCE);
        let out = render_settings(&panel, 1400.0, 1000.0, None);
        assert!(!out.layout.settings_tracks.is_empty(), "no slider to look at");
        let holds = |out: &Rendered, track: Panel, rgba: [f32; 4]| {
            let (cx, cy) = middle(track);
            out.scene.rects.iter().any(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == rgba && cx >= x && cx <= x + w && cy >= y && cy <= y + h
            })
        };
        for (_, _, track) in &out.layout.settings_tracks {
            // The track is drawn, and nothing boxes it: not the input's fill,
            // not a hover band.
            assert!(holds(&out, *track, out.skin.gauge_track), "no track at {track:?}");
            for rgba in [out.skin.input, out.skin.hot] {
                assert!(
                    !holds(&out, *track, rgba),
                    "a slider stands in a box: {rgba:?} behind {track:?}"
                );
            }
        }
        // Pointing at the track, or at the value beside it, changes not one
        // rectangle of the scene.
        let (index, side, _) = out.layout.settings_tracks[0];
        let resting: Vec<([f32; 4], [f32; 4])> = out
            .scene
            .rects
            .iter()
            .map(|rect| (rect.xywh(), rect.rgba()))
            .collect();
        for hot in [
            Hit::SettingsSlider(index, side),
            Hit::SettingsValue(index, side),
        ] {
            let lit = render_settings(&panel, 1400.0, 1000.0, Some(hot));
            let now: Vec<([f32; 4], [f32; 4])> = lit
                .scene
                .rects
                .iter()
                .map(|rect| (rect.xywh(), rect.rgba()))
                .collect();
            assert_eq!(now, resting, "{hot:?} changed the slider's look");
        }
    }
    /// The column beside the list is a card of its own, and the text in it
    /// wraps. Every line of it used to be cut to the width of the column and
    /// ended in an ellipsis, which is the left edge of a document rather than a
    /// document.
    ///
    /// This asserted a bare line of text over a filled, outlined box. That is
    /// the shape everything on this panel was taken out of: the column is a
    /// card now, with the skill's name in its header at the card title size,
    /// one divider under it and the text in its body.
    #[test]
    fn the_document_is_a_card_whose_header_names_the_skill() {
        let panel = a_wordy_servers_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        let line = Text::line_for(13.0);
        let doc = layout.settings_doc;
        assert!(doc.w >= 1.0, "there is no second column");

        let box_ = settings_doc_box(doc, line);
        let parts = settings_doc_parts(box_, line, PANE_TEXT.0);
        assert!(box_.y >= doc.y - 0.01, "the card is not the whole column");
        assert!(box_.y + box_.h <= doc.y + doc.h + 0.01);

        // The name in the header, at the card title size, the way an entry's own
        // card carries its name.
        let title = out
            .scene
            .texts
            .iter()
            .find(|text| {
                (text.at.x - parts.title.x).abs() < 0.51
                    && (text.at.y - parts.title.y).abs() < 0.51
            })
            .expect("the header");
        assert_eq!(
            title.runs.iter().map(|run| run.text.as_str()).collect::<String>(),
            "docs"
        );
        assert_eq!(title.size, design::card_title_size(PANE_TEXT.0));

        // The border, stroked and cut like every other card, and the one
        // divider under the header.
        let border = out
            .scene
            .rects
            .iter()
            .find(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == out.skin.edge
                    && (x - box_.x).abs() < 0.01
                    && (y - box_.y).abs() < 0.01
                    && (w - box_.w).abs() < 0.01
                    && (h - box_.h).abs() < 0.01
            })
            .expect("the document card has no border");
        assert!(border.extra()[1] > 0.0, "the border is filled, not stroked");
        assert!(
            out.scene.rects.iter().any(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == out.skin.edge
                    && h <= 1.01
                    && (x - parts.rule.x).abs() < 0.01
                    && (w - parts.rule.w).abs() < 0.01
                    && (y - parts.rule.y).abs() < 1.01
            }),
            "no divider under the document's header"
        );

        // The text is in the body, off the border on every side.
        let inside = layout.settings_doc_text;
        assert_eq!((inside.x, inside.y), (parts.body.x, parts.body.y));
        assert!(inside.x >= box_.x + design::room(line) - 0.01);
        assert!(inside.y >= parts.rule.y + design::room(line) - 0.01);
        assert!(inside.x + inside.w <= box_.x + box_.w + 0.01);
        assert!(inside.y + inside.h <= box_.y + box_.h - design::room(line) + 0.01);
        assert!(inside.x > layout.settings_list.x);

        // The text wraps at the columns the box holds, by the same rule the
        // panes wrap at, and the long line is written whole.
        let cols = layout.settings_doc_columns(8.0);
        assert!(cols > 0);
        assert!(
            A_LONG_DOC_LINE.chars().count() > cols,
            "the line fits, so this proves nothing"
        );
        let text = out
            .scene
            .texts
            .iter()
            .find(|text| {
                (text.at.x - inside.x).abs() < 0.01 && (text.at.y - inside.y).abs() < 0.01
            })
            .expect("the document");
        assert_eq!(text.wrap_cols, Some(cols));
        assert_eq!(text.wrap_break, text_geometry::Break::Word);
        let written: String = text.runs.iter().map(|run| run.text.as_str()).collect();
        assert!(
            written.contains(A_LONG_DOC_LINE),
            "the line is not written whole: {written:?}"
        );
        assert!(!written.contains('\u{2026}'), "it is still clipped: {written:?}");
    }
    /// Both columns scroll, each in its own box: the document is drawn from
    /// wherever it was scrolled to, in rows of the box it is drawn in, and the
    /// list stays where it was.
    #[test]
    fn the_document_scrolls_inside_its_own_box() {
        let mut panel = a_wordy_servers_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let cols = out.layout.settings_doc_columns(8.0);
        let rows = out.layout.settings_doc_rows(13.0);
        assert!(rows > 1, "the box holds more than a line");

        // A document longer than any window, scrolled past its first screenful.
        let mut agent = an_agent_with_servers();
        agent.mcp.servers[0].entry = (0..200)
            .map(|n| format!("line {n} of it"))
            .collect::<Vec<String>>()
            .join("\n");
        panel.adopt_agent(agent, &Config::default());
        let before = render_settings(&panel, 1400.0, 900.0, None);
        let first_row = before.layout.settings_rows[0];
        assert!(panel.scroll_doc(3, true, cols, rows), "the wheel moves it");
        let after = render_settings(&panel, 1400.0, 900.0, None);
        let inside = after.layout.settings_doc_text;
        let written: String = after
            .scene
            .texts
            .iter()
            .filter(|text| {
                (text.at.x - inside.x).abs() < 0.01 && (text.at.y - inside.y).abs() < 0.01
            })
            .flat_map(|text| text.runs.iter())
            .map(|run| run.text.as_str())
            .collect();
        assert!(written.contains("line 3 of it"), "{written:?}");
        assert!(!written.contains("line 0 of it"), "{written:?}");
        // The list did not move with it: the two columns are two scrolls.
        assert_eq!(after.layout.settings_rows[0], first_row);

        // And the bar that says how far down it is: inside the card, down its
        // right padding, and beside the body rather than beside the header,
        // which is not part of what scrolls.
        let line = Text::line_for(PANE_TEXT.0);
        let box_ = settings_doc_box(after.layout.settings_doc, line);
        let parts = settings_doc_parts(box_, line, PANE_TEXT.0);
        let track = after
            .scene
            .rects
            .iter()
            .find(|rect| rect.rgba() == after.skin.scroll_track)
            .map(|rect| rect.xywh())
            .expect("the document has no bar");
        assert!(track[0] > inside.x + inside.w, "the bar is over the text");
        assert!(track[0] + track[2] <= box_.x + box_.w, "outside the card");
        assert!(track[1] >= parts.body.y, "the bar reaches up past the header");
    }
    /// A window too narrow to hold both columns is one column: the entries win,
    /// because a document forty characters wide is a column of broken words.
    #[test]
    fn a_narrow_panel_keeps_the_entries_and_drops_the_column() {
        let panel = a_panel_on(&Config::default(), crate::settings::SKILLS);
        let out = render_settings(&panel, 520.0, 400.0, None);
        assert!(out.layout.settings_doc.w < 1.0, "{:?}", out.layout.settings_doc);
        assert!(!out.layout.settings_rows.is_empty(), "and the list is still there");
    }
    /// The toggle and the uninstall on an entry are pressed where they are
    /// drawn, and neither of them is the row: a press on the name still puts
    /// the cursor there rather than deleting a skill.
    #[test]
    fn an_entry_carries_a_toggle_and_an_uninstall_of_its_own() {
        let panel = a_wordy_servers_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        let entries: Vec<usize> = layout
            .settings_rows
            .iter()
            .filter(|(index, _, _)| {
                matches!(panel.row(*index), Some(crate::settings::Row::Entry(_)))
            })
            .map(|(index, _, _)| *index)
            .collect();
        assert_eq!(entries.len(), 2, "the servers are not rows of their own");
        let index = entries[0];
        let row = layout
            .settings_rows
            .iter()
            .find(|(at, _, _)| *at == index)
            .map(|(_, _, row)| *row)
            .expect("the row");

        let toggle = layout
            .settings_toggles
            .iter()
            .find(|(at, _)| *at == index)
            .map(|(_, at)| *at)
            .expect("a toggle");
        let remove = layout
            .settings_removes
            .iter()
            .find(|(at, _)| *at == index)
            .map(|(_, at)| *at)
            .expect("an uninstall");
        for (at, hit) in [
            (toggle, Hit::SettingsToggle(index)),
            (remove, Hit::SettingsRemove(index)),
        ] {
            let (x, y) = middle(at);
            assert_eq!(layout.hit(x, y), Some(hit), "{at:?}");
            assert!(row.contains(x, y), "{at:?} is outside its row {row:?}");
        }
        assert!(
            toggle.x + toggle.w <= remove.x,
            "the two controls overlap: {toggle:?} and {remove:?}"
        );
        // The row itself is still the row.
        assert_eq!(
            layout.hit(row.x + 2.0, row.y + 2.0),
            Some(Hit::SettingsRow(index, crate::settings::Side::Left))
        );
        let text = text_of(&out.scene);
        assert!(text.contains("uninstall"), "{text}");
        assert!(text.contains("on"), "{text}");

        // Nothing else on the panel grows one: a setting is not an entry.
        let plain = render_settings(
            &a_panel_on(&Config::default(), crate::settings::APPEARANCE),
            1400.0,
            900.0,
            None,
        );
        assert!(plain.layout.settings_toggles.is_empty());
        assert!(plain.layout.settings_removes.is_empty());
    }
    /// An entry is a card: its name in the header, what it is for in the body,
    /// where it came from under that, and its buttons in the footer.
    ///
    /// The name and the description used to share one line, with the
    /// description cut off wherever the buttons at the end of the row began.
    /// Then they were three bare lines with the buttons still beside the name.
    /// Now the three strings are three roles in three places and the buttons
    /// have a strip of their own.
    #[test]
    fn an_entry_is_a_card_with_its_name_its_words_and_its_path_in_it() {
        let panel = a_wordy_servers_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let (index, row) = the_entry_row(&out, &panel);
        let (card, parts) = the_card(&out, row, true);
        assert!(
            card.h <= row.h - design::apart(line) + 0.01,
            "the card fills the space between itself and the next one"
        );

        // The name in the header, at the card title size rather than the size
        // everything under it is drawn at.
        let title = out
            .scene
            .texts
            .iter()
            .find(|text| (text.at.y - parts.title.y).abs() < 0.51)
            .expect("the title");
        assert_eq!(
            title.runs.iter().map(|run| run.text.as_str()).collect::<String>(),
            "docs"
        );
        assert_eq!(title.size, design::card_title_size(PANE_TEXT.0));
        assert!(title.size > PANE_TEXT.0, "the title is not bigger than a value");

        // What it is for, in the body, at the value size.
        assert_eq!(line_of(&out, parts.body.x, parts.body.y), A_LONG_ABOUT);
        // And where it came from under it, in the hint size: the quietest of
        // the three, and the one that used to look exactly like the other two.
        let wrapped = crate::settings::about_rows(
            A_LONG_ABOUT,
            design::card_cols(settings_entry_cols(row.w, PANE_TEXT.1)),
        );
        let under = out
            .scene
            .texts
            .iter()
            .find(|text| {
                (text.at.x - parts.body.x).abs() < 0.51
                    && (text.at.y
                        - (parts.body.y + wrapped as f32 * line + design::tight(line)))
                    .abs()
                        < 0.51
            })
            .expect("the path");
        assert_eq!(
            under.runs.iter().map(|run| run.text.as_str()).collect::<String>(),
            "global: /home/hec/.config/noob/mcp.json"
        );
        assert_eq!(under.size, design::hint_size(PANE_TEXT.0));
        assert!(under.size < PANE_TEXT.0, "the path is not quieter than the words");

        // The buttons are in the footer, at the bottom of the card, and the
        // description runs the full width of the body under them rather than
        // stopping where they begin.
        let toggle = out
            .layout
            .settings_toggles
            .iter()
            .find(|(at, _)| *at == index)
            .map(|(_, at)| *at)
            .expect("a toggle");
        assert!(toggle.y >= parts.body.y + parts.body.h - 0.01, "{toggle:?}");
        let beside_the_name = ((toggle.x - 8.0 - parts.body.x) / 8.0).floor() as usize;
        assert!(
            A_LONG_ABOUT.chars().count() > beside_the_name,
            "the description would have fitted beside the buttons, so this proves nothing"
        );
        // And nothing an entry says is drawn over the document beside it.
        for text in out.scene.texts.iter().filter(|text| {
            (text.at.x - parts.body.x).abs() < 0.51 && text.at.y >= card.y && text.at.y <= card.y + card.h
        }) {
            assert!(
                text.at.x + text.at.w <= out.layout.settings_doc.x + 0.01,
                "a line of the entry runs into the document: {:?}",
                text.at
            );
        }
    }
    /// A description too long for the card wraps onto as many rows as it needs
    /// and the card grows to hold them.
    ///
    /// Moving it off the name's line stopped the buttons cutting it; the column
    /// itself was still cutting it, so a skill whose description ran past the
    /// width of the list ended in three dots with the rest unreadable. It is
    /// broken by the rule the panes and the document column use, in the columns
    /// the model counted its height in, which are the card's own and not the
    /// list's.
    #[test]
    fn a_long_description_wraps_instead_of_ending_in_an_ellipsis() {
        let panel = a_wrapping_skills_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let (_, row) = the_entry_rows(&out, &panel)[0];
        let cols = design::card_cols(settings_entry_cols(row.w, PANE_TEXT.1));
        assert!(
            A_WRAPPING_ABOUT.chars().count() > cols,
            "the description fits in {cols} columns, so this proves nothing"
        );
        let (_, parts) = the_card(&out, row, true);
        let drawn = out
            .scene
            .texts
            .iter()
            .find(|text| {
                (text.at.x - parts.body.x).abs() < 0.51 && (text.at.y - parts.body.y).abs() < 0.51
            })
            .expect("the description");
        let said: String = drawn.runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(said, A_WRAPPING_ABOUT, "the description was cut");
        assert!(!said.contains('\u{2026}'), "it still ends in an ellipsis");
        assert_eq!(
            drawn.wrap_cols,
            Some(cols),
            "it wraps in the columns its height was counted in"
        );
        // As many rows as it wrapped to, and the card is that body, its header
        // and its footer.
        let wrapped = crate::settings::about_rows(A_WRAPPING_ABOUT, cols);
        assert!(wrapped > 1, "{wrapped} rows in {cols} columns");
        assert!(
            drawn.at.h >= wrapped as f32 * line - 0.01,
            "the box holds {} of {wrapped} rows: {:?}",
            drawn.at.h / line,
            drawn.at
        );
        assert!(
            parts.body.h >= (wrapped as f32 + 1.0) * line - 0.01,
            "the body did not grow: {:?} at {line}",
            parts.body
        );
        assert_eq!(
            row.h,
            crate::design::card_row_lines(
                crate::settings::entry_body_lines(
                    match panel.row(the_entry_rows(&out, &panel)[0].0) {
                        Some(crate::settings::Row::Entry(entry)) => entry,
                        other => panic!("{other:?}"),
                    },
                    settings_entry_cols(row.w, PANE_TEXT.1)
                ),
                true
            ) as f32
                * line,
            "the row is not the height the model counted"
        );
    }
    /// And the card under it starts below those rows rather than over them.
    #[test]
    fn the_entry_under_a_wrapped_one_is_drawn_below_its_rows() {
        let panel = a_wrapping_skills_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let rows = the_entry_rows(&out, &panel);
        let cols = design::card_cols(settings_entry_cols(rows[0].1.w, PANE_TEXT.1));
        let wrapped = crate::settings::about_rows(A_WRAPPING_ABOUT, cols);
        assert_eq!(rows.len(), 2, "two servers are two cards");
        let ((_, first), (_, second)) = (rows[0], rows[1]);
        assert!(
            second.y >= first.y + first.h - 0.01,
            "the rows overlap: {first:?} then {second:?}"
        );
        let (first_card, first_parts) = the_card(&out, first, true);
        let (_, second_parts) = the_card(&out, second, true);
        // The path sits under the last row of the description, not on top of it.
        assert_eq!(
            line_of(
                &out,
                first_parts.body.x,
                first_parts.body.y + wrapped as f32 * line + design::tight(line)
            ),
            "global: /home/hec/.config/noob/mcp.json"
        );
        // And the next name is under the whole of the first card, with the
        // space between two cards left between them.
        assert_eq!(
            line_of(&out, second_parts.title.x, second_parts.title.y),
            "shell"
        );
        assert!(
            second.y >= first_card.y + first_card.h + design::apart(line) - 1.01,
            "the second card is drawn over the first: {second:?} under {first_card:?}"
        );
    }
    /// A press on the entry after a wrapped one lands on that entry.
    ///
    /// The height of a row is read by the layout and by the scroll window
    /// alike, so a description counted at one row and drawn as four would put
    /// every press below it on its neighbour.
    #[test]
    fn a_press_below_a_wrapped_entry_lands_on_the_entry_under_it() {
        let panel = a_wrapping_skills_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let rows = the_entry_rows(&out, &panel);
        assert_eq!(rows.len(), 2, "two servers are two rows");
        let named = |index: usize| match panel.row(index) {
            Some(crate::settings::Row::Entry(entry)) => entry.name.clone(),
            other => panic!("row {index} is {other:?}"),
        };
        assert_eq!(named(rows[0].0), "docs");
        assert_eq!(named(rows[1].0), "shell");
        for (index, row) in rows {
            let (x, y) = middle(row);
            assert_eq!(
                out.layout.hit(x, y),
                Some(Hit::SettingsRow(index, crate::settings::Side::Left)),
                "the middle of {} answers for another row",
                named(index)
            );
            // Its own last line as well, which is the press a row measured
            // short hands to the entry below it.
            assert_eq!(
                out.layout.hit(x, row.y + row.h - 1.0),
                Some(Hit::SettingsRow(index, crate::settings::Side::Left)),
                "the last line of {} answers for another row",
                named(index)
            );
        }
    }
    /// The uninstall is a button with a word in it, and both of them fit: it
    /// ended exactly on the edge of the column, three pixels from the document's
    /// own border, in a box a column and a half wider than the word it holds, so
    /// the shaper's bounds took the last letter off it.
    #[test]
    fn the_uninstall_sits_inside_the_row_with_room_for_its_word() {
        let panel = a_wordy_servers_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let (index, row) = the_entry_row(&out, &panel);
        let remove = out
            .layout
            .settings_removes
            .iter()
            .find(|(at, _)| *at == index)
            .map(|(_, at)| *at)
            .expect("an uninstall");
        let (card, _) = the_card(&out, row, true);
        assert!(
            remove.x + remove.w
                <= card.x + card.w - design::room(Text::line_for(PANE_TEXT.0)) + 0.01,
            "the uninstall is against the edge of the card: {remove:?} in {card:?}"
        );
        assert!(
            remove.x + remove.w <= out.layout.settings_doc.x,
            "it reaches into the document: {remove:?}"
        );
        // The word inside it is written in the box the button leaves after its
        // own padding, and that box holds every letter of it.
        let word = "uninstall";
        let room = remove.w - INPUT_PAD * 2.0;
        assert!(
            room >= word.chars().count() as f32 * 8.0,
            "{word} needs {} pixels and has {room}",
            word.chars().count() as f32 * 8.0
        );
        assert!(text_of(&out.scene).contains(word));
    }
    /// The card the cursor is on carries the focus colour on its own border and
    /// the mark down its edge.
    ///
    /// It used to wear a solid band across the row. A row was one line tall then;
    /// a card is nine, and a filled block that tall is a highlight nobody can
    /// read through. The border says the same thing and leaves the words alone.
    #[test]
    fn the_entry_under_the_cursor_is_the_card_with_the_focus_border() {
        let mut panel = a_wordy_servers_panel();
        // The section opens on the install form, so the cursor is walked down
        // to the entry this test is about.
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let (index, _) = the_entry_row(&out, &panel);
        panel.point_at(index, crate::settings::Side::Left);
        assert_eq!(panel.cursor(), index, "the entry cannot hold the cursor");
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let (index, row) = the_entry_row(&out, &panel);
        let (card, _) = the_card(&out, row, true);
        assert_eq!(panel.cursor(), index, "the cursor is not on the entry");
        assert!(
            covered(&out, card, card.h, out.skin.edge_focus),
            "the card is not outlined in the focus colour: {card:?}"
        );
        assert!(
            covered(&out, Panel::new(card.x, card.y, MARK_W, card.h), card.h, out.skin.edge_focus),
            "no mark down the edge of the card the keys are on"
        );
        assert!(
            !covered(&out, row, row.h, out.skin.picked),
            "the band across the whole card is back"
        );
        // And the card that is not under the cursor keeps the quiet border, so
        // the two are told apart.
        let panel = a_wrapping_skills_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let rows = the_entry_rows(&out, &panel);
        let (quiet, _) = the_card(&out, rows[1].1, true);
        assert!(
            covered(&out, quiet, quiet.h, out.skin.edge),
            "the card the keys are not on is outlined in the focus colour too"
        );
    }
    /// The table is used at every width the window has, and at each of them its
    /// rows stay inside its card and under the names of their columns.
    ///
    /// A card is full width and stacks, so what a narrow window costs the table
    /// is the width of its last column and, below the width the three buttons
    /// need, the buttons themselves. Nothing is ever drawn outside the card it
    /// belongs to, which is what a press on a row of it depends on.
    #[test]
    fn the_table_holds_its_rows_inside_its_card_at_every_width() {
        let panel = a_sessions_panel();
        let index = panel
            .rows()
            .iter()
            .position(|row| matches!(row, crate::settings::Row::Table(_)))
            .expect("the section carries a table");
        for (w, h) in [(1400.0, 900.0), (900.0, 700.0), (700.0, 460.0), (520.0, 400.0)] {
            let out = render_settings(&panel, w, h, None);
            let line = Text::line_for(PANE_TEXT.0);
            let Some(row) = out
                .layout
                .settings_rows
                .iter()
                .find(|(at, _, _)| *at == index)
                .map(|(_, _, at)| *at)
            else {
                continue;
            };
            let card = settings_card(row, line);
            // The bottom bound is the visible row box rather than the card
            // arithmetic: a card cut by the bottom of the list shows its rows
            // down to the cut itself, and the layout hands over exactly the
            // visible part.
            for (_, on, at) in out
                .layout
                .settings_picks
                .iter()
                .filter(|(at, _, _)| *at == index)
            {
                assert!(
                    at.x >= card.x - 0.01
                        && at.x + at.w <= card.x + card.w + 0.01
                        && at.y >= card.y - 0.01
                        && at.y + at.h <= row.y + row.h + 0.01,
                    "{w}x{h}: row {on} is outside its card: {at:?} in {card:?}"
                );
                let (x, y) = middle(*at);
                assert_eq!(
                    out.layout.hit(x, y),
                    Some(Hit::SettingsPick(index, *on)),
                    "{w}x{h}: row {on} is not pressed where it is drawn"
                );
            }
            for (_, act, at) in out
                .layout
                .settings_acts
                .iter()
                .filter(|(at, _, _)| *at == index)
            {
                let (x, y) = middle(*at);
                assert!(card.contains(x, y), "{w}x{h}: {act:?} is outside its card");
                assert_eq!(out.layout.hit(x, y), Some(Hit::SettingsAct(index, *act)));
            }
        }
    }
}
