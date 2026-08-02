//! What the picker measures itself in: the rows it spends on its heading, the
//! room its field and its button take, the columns a row indents by, and the
//! session table's own widths.
//!
//! These lived in `view`, which reads none of them. They are the picker's
//! numbers, so they are the picker's file.

use noob_draw::Panel;

use crate::view::{clip, GAP, ROW_ICON_COLUMNS};

/// The rows the picker spends above its list on plain writing: the heading and
/// the folder it is listing. What has been typed sits under them in a bordered
/// field of its own, which is [`picker_field_h`] rather than one row.
pub(crate) const PICKER_HEAD_ROWS: f32 = 2.0;

/// How much taller the search field is than the line of text in it, on each
/// side.
///
/// The field carries the same cut corner and the same hairline every panel in
/// this window carries, and a box drawn tight around a line of text reads as a
/// line of text with a box round it rather than as something to type in.
pub(crate) const PICKER_FIELD_PAD: f32 = 4.0;

/// How far in from the left edge of a picker row its first mark sits.
///
/// A pane's row runs to its own edge because the band behind it is the width of
/// the pane. The picker's band is green and solid, so it needs an edge of its
/// own rather than starting under the glyph.
pub(crate) const PICKER_ROW_PAD: f32 = 5.0;

/// Columns a picker row spends on the mark that opens and shuts it, the mark
/// included and a space after it, and columns a step further into the tree costs.
///
/// Every row reserves the mark's column whether it has a mark or not, so the
/// folder glyphs line up in one column down the list instead of the ones with a
/// plus in front of them standing out of the ones without.
pub(crate) const PICKER_MARK_COLUMNS: usize = 2;
pub(crate) const PICKER_INDENT_COLUMNS: usize = 2;
/// The columns a row keeps for what it says, however deep it sits. Past this the
/// indent stops growing: a name at depth twelve pushed off the right of the box
/// is a row that says nothing.
pub(crate) const PICKER_LABEL_COLUMNS: usize = 12;

/// The session table's fixed columns: what each one is called at the top of the
/// list, and how many characters wide it is.
///
/// The last cell of a row, the opening line, is not here: it takes whatever is
/// left of the row, which is what makes the box's width worth having. Each cell
/// is written into one less column than it is given, so two full cells still
/// have a space between them and the table reads as columns rather than as one
/// run of words.
///
/// One row of a session list used to be a single string, `"5m ago  hec  first"`,
/// and item A7 is exactly the complaint that nobody could tell which part of it
/// was what.
pub(crate) const SESSION_COLUMNS: [(&str, usize); 4] = [
    ("when", 10),
    ("folder", 18),
    ("size", 10),
    ("context", 9),
];
/// What the last column is called. It has no width of its own.
pub(crate) const SESSION_OPENING: &str = "opening";

/// Where a session row's table starts and how many columns it has, for a row
/// panel `row` wide.
///
/// One answer for the header above the list and for every row in it, so the two
/// cannot come apart. In from the left by the same indent a folder row's mark
/// takes plus the row's own glyph and the space after it, because a session row
/// carries that glyph too and the table begins after it.
pub(crate) fn session_table(row: Panel, column: f32) -> (f32, usize) {
    let column = column.max(1.0);
    let x = row.x + PICKER_ROW_PAD + (PICKER_MARK_COLUMNS + ROW_ICON_COLUMNS + 1) as f32 * column;
    let room = ((row.x + row.w - x) / column).floor().max(0.0) as usize;
    (x, room)
}

/// One line of the session table: each cell written into its own columns, in
/// `room` columns altogether.
///
/// Space padded rather than drawn cell by cell. The list is monospace, so a
/// padded string is a table, and one shaped run per row is one run to tint: a
/// row of five texts on the cursor's green band would be five chances for one of
/// them to be tinted wrong.
pub(crate) fn session_line(cells: &[String], room: usize) -> String {
    let mut out = String::new();
    let mut left = room;
    for (step, cell) in cells.iter().enumerate() {
        if left == 0 {
            break;
        }
        let width = match SESSION_COLUMNS.get(step) {
            Some((_, wide)) => (*wide).min(left),
            // The last cell takes the rest of the row.
            None => left,
        };
        // Two columns short of its own, so a cell that fills its column still
        // has a space after it, and hard capped at the column either way: in a
        // window too narrow for a column the ellipsis would be the character
        // that ran over.
        let text = clip(cell, width.saturating_sub(2).max(1));
        let text: String = text.chars().take(width).collect();
        let written = text.chars().count();
        out.push_str(&text);
        // Not after the last cell: trailing spaces are columns nobody sees, and
        // they would push a clipped opening line past the end of the row.
        if step + 1 < cells.len() {
            for _ in written..width {
                out.push(' ');
            }
        }
        left -= width;
    }
    out
}

/// What the picker calls itself, in both of its lists.
///
/// One string rather than a heading that swapped between OPEN A FOLDER and OPEN
/// A SESSION. The title says what the box is for; which of the two lists is in
/// front of you is said by the pair of buttons under it, and a title that also
/// said it was a second thing to read for an answer already on screen.
pub(crate) const PICKER_TITLE: &str = "OPEN FOLDER OR CONTINUE SESSION";

/// What the picker says on the button that opens the row the cursor is on.
///
/// It used to spell out the folder that would be opened, which made the button
/// as wide as a path and made it change width every time the cursor moved. The
/// path is already written above the list. "selected" rather than the folder or
/// the session, because one button opens whichever of the two the cursor is on.
pub(crate) const PICKER_OPEN_LABEL: &str = "Open selected";

/// The two buttons that choose which list is showing.
///
/// Both are drawn in a box sized for the longer of the two, so the pair does not
/// shuffle sideways when the list swaps, and the one whose list is showing is
/// filled in the colour the chosen row is filled in.
pub(crate) const PICKER_SESSIONS_LABEL: &str = "Sessions";
pub(crate) const PICKER_FOLDERS_LABEL: &str = "Folders";

/// How much taller that button is than the line of text in it, on each side.
///
/// A button reads as a button because there is room around what it says. The
/// same string with a hairline drawn around it reads as a label with a box.
pub(crate) const PICKER_OPEN_PAD: f32 = 5.0;

/// How tall the picker's list is allowed to get, in rows, and how short.
///
/// Bounds on the window, not on the folder. The box takes as many rows as there
/// is room for between these two and then holds that height whatever it is
/// listing. It used to take as many rows as the folder had entries, so walking
/// from a folder with three subfolders into one with forty resized the dialog
/// and recentred it under the pointer, moving every row while the pointer was
/// still on one of them. A short folder now gets empty rows under its list,
/// which is the cheaper of the two: a box that does not move is worth more than
/// a box with no whitespace in it.
pub(crate) const PICKER_MIN_ROWS: usize = 6;
pub(crate) const PICKER_MAX_ROWS: usize = 24;

/// How tall the Open button is for text of this line height.
pub(crate) fn picker_open_h(line: f32) -> f32 {
    line + PICKER_OPEN_PAD * 2.0
}

/// How tall the search field is, for the same line height.
pub(crate) fn picker_field_h(line: f32) -> f32 {
    line + PICKER_FIELD_PAD * 2.0
}

/// What the picker keeps above its list: the row of buttons, a gap,
/// [`PICKER_HEAD_ROWS`] of writing, the search field, and a gap between that
/// field and the first row.
///
/// One answer, the way [`picker_foot`] is one answer for the bottom, and it does
/// not read the picker: the head is the same height on the folder list and on
/// the session list, so swapping between the two cannot move the box.
///
/// The buttons moved up here from the foot and the foot kept the line of keys,
/// so this is the same total as before and the list holds the same number of
/// rows it always did.
pub(crate) fn picker_head_h(line: f32) -> f32 {
    picker_open_h(line) + GAP + PICKER_HEAD_ROWS * line + picker_field_h(line) + GAP
}

/// What the picker keeps below its list: the line of keys. One answer, so the
/// box that is measured and the rows that are drawn into it cannot disagree
/// about where the bottom is.
pub(crate) fn picker_foot(line: f32) -> f32 {
    line
}
