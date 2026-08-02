//! Where everything on the settings panel sits: the rail, cards, tables,
//! sliders, swatch grids, and the doc viewer, all from one panel and the
//! shape. Placement and paint read the same rectangles the hit tests do.

use noob_draw::{Panel, Run, Scene, Text};

#[allow(unused_imports)]
use crate::design;
use crate::style::skin::Skin;
use crate::settings::Act;
use crate::settings::{Row as SettingRow, Settings, Side};
#[allow(clippy::wildcard_imports)]
use crate::view::*;


/// Where the settings panel's pieces are. A struct rather than a tuple: six
/// panels in a row is a call site nobody can read, and four of them are lists.
///
/// The whole area rather than a centred box: this is a takeover, and a list in a
/// box the size of the picker's would be six rows of content and a lot of
/// margin. Two columns inside it: the rail of section names down the left, and
/// the chosen section beside it. The value column is a fixed number of columns in
/// from the right so every value lines up, which is what makes a screen of
/// settings scannable rather than a wall of words.
pub(crate) struct SettingsPlaces {
    pub(crate) box_: Panel,
    pub(crate) rail: Vec<(usize, Panel)>,
    pub(crate) divider: Divider,
    pub(crate) list: Panel,
    /// Every row, and every half of a form row: the box it is drawn in and
    /// clicked in. A row that is not a form is one entry on the left.
    pub(crate) rows: Vec<(usize, Side, Panel)>,
    pub(crate) values: Vec<(usize, Side, Panel)>,
    pub(crate) tracks: Vec<(usize, Side, Panel)>,
    pub(crate) cells: Vec<(usize, usize, Panel)>,
    /// One box per option of a choice, so every option can be pressed.
    pub(crate) choices: Vec<(usize, Side, usize, Panel)>,
    pub(crate) toggles: Vec<(usize, Panel)>,
    pub(crate) removes: Vec<(usize, Panel)>,
    /// The rows of the saved-conversations table that are inside its body, the
    /// mark in front of each of them, and the buttons under them.
    pub(crate) picks: Vec<(usize, usize, Panel)>,
    pub(crate) marks: Vec<(usize, usize, Panel)>,
    pub(crate) acts: Vec<(usize, Act, Panel)>,
    pub(crate) doc: Panel,
    /// The box the document itself is written in: inside the outlined wrapper,
    /// under the title over it. What the wheel counts a page in and what the
    /// wrapping is measured in, so the rows counted and the rows drawn are one
    /// number.
    pub(crate) doc_text: Panel,
    pub(crate) close: Panel,
}

/// Where every cell of the saved-conversations table sits inside one row.
///
/// One answer for the row that names the columns and for every row under it, so
/// a cell cannot drift out from under its own header. One box per entry of
/// the table's own columns, in that order, so the name of a column,
/// the cell under it and the alignment they share are all read off the same
/// index. The widths come from the model; the one written as zero there is the
/// first message, which takes whatever is left of the row.
///
/// From the row's own left edge, because the table stands inside a card: the
/// card's border and the mark down it say which row of the panel the keys are
/// on, and the strip an ordinary row leaves for that mark would push the first
/// column out from under its name.
///
/// Cell by cell rather than one space padded string, which is what the picker's
/// list does: this table has a column that is a mark and a line between every
/// pair of columns, and both of those need each cell's own x.
pub(crate) fn settings_session_cells(
    row: Panel,
    column: f32,
    columns: &[(&str, usize, crate::settings::Align)],
) -> Vec<Panel> {
    let right = row.x + row.w;
    let mut out = Vec::new();
    let mut at = row.x;
    for (_, wide, _) in columns {
        let wide = *wide;
        let room = (right - at).max(0.0);
        let want = match wide {
            0 => room,
            wide => wide as f32 * column,
        };
        let w = want.min(room);
        out.push(Panel::new(at, row.y, w, row.h));
        at += w;
    }
    out
}

/// Where the column names and the rows of a table sit inside the card's body.
///
/// The names on the first line, the rows [`design::TIGHT`] under them, one line
/// each, exactly as many as the body was measured for
/// ([`crate::settings::table_body_lines`]). Asked by the placement and by the
/// drawing, the way [`settings_card_parts`] is, so a row is drawn where the
/// press on it is tested for.
pub(crate) fn settings_table_parts(body: Panel, line: f32) -> (Panel, Vec<Panel>) {
    let names = Panel::new(body.x, body.y, body.w, line);
    let top = body.y + line + design::tight(line);
    // As many rows as the body holds: a stretched table fills what it was
    // given. The cap is a backstop, far above any window.
    let rows = (0..512)
        .map(|step| Panel::new(body.x, top + step as f32 * line, body.w, line))
        .take_while(|at| at.y + at.h <= body.y + body.h + 0.01)
        .collect();
    (names, rows)
}

/// Where the text of a block sits inside the card's body: under the line saying
/// where it came from, [`design::TIGHT`] below it, in whole lines.
///
/// Whole lines because it is what the counter in the header and the bar down
/// the right padding both count in, and a box half a line taller than the last
/// line drawn would have both of them saying there is a line on screen that is
/// not. Never more than [`crate::settings::PAPER_LINES`], which is what
/// [`crate::settings::paper_body_lines`] measured the card at; fewer only when
/// the bottom of the list cut the card off.
pub(crate) fn settings_paper_text(parts: &CardParts, line: f32) -> Panel {
    let top = parts.body.y + line + design::tight(line);
    let room = (parts.body.y + parts.body.h - top).max(0.0);
    let held = ((room / line).floor() as usize).min(crate::settings::PAPER_LINES);
    Panel::new(parts.body.x, top, parts.body.w, held as f32 * line)
}

/// How wide each button in the table's footer is, in columns of pane text.
///
/// Sized for the longest word each of them can hold rather than for the word it
/// is holding now, so arming the delete does not resize the thing that was just
/// pressed and the second press lands on the same box.
const SETTING_ACT_COLUMNS: usize = 13;

/// The buttons in the table's footer: select all, select none, and the
/// delete, one group at the footer's right end, where every card's own
/// action already sits, so the panel's buttons hang on one edge instead of
/// floating in the middle of it. Nothing at all in a footer too narrow to
/// hold the three of them side by side, since a button drawn over another
/// one is a press nobody can aim.
pub(crate) fn settings_act_boxes(
    footer: Panel,
    line: f32,
    column: f32,
    of: crate::settings::TableOf,
) -> Vec<(Act, Panel)> {
    let acts: &[Act] = match of {
        crate::settings::TableOf::Sessions => &[Act::All, Act::None, Act::Forget],
        // No marks on the skills table, so nothing to select: the two buttons
        // act on the row the keys are on.
        crate::settings::TableOf::Skills => &[Act::Turn, Act::Uninstall],
    };
    let wide = SETTING_ACT_COLUMNS as f32 * column;
    let step = design::step(line);
    let whole = wide * acts.len() as f32 + step * (acts.len() - 1) as f32;
    if footer.w < whole || footer.h < 1.0 {
        return Vec::new();
    }
    let x = footer.x + footer.w - whole;
    acts.iter()
        .enumerate()
        .map(|(step_at, act)| {
            (
                *act,
                Panel::new(x + step_at as f32 * (wide + step), footer.y, wide, footer.h),
            )
        })
        .collect()
}

/// How wide a card's own action is, in columns of pane text.
///
/// Sized for its word and the padding on both sides of it, the way the two
/// buttons on an entry are: a button whose width follows what it is holding
/// moves under the pointer that is pressing it.
const SETTING_DOING_COLUMNS: usize = 11;

/// The one button in a card's own footer: at the bottom right, where a card's
/// own action belongs.
///
/// Nothing at all in a footer too narrow for it, since a button drawn over the
/// sentence beside it is a press nobody can aim.
pub(crate) fn settings_doing_box(footer: Panel, column: f32) -> Option<Panel> {
    let wide = SETTING_DOING_COLUMNS as f32 * column;
    match footer.w >= wide && footer.h >= 1.0 {
        true => Some(Panel::new(
            footer.x + footer.w - wide,
            footer.y,
            wide,
            footer.h,
        )),
        false => None,
    }
}

/// What a card's action is, as the press the routing answers.
pub(crate) fn settings_act_for(doing: crate::settings::Doing) -> Act {
    match doing {
        crate::settings::Doing::Validate => Act::Validate,
        crate::settings::Doing::Install => Act::Install,
        crate::settings::Doing::AddServer => Act::AddServer,
        crate::settings::Doing::Restore => Act::Restore,
        crate::settings::Doing::Check => Act::Check,
        crate::settings::Doing::DefaultEndpoint => Act::DefaultEndpoint,
        crate::settings::Doing::Reveal | crate::settings::Doing::Hide => Act::Reveal,
    }
}

/// How wide the enable-edition checkbox is, in columns of pane text: the mark,
/// a space, and the words `enable edition`.
const SETTING_EDIT_COLUMNS: usize = 18;

/// The footer of an editable document block: the enable-edition checkbox at
/// the left, and the actions at the right, where every card action sits.
///
/// Every action the block has, always, whether or not edition is on: they are
/// drawn dim and do nothing while it is off. Buttons that come and go under
/// the pointer are a footer whose shape depends on a checkbox two clicks ago,
/// and the two that appeared were the two anybody was looking for. Asked by
/// the placement and by the drawing, so a button is drawn exactly where the
/// press on it is tested for; buttons that do not fit the footer are dropped
/// as a group, since a button drawn over the checkbox is a press nobody can
/// aim.
pub(crate) fn settings_paper_acts(footer: Panel, line: f32, column: f32) -> Vec<(Act, Panel)> {
    if footer.w < 1.0 || footer.h < 1.0 {
        return Vec::new();
    }
    let check_w = (SETTING_EDIT_COLUMNS as f32 * column).min(footer.w);
    let mut out = vec![(
        Act::EditPrompt,
        Panel::new(footer.x, footer.y, check_w, footer.h),
    )];
    let mut acts: Vec<Act> = vec![Act::LoadPrompt, Act::RestorePrompt];
    acts.push(Act::SavePrompt);
    let wide = SETTING_DOING_COLUMNS as f32 * column;
    let step = design::step(line);
    let whole = wide * acts.len() as f32 + step * acts.len().saturating_sub(1) as f32;
    if footer.w < check_w + step + whole {
        return out;
    }
    let x = footer.x + footer.w - whole;
    for (at, act) in acts.into_iter().enumerate() {
        out.push((
            act,
            Panel::new(x + at as f32 * (wide + step), footer.y, wide, footer.h),
        ));
    }
    out
}

/// Where each option of a choice sits inside the box its field gives it.
///
/// All of them, side by side, sharing the width: a choice the user cannot see
/// the options of is a choice they will not know they have, which is what one
/// box holding one word was. Asked by the placement and by the drawing, so an
/// option is drawn exactly where the press that names it is tested for.
///
/// Equal shares rather than each one's own width, because the widths would then
/// move under the pointer every time the value changed.
pub(crate) fn settings_choice_boxes(input: Panel, options: usize, line: f32) -> Vec<Panel> {
    let options = options.max(1);
    let step = design::step(line);
    let wide = ((input.w - step * (options - 1) as f32) / options as f32).max(1.0);
    (0..options)
        .map(|at| Panel::new(input.x + at as f32 * (wide + step), input.y, wide, input.h))
        .filter(|box_| box_.x + box_.w <= input.x + input.w + 0.01)
        .collect()
}

/// The options a field's control offers, or nothing when it is not a choice.
pub(crate) fn settings_choices_of(row: &SettingRow) -> Option<&'static [&'static str]> {
    match row {
        SettingRow::Setting {
            kind: crate::settings::Kind::Choice(names),
            ..
        } => Some(names),
        _ => None,
    }
}

/// Where each colour of a palette card sits inside its body.
///
/// One line per row of colours, as many across as the model counted the card's
/// height with ([`crate::settings::palette_body_lines`]): a grid drawn wider
/// than it was measured puts every press below it on another colour.
pub(crate) fn settings_palette_slots(body: Panel, line: f32, cells: usize, across: usize) -> Vec<Panel> {
    let across = across.max(1);
    let wide = (body.w / across as f32).max(1.0);
    (0..cells)
        .map(|at| {
            Panel::new(
                body.x + (at % across) as f32 * wide,
                body.y + (at / across) as f32 * line,
                wide,
                line,
            )
        })
        .filter(|slot| slot.y + slot.h <= body.y + body.h + 0.01)
        .collect()
}

/// Where a cell's text is written inside the box its column gives it.
///
/// Every cell used to start at its column's left edge, which left the two
/// columns of numbers ragged: `283 B` under `1.2 MB` puts the digits in a
/// different place on every row, and a column of numbers nobody can compare down
/// is a column nobody reads. The model says which columns are numbers
/// ([`crate::settings::Align`]) and those are written against the right edge of
/// their box instead. The header takes the same treatment, so a name still sits
/// over its own cells.
///
/// Takes the text after it has been clipped to what the box holds, because what
/// the right edge is measured back from is what is really drawn.
pub(crate) fn settings_session_ink(
    at: Panel,
    step: usize,
    shown: &str,
    column: f32,
    columns: &[(&str, usize, crate::settings::Align)],
) -> Panel {
    // Off the rules on both sides: a cell whose text touches the hairline
    // beside it reads as text stuck to a line, header and data alike.
    let pad = (column * 0.75).floor();
    let right = matches!(
        columns.get(step),
        Some((_, _, crate::settings::Align::Right))
    );
    if !right {
        return Panel::new(at.x + pad, at.y, (at.w - pad).max(1.0), at.h);
    }
    let wide = (shown.chars().count() as f32 * column).min((at.w - pad).max(1.0));
    Panel::new(at.x + at.w - pad - wide, at.y, wide, at.h)
}

/// The lines between the columns of the saved-conversations table.
///
/// One down the left edge of every column but the first, on the header and on
/// every row under it, so the table reads as a grid rather than as words that
/// happen to line up. The hairline along the bottom of a row is drawn by the
/// list itself, the same way it is for every other row on the panel.
pub(crate) fn settings_session_lines(scene: &mut Scene, cells: &[Panel], row: Panel, edge: [f32; 4]) {
    if row.h < 2.0 {
        return;
    }
    for at in cells.iter().skip(1) {
        if at.w < 1.0 || at.x - 1.0 < row.x {
            continue;
        }
        scene.rect(Panel::new(at.x - 1.0, row.y, 1.0, row.h).fill(edge));
    }
}

/// The box a card is drawn in, inside the row it stands in.
///
/// The row is the card and the [`design::APART`] under it: two cards never
/// touch, and the space between them belongs to the one above so the row height
/// the model counted is the room the layout has.
pub(crate) fn settings_card(row: Panel, line: f32) -> Panel {
    Panel::new(row.x, row.y, row.w, (row.h - design::apart(line)).max(1.0))
}

/// Where a card's header, its divider, its body and its footer sit.
///
/// Asked by the placement and by the drawing, the way [`settings_control`] is,
/// so a button is drawn exactly where the press on it is tested for.
pub(crate) struct CardParts {
    /// The box the title is written in, at the card title size.
    pub(crate) title: Panel,
    /// The hairline under the title, the full width of the card. The one
    /// divider a card gets.
    pub(crate) rule: Panel,
    /// The fields, with [`design::ROOM`] inside the border on every side.
    pub(crate) body: Panel,
    /// The strip the buttons stand in: at the bottom of the card, whatever the
    /// body's height. Empty on a card with no actions.
    pub(crate) footer: Panel,
}

pub(crate) fn settings_card_parts(
    card: Panel,
    line: f32,
    size: f32,
    column: f32,
    cols: usize,
    footer: bool,
) -> CardParts {
    let room = design::room(line);
    // Never taller than the band the model claimed for it: a title that
    // overran its own header would push the divider onto the first field.
    let title_h = Text::line_for(design::card_title_size(size)).min(design::TITLE_LINES * line);
    let head = room + design::TITLE_LINES * line + room;
    // The body's width in columns is what a description wraps in and what the
    // model counted the card's height with, so it is taken in columns and not
    // in pixels. Held to what is really inside the border, which it is under at
    // every size the panel is used at.
    let inset = room * 2.0;
    let body_w = (design::card_cols(cols) as f32 * column).min((card.w - inset * 2.0).max(1.0));
    let foot_h = match footer {
        true => (design::BUTTON_LINES + design::ROOM) * line,
        false => 0.0,
    };
    let body_y = card.y + head + room;
    CardParts {
        title: Panel::new(card.x + inset, card.y + room, body_w, title_h),
        rule: Panel::new(card.x, (card.y + head).floor(), card.w, 1.0),
        body: Panel::new(
            card.x + inset,
            body_y,
            body_w,
            (card.y + card.h - room - foot_h - body_y).max(0.0),
        ),
        footer: match footer {
            true => Panel::new(
                card.x + inset,
                card.y + card.h - room - design::BUTTON_LINES * line,
                body_w,
                design::BUTTON_LINES * line,
            ),
            false => nowhere(),
        },
    }
}

/// Where each field of a card sits inside its body: one band per row of them,
/// [`design::STEP`] between the bands and between two fields side by side.
///
/// `hints` is one flag per field, saying whether it carries a sentence, and it
/// is the same list [`crate::settings::card_body_lines`] counted the card's
/// height with: a band drawn taller than it was measured puts every press below
/// it on another field.
pub(crate) fn settings_card_slots(
    body: Panel,
    line: f32,
    hints: &[bool],
    across: usize,
    group: Option<usize>,
) -> Vec<Panel> {
    let across = across.max(1);
    let step = design::step(line);
    // Three steps between two fields across: the halves read as two columns
    // with air between them, not as one line that collides mid-card.
    let gutter = step * 3.0;
    let wide = ((body.w - gutter * (across - 1) as f32) / across as f32).max(1.0);
    design::field_slots(hints, across, group)
        .into_iter()
        .enumerate()
        .map(|(at, (top, tall))| {
            Panel::new(
                body.x + (at % across) as f32 * (wide + gutter),
                body.y + top * line,
                wide,
                tall * line,
            )
        })
        .collect()
}

/// Where a field's label and its value sit inside the slot the card gave it:
/// the label on the first line, the input [`design::TIGHT`] under it.
///
/// Never side by side. A label and a value on one line read as one sentence,
/// which is what made every value on this panel look like part of its own key.
pub(crate) fn settings_field_boxes(slot: Panel, line: f32) -> (Panel, Panel) {
    (
        Panel::new(slot.x, slot.y, slot.w, line),
        Panel::new(slot.x, slot.y + line + design::tight(line), slot.w, line),
    )
}

/// The sentence under a field, on the line under its input.
///
/// Drawn at the hint size and given a whole line of pane text, which is what
/// [`design::field_lines`] claimed for it: the smaller line leaves a little
/// slack under it rather than needing an arithmetic of its own.
pub(crate) fn settings_hint_box(slot: Panel, line: f32, size: f32) -> Panel {
    let (_, input) = settings_field_boxes(slot, line);
    Panel::new(
        slot.x,
        input.y + line + design::tight(line),
        slot.w,
        Text::line_for(design::hint_size(size)).min(line),
    )
}

/// One field of a card: its label, what it holds under that, and the sentence
/// that says what it is for under that.
///
/// The one place a field is drawn, whatever it holds, so a reading, a line that
/// is typed into and a number on a track are the same shape at the same left
/// edge. `here` is whether the keys are on this field, which is what the focus
/// edge says; the card's own border says they are on the card.
#[allow(clippy::too_many_arguments)]
pub(crate) fn settings_card_field(
    scene: &mut Scene,
    frame: &Frame,
    panel: &Settings,
    field: &crate::settings::CardField,
    slot: Panel,
    at: (usize, Side),
    here: bool,
    line: f32,
) {
    let (skin, size) = (frame.skin, frame.pane_size);
    let column = frame.pane_column.max(1.0);
    let (index, side) = at;
    let (label_at, input_at) = settings_field_boxes(slot, line);
    // The name of the field, in the quiet tint: a label as loud as its value is
    // a label competing with the thing it names.
    scene.text(Text::rich(
        vec![Run::tinted(
            clip(
                &field.label,
                columns_in(label_at.w, column).saturating_sub(1),
            ),
            skin.dim,
        )],
        label_at,
        design::label_size(size),
        skin.dim,
    ));
    let hot_value = frame.hot == Some(Hit::SettingsValue(index, side));
    let typing = here.then(|| panel.editing()).flatten();
    let options = settings_choices_of(field.holds.as_ref());
    let track = frame
        .layout
        .settings_tracks
        .iter()
        .find(|(row, half, _)| *row == index && *half == side)
        .map(|(_, _, track)| *track);
    // The border is the whole of what says a value is typed or nudged in
    // place. A reading is the same shape without one, so the two are told
    // apart without pressing either. A choice draws its options instead of a
    // box, and a slider is its own control: the bare track is its resting
    // look, and boxing it made it read as an input.
    let boxed = field.editable() && options.is_none() && track.is_none() && !field.link;
    // A link: the chain in front of the address and a dotted rule under it, in
    // the accent every link in this window wears. No box, because the thing it
    // holds is an address rather than a setting.
    if field.link {
        let ink = match typing.is_some() || here || hot_value {
            true => skin.bright,
            false => skin.body,
        };
        scene.text(Text::rich(
            vec![Run::icon(crate::design::icons::LINK.to_string(), ink)],
            Panel::new(input_at.x, input_at.y, column * 2.0, input_at.h),
            size,
            ink,
        ));
        settings_dotted_rule(
            scene,
            Panel::new(
                input_at.x,
                (input_at.y + input_at.h - 1.0).floor(),
                input_at.w,
                1.0,
            ),
            column,
            match typing.is_some() || here {
                true => skin.edge_focus,
                false => skin.edge,
            },
        );
    }
    if boxed {
        scene.rect(panel_fill(input_at, skin.input));
        if hot_value {
            scene.rect(panel_fill(input_at, skin.hot));
        }
        scene.rect(panel_edge(
            input_at,
            match typing.is_some() || here {
                true => skin.edge_focus,
                false => skin.edge,
            },
        ));
    }
    let room = match (boxed, field.link) {
        (true, _) => Panel::new(
            input_at.x + INPUT_PAD,
            input_at.y,
            (input_at.w - INPUT_PAD * 2.0).max(1.0),
            input_at.h,
        ),
        // Past the chain, which stands where the box's padding would.
        (_, true) => Panel::new(
            input_at.x + column * 2.0,
            input_at.y,
            (input_at.w - column * 2.0).max(1.0),
            input_at.h,
        ),
        _ => input_at,
    };
    let value = panel.preview(index, side).unwrap_or_else(|| field.value());
    // A choice, drawn as all of its options with the one that is set filled in:
    // a choice the user cannot see the options of is a choice they will not
    // know they have, and one box holding one word is what "i only see noob
    // matrix the others i asked are absent" was. The filled one is the primary
    // kind and the rest are secondaries, the same way a toggle says which of
    // its two states is live.
    if let Some(names) = options {
        for (option, box_) in settings_choice_boxes(input_at, names.len(), line)
            .into_iter()
            .enumerate()
        {
            let Some(name) = names.get(option) else {
                break;
            };
            let on = *name == value;
            let ink = match on {
                true => skin.bright,
                false => skin.body,
            };
            settings_button(
                scene,
                box_,
                match on {
                    true => ButtonKind::Primary,
                    false => ButtonKind::Secondary,
                },
                vec![Run::tinted(
                    clip(
                        name,
                        columns_in(box_.w - INPUT_PAD * 2.0, column).saturating_sub(1),
                    ),
                    ink,
                )],
                ink,
                frame.hot == Some(Hit::SettingsChoice(index, side, option)),
                skin,
                size,
            );
            // Which option the keys would leave the field on, said the way
            // every other field says the keys are in it.
            if here && on {
                scene.rect(panel_edge(box_, skin.edge_focus));
            }
        }
        return settings_card_hint(scene, field, slot, line, size, column, skin);
    }
    match (field.holds.as_ref(), track) {
        // A number with a range is a position on a track, with the number it is
        // at against the right edge of the same room. Nothing changes on
        // rollover: a slider that lights up under a passing pointer read as a
        // selection effect nobody asked for, the same rule the flat rows keep.
        (SettingRow::Setting { kind, .. }, Some(track)) if track.w >= 1.0 => {
            let thick = (line * 0.3).floor().max(2.0);
            let up = ((line - thick) * 0.5).floor();
            let along = panel.fraction(index, side).unwrap_or(0.0);
            scene.rect(Panel::new(track.x, track.y + up, track.w, thick).fill(skin.gauge_track));
            scene.rect(
                Panel::new(track.x, track.y + up, (track.w * along).floor(), thick)
                    .fill(skin.gauge),
            );
            settings_track_ticks(scene, track, *kind, line, skin);
            scene.rect(
                Panel::new(
                    track.x + ((track.w - CARET_W) * along).floor(),
                    track.y + 1.0,
                    CARET_W,
                    (line - 2.0).max(1.0),
                )
                .fill(skin.edge_focus),
            );
            // A gap between the track and its number: the two run together
            // otherwise, and the room for it is already taken off the track's
            // own width above.
            let gap = column * SETTING_TRACK_GAP_COLUMNS;
            let number = Panel::new(
                track.x + track.w + gap,
                input_at.y,
                (input_at.x + input_at.w - INPUT_PAD - track.x - track.w - gap).max(1.0),
                input_at.h,
            );
            scene.text(Text::rich(
                vec![Run::tinted(
                    clip(value, SETTING_TRACK_VALUE_COLUMNS.saturating_sub(1)),
                    skin.bright,
                )],
                number,
                design::value_size(size),
                skin.bright,
            ));
        }
        // A line that is typed into: what has been typed with a caret after it
        // while it is being typed, and the end of the value rather than the
        // start when it is not, because what changes in an address is the port
        // and the path.
        (SettingRow::Field { .. }, _) => {
            let (shown, ink) = match (typing, value.is_empty()) {
                (Some(typed), _) => (String::from(typed), skin.bright),
                (None, true) => (String::from(crate::settings::UNSET), skin.dim),
                (None, false) => (String::from(value), skin.bright),
            };
            let shown = tail(&shown, columns_in(room.w, column).saturating_sub(1));
            scene.text(Text::rich(
                vec![Run::tinted(shown.clone(), ink)],
                room,
                design::value_size(size),
                ink,
            ));
            if typing.is_some() {
                scene.rect(
                    Panel::new(
                        room.x + shown.chars().count() as f32 * column,
                        room.y,
                        2.0,
                        line,
                    )
                    .fill(skin.caret),
                );
            }
        }
        // Everything else is the value as it reads: a name out of a list inside
        // its box, or a reading with no box at all. A field nobody has set is
        // drawn in the quiet tint, so an empty one reads as empty rather than as
        // a value somebody chose.
        _ => {
            let ink = match value == crate::settings::UNSET {
                true => skin.dim,
                false => skin.bright,
            };
            scene.text(Text::rich(
                vec![Run::tinted(
                    clip(value, columns_in(room.w, column).saturating_sub(1)),
                    ink,
                )],
                room,
                design::value_size(size),
                ink,
            ));
        }
    }
    settings_card_hint(scene, field, slot, line, size, column, skin);
}

/// The ticks on a slider's track, one per detent: a mark at each value the
/// drag snaps to, so the checkpoints can be seen before the thumb feels them.
///
/// Drawn where the fractions say, off the same [`crate::settings::Kind`] the
/// snap itself reads, so a tick and the value it magnets to cannot drift.
pub(crate) fn settings_track_ticks(
    scene: &mut Scene,
    track: Panel,
    kind: crate::settings::Kind,
    line: f32,
    skin: &Skin,
) {
    let tall = (line * 0.6).floor().max(4.0);
    let up = ((line - tall) * 0.5).floor();
    for at in kind.stop_fractions() {
        let x = track.x + ((track.w - 1.0) * at).floor();
        scene.rect(Panel::new(x, track.y + up, 1.0, tall).fill(skin.edge_focus));
    }
}

/// The sentence under a field, in the hint role: what this field decides, and
/// which line of which file writes it. Half the rows of the agent's section
/// said nothing at all about what they did.
///
/// Its own function because a choice draws its options and then leaves, and a
/// field that lost its sentence on the way out would be three names with
/// nothing saying what they are.
pub(crate) fn settings_card_hint(
    scene: &mut Scene,
    field: &crate::settings::CardField,
    slot: Panel,
    line: f32,
    size: f32,
    column: f32,
    skin: &Skin,
) {
    let Some(hint) = &field.hint else {
        return;
    };
    let small = design::hint_size(size);
    let hint_at = settings_hint_box(slot, line, size);
    if hint_at.y + hint_at.h > slot.y + slot.h + 0.01 {
        return;
    }
    scene.text(Text::rich(
        vec![Run::tinted(
            clip(
                hint,
                columns_in(hint_at.w, design::column_for(column, size, design::HINT))
                    .saturating_sub(1),
            ),
            skin.dim,
        )],
        hint_at,
        small,
        skin.dim,
    ));
}

/// The three kinds of button on this window, and no others.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ButtonKind {
    /// The action the card exists for: filled in the accent.
    Primary,
    /// Anything reversible: outlined in the ordinary edge.
    Secondary,
    /// Anything that loses work: outlined in the colour this window uses for
    /// exactly that.
    Danger,
}

/// One button, drawn where it was placed.
///
///
/// The fill, the outline and the hover all come from the kind, so two buttons of
/// one kind cannot end up looking like two different things. Every one of them
/// carries the window's corner cut, the way every other surface does.
#[allow(clippy::too_many_arguments)]
pub(crate) fn settings_button(
    scene: &mut Scene,
    box_: Panel,
    kind: ButtonKind,
    runs: Vec<Run>,
    tint: [u8; 4],
    hot: bool,
    skin: &Skin,
    size: f32,
) {
    if box_.w < 1.0 || box_.h < 1.0 {
        return;
    }
    match kind {
        ButtonKind::Primary => {
            scene.rect(panel_fill(
                box_,
                match hot {
                    true => skin.button_hot,
                    false => skin.button,
                },
            ));
        }
        ButtonKind::Secondary => {
            scene.rect(panel_fill(box_, skin.input));
            if hot {
                scene.rect(panel_fill(box_, skin.hot));
            }
            scene.rect(panel_edge(box_, skin.edge));
        }
        ButtonKind::Danger => {
            if hot {
                scene.rect(panel_fill(box_, skin.hot));
            }
            scene.rect(panel_edge(box_, skin.close_hot));
        }
    }
    scene.text(Text::rich(
        runs,
        Panel::new(
            box_.x + INPUT_PAD,
            box_.y,
            (box_.w - INPUT_PAD * 2.0).max(1.0),
            box_.h,
        ),
        size,
        tint,
    ));
}

/// The box, the title bar and the one divider every card carries, held to the
/// clip the list is drawn through.
///
/// The border goes to the focus colour with the mark down its edge while the
/// keys are on the card. That is what a full row band said on a row one line
/// tall; over a box nine lines tall a band is a highlight nobody can read
/// through.
#[allow(clippy::too_many_arguments)]
pub(crate) fn settings_card_shell(
    scene: &mut Scene,
    card: Panel,
    parts: &CardParts,
    title: &str,
    tint: [u8; 4],
    on: bool,
    skin: &Skin,
    size: f32,
    column: f32,
    held: ListClip,
) {
    settings_card_border(
        scene,
        card,
        match on {
            true => skin.edge_focus,
            false => skin.edge,
        },
        held,
    );
    if on && let Some(mark) = held.cut(Panel::new(card.x, card.y, MARK_W, card.h)) {
        scene.rect(mark.fill(skin.edge_focus));
    }
    if parts.title.w >= 1.0 {
        let big = design::card_title_size(size);
        let big_column = design::column_for(column, size, design::CARD_TITLE);
        if let Some(text) = held.text(Text::rich(
            vec![Run::tinted(
                clip(title, columns_in(parts.title.w, big_column).saturating_sub(1)),
                tint,
            )],
            parts.title,
            big,
            tint,
        )) {
            scene.text(text);
        }
    }
    if parts.rule.w >= 1.0 && parts.rule.y + 1.0 <= card.y + card.h && held.holds(parts.rule) {
        scene.rect(parts.rule.fill(skin.edge));
    }
}

/// The rule under a link: a dash every other column, which is what a dotted
/// underline is at this size. Whole pixels, or the dashes land on half ones and
/// read as a grey line.
pub(crate) fn settings_dotted_rule(scene: &mut Scene, at: Panel, column: f32, edge: [f32; 4]) {
    if at.w < 1.0 || at.h < 1.0 {
        return;
    }
    let dash = (column * 0.5).floor().max(1.0);
    let step = dash * 2.0;
    let mut x = at.x;
    while x + dash <= at.x + at.w {
        scene.rect(Panel::new(x.floor(), at.y, dash, at.h).fill(edge));
        x += step;
    }
}

/// A card's border, held to the clip: the stroked ring while the card is
/// wholly on screen, and only the sides that are really there while it slides
/// past an edge. A stroke trimmed instead would close the box and draw an
/// edge along the cut, a line the card does not have there: what a cut card
/// shows is its two sides running off the screen.
fn settings_card_border(scene: &mut Scene, card: Panel, edge: [f32; 4], held: ListClip) {
    if held.holds(card) {
        scene.rect(panel_edge(card, edge));
        return;
    }
    let Some(shown) = held.cut(card) else {
        return;
    };
    scene.rect(shown.left_edge(edge));
    if held.under(card) < 1.0 {
        scene.rect(shown.bottom_edge(edge));
    }
    match held.over(card) >= 1.0 {
        // The top is gone, and the cut corner with it: the chamfer reaches
        // [`CUT`] down the card and the window scrolls by whole rows of text,
        // which are taller than that. The right edge runs the whole visible
        // height.
        true => {
            scene.rect(shown.right_edge(edge));
        }
        // The top is on screen: the top edge stops where the corner's cut
        // starts, the diagonal is the stair the panes draw theirs with, and
        // the right edge picks up under it and runs to the clip.
        false => {
            let cut = cut_of(card);
            scene.rect(Panel::new(card.x, card.y, (card.w - cut).max(1.0), 1.0).fill(edge));
            cut_line(scene, card, edge, 1.0);
            scene.rect(
                Panel::new(
                    card.x + card.w - 1.0,
                    card.y + cut,
                    1.0,
                    (shown.y + shown.h - card.y - cut).max(0.0),
                )
                .fill(edge),
            );
        }
    }
}

/// Where a row's control sits: one column in from the label, the width a value
/// needs and no more.
///
/// The least the rail of section names goes down to, in columns of pane text.
///
/// It holds the longest section name (SYSTEM PROMPT) with room for its mark. A
/// floor rather than a width, because the rail is dragged: it is the room the
/// names need, and the settings beside them are held to the same floor, so
/// neither side of the drag can be squeezed away.
const SETTING_RAIL_COLUMNS: usize = 17;

pub(crate) fn settings_rail_floor(column: f32) -> f32 {
    SETTING_RAIL_COLUMNS as f32 * column.max(1.0)
}

/// A box for every section name, inside the rail's own width.
///
/// The names used to be one column that stopped at the last one that fitted, so
/// a big font in a small window drew three of the five and the other two had no
/// row to press and no row to hit. One of the two is APPEARANCE, which is where
/// the font size is put back down: raising it far enough hid the only way to
/// lower it again. So the column wraps. As many names go down the rail as there
/// is height for, the rest start another column beside them, and the rail's
/// width is shared between however many columns that takes. Every section keeps
/// a box at every size, and a box too narrow for the whole name clips it the way
/// every other name in this window is clipped, which is a name you can still
/// read and still press.
///
/// The rail is not scrolled instead, because a scrolled rail is a section you
/// have to know is there before you can reach it, and the thing being fixed is
/// exactly that a section nobody could see was a section nobody could get to.
pub(crate) fn settings_rail_cells(body: Panel, rail_w: f32, line: f32, names: usize) -> Vec<(usize, Panel)> {
    let names = names.max(1);
    // A body shorter than a line still gets one row of names rather than none,
    // cut to the room there is: a rail drawn over the footer is worse than a
    // short one, and a rail with nothing on it is the bug itself. Half a line
    // of air per row when the column has room for it: the rail is a resting
    // column of five names, not a packed list. When it does not (a huge font
    // in a small window), the rows pack back to a line each so the names keep
    // their width instead of wrapping into slivers.
    let roomy = line * 1.5;
    // The rail stands a padding in from its left edge and half a line down
    // from its top, so the names are not stuck to the panel's corner.
    let pad = crate::view::PAD;
    let top = (line * 0.5).min(body.h * 0.2);
    let room_h = (body.h - top).max(1.0);
    let tall = match room_h >= names as f32 * roomy {
        true => roomy,
        false => line,
    }
    .min(room_h)
    .max(1.0);
    let down = ((room_h / tall).floor() as usize).max(1);
    let across = names.div_ceil(down);
    let wide = (((rail_w - pad).max(1.0)) / across as f32).floor().max(1.0);
    (0..names)
        .map(|index| {
            let at = Panel::new(
                body.x + pad + (index / down) as f32 * wide,
                body.y + top + (index % down) as f32 * tall,
                wide,
                tall,
            );
            (index, at)
        })
        .collect()
}


/// How much of a slider's row the number beside the track takes. The track gets
/// the rest.
///
/// Eight, because the widest number on any track is the agent's context window
/// and that is seven digits at the top of its range. Six fitted every setting of
/// the window's own and clipped `1048576` to `10485\u{2026}`, which is a slider
/// whose number cannot be read at the end anybody would drag it to.
pub(crate) const SETTING_TRACK_VALUE_COLUMNS: usize = 8;

/// The air between the end of a track and the number that reads it, in columns
/// of pane text.
///
/// Four. Two was still read as stuck to it: at a pane font that is fifteen
/// pixels, and the track's grip sits at the very end of the track at the top
/// of its range, which puts the mark and the digits a hair apart exactly where
/// somebody drags to. Four columns is a gap nobody has to look twice at.
pub(crate) const SETTING_TRACK_GAP_COLUMNS: f32 = 4.0;

/// What the two controls on an entry row take, in columns of pane text: the
/// toggle that turns it on and off, and the uninstall beside it.
///
/// Both are boxes with a word in them, sized for the longer of the two words
/// they can hold, so pressing one does not change the width of the thing that
/// was just pressed.
///
/// The uninstall holds `uninstall` and the padding on both sides of it. Eleven
/// columns was nine glyphs, [`INPUT_PAD`] twice and a column and a half of
/// slack, and a column here is measured on a digit: the moment the real letter
/// advance ran past it the shaper's own bounds took the last `l` off, so the
/// button read `uninstal`.
const SETTING_TOGGLE_COLUMNS: usize = 5;
const SETTING_REMOVE_COLUMNS: usize = 13;

/// How many characters an entry's text wraps in: the list's own width, less the
/// mark down the edge of a row and the air after it.
///
/// The one number. A description is wrapped by the renderer, its height is
/// counted by the model and the room for it is left by the layout, and a row
/// measured in one width and drawn in another puts every click below it on the
/// wrong row. Read off the list rather than off a row, because the row is what
/// it decides the height of.
pub(crate) fn settings_entry_cols(list_w: f32, column: f32) -> usize {
    columns_in((list_w - MARK_W - 3.0).max(1.0), column)
}

/// The two halves of a band two cards share: the left one against the left edge
/// and the right one against the right, with the rest of the width as air
/// between them.
///
/// Each half is exactly the width [`crate::settings::half_cols`] counted it in,
/// so a card measured in half the list is drawn in half the list.
pub(crate) fn settings_card_halves(band: Panel, column: f32) -> (Panel, Panel) {
    let cols = crate::settings::half_cols(settings_entry_cols(band.w, column));
    let wide = (cols as f32 * column + MARK_W + 3.0).min(band.w);
    (
        Panel::new(band.x, band.y, wide, band.h),
        Panel::new(band.x + band.w - wide, band.y, wide, band.h),
    )
}

/// The strip down the right of the list that belongs to the list's own
/// scrollbar, and to nothing else.
///
/// The bar used to be painted at the right edge of the list itself, which is
/// the right edge of every card in it: the track ran through the last four
/// pixels of every card border, through the buttons at the right of a footer,
/// and through the tail of every wrapped description. A rectangle cannot cover
/// a glyph on this layer, so the letters were drawn on top of the bar rather
/// than the other way round, and a press in those pixels answered for whatever
/// button was under them.
///
/// So the cards stop short of it. [`GAP`] of air, then the track and the
/// [`SCROLL_GAP`] it already sits in from the right. Reserved whether or not
/// the list can scroll, because a gutter that came and went with the scroll
/// would change the width a description wraps in, which changes the height of
/// the row it is in, which changes whether the list scrolls at all.
const SETTING_GUTTER: f32 = GAP + SCROLL_W + SCROLL_GAP;

/// The part of the list the rows are drawn in: everything left of the gutter.
pub(crate) fn settings_list_rows(list: Panel) -> Panel {
    Panel::new(list.x, list.y, (list.w - SETTING_GUTTER).max(1.0), list.h)
}

/// The band of the list the rows show through.
///
/// The scroll window can start and end inside a row now
/// ([`Settings::window`]), so a row's boxes are laid out at their true place
/// and then held to this band: a hit region is cut to what is on screen, a
/// control that is not wholly on screen is dropped, and the painter cuts its
/// drawing by the same two numbers off the same list box. That is what keeps
/// a press and a pixel in step while a card slides past either edge.
///
/// [`Settings::window`]: crate::settings::Settings::window
#[derive(Clone, Copy)]
pub(crate) struct ListClip {
    top: f32,
    bottom: f32,
}

impl ListClip {
    pub(crate) fn of(cards: Panel) -> ListClip {
        ListClip {
            top: cards.y,
            bottom: cards.y + cards.h,
        }
    }

    /// A clip that holds everything, for a card that does not scroll past
    /// anything: the document beside the entry list is never cut.
    pub(crate) fn open() -> ListClip {
        ListClip {
            top: f32::NEG_INFINITY,
            bottom: f32::INFINITY,
        }
    }

    /// Whether a box is wholly on screen. A control that is not is neither
    /// drawn nor pressable: half a button is a press nobody can aim, so it
    /// appears once the scroll brings the whole of it on.
    pub(crate) fn holds(&self, panel: Panel) -> bool {
        panel.y >= self.top - 0.01 && panel.y + panel.h <= self.bottom + 0.01
    }

    /// The visible part of a box, or nothing when it is off screen: for a hit
    /// region or a fill that can be cut, since what comes back is
    /// exactly the part that is on screen.
    pub(crate) fn cut(&self, panel: Panel) -> Option<Panel> {
        let y = panel.y.max(self.top);
        let h = (panel.y + panel.h).min(self.bottom) - y;
        (h >= 1.0).then(|| Panel::new(panel.x, y, panel.w, h))
    }

    /// How much of a box hangs above the band, in pixels.
    pub(crate) fn over(&self, panel: Panel) -> f32 {
        (self.top - panel.y).max(0.0)
    }

    /// How much of a box hangs below the band, in pixels.
    pub(crate) fn under(&self, panel: Panel) -> f32 {
        (panel.y + panel.h - self.bottom).max(0.0)
    }

    /// A text box held to the band, or nothing when none of it is on.
    ///
    /// What the band cuts off the top is scrolled out of a box that now
    /// starts at the band, in the box's own line height, so every glyph row
    /// keeps its pixels and the renderer's bounds cut the split row exactly
    /// where the band is. The bottom needs only the shorter box, since the
    /// bounds already end the drawing there.
    pub(crate) fn text(&self, text: Text) -> Option<Text> {
        let over = self.over(text.at);
        let under = self.under(text.at);
        if over <= 0.0 && under <= 0.0 {
            return Some(text);
        }
        let h = text.at.h - over - under;
        if h < 1.0 {
            return None;
        }
        let mut text = text;
        text.at = Panel::new(text.at.x, text.at.y + over, text.at.w, h);
        text.scroll_lines += over / text.line_height;
        Some(text)
    }
}

/// The gutter itself, which is the box the list's bar is drawn in.
pub(crate) fn settings_list_bar(list: Panel) -> Panel {
    settings_bar_box(list, list)
}

/// Where a card draws a bar for something that scrolls inside it: down the
/// card's own right padding, beside the part of it that actually moves.
///
/// At the right edge of the card, so the track lands in the padding inside the
/// border and clear of the body, and only as tall as `from`, so the head of the
/// bar is not beside a header that does not scroll.
pub(crate) fn settings_card_bar(card: Panel, from: Panel) -> Panel {
    settings_bar_box(card, from)
}

/// The box a bar is drawn in: as wide as the track and the air to its right, at
/// the right edge of `of`, and as tall as the `down` it counts.
///
/// A box that narrow on purpose. [`scrollbar`] drops the head of a bar by the
/// corner cut of whatever box it is handed, because a track that started three
/// pixels down a chamfered pane hung in the air outside it. Nothing here is
/// chamfered at the point the bar starts: the list is not a box at all, and a
/// card's bar starts beside its body rather than beside its cut corner. Handing
/// the whole width in is what made the list's bar start eight unexplained
/// pixels below the first row it counts.
pub(crate) fn settings_bar_box(of: Panel, down: Panel) -> Panel {
    let wide = SCROLL_W + SCROLL_GAP;
    Panel::new(
        of.x + of.w - wide,
        down.y,
        wide.min(of.w.max(1.0)),
        down.h.max(1.0),
    )
}

/// The least the column beside the entry list goes down to, in columns of pane
/// text, and the least the list itself keeps.
///
/// Below either of them the split is not made at all and the section is one
/// column: a document forty characters wide is a column of hyphenated words,
/// and a list of skills squeezed to nothing is a list nobody can read.
pub(crate) const SETTING_DOC_MIN_COLUMNS: usize = 40;
pub(crate) const SETTING_ENTRY_MIN_COLUMNS: usize = 34;

/// How much of the two-column split the list takes. The document gets the rest.
pub(crate) const SETTING_ENTRY_SHARE: f32 = 0.45;

pub(crate) fn place_settings(area: Panel, shape: &Shape, panel: &Settings) -> SettingsPlaces {
    if area.w < 1.0 || area.h < 1.0 {
        return SettingsPlaces {
            box_: nowhere(),
            rail: Vec::new(),
            divider: Divider::none(),
            list: nowhere(),
            rows: Vec::new(),
            values: Vec::new(),
            tracks: Vec::new(),
            cells: Vec::new(),
            choices: Vec::new(),
            toggles: Vec::new(),
            removes: Vec::new(),
            picks: Vec::new(),
            marks: Vec::new(),
            acts: Vec::new(),
            doc: nowhere(),
            doc_text: nowhere(),
            close: nowhere(),
        };
    }
    let column = shape.pane_column.max(1.0);
    let line = Text::line_for(shape.pane_size);
    let content = area.inset(PAD);
    // The heading, and the footer that says what the keys do. The heading is
    // the one thing on the panel drawn in the panel title role, so the strip it
    // takes is that role's own line rather than a line of body text.
    let head = Text::line_for(design::panel_title_size(shape.pane_size));
    let foot = line;
    let body = Panel::new(
        content.x,
        content.y + head + GAP,
        content.w,
        (content.h - head - foot - GAP * 2.0).max(0.0),
    );
    // The rail takes its share of the body the way a column of panes takes its
    // share of the grid: a fraction off the settings file, held so neither the
    // names nor the settings beside them are squeezed below the room the names
    // need. Floored to a whole pixel so the line does not sit on a half one.
    let room = (body.w - GAP - PAD).max(1.0);
    let rail_floor = settings_rail_floor(column);
    // Floored to a whole pixel, then held to the floor again: the flooring
    // can shave the clamped width under it by a fraction that reads as a
    // whole column lost.
    let rail_w = (room * held(shape.settings_rail, room, rail_floor))
        .floor()
        .max(rail_floor.ceil().min(room));
    let rail = settings_rail_cells(body, rail_w, line, panel.section_names().len());
    // The list stands its own padding in from the divider, the same margin it
    // keeps on its right, so the cards are not glued to the draggable line.
    let list = Panel::new(
        body.x + rail_w + GAP + PAD,
        body.y,
        (body.w - rail_w - GAP - PAD).max(0.0),
        body.h,
    );
    // What the rail is dragged by. The gap between the two, plus [`GRAB`] on the
    // rail's side of it and a single pixel past the hairline: the list's rows
    // stand a padding further right, and a band that reached into them would
    // take the press that puts the cursor on a row.
    let divider = Divider {
        band: Panel::new(body.x + rail_w - GRAB, body.y, GAP + GRAB + 1.0, body.h),
        track: body,
        floor: rail_floor,
    };
    let rows_fit = Text::rows_for(shape.pane_size, list.h);
    // The rows themselves stop short of the list's own scrollbar. Everything
    // below is placed and measured in this box, never in `list`, or the bar is
    // drawn through the right edge of every card again.
    let cards = settings_list_rows(list);
    // The width a wrapped description is measured in, taken after the document
    // column has been split off and after the gutter has been taken away: a row
    // is as tall as it wraps to in the box it is actually drawn in, not in the
    // whole panel.
    let entry_cols = settings_entry_cols(cards.w, column);
    let window = panel.window(rows_fit, entry_cols);
    let clip = ListClip::of(cards);
    let mut rows = Vec::new();
    let mut values = Vec::new();
    let mut tracks = Vec::new();
    let mut cells = Vec::new();
    let mut choices = Vec::new();
    let mut toggles = Vec::new();
    let mut removes = Vec::new();
    let mut picks = Vec::new();
    let mut marks = Vec::new();
    let mut acts = Vec::new();
    // A running height rather than the row number times a line: a card is many
    // rows of text tall and everything under it moves down by that much. The
    // heights come from the model, which is what the scroll window counts in.
    // The first row starts above the top by the rows of text the window says
    // are scrolled past: every box below is laid out at its true place and
    // then held to the clip, so a card sliding off the top keeps its geometry
    // and loses only the part that is really gone.
    let mut y = cards.y - window.skip as f32 * line;
    // Where the band a pair shares stands, kept from the left half for the right
    // one: the running y has already passed it by the time the second arrives.
    let mut band = Panel::new(cards.x, cards.y, cards.w, 0.0);
    // The top and the bottom of the rows the document stands beside, so the
    // column beside them is exactly as tall as they are.
    let mut beside_doc: Option<(f32, f32)> = None;
    // One row past what the window counted when that row is the right half of a
    // pair: it carries no height of its own, so the walk that counts rows of
    // text stops before it.
    let count = match panel.row(window.first + window.count) {
        Some(_) if window.count > 0
            && crate::settings::stands_beside(
                panel.rows(),
                window.first + window.count - 1,
            ) =>
        {
            window.count + 1
        }
        _ => window.count,
    };
    for step in 0..count {
        let index = window.first + step;
        let Some(entry) = panel.row(index) else {
            break;
        };
        let all = panel.rows();
        let tall = crate::settings::band_lines(all, index, entry_cols) as f32 * line;
        // The last row of a section, when it is a table, takes the room under
        // it: a list with dead space below it is a list that stopped short.
        // Nothing scrolls differently, because nothing scrolls when it fits.
        let tall = match entry {
            SettingRow::Table(_) if panel.row(index + 1).is_none() => {
                tall.max(cards.y + cards.h - y)
            }
            _ => tall,
        };
        // Two cards to a band: the left half opens it and the right half stands
        // in the same band, so the running height passes it once.
        let beside = crate::settings::stands_beside(all, index);
        let second = index > 0 && crate::settings::stands_beside(all, index - 1);
        let row = match (beside, second) {
            (true, _) => {
                band = Panel::new(cards.x, y, cards.w, tall);
                y += tall;
                settings_card_halves(band, column).0
            }
            (_, true) => settings_card_halves(band, column).1,
            _ => {
                let whole = Panel::new(cards.x, y, cards.w, tall);
                y += tall;
                // A row the document belongs to keeps its share of the width
                // and the document stands in the rest of that band. Only those
                // rows: a form over a list with a document is a form squeezed
                // into half a panel for no reason of its own.
                match crate::settings::shows_doc(entry) {
                    false => whole,
                    true => {
                        let cols = crate::settings::beside_doc_cols(entry_cols);
                        let wide = match cols == entry_cols {
                            true => whole.w,
                            false => (cols as f32 * column + MARK_W + 3.0).min(whole.w),
                        };
                        let (top, bottom) = beside_doc.unwrap_or((whole.y, whole.y));
                        beside_doc =
                            Some((top.min(whole.y), bottom.max(whole.y + whole.h)));
                        Panel::new(whole.x, whole.y, wide, whole.h)
                    }
                }
            }
        };
        // The columns this row wraps in: its own width, which is half the list
        // for either card of a pair.
        let entry_cols = settings_entry_cols(row.w, column);
        // One box per row, always, cut to what is on screen: a press can only
        // land on the part of a card that is really there. It was two side by
        // side on a form row, which is what the AGENT section was before it
        // was cards: a card is what holds two settings side by side now, and
        // it is one row with two fields in it rather than one row cut in half.
        let Some(shown) = clip.cut(row) else {
            continue;
        };
        rows.push((index, Side::Left, shown));
        // A setting, a typed line and a reading are fields of a card: nothing
        // builds one as a row of the list, so the list places none.
        match entry {
            // A group of the palette: a card whose body is all controls, one
            // cell per colour, as many across as the model counted it with.
            SettingRow::Palette(palette) => {
                let parts = settings_card_parts(
                    settings_card(row, line),
                    line,
                    shape.pane_size,
                    column,
                    entry_cols,
                    false,
                );
                let across = design::swatch_across(design::card_cols(entry_cols));
                for (cell, at) in
                    settings_palette_slots(parts.body, line, palette.cells.len(), across)
                        .into_iter()
                        .enumerate()
                {
                    // Only the colours wholly on screen: the painter drops the
                    // same cells, so a swatch is pressable exactly when it can
                    // be seen.
                    if clip.holds(at) {
                        cells.push((index, cell, at));
                    }
                }
            }
            // The toggle and, where there is one, the uninstall: both at the
            // right of the row's first line, so they line up down the list the
            // way every value on the panel does. Nothing at all in a column too
            // narrow to hold them beside a name, since a button drawn over the
            // name it belongs to is a press nobody can aim.
            // The saved conversations: one region per row inside the body, the
            // mark in front of each of them, and the three buttons in the
            // footer. Only the rows the body is showing, since the table scrolls
            // inside itself: a region for a row that is not drawn is a press
            // that answers for something nobody can see.
            SettingRow::Table(table) => {
                let card = settings_card(row, line);
                let parts = settings_card_parts(
                    card,
                    line,
                    shape.pane_size,
                    column,
                    entry_cols,
                    true,
                );
                let (_, boxes) = settings_table_parts(parts.body, line);
                for (step, at) in boxes.iter().enumerate() {
                    let Some(_) = table.rows.get(table.first + step) else {
                        break;
                    };
                    let on = table.first + step;
                    // Cut, not dropped: the painter draws the visible part of
                    // a row on the card's cut edge, so that part answers.
                    let Some(shown) = clip.cut(*at) else {
                        continue;
                    };
                    picks.push((index, on, shown));
                    if let Some(mark) = table
                        .of
                        .mark()
                        .and_then(|cell| {
                            settings_session_cells(*at, column, table.columns)
                                .get(cell)
                                .copied()
                        })
                        .as_ref()
                        && mark.w >= 1.0
                        && let Some(mark) = clip.cut(*mark)
                    {
                        marks.push((index, on, mark));
                    }
                }
                let inside = parts.footer.y >= card.y
                    && parts.footer.y + parts.footer.h <= card.y + card.h + 0.01;
                if inside {
                    for (act, at) in settings_act_boxes(parts.footer, line, column, table.of) {
                        if clip.holds(at) {
                            acts.push((index, act, at));
                        }
                    }
                }
            }
            // An entry is a card: its buttons stand in the footer, at the
            // bottom of the card whatever its description wrapped to. They used
            // to be pinned to the right of the row's first line, beside the
            // name, which put every button on the panel at a different height
            // and left them fighting the text for the same pixels. Nothing at
            // all in a footer too narrow to hold them, since a button drawn over
            // another one is a press nobody can aim.
            SettingRow::Entry(entry) => {
                // A fixed entry is only read: no footer buttons at all, so
                // nothing here to place and nothing for a press to land on.
                if matches!(entry.what, crate::settings::Which::Fixed) {
                    continue;
                }
                let parts = settings_card_parts(
                    settings_card(row, line),
                    line,
                    shape.pane_size,
                    column,
                    entry_cols,
                    true,
                );
                let remove_w = match entry.removable {
                    true => SETTING_REMOVE_COLUMNS as f32 * column,
                    false => 0.0,
                };
                let toggle_w = SETTING_TOGGLE_COLUMNS as f32 * column;
                let gap = match entry.removable {
                    true => design::step(line),
                    false => 0.0,
                };
                let foot = parts.footer;
                let x = foot.x + foot.w - remove_w - gap - toggle_w;
                // Nothing at all when the card was cut off by the bottom of the
                // list and has no footer left inside itself: a button placed
                // above its own card is a press that answers for the card over
                // it.
                let card = settings_card(row, line);
                let inside = foot.y >= card.y && foot.y + foot.h <= card.y + card.h + 0.01;
                if foot.w >= 1.0 && x >= foot.x && inside {
                    let toggle = Panel::new(x, foot.y, toggle_w, foot.h);
                    if clip.holds(toggle) {
                        toggles.push((index, toggle));
                    }
                    let remove = Panel::new(x + toggle_w + gap, foot.y, remove_w, foot.h);
                    if entry.removable && clip.holds(remove) {
                        removes.push((index, remove));
                    }
                }
            }
            // An editable document block: the enable-edition checkbox and the
            // actions in its footer, exactly where the painter draws them.
            // Nothing when the card was cut off by the bottom of the list and
            // has no footer left inside itself, the same rule every card's
            // buttons follow. A block with no footer places nothing.
            SettingRow::Paper(paper) => {
                if !paper.does {
                    continue;
                }
                let card = settings_card(row, line);
                let parts =
                    settings_card_parts(card, line, shape.pane_size, column, entry_cols, true);
                let inside = parts.footer.y >= card.y
                    && parts.footer.y + parts.footer.h <= card.y + card.h + 0.01;
                if inside {
                    for (act, at) in
                        settings_paper_acts(parts.footer, line, column)
                    {
                        if clip.holds(at) {
                            acts.push((index, act, at));
                        }
                    }
                }
            }
            // A card's own fields. Only one that can be typed into gets a
            // region, keyed by its slot the way the two halves of a form row
            // are: a reading with a press region over it would answer a press
            // with nothing. Two slots, because a hit carries a [`Side`] and a
            // side is one of two; a card with more than two fields that can be
            // changed is a card that needs the hit to carry its slot.
            SettingRow::Card(card) => {
                let box_ = settings_card(row, line);
                let parts = settings_card_parts(
                    box_,
                    line,
                    shape.pane_size,
                    column,
                    entry_cols,
                    card.does.is_some(),
                );
                // The card's own action, at the bottom right of it. Nothing
                // when the card was cut off by the bottom of the list and has
                // no footer left inside itself, the same rule the two buttons
                // on an entry follow.
                if let Some(doing) = card.does
                    && parts.footer.y >= box_.y
                    && parts.footer.y + parts.footer.h <= box_.y + box_.h + 0.01
                    && let Some(at) = settings_doing_box(parts.footer, column)
                    && clip.holds(at)
                {
                    acts.push((index, settings_act_for(doing), at));
                }
                let across = design::across(card.fields.len(), design::card_cols(entry_cols));
                let slots = settings_card_slots(
                    parts.body,
                    line,
                    &crate::settings::card_hints(card),
                    across,
                    card.group.as_ref().map(|group| group.at),
                );
                for (at, slot) in slots.iter().enumerate() {
                    let Some(side) = Side::of(at) else {
                        continue;
                    };
                    // A field cut by either edge of the list is not a control:
                    // the painter draws the whole slot or none of it, so half
                    // an input box is never on screen to press.
                    if !clip.holds(*slot) {
                        continue;
                    }
                    let input = settings_field_boxes(*slot, line).1;
                    // A number with a range is a track with its own number
                    // beside it, inside the input box: the same slider the
                    // panel has always had, in the room a field gives it.
                    match card.fields[at].holds.as_ref() {
                        // A choice is every one of its options, side by side in
                        // that same box. No region over the box itself: the
                        // options fill it, and one press for the field would be
                        // the single word it used to hold.
                        holds if settings_choices_of(holds).is_some() => {
                            let names = settings_choices_of(holds).unwrap_or_default();
                            for (option, at) in
                                settings_choice_boxes(input, names.len(), line)
                                    .into_iter()
                                    .enumerate()
                            {
                                choices.push((index, side, option, at));
                            }
                        }
                        SettingRow::Setting { kind, .. } if kind.fraction(0.0).is_some() => {
                            let number =
                                (SETTING_TRACK_VALUE_COLUMNS as f32 * column).min(input.w * 0.5);
                            tracks.push((
                                index,
                                side,
                                Panel::new(
                                    input.x + column,
                                    input.y,
                                    (input.w
                                        - number
                                        - column * (1.0 + SETTING_TRACK_GAP_COLUMNS))
                                        .max(1.0),
                                    input.h,
                                ),
                            ));
                        }
                        SettingRow::Setting { .. } | SettingRow::Field { .. } => {
                            values.push((index, side, input));
                        }
                        // A reading. No region at all: one over it would answer
                        // a press with nothing.
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    // The column beside the rows that own it: the rest of their band, from the
    // top of the first of them to the bottom of the last. Nothing at all when
    // the list is too narrow to split, or when none of those rows is on screen.
    let doc = match beside_doc {
        Some((top, bottom))
            if crate::settings::beside_doc_cols(entry_cols) != entry_cols =>
        {
            let wide = crate::settings::beside_doc_cols(entry_cols) as f32 * column
                + MARK_W
                + 3.0;
            let x = cards.x + wide + GAP;
            let top = top.max(cards.y);
            let bottom = bottom.min(cards.y + cards.h);
            match (cards.x + cards.w - x >= 1.0, bottom - top >= line) {
                (true, true) => Panel::new(x, top, cards.x + cards.w - x, bottom - top),
                _ => nowhere(),
            }
        }
        _ => nowhere(),
    };
    // Top right, its right edge on the content's own right edge: the padding
    // already clears the corner's cut, and a cross floating a glyph further
    // in reads as an offset, not a corner.
    let close = Panel::new(content.x + content.w - line - 2.0, content.y, line, line);
    SettingsPlaces {
        box_: area,
        rail,
        divider,
        list,
        rows,
        values,
        tracks,
        cells,
        choices,
        toggles,
        removes,
        picks,
        marks,
        acts,
        doc,
        doc_text: settings_doc_text(doc, line, shape.pane_size),
        close,
    }
}

/// Where the document's own text goes: the body of the card it is written in.
pub(crate) fn settings_doc_text(doc: Panel, line: f32, size: f32) -> Panel {
    let box_ = settings_doc_box(doc, line);
    match box_.w >= 1.0 {
        true => settings_doc_parts(box_, line, size).body,
        false => nowhere(),
    }
}

/// The card the document is written in: the whole of the column beside the
/// list.
///
/// One function, so the box that is outlined, the columns the text wraps in and
/// the rows the wheel pages by all come off the same rectangle. A column too
/// short for a header and a line under it is no card at all rather than a border
/// with nothing inside it.
pub(crate) fn settings_doc_box(doc: Panel, line: f32) -> Panel {
    let room = design::room(line);
    let least = room + design::TITLE_LINES * line + room * 2.0 + line + room;
    if doc.w < PAD * 3.0 || doc.h < least {
        return nowhere();
    }
    Panel::new(doc.x + PAD, doc.y, (doc.w - PAD).max(1.0), doc.h)
}

/// Where the document card's title, its divider and its text sit.
///
/// The same shape [`settings_card_parts`] gives every other card, with one
/// difference: its body is as wide as the column it stands in rather than as
/// wide as a card of the list, because this column has a width of its own and
/// nothing in it is measured against the list's.
pub(crate) fn settings_doc_parts(card: Panel, line: f32, size: f32) -> CardParts {
    let room = design::room(line);
    let title_h = Text::line_for(design::card_title_size(size)).min(design::TITLE_LINES * line);
    let head = room + design::TITLE_LINES * line + room;
    let body_y = card.y + head + room;
    let wide = (card.w - room * 2.0).max(1.0);
    CardParts {
        title: Panel::new(card.x + room, card.y + room, wide, title_h),
        rule: Panel::new(card.x, (card.y + head).floor(), card.w, 1.0),
        body: Panel::new(
            card.x + room,
            body_y,
            wide,
            (card.y + card.h - room - body_y).max(0.0),
        ),
        footer: nowhere(),
    }
}
