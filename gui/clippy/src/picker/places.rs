//! Where everything on the startup chooser sits: the field, the tree rows,
//! marks, and the two buttons, from one panel and the shape.

use noob_draw::{Panel, Text};

#[allow(unused_imports)]
use crate::design::{self, icons};
use crate::picker::Picker;
#[allow(clippy::wildcard_imports)]
use crate::picker::metrics::*;
#[allow(clippy::wildcard_imports)]
use crate::view::*;


/// Where the picker's pieces are. A struct rather than a tuple, the way the
/// settings panel's are: five panels in a row is a call site nobody can read.
pub(crate) struct PickerPlaces {
    pub(crate) box_: Panel,
    pub(crate) list: Panel,
    pub(crate) rows: Vec<(usize, Panel)>,
    pub(crate) marks: Vec<(usize, Panel)>,
    pub(crate) open: Panel,
    pub(crate) filter: Panel,
    pub(crate) folders: Panel,
    pub(crate) sessions: Panel,
}

/// How far in from the left of a picker row its mark sits, and how wide that
/// mark is, for a row at this depth in the tree.
///
/// One answer, so the region a press is tested against and the glyph that is
/// drawn cannot end up in two places. The indent stops growing once the label
/// is down to [`PICKER_LABEL_COLUMNS`]: a deep tree in a narrow box would
/// otherwise push its names off the right of the list.
pub(crate) fn picker_indent(depth: usize, column: f32, cols: usize) -> (f32, f32) {
    let column = column.max(1.0);
    let room = cols
        .saturating_sub(PICKER_LABEL_COLUMNS + PICKER_MARK_COLUMNS + ROW_ICON_COLUMNS + 1)
        / PICKER_INDENT_COLUMNS.max(1);
    let steps = depth.min(room);
    (
        PICKER_ROW_PAD + (steps * PICKER_INDENT_COLUMNS) as f32 * column,
        PICKER_MARK_COLUMNS as f32 * column,
    )
}

/// How much of the region that answers for the mark in front of a folder the
/// mark itself is drawn in, and the least it is ever drawn at.
///
/// Well under half of it. The mark used to be a filled glyph as tall as the row,
/// which made it the loudest thing on a row whose point is the folder's name,
/// and a solid block is a state rather than a control.
const PICKER_MARK_SIDE: f32 = 0.6;
const PICKER_MARK_MIN: f32 = 5.0;

/// The box that mark is drawn in, centred in the region that answers for
/// pressing it.
///
/// An odd side, so the plus inside it has a middle column and a middle row to
/// sit on. An even one puts the two bars off centre by half a pixel each and the
/// mark reads as a lower-case t.
pub(crate) fn picker_mark_box(mark: Panel) -> Panel {
    let side = (mark.w.min(mark.h) * PICKER_MARK_SIDE)
        .floor()
        .max(PICKER_MARK_MIN);
    let side = match side as i32 % 2 {
        0 => side + 1.0,
        _ => side,
    };
    Panel::new(
        mark.x + ((mark.w - side) * 0.5).floor(),
        mark.y + ((mark.h - side) * 0.5).floor(),
        side,
        side,
    )
}

/// The folder picker's box, its list, the rows on screen, the mark that opens
/// and shuts each of them, and its button.
///
/// Centred in `area` and no wider than [`PICKER_COLUMNS`], because the thing
/// being read is a column of folder names: stretched across a 2200 pixel window
/// the eye has to travel the whole width to get from a name to the button under
/// it.
///
/// One shape, and it is not the folder's shape. The height is chosen from the
/// room the window has, between [`PICKER_MIN_ROWS`] and [`PICKER_MAX_ROWS`], and
/// then held: `picker` says what goes in the box and never how big it is. That
/// is why nothing here reads `picker.rows().len()`. Walking into a folder with a
/// different number of entries used to resize and recentre the whole dialog
/// under the pointer, so every row moved out from under the click that was about
/// to happen.
pub(crate) fn place_picker(area: Panel, shape: &Shape, picker: &Picker) -> PickerPlaces {
    if area.w < 1.0 || area.h < 1.0 {
        return PickerPlaces {
            box_: nowhere(),
            list: nowhere(),
            rows: Vec::new(),
            marks: Vec::new(),
            open: nowhere(),
            filter: nowhere(),
            folders: nowhere(),
            sessions: nowhere(),
        };
    }
    let column = shape.pane_column.max(1.0);
    let line = Text::line_for(shape.pane_size);
    let head = picker_head_h(line);
    let foot = picker_foot(line);
    // Everything the box spends on something other than its list.
    let chrome = PAD * 2.0 + head + GAP + foot;
    let fits = ((area.h - chrome) / line).floor().max(0.0) as usize;
    let want = fits.clamp(PICKER_MIN_ROWS, PICKER_MAX_ROWS);
    let w = (PICKER_COLUMNS as f32 * column + PAD * 2.0).min(area.w);
    let h = (chrome + want as f32 * line).min(area.h);
    let box_ = Panel::new(
        area.x + ((area.w - w) * 0.5).floor(),
        area.y + ((area.h - h) * 0.5).floor(),
        w,
        h,
    );
    let content = box_.inset(PAD);
    // The session list keeps one line above itself for the row that names its
    // columns. Taken out of the list rather than added to the head, so the box
    // is the same box in both lists and only the rows inside it move: a head
    // that changed height with the mode would move the whole dialog every time
    // the Sessions button was pressed.
    let header = match picker.on_sessions() {
        true => line,
        false => 0.0,
    };
    // Never past the room the box has for it: the head, the field and the
    // button all want a height of their own, and in a window too short for
    // them the list would otherwise start below the bottom of the box with the
    // field above it sized as if that room were there.
    let list = Panel::new(
        content.x,
        (content.y + head + header)
            .min(content.y + content.h - foot - GAP)
            .max(content.y),
        content.w,
        (content.h - head - header - foot - GAP).max(0.0),
    );
    let rows_fit = Text::rows_for(shape.pane_size, list.h);
    let heights = picker.heights();
    let back = text_geometry::scrollback_for(&heights, rows_fit, picker.first());
    let window = text_geometry::window(&heights, rows_fit, back);
    let rows: Vec<(usize, Panel)> = (0..window.count)
        .map(|step| {
            // The full width of the list, so the whole row answers the click the
            // way a row of a file manager does, not just the characters of the
            // name.
            let index = window.first + step;
            (
                index,
                Panel::new(list.x, list.y + step as f32 * line, list.w, line),
            )
        })
        .collect();
    // A mark only where there is a folder to open: the folder being listed, the
    // way out of it, a folder remembered from an earlier session and the message
    // under a folder that could not be read are not branches of the tree.
    let cols = cols_of(list, column);
    let marks = rows
        .iter()
        .filter_map(|(index, row)| {
            let entry = picker.row(*index)?;
            entry.open()?;
            let (indent, wide) = picker_indent(entry.depth(), column, cols);
            Some((*index, Panel::new(row.x + indent, row.y, wide, row.h)))
        })
        .collect();
    // The three buttons, all on the head's first row and all the same height.
    //
    // Open sits at the right limit of the box and the two that choose the list
    // sit at the left, so the button that acts on the row the cursor is on is
    // the furthest thing on the row from the two that only change what is being
    // listed. It used to be the other way round, with Open and the swap side by
    // side in the bottom left corner, which put a button that starts a session
    // one gap away from a button that does not.
    let button_h = picker_open_h(line).min(content.h);
    // Exactly as wide as what it says: the confirm glyph, the space after it,
    // [`PICKER_OPEN_LABEL`], a column of indent on the left and two on the right
    // so the cut corner never reaches the text.
    let open_w = ((ROW_ICON_COLUMNS + 1 + PICKER_OPEN_LABEL.chars().count() + 3) as f32 * column)
        .min(content.w);
    let open = Panel::new(
        content.x + content.w - open_w,
        content.y,
        open_w,
        button_h,
    );
    // The pair at the left, both sized for the longer of the two words, so
    // swapping the list does not move either of them, and both clipped to what
    // is left of the row once Open and a gap on either side of the pair have
    // been taken off: in a box too narrow for all three there are no mode
    // buttons rather than buttons sticking out of the picker.
    let mode_room = (content.w - open_w - GAP * 2.0).max(0.0);
    let mode_w = ((ROW_ICON_COLUMNS
        + 1
        + PICKER_SESSIONS_LABEL
            .chars()
            .count()
            .max(PICKER_FOLDERS_LABEL.chars().count())
        + 3) as f32
        * column)
        .min(((mode_room - GAP) * 0.5).max(0.0));
    let (folders, sessions) = match mode_w >= 1.0 {
        true => (
            Panel::new(content.x, content.y, mode_w, button_h),
            Panel::new(content.x + mode_w + GAP, content.y, mode_w, button_h),
        ),
        false => (nowhere(), nowhere()),
    };
    // The search field, under the buttons and the two lines of writing and above
    // the list. Its own panel rather than a rectangle worked out where the text
    // is drawn, so the border, the icon and what has been typed all come off one
    // shape.
    //
    // Its height is the room between the writing and the list, not the height a
    // field would like to be: in a window short enough that the head has no
    // room, a field taking its own height is drawn over the first rows of the
    // list. It ends up at nothing at that size, which is a picker with no field
    // rather than a field over the list. Its top is held inside the box for the
    // same reason: a field with no height still has a position, and a position
    // below the box is one the head of a short window would otherwise hand it.
    let field_top = (content.y + button_h + GAP + PICKER_HEAD_ROWS * line)
        .min(content.y + content.h)
        .max(content.y);
    let filter = Panel::new(
        content.x,
        field_top,
        content.w,
        picker_field_h(line).min((list.y - header - GAP - field_top).max(0.0)),
    );
    PickerPlaces {
        box_,
        list,
        rows,
        marks,
        open,
        filter,
        folders,
        sessions,
    }
}
