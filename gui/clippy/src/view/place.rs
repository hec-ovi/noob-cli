//! Where everything lands: the grid the panes share, the strip a space's tabs
//! are laid along, the file explorer's two columns, and the floors nothing is
//! measured below.
//!
//! [`Layout::compute`] stays in `view` with the record it fills; this file is
//! the arithmetic it calls.

use noob_draw::{Panel, Text};

#[allow(clippy::wildcard_imports)]
use super::*;

/// The four cells of the grid, in [`Space::ALL`] order, from the box the panes
/// share and the two ratios.
///
/// One line across the whole box and then a line inside each half of it, so the
/// left column and the right one can break at heights of their own. Which axis
/// gets the single line is `rows_first`: a grid reading in columns is cut down
/// the middle once and each column is then cut across, and a grid reading in
/// rows is cut across once and each row is then cut down.
///
/// The pair on the axis with the single line is not read past its first number.
/// It is still carried, because turning the grid round makes the second one live
/// again and a number that was thrown away comes back as the default rather than
/// as where the line was left.
///
/// Every ratio is held off the same floors the dividers are dragged with, so a
/// cell is never narrower than a file view can be read in nor shorter than a
/// gauge can be drawn in.
pub(crate) fn grid_cells(
    body: Panel,
    rows_first: bool,
    left_width: [f32; 2],
    top_height: [f32; 2],
    column_floor: f32,
) -> [Panel; 4] {
    let room_w = (body.w - GAP).max(0.0);
    let room_h = (body.h - GAP).max(0.0);
    // The width of the left cell of a row, and the height of the top cell of a
    // column. One answer for every row, or for every column, on whichever axis
    // the single line runs.
    let left_w = |row: usize| {
        let ratio = left_width[if rows_first { row } else { 0 }];
        (room_w * held(ratio, room_w, column_floor)).floor()
    };
    let top_h = |column: usize| {
        let ratio = top_height[if rows_first { 0 } else { column }];
        (room_h * held(ratio, room_h, MIN_SPACE_H)).floor()
    };
    let cell = |row: usize, column: usize| {
        let (left, top) = (left_w(row), top_h(column));
        let (x, w) = match column {
            0 => (body.x, left),
            _ => (body.x + left + GAP, body.w - left - GAP),
        };
        let (y, h) = match row {
            0 => (body.y, top),
            _ => (body.y + top + GAP, body.h - top - GAP),
        };
        Panel::new(x, y, w, h)
    };
    [cell(0, 0), cell(1, 0), cell(0, 1), cell(1, 1)]
}
pub(crate) fn empty_placed() -> Placed {
    Placed {
        strip: nowhere(),
        body: nowhere(),
        tabs: Vec::new(),
        first_tab: 0,
        arrow_left: nowhere(),
        arrow_right: nowhere(),
    }
}
/// The box around two boxes. A box with nothing in it is not a corner of the
/// answer: two cells and one empty cell would otherwise reach back to the
/// window's origin.
pub(crate) fn around(a: Panel, b: Panel) -> Panel {
    if a.w < 1.0 || a.h < 1.0 {
        return b;
    }
    if b.w < 1.0 || b.h < 1.0 {
        return a;
    }
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    Panel::new(
        x,
        y,
        (a.x + a.w).max(b.x + b.w) - x,
        (a.y + a.h).max(b.y + b.h) - y,
    )
}
pub(crate) fn nowhere() -> Panel {
    Panel::new(0.0, 0.0, 0.0, 0.0)
}
/// The file view's two columns, and where each visible row of the list is.
///
/// The list is as wide as the longest name it holds and no wider, capped twice:
/// at [`LIST_MAX_COLUMNS`], and at whatever leaves the file [`DIFF_MIN_COLUMNS`]
/// to be read in. The file is the thing being looked at, so it is the half with
/// the floor; below the size where even that cannot be met the two split what
/// there is, because a pane that hid either half would be worse than a cramped
/// one.
pub(crate) fn place_files(body: Panel, shape: &Shape) -> (Panel, Panel, Vec<(usize, Panel)>) {
    if body.w < 1.0 || body.h < 1.0 {
        return (nowhere(), nowhere(), Vec::new());
    }
    // Nothing touched yet: no column, no divider, and the pane says so where the
    // file would be.
    if shape.file_labels.is_empty() {
        return (nowhere(), body, Vec::new());
    }
    let column = shape.pane_column.max(1.0);
    let total = cols_of(body, column);
    let widest = shape
        .file_labels
        .iter()
        .map(|label| label.chars().count())
        .max()
        .unwrap_or(0);
    let want = (widest + ROW_ICON_COLUMNS + ROW_MARK_COLUMNS).clamp(LIST_MIN_COLUMNS, LIST_MAX_COLUMNS);
    // Two columns cost two sets of margins, so the split itself spends columns
    // before either half gets a character. Leaving that out is how the file
    // ended up two columns under its floor at the smallest window size.
    let split_cost = (PAD * 2.0 / column).ceil() as usize;
    let cols = match total.checked_sub(DIFF_MIN_COLUMNS + split_cost) {
        Some(room) if room >= 1 => want.min(room),
        // Nothing to protect at this size. The two halves split what there is,
        // rather than one of them disappearing: a file view with no list cannot
        // be navigated, and a list with no file shows nothing.
        _ => (total / 2).max(1),
    };
    let (list, diff) = body.split_left((cols as f32 * column + PAD * 2.0).min(body.w));

    let line = Text::line_for(shape.pane_size);
    let content = list.inset(PAD);
    let rows = Text::rows_for(shape.pane_size, content.h);
    let heights = file_heights(shape.file_labels.len());
    let back = text_geometry::scrollback_for(&heights, rows, shape.file_first);
    let window = text_geometry::window(&heights, rows, back);
    let panels = (0..window.count)
        .map(|step| {
            // The full width of the column, so the whole row answers the click
            // the way a row of an explorer does, not just the characters of the
            // name.
            let index = window.first + step;
            (
                index,
                Panel::new(list.x, content.y + step as f32 * line, list.w, line),
            )
        })
        .collect();
    (list, diff, panels)
}
/// Where the activity popup sits: the whole surface under the title strip,
/// a margin in from every edge.
///
/// It was a floating note sized to its lines. Full panel now, so the two
/// halves have the room a long command and a stack trace need, and the
/// window behind it stops competing with them. Each half scrolls on its own.
pub(crate) fn place_popup(width: f32, height: f32) -> Panel {
    let margin = 2.0 * GAP;
    Panel::new(
        margin,
        TITLE_H + margin,
        (width - margin * 2.0).max(1.0),
        (height - TITLE_H - margin * 2.0).max(1.0),
    )
}
/// Lay tabs left to right at the width their labels need, dropping any that do
/// not fit rather than squeezing them into unreadable slivers.
///
/// A strip that cannot hold all of its tabs keeps room for two arrows at its
/// right end and shows a window of tabs starting at `first`, so nothing is a
/// sliver and nothing is unreachable either. The room comes off before the window
/// of tabs is chosen: reserving it afterwards would push one more tab off the
/// edge, which is the same complaint one tab further along. A strip that fits
/// them all gets no arrows and loses no room to them.
///
/// `first` is a request rather than an instruction, and is answered twice. It is
/// clamped so the tabs at the end of the strip always fill it, because a space
/// left scrolled past its last tab (by a resize, or by closing the tabs it was
/// scrolled to) would show an empty strip. Then it is moved far enough that
/// `active` is on screen, because the pane below the strip belongs to that tab
/// and a pane whose own tab is missing cannot be read. Both answers are given
/// here, on every frame, rather than at each of the several places that can move
/// a tab or resize a window: a rule that runs every time cannot be forgotten by
/// the next thing that moves a tab.
///
/// That second rule is why the strip's arrows walk the showing tab as well as the
/// strip ([`crate::main`]'s `walk_tabs`): a scroll that left the showing tab
/// behind would be undone here, on the same frame.
pub(crate) fn strip_tabs(
    bar: Panel,
    widths: &[usize],
    column: f32,
    first: usize,
    active: Option<usize>,
) -> Strip {
    let each: Vec<f32> = widths
        .iter()
        .map(|chars| (*chars as f32 + 3.0) * column)
        .collect();
    // As many tabs as fit in `room`, starting at `from`.
    let lay = |from: usize, room: f32| -> Vec<Panel> {
        let mut out = Vec::new();
        let mut x = bar.x;
        for w in each.iter().skip(from) {
            if x + w > bar.x + room {
                break;
            }
            out.push(Panel::new(x, bar.y, *w, bar.h));
            x += w;
        }
        out
    };
    let plain = |room: f32| Strip {
        tabs: lay(0, room),
        first: 0,
        left: nowhere(),
        right: nowhere(),
    };
    let total: f32 = each.iter().sum();
    if total <= bar.w {
        return plain(bar.w);
    }
    let arrow = TAB_ARROW_COLUMNS as f32 * column;
    let room = bar.w - arrow * 2.0;
    // A strip too narrow to hold the arrows and the widest of its tabs both keeps
    // the tabs: two arrows over an empty strip are a control for reaching
    // nothing. Measured against the widest rather than the narrowest so that
    // every offset shows at least one tab, which is what makes the clamp below
    // enough on its own. The window has no size where this happens: the widest
    // label is eleven columns and the narrowest strip is over thirty.
    if each.iter().copied().fold(0.0, f32::max) > room {
        return plain(bar.w);
    }
    // The furthest it can be scrolled: past this the tabs at the end no longer
    // fill it and the strip is showing gap.
    let mut furthest = each.len().saturating_sub(1);
    let mut used = 0.0;
    for (i, w) in each.iter().enumerate().rev() {
        used += w;
        if used > room {
            break;
        }
        furthest = i;
    }
    let mut at = first.min(furthest);
    if let Some(active) = active {
        // Behind the window, the strip starts at the showing tab; ahead of it,
        // it walks forward until that tab is in view. `max(1)` is only there so
        // an offset showing no tabs cannot spin the loop; the check above rules
        // that out for every offset a caller can reach.
        at = at.min(active);
        while active >= at + lay(at, room).len().max(1) {
            at += 1;
        }
    }
    let x = bar.x + bar.w - arrow * 2.0;
    Strip {
        tabs: lay(at, room),
        first: at,
        left: Panel::new(x, bar.y, arrow, bar.h),
        right: Panel::new(x + arrow, bar.y, arrow, bar.h),
    }
}
/// One strip's tabs, and the arrows for reaching the ones that did not fit.
pub(crate) struct Strip {
    /// The tabs on screen, left to right, starting at `first`.
    pub(crate) tabs: Vec<Panel>,
    pub(crate) first: usize,
    /// Both `nowhere()` when every tab fits.
    pub(crate) left: Panel,
    pub(crate) right: Panel,
}
/// One row per file, as heights the scroll window can be taken from.
///
/// The explorer clips a name that does not fit rather than wrapping it, so a row
/// is always exactly one row. That is what keeps a click from resolving to a
/// different file than the one under the pointer. Written as heights, and read
/// through [`text_geometry`], so the window and the clamp come from the one
/// place that owns them rather than from arithmetic at two call sites.
pub fn file_heights(count: usize) -> Vec<usize> {
    flat_heights(count)
}
/// The least width a column can be dragged down to, for text drawn at `column`
/// pixels a character: [`DIFF_MIN_COLUMNS`] of them, which is the floor the file
/// view already refuses to go below (a line-number gutter and twenty columns of
/// code beside it), plus the [`PAD`] on either side of a pane's content.
///
/// It moves with the font size, because the floor is about columns of text
/// rather than about pixels: the same 24 columns cost more room at 20 point.
pub(crate) fn min_column_w(column: f32) -> f32 {
    DIFF_MIN_COLUMNS as f32 * column.max(1.0) + PAD * 2.0
}
/// A divider's ratio, held so neither side of it ends up smaller than `floor`.
///
/// `room` is what the two sides share once the gap between them is taken off. A
/// box with no room for two floors and the gap splits down the middle: both
/// sides are then equally short of what they wanted, which is the only answer
/// that does not collapse one of them to nothing.
pub(crate) fn held(ratio: f32, room: f32, floor: f32) -> f32 {
    if room <= floor * 2.0 {
        return 0.5;
    }
    let edge = floor / room;
    ratio.clamp(edge, 1.0 - edge)
}
/// Whether a point is inside the notch cut out of a box's top right corner:
/// pixels the box does not paint and must not answer for.
///
/// The shader removes every point where the distance in from the right and the
/// distance down from the top add up to less than the cut ([`cut_of`]). Nothing
/// else in this window asks: [`Panel::contains`] is a bare rectangle and every
/// other surface has the pane behind it, so a click in the notch lands on the
/// same thing it looks like it landed on. A menu floats over the window and
/// takes the click before anything else, so its notch answered for its first
/// row: a press on transparent pixels opened the settings panel.
pub(crate) fn in_cut(panel: Panel, x: f32, y: f32) -> bool {
    (panel.x + panel.w - x) + (y - panel.y) < cut_of(panel)
}
/// Which edge, if any, a point is on. An undecorated window loses the window
/// manager's resize handles, so these are ours to provide.
pub fn edge(x: f32, y: f32, width: f32, height: f32) -> Option<winit::window::ResizeDirection> {
    use winit::window::ResizeDirection as Dir;
    let left = x <= RESIZE_EDGE;
    let right = x >= width - RESIZE_EDGE;
    let top = y <= RESIZE_EDGE;
    let bottom = y >= height - RESIZE_EDGE;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(Dir::NorthWest),
        (_, true, true, _) => Some(Dir::NorthEast),
        (true, _, _, true) => Some(Dir::SouthWest),
        (_, true, _, true) => Some(Dir::SouthEast),
        (true, ..) => Some(Dir::West),
        (_, true, ..) => Some(Dir::East),
        (_, _, true, _) => Some(Dir::North),
        (_, _, _, true) => Some(Dir::South),
        _ => None,
    }
}
/// How tall a window has to be to be a title strip and nothing else.
///
/// What shading asks the window for, in the space the layout works in. That
/// space is physical pixels: [`Layout::compute`] is handed the surface
/// configuration `noob-gpu` reports, which is `Window::inner_size` verbatim, and
/// nothing between winit and here applies a scale factor. So the number a window
/// is asked for is this number, not this number through a conversion.
///
/// [`TITLE_H`] and never less than the line the strip writes, because a strip
/// too short to draw its own name is not a strip. Whole pixels, rounded up: a
/// window is asked for in integers and a request half a pixel short would come
/// back half a pixel short.
pub fn strip_height() -> f32 {
    TITLE_H.max(Text::line_for(SMALL)).ceil()
}
/// The box the title strip writes one line into, given the strip it actually
/// has.
///
/// Every run in the strip goes through here, so none of them can be written
/// outside the surface. glyphon clips a run to the surface as well as to the box
/// it was given, so a 17 pixel line centred in a 30 pixel box is drawn nowhere
/// at all once the surface comes back 12 pixels tall: the strip kept its bar and
/// lost the name, the version, the build stamp and all three window buttons,
/// which is every glyph it has. A strip shorter than a line keeps its line at
/// the top and gives it every pixel there is instead, because the writing is
/// what a strip is for and is the last thing that should go.
pub(crate) fn strip_row(panel: Panel) -> Panel {
    panel.row(0.0, Text::line_for(SMALL))
}
