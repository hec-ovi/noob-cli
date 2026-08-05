//! The drawing vocabulary the whole window shares: a filled panel, a cut
//! corner, a border, a selection band, a list of rows, a scrollbar, and the
//! text arithmetic that clips a line to the columns it has.
//!
//! Every box reaches for these through `use crate::view::*`, which is why they
//! are one file rather than spread through the painters that use them.

use noob_draw::{Panel, Rect, Run, Scene, Text};

use crate::dock::View;
use crate::style::skin::Skin;
#[allow(clippy::wildcard_imports)]
use super::*;

/// The body of a panel: the fill, cut corner and all.
///
/// The cut lives on the fill as well as on the outline because they are the
/// same shape twice. A square fill under a cut outline shows a triangle of the
/// wrong colour poking out of the corner.
pub(crate) fn panel_fill(panel: Panel, rgba: [f32; 4]) -> Rect {
    panel.fill(rgba).chamfer(CUT, Rect::TOP_RIGHT)
}
/// How far the cut actually reaches on a box this size.
///
/// The shader caps the reach at half the shorter side, so a short box loses a
/// smaller corner than [`CUT`]. Anything that has to stop where the cut starts
/// has to cap it the same way, or it stops short of a corner nothing took.
pub(crate) fn cut_of(panel: Panel) -> f32 {
    CUT.min(panel.w * 0.5).min(panel.h * 0.5).max(0.0)
}
/// A line along the cut itself, so the corner is a drawn edge rather than only a
/// missing one.
///
/// [`Rect::stroke`] follows the whole shape and cannot be asked for one side, so
/// a box that wants its diagonal without its other three sides has to draw the
/// diagonal itself. One rectangle per pixel row of the cut, `weight` wide and a
/// single pixel tall, each one starting a pixel further right than the row above
/// it: a stair whose thickness is read across, which is how a `weight` that is
/// not the hairline gets to be bolder without any two rectangles overlapping.
/// They must not overlap, because the colours these are drawn in are translucent
/// and two of them stacked composite darker than the straight edges they meet.
///
/// It runs from where the top edge would stop, `(right - cut, top)`, down to the
/// row the right edge starts on, `top + cut`. The last rows are clipped to the
/// box's right edge, so the stair narrows to a hairline exactly where a hairline
/// right edge picks it up.
pub(crate) fn cut_line(scene: &mut Scene, panel: Panel, rgba: [f32; 4], weight: f32) {
    let cut = cut_of(panel);
    if cut < weight {
        // A box squeezed smaller than the line is meant to be thick lost its
        // corner to the cap in `cut_of`; there is no diagonal left to draw.
        return;
    }
    let right = panel.x + panel.w;
    for row in 0..cut as usize {
        let at = row as f32;
        let x = right - cut + at;
        scene.rect(Panel::new(x, panel.y + at, weight.min(right - x), 1.0).fill(rgba));
    }
}
/// Its hairline border, as one rectangle. Four of them could not follow the
/// cut.
///
/// For a box that wants all four sides: the prompt, the picker, the menu. A
/// pane's body uses [`pane_edges`] instead, which leaves the top one out.
pub(crate) fn panel_edge(panel: Panel, rgba: [f32; 4]) -> Rect {
    panel_fill(panel, rgba).stroke(1.0)
}
/// A pane's border, minus the top edge.
///
/// That top edge was the line under every tab strip. A tab and the pane below it
/// are one surface (the same fill, and the strip is flush with the body), so a
/// hairline between them read as the pane being a box hung off the strip instead
/// of the strip being the top of the pane. The other three sides still tell two
/// panes over a busy desktop apart, so only the top one goes.
///
/// Thin rectangles rather than the one stroked rect [`panel_edge`] draws, because
/// a stroke follows the whole shape and cannot leave a side out. The three that
/// are left are straight lines, and the cut is on the top right, which is the
/// corner the top edge had.
///
/// The cut is bordered too ([`cut_line`]), in the same colour as the other three
/// sides and at [`CUT_EDGE_H`], twice their weight. Same colour because a pane is
/// one material and a corner in a second colour reads as a second thing stuck on
/// it; heavier because the diagonal is the mark that says what shape the pane is,
/// and a hairline down it was lost against three hairline sides.
pub(crate) fn pane_edges(scene: &mut Scene, panel: Panel, rgba: [f32; 4]) {
    let cut = cut_of(panel);
    scene.rect(panel.left_edge(rgba));
    scene.rect(panel.bottom_edge(rgba));
    scene.rect(
        Panel::new(
            panel.x + panel.w - 1.0,
            panel.y + cut,
            1.0,
            (panel.h - cut).max(0.0),
        )
        .fill(rgba),
    );
    cut_line(scene, panel, rgba, CUT_EDGE_H);
}
/// The band behind selected text, drawn before the glyphs go over it.
///
/// One rectangle per visible line of the selection rather than one for the
/// whole block, because the first and last lines start and stop mid-line and a
/// single rectangle would cover text that is not selected.
pub(crate) fn selection_band(scene: &mut Scene, frame: &Frame, panel: Panel, showing: Option<View>) {
    let (Some(selection), Some(view)) = (frame.selection, showing) else {
        return;
    };
    if selection.at != crate::select::Where::Pane(view) || selection.is_empty() {
        return;
    }
    let Some(pane) = frame.state.pane_of(view) else {
        return;
    };
    // The pane's own size, not the pane size for everything: the output pane is
    // drawn at the transcript size, and banding it at the smaller one is what
    // put the highlight off the glyphs it was supposed to cover.
    let (size, column) = frame.metrics_of(view);
    let content = panel.inset(PAD);
    let fit = frame.layout.rows(panel, size);
    // The transcript gives its bottom rows to the queued messages, so the
    // band is measured over the rows the text was really drawn in.
    let rows = match view {
        View::Output => fit - frame.state.output_reserved(fit),
        _ => fit,
    };
    // The columns the text is in and the columns in front of it, from the one
    // place that says so: the file view keeps four for its line numbers, and a
    // band measured in the full width of the box was four columns wide of the
    // glyphs on every row of a file.
    let (cols, chrome) = text_columns(view, panel, column);
    // Temporary: NOOB_BAND_DEBUG=1 prints what the band was computed from.
    if std::env::var("NOOB_BAND_DEBUG").is_ok() {
        let window = pane.window(rows, cols);
        eprintln!(
            "band: panel={:?} content={:?} size={size} column={column} fit={fit} rows={rows} \
             cols={cols} chrome={chrome} window=({},{},{}) selection={:?}",
            panel,
            content,
            window.first,
            window.count,
            window.skip,
            selection.range(),
        );
    }
    paint_selection(scene, selection, pane, Painted {
        content,
        rows,
        cols,
        chrome,
        size,
        column,
        tint: frame.skin.select,
    });
}
/// The rectangles behind one selection, over the pane that resolves it.
///
/// Split out of [`selection_band`] so the settings document bands with the same
/// arithmetic rather than with a second copy of it: the two boxes differ in
/// where they are and in nothing else, and a document highlighted by its own
/// rule would be a highlight the copy disagreed with.
pub(crate) fn paint_selection(
    scene: &mut Scene,
    selection: crate::select::Selection,
    pane: &crate::state::Pane,
    at: Painted,
) {
    let Painted {
        content,
        rows,
        cols,
        chrome,
        size,
        column,
        tint,
    } = at;
    let line_h = Text::line_for(size);
    let window = pane.window(rows, cols);
    let first = pane.showing_from(rows, cols);
    for step in 0..window.count {
        let number = first + step;
        let Some(line) = pane.line(number) else {
            continue;
        };
        // Counted in what is on screen: a Markdown line is drawn without its
        // marks, and a band measured on the source runs past the glyphs.
        let chars = line.shown().chars().count();
        let Some((from, to)) = selection.columns_on(number, chars) else {
            continue;
        };
        let Some((top, height)) = pane.band_of(rows, cols, number) else {
            continue;
        };
        // A wrapped line needs one rectangle per visual row, each covering only
        // the part of the selection that lands on that row. The first line in
        // the window may start partway down, which is what `skip` records.
        // Which characters a row holds comes from the pane, which is the same
        // answer the renderer breaks the rows by: a band drawn on its own
        // arithmetic is a highlight over text the clipboard does not have.
        let from_row = if step == 0 { window.skip } else { 0 };
        let spans = pane.rows_of_line(number, cols);
        for i in 0..height {
            let Some(span) = spans.get(from_row + i) else {
                continue;
            };
            let (row_start, row_end) = (span.start, span.end);
            let a = from.max(row_start);
            let b = to.min(row_end);
            if a >= b {
                continue;
            }
            // Past the chrome, which every row of the line carries: the gutter
            // on the first row and the indent under it on the rest. Measured in
            // columns rather than characters: an emoji is two of them, and a
            // band counted in characters covered six and a half of eight.
            let shown = line.shown();
            let into = text_geometry::columns_between(shown, row_start, a);
            let across = text_geometry::columns_between(shown, a, b);
            let x = content.x + (chrome + into) as f32 * column;
            let width = (across as f32 * column).min(content.x + content.w - x);
            let y = content.y + (top + i) as f32 * line_h;
            if width <= 0.0 || y + line_h > content.y + content.h {
                continue;
            }
            scene.rect(Panel::new(x, y, width, line_h).fill(tint));
        }
    }
}
pub(crate) fn text_box(scene: &mut Scene, frame: &Frame, panel: Panel, size: f32, runs: Vec<Run>) {
    scene.text(Text::rich(runs, panel.inset(PAD), size, frame.skin.body));
}
/// A pane that is a list of lines, scrolled inside its own box.
///
/// PLAN and AGENTS. Both drew every row they had, with no window and no bar,
/// into one text box that ran off the bottom of the pane. What was past the edge
/// could not be reached at all, which is what item 14 reported.
///
/// The window, the clamp and the thumb come from `text_geometry` through
/// [`crate::scroll::Scrolls`], the same numbers the transcript is drawn from, so a row of
/// a list and a row of a transcript mean the same thing. A line partly scrolled
/// off the top is drawn in full and offset by `skip` rather than dropped, which is
/// what lets a wrapped todo scroll a row at a time.
pub(crate) fn list_pane(scene: &mut Scene, frame: &Frame, panel: Panel, view: View, rows: Vec<ListRow>) {
    let size = frame.pane_size;
    let fit = frame.layout.rows(panel, size);
    let cols = cols_of(panel, frame.pane_column);
    let heights: Vec<usize> = rows.iter().map(|row| row.rows(cols)).collect();
    let scrolls = frame.scrolls;
    let window = scrolls.window(view, &heights, fit);
    let mut runs = Vec::new();
    for row in rows.into_iter().skip(window.first).take(window.count) {
        runs.extend(row.runs);
        runs.push(Run::plain("\n"));
    }
    if !runs.is_empty() {
        scene.text(
            Text::rich(runs, panel.inset(PAD), size, frame.skin.body)
                .scrolled(window.skip as f32)
                .wrap_at(cols),
        );
    }
    scrollbar(scene, frame.skin, panel, scrolls.thumb(view, &heights, fit));
}
/// Which list row is under a point of a list pane, by index into the same
/// rows the painter draws. The window, the skip and the heights come from
/// the one geometry [`list_pane`] draws with, so a press lands on the row
/// the eye is on however the list is scrolled or wrapped.
pub(crate) fn list_row_at(
    frame: &Frame,
    panel: Panel,
    view: View,
    rows: &[ListRow],
    x: f32,
    y: f32,
) -> Option<usize> {
    let inset = panel.inset(PAD);
    if !inset.contains(x, y) {
        return None;
    }
    let size = frame.pane_size;
    let line = Text::line_for(size);
    let fit = frame.layout.rows(panel, size);
    let cols = cols_of(panel, frame.pane_column);
    let heights: Vec<usize> = rows.iter().map(|row| row.rows(cols)).collect();
    let window = frame.scrolls.window(view, &heights, fit);
    let visual = ((y - inset.y) / line).floor().max(0.0) as usize + window.skip;
    let mut above = 0usize;
    for (at, tall) in heights.iter().enumerate().skip(window.first) {
        if visual < above + tall {
            return Some(at);
        }
        above += tall;
    }
    None
}
/// The bar down the right edge of a pane. Absent when everything fits, because
/// a scrollbar that is always full length says nothing.
/// Where a pane's scroll track runs: down the right edge, below the cut.
///
/// One function for the drawing, the hit band and the drag arithmetic, so a
/// press lands on the track exactly where the thumb is drawn and a dragged
/// fraction means the same place on both.
pub(crate) fn scroll_track(panel: Panel) -> Panel {
    // The track runs down the right edge, which is the edge the cut takes a
    // triangle out of. Starting it three pixels down put its head inside that
    // triangle, hanging in the air outside the pane, so it starts below the cut
    // instead: the cut reaches `cut` in from the corner along both edges, and
    // the track is already `SCROLL_GAP` in from the right.
    let head = (cut_of(panel) - SCROLL_GAP).max(3.0);
    Panel::new(
        panel.x + panel.w - SCROLL_W - SCROLL_GAP,
        panel.y + head,
        SCROLL_W,
        (panel.h - head - 3.0).max(1.0),
    )
}
pub(crate) fn scrollbar(scene: &mut Scene, skin: &Skin, panel: Panel, thumb: Option<(f32, f32)>) {
    let Some((top, size)) = thumb else {
        return;
    };
    let track = scroll_track(panel);
    scene.rect(track.fill(skin.scroll_track));
    scene.rect(
        Panel::new(
            track.x,
            track.y + track.h * top,
            track.w,
            (track.h * size).max(8.0).min(track.h),
        )
        .fill(skin.scroll_thumb),
    );
}
/// One row each, for a list of lines that are clipped rather than wrapped.
///
/// Written as heights and read through [`text_geometry`] so the window and the
/// clamp come from the one place that owns them.
pub fn flat_heights(count: usize) -> Vec<usize> {
    text_geometry::heights((0..count).map(|_| 0), 1)
}
/// How many whole rows of text a box holds. One at the least: a box too short
/// for a line still has a line in it, clipped, rather than dividing by nothing.
pub(crate) fn rows_in(box_: Panel, line: f32) -> usize {
    ((box_.h / line.max(1.0)).floor() as usize).max(1)
}
/// How many characters fit across a box of this width.
pub(crate) fn columns_in(width: f32, column: f32) -> usize {
    ((width / column.max(1.0)).floor() as usize).max(1)
}
/// How many characters fit across a panel's content box.
///
/// The one place a pane's width becomes a column count. Wrapping, hit testing
/// and the selection band all have to agree on this number, so they all ask
/// here rather than each dividing by the column width themselves.
pub(crate) fn cols_of(panel: Panel, column: f32) -> usize {
    columns_in(panel.inset(PAD).w, column)
}
pub(crate) fn clip(text: &str, chars: usize) -> String {
    let mut out: String = text.chars().take(chars).collect();
    if text.chars().count() > chars {
        out.push('\u{2026}');
    }
    out
}
/// The same, from the other end: the last `chars` characters, with a mark where
/// the front was cut off.
///
/// For a value whose end is the part that changes. A URL clipped from the left
/// keeps the port and the path, which is what somebody typing one is looking at;
/// clipped from the right it says `http://localho…` on every endpoint there is.
pub(crate) fn tail(text: &str, chars: usize) -> String {
    let count = text.chars().count();
    if count <= chars {
        return text.to_string();
    }
    let mut out = String::from("\u{2026}");
    out.extend(text.chars().skip(count - chars.saturating_sub(1)));
    out
}
/// The file name, and enough of its parent to tell two `mod.rs` apart.
pub fn short_name(path: &str) -> String {
    let parts: Vec<&str> = path.rsplit('/').take(2).collect();
    match parts.as_slice() {
        [] => String::new(),
        [name] => (*name).to_string(),
        [name, parent, ..] if *name == "mod.rs" || *name == "index.ts" || *name == "__init__.py" => {
            format!("{parent}/{name}")
        }
        [name, ..] => (*name).to_string(),
    }
}
/// How a view's text sits in its box: the columns it wraps in, and the columns
/// of chrome drawn in front of it.
///
/// The file view spends four columns on its line-number gutter. The gutter is
/// drawn as part of each row but is no part of the line, so wrapping has to
/// happen in what is left and a click has to have it taken off again. Both
/// numbers come from here, because the wrapping and the hit testing being
/// derived separately is what put file selection four columns out.
/// One line number, in exactly `chrome` columns.
///
/// Fixed width because the wrap is: the text of a file line starts `chrome`
/// columns in on its first row and on every row it continues onto, so a number
/// that took one column more would push the first row one character out from
/// the rows under it. Three digits and a blank is the usual answer; a file long
/// enough spends the blank, and one longer still says it was cut rather than
/// quietly showing a different line's number.
pub(crate) fn file_number(number: u32, chrome: usize) -> String {
    let digits = number.to_string();
    let width = chrome.saturating_sub(1);
    if digits.chars().count() <= width {
        // Zero padded, so a column of numbers reads as a column, and a blank
        // between the number and the text.
        return format!("{digits:0>width$} ");
    }
    // A file past that spends the blank, and past that says it was cut. `clip`
    // adds the mark on top of what it kept, so it is asked for one less.
    match digits.chars().count() <= chrome {
        true => digits,
        false => clip(&digits, width),
    }
}
pub fn text_columns(view: View, panel: Panel, column: f32) -> (usize, usize) {
    match view {
        View::Files => (cols_of(panel, column).saturating_sub(GUTTER).max(1), GUTTER),
        _ => (cols_of(panel, column), 0),
    }
}
/// How tall the prompt is: the rows it was set to, whatever is in it.
///
/// It used to be the rows it took to hold what had been typed, climbing to
/// `rows` a line at a time, and that is what this stopped doing. A box that
/// grows moves the conversation above it on the character that wraps a line and
/// is a different size every time you look at it; the setting is a height, so
/// three rows is three rows empty and three rows full. Past that the text
/// scrolls inside the box, which is what [`prompt_skip`] is for.
pub fn input_height(rows: usize, line: f32) -> f32 {
    // The strip, not the box inside it: the layout insets this by `GAP` before
    // the prompt gets it, and forgetting that cost the last row of a full one.
    (rows.max(1) as f32 * line + 2.0 * INPUT_PAD + 2.0 * GAP).max(INPUT_H)
}
