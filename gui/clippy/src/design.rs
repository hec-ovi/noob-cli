//! The window's space scale and type scale, as arithmetic.
//!
//! `gui/DESIGN.md` is the prose version of this file. It exists because the
//! settings panel is being rebuilt one section at a time: two people building
//! two sections have to reach for the same four gaps and the same five text
//! sizes, or the panel comes out as the flat list of same-sized rows it was.
//!
//! Everything here is derived from one number, the pane text size, so raising
//! that setting scales the whole panel instead of making the text overrun the
//! boxes it is drawn in.
//!
//! Two rules govern the arithmetic, and both have already cost a day when they
//! were ignored:
//!
//! - A row's height is counted in whole lines of pane text by
//!   [`crate::settings::lines`] and drawn in pixels by `view::place_settings`.
//!   Every vertical token here is therefore a fraction of one line, the model
//!   adds those fractions up and rounds the total up to a whole line, and the
//!   layout multiplies the same fractions by the same line height. The two
//!   cannot drift, and the rounding only ever leaves slack.
//! - A width in columns is a width in columns of one text size. Text drawn at
//!   another size is clipped with a column width derived for that size
//!   ([`column_for`]) and never measured against a rectangle anything is
//!   pressed on, because only the pane size's advance is really measured.

/// Space, as a fraction of one line of pane text. Four tokens and nothing in
/// between them: `TIGHT` between a label and the thing it labels, `STEP`
/// between two fields, `ROOM` inside a card on all four sides, `APART` between
/// one card and the next. At the size the window opens at they are 4, 8, 14 and
/// 20 pixels.
pub const TIGHT: f32 = 0.2;
pub const STEP: f32 = 0.4;
pub const ROOM: f32 = 0.7;
pub const APART: f32 = 1.0;

pub fn tight(line: f32) -> f32 {
    line * TIGHT
}

pub fn step(line: f32) -> f32 {
    line * STEP
}

pub fn room(line: f32) -> f32 {
    line * ROOM
}

pub fn apart(line: f32) -> f32 {
    line * APART
}

/// Type: five roles, five sizes, as multiples of the pane size. Nothing on a
/// panel is drawn at a size that is not one of these.
///
/// The panel title is the one heading across the top; a card title is the bar
/// of a card; a label names a field and a value is what the field holds, both
/// at the pane size; a hint is the sentence under a field.
pub const PANEL_TITLE: f32 = 1.6;
pub const CARD_TITLE: f32 = 1.15;
pub const LABEL: f32 = 1.0;
pub const VALUE: f32 = 1.0;
pub const HINT: f32 = 0.85;

pub fn panel_title_size(size: f32) -> f32 {
    role_size(size, PANEL_TITLE)
}

pub fn card_title_size(size: f32) -> f32 {
    role_size(size, CARD_TITLE)
}

pub fn hint_size(size: f32) -> f32 {
    role_size(size, HINT)
}

/// The name of a field, and what the field holds. Both the pane size today,
/// and both named rather than written as `size` so a change to either role is
/// a change in one place.
pub fn label_size(size: f32) -> f32 {
    role_size(size, LABEL)
}

pub fn value_size(size: f32) -> f32 {
    role_size(size, VALUE)
}

/// A whole pixel size for one role, never smaller than a pixel: a font size of
/// zero is a run of glyphs with no bounds, which the shaper answers with
/// nothing at all.
fn role_size(size: f32, scale: f32) -> f32 {
    (size * scale).round().max(1.0)
}

/// The advance of one character at another role's size.
///
/// Only the pane size's advance is measured off the real face
/// (`renderer.column_width`), so a second size has to be scaled from it. Good
/// enough to clip a line with, which is all it is used for: no hit region is
/// measured in these columns, because a target that is a fraction wide of what
/// is drawn is a target that misses.
pub fn column_for(column: f32, size: f32, role: f32) -> f32 {
    match size > 0.0 {
        true => column * role_size(size, role) / size,
        false => column,
    }
}

/// How many lines of pane text a card's title band is allowed.
///
/// A card title is drawn at [`CARD_TITLE`], whose own line height is a little
/// over one line of pane text at every size the panel is used at. It claims a
/// quarter of a line more than that so the title has air rather than sitting on
/// the divider under it.
pub const TITLE_LINES: f32 = 1.25;

/// One line of anything else inside a card. A hint is drawn smaller and still
/// claims a whole line: half a line of slack is not worth the arithmetic of
/// counting a fifth of one.
pub const TEXT_LINES: f32 = 1.0;

/// The height of a button, which is what a footer is.
pub const BUTTON_LINES: f32 = 1.0;

/// How tall a card is, in lines of pane text, for a body that tall.
///
/// Header, then the body with [`ROOM`] on every side of it, then the footer
/// when the card has actions. The divider under the title has no height of its
/// own: it is a hairline drawn in the air the header already claimed.
pub fn card_lines(body: f32, footer: bool) -> f32 {
    let header = ROOM + TITLE_LINES + ROOM;
    let footer = match footer {
        true => BUTTON_LINES + ROOM,
        false => 0.0,
    };
    header + ROOM + body + ROOM + footer
}

/// How many whole lines the row a card stands in takes: the card itself, and
/// the [`APART`] under it that separates it from the next one.
///
/// Rounded up, so the rectangle the layout draws the card in is never shorter
/// than the card needs. What the rounding leaves over is slack at the bottom of
/// the card, which is where the footer already sits.
pub fn card_row_lines(body: f32, footer: bool) -> usize {
    (card_lines(body, footer) + APART).ceil().max(1.0) as usize
}

/// How tall one field is, in lines: its label, the input under it, and the
/// sentence under that when it has one.
pub fn field_lines(hint: bool) -> f32 {
    let hint = match hint {
        true => TIGHT + TEXT_LINES,
        false => 0.0,
    };
    TEXT_LINES + TIGHT + TEXT_LINES + hint
}

/// Where every field of a card body sits, in lines: the top of the band it
/// stands in, and how tall that band is.
///
/// One band per row of fields, `across` of them to a band, [`STEP`] between one
/// band and the next. A band is as tall as the tallest field in it, which is
/// what a sentence under one of them costs: two fields side by side sit on the
/// same lines, so their labels line up and the band under them is where the
/// model counted it.
///
/// `hints` is one flag per field, in order, saying whether it carries a
/// sentence. The model counts a card's height with this and the layout places
/// the fields with it, which is what keeps the counted height and the drawn one
/// the same height.
pub fn field_slots(hints: &[bool], across: usize) -> Vec<(f32, f32)> {
    let across = across.max(1);
    let mut out = Vec::with_capacity(hints.len());
    let mut top = 0.0;
    for band in hints.chunks(across) {
        let tall = field_lines(band.iter().any(|hint| *hint));
        for _ in band {
            out.push((top, tall));
        }
        top += tall + STEP;
    }
    out
}

/// How tall those fields come to, in lines, with nothing under them. Zero for a
/// card with no fields at all, which is a card that is all sentence.
pub fn fields_lines(hints: &[bool], across: usize) -> f32 {
    field_slots(hints, across)
        .last()
        .map(|(top, tall)| top + tall)
        .unwrap_or(0.0)
}

/// Columns a card's border and its two [`ROOM`] paddings take off the list it
/// stands in.
///
/// Four, which is more than [`ROOM`] twice comes to in columns at every size
/// the panel is used at: a column is around six tenths of the text size and
/// `ROOM` is around one. The number is deliberately generous because it is what
/// decides the width a description wraps in, and text measured in one width and
/// drawn in another is a row measured at one height and drawn at another.
pub const CARD_COLUMNS: usize = 4;

/// How wide a card's body is, in columns, inside a list `cols` wide.
pub fn card_cols(cols: usize) -> usize {
    cols.saturating_sub(CARD_COLUMNS)
}

/// The least a field keeps, in columns, before the card stops putting two of
/// them across.
///
/// Twenty four, which is the label column the panel has always used: a field
/// narrower than its own label is a field whose name is three dots.
pub const FIELD_COLUMNS: usize = 24;

/// The gap between two fields side by side, in columns. [`STEP`] is under a
/// column wide at every size, and a column is the honest way to spend it in an
/// arithmetic that is counting columns.
pub const STEP_COLUMNS: usize = 1;

/// How many fields go across a card body `cols` wide.
///
/// Two when both keep [`FIELD_COLUMNS`], one when they cannot. This is the
/// whole of the reflow: cards are full width and stack, and it is a card's own
/// contents that answer a narrow window.
pub fn across(fields: usize, cols: usize) -> usize {
    match fields >= 2 && cols + CARD_COLUMNS >= reflow_columns() {
        true => 2,
        false => 1,
    }
}

/// The width in columns, of the list a card stands in, at which the card flips
/// from one field across to two.
pub fn reflow_columns() -> usize {
    FIELD_COLUMNS * 2 + STEP_COLUMNS + CARD_COLUMNS
}

/// The least one colour of the palette keeps, in columns: the block itself and
/// the longest thing said beside it ("gtt and generated", "running a command").
pub const SWATCH_COLUMNS: usize = 22;

/// The most colours that go across, however wide the window is.
///
/// A palette read in more columns than this is a wall again: the eye stops
/// finding a row and starts scanning a field of blocks. Six is where the longest
/// group (fourteen tools) still comes to three rows.
pub const SWATCH_ACROSS_MOST: usize = 6;

/// How many colours go across a card body `cols` wide.
///
/// The palette was chunked three to a row whatever the window was, because the
/// model was never handed a width. It is handed one now ([`crate::settings::lines`]
/// takes the list's columns), so the grid reflows the way a card's fields do:
/// as many as keep [`SWATCH_COLUMNS`], one at the narrowest, never more than
/// [`SWATCH_ACROSS_MOST`].
pub fn swatch_across(cols: usize) -> usize {
    (cols / SWATCH_COLUMNS).clamp(1, SWATCH_ACROSS_MOST)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four gaps are the four the design says they are at the size the
    /// window opens at, and every one of them is a whole number of pixels
    /// there: a scale whose tokens land on half pixels draws hairlines that
    /// blur.
    #[test]
    fn the_space_scale_is_four_eight_fourteen_and_twenty() {
        let line = 20.0;
        assert_eq!(tight(line), 4.0);
        assert_eq!(step(line), 8.0);
        assert_eq!(room(line), 14.0);
        assert_eq!(apart(line), 20.0);
    }

    /// Every role is bigger than the one under it, and the two that are not the
    /// pane size are really a different size at every size the panel is used
    /// at: a scale that collapses at small text is one size again.
    #[test]
    fn the_type_scale_is_five_sizes_at_every_pane_size() {
        for size in 8..=40 {
            let size = size as f32;
            let title = panel_title_size(size);
            let card = card_title_size(size);
            let hint = hint_size(size);
            assert!(title > card, "{size}: {title} is not over {card}");
            assert!(card > size, "{size}: a card title is not over its labels");
            assert!(hint < size, "{size}: a hint is not under its value");
            assert_eq!(title, title.round(), "{size}: a size off the pixel grid");
        }
    }

    /// A card claims whole lines, always at least what it draws, and a footer
    /// costs it a line and its air.
    #[test]
    fn a_card_claims_whole_lines_and_never_fewer_than_it_draws() {
        for body in [1.0, 2.2, 3.6, 7.4] {
            for footer in [false, true] {
                let rows = card_row_lines(body, footer);
                assert!(
                    rows as f32 >= card_lines(body, footer) + APART,
                    "{body} lines of body in {rows} rows"
                );
                assert!(rows as f32 - 1.0 < card_lines(body, footer) + APART + 1.0);
            }
            assert!(
                card_row_lines(body, true) > card_row_lines(body, false)
                    || card_lines(body, true) > card_lines(body, false),
                "a footer costs nothing"
            );
        }
    }

    /// Two fields on one band sit on the same lines, a band with a sentence in
    /// it is taller than one without, and the bands add up to what
    /// [`fields_lines`] says the body is. A band measured at one height and
    /// drawn at another is every press below it on the wrong field.
    #[test]
    fn the_fields_of_a_card_stand_in_bands_that_add_up_to_the_body() {
        let hints = [false, true, false];
        let across = field_slots(&hints, 2);
        assert_eq!(across[0].0, across[1].0, "two across are not on one band");
        assert_eq!(across[0].1, across[1].1, "two across are not one height");
        assert_eq!(across[0].1, field_lines(true), "the sentence costs nothing");
        assert!(across[2].0 >= across[0].0 + across[0].1 + STEP - 0.001);
        assert_eq!(across[2].1, field_lines(false));
        assert_eq!(fields_lines(&hints, 2), across[2].0 + across[2].1);

        // Stacked, every field is its own band, and the whole is taller than it
        // was two across.
        let down = field_slots(&hints, 1);
        assert_eq!(down.len(), 3);
        assert!(down[1].1 > down[0].1, "the sentence is not a line taller");
        assert!(fields_lines(&hints, 1) > fields_lines(&hints, 2));
        assert_eq!(fields_lines(&[], 2), 0.0, "an empty body claims a band");
    }

    /// The palette reflows with the window instead of chunking at three
    /// whatever the width is, and it never goes so wide that a row stops being
    /// a row.
    #[test]
    fn the_palette_goes_as_wide_as_the_card_it_stands_in() {
        assert_eq!(swatch_across(0), 1, "a card with no width still draws one");
        assert_eq!(swatch_across(SWATCH_COLUMNS - 1), 1);
        assert_eq!(swatch_across(SWATCH_COLUMNS), 1);
        assert_eq!(swatch_across(SWATCH_COLUMNS * 3), 3);
        assert_eq!(swatch_across(SWATCH_COLUMNS * 40), SWATCH_ACROSS_MOST);
        // Wider is never narrower, or the grid would jump about as the window
        // is dragged.
        let mut was = 0;
        for cols in 0..400 {
            let across = swatch_across(cols);
            assert!(across >= was, "{cols} columns fits fewer than {}", cols - 1);
            was = across;
        }
    }

    /// Two fields go across only while both keep their columns, and the flip is
    /// at the width [`reflow_columns`] names.
    #[test]
    fn two_fields_go_across_exactly_while_both_keep_their_columns() {
        let flip = reflow_columns();
        assert_eq!(across(2, card_cols(flip)), 2);
        assert_eq!(across(2, card_cols(flip - 1)), 1);
        assert_eq!(across(1, card_cols(flip)), 1, "one field is one column");
        assert!(card_cols(flip) >= FIELD_COLUMNS * 2);
    }
}
