//! The call popup: one activity row opened out, full panel, scrolled.

use noob_draw::{Panel, Run, Scene, Text};

#[allow(unused_imports)]
use crate::dock::View;
#[allow(unused_imports)]
use crate::monitor::Gauge;
#[allow(unused_imports)]
use crate::state::{Call, Cell, State, Tone, TodoState};
#[allow(unused_imports)]
use crate::style::skin::Skin;
#[allow(clippy::wildcard_imports)]
use crate::view::*;

/// The extra room the popup keeps inside its box and inside each block.
const PAD: f32 = 10.0;
/// The tone bar's columns: the bar itself and the air after it.
const BAR_COLS: usize = 2;

/// How many text columns the popup's blocks wrap in, from the same box the
/// drawing uses, so the wheel, the track and the glyphs agree.
fn block_cols(box_: Panel, column: f32) -> usize {
    let inside = box_.inset(PAD);
    let text = inside.w - BAR_COLS as f32 * column - SCROLL_W - SCROLL_GAP * 2.0 - PAD;
    ((text / column.max(1.0)).floor() as usize).max(8)
}

/// One block's visual rows at this width: its label row and its wrapped lines.
fn block_rows(cell: &Cell, cols: usize) -> usize {
    1 + cell
        .lines
        .iter()
        .map(|text| text_geometry::rows_in(text, cols, crate::state::PANE_WRAP).len())
        .sum::<usize>()
}

/// The whole content's visual rows: every block and the blank row between
/// two of them.
fn content_rows(call: &Call, cols: usize) -> usize {
    let cells = call.cells();
    cells.iter().map(|cell| block_rows(cell, cols)).sum::<usize>()
        + cells.len().saturating_sub(1)
}

/// How far the popup can scroll and how many rows it shows: what the wheel,
/// the dragged track and the per-frame clamp all measure with. `None` while
/// no popup is up.
pub(crate) fn scroll_geometry(frame: &Frame) -> Option<(usize, usize)> {
    let call = frame.state.popped()?;
    let box_ = frame.layout.call_popup;
    if box_.w < 1.0 || box_.h < 1.0 {
        return None;
    }
    let line = Text::line_for(frame.pane_size);
    let fit = (((viewport(box_, line).h) / line).floor() as usize).max(1);
    Some((content_rows(call, block_cols(box_, frame.pane_column)), fit))
}

/// The box the blocks scroll in: under the header row, inside the padding.
fn viewport(box_: Panel, line: f32) -> Panel {
    let inside = box_.inset(PAD);
    Panel::new(
        inside.x,
        inside.y + line * 2.0,
        inside.w,
        (inside.h - line * 2.0).max(1.0),
    )
}

/// The popup itself: heading and close mark over a stack of blocks, one per
/// cell, each with a bar down its left in the cell's tone. On the floating
/// layer for the reason the menu is: a box on the base layer is painted
/// before every glyph in the window and comes out under the pane text.
pub(crate) fn popup(scene: &mut Scene, frame: &Frame) {
    let Some(call) = frame.state.popped() else {
        return;
    };
    let (skin, box_) = (frame.skin, frame.layout.call_popup);
    if box_.w < 1.0 || box_.h < 1.0 {
        return;
    }
    scene.over_rect(panel_fill(box_, skin.menu));
    scene.over_rect(panel_edge(box_, skin.edge_focus));
    let size = frame.pane_size;
    let line = Text::line_for(size);
    let inside = box_.inset(PAD);

    // The header: the tool, its target and how it stands, with the close
    // mark at the far end, the same close the settings panel has.
    scene.over_text(Text::rich(
        vec![Run::tinted(call.heading(), skin.bright)],
        Panel::new(inside.x, inside.y, (inside.w - line * 2.0).max(1.0), line),
        size,
        skin.bright,
    ));
    let close = frame.layout.call_popup_close;
    if close.w >= 1.0 {
        let ink = match frame.hot == Some(Hit::CallPopupClose) {
            true => skin.bad,
            false => skin.bright,
        };
        scene.over_text(Text::rich(
            vec![Run::icon(crate::design::icons::CLOSE.to_string(), ink)],
            close,
            size,
            ink,
        ));
    }

    // The blocks, in the window the scroll offset names. Measured and drawn
    // in the same columns, and a block crossing the edge is drawn partially
    // through the same `scrolled` the panes clip with, so nothing lands over
    // the header or past the box.
    let view = viewport(box_, line);
    let fit = ((view.h / line).floor() as usize).max(1);
    let cols = block_cols(box_, frame.pane_column);
    let cells = call.cells();
    let total = content_rows(call, cols);
    let scroll = frame.popup_scroll.min(total.saturating_sub(fit));
    let text_x = view.x + BAR_COLS as f32 * frame.pane_column;
    let mut at = 0usize;
    for (step, cell) in cells.iter().enumerate() {
        if step > 0 {
            at += 1;
        }
        let rows = block_rows(cell, cols);
        let (start, end) = (at, at + rows);
        at = end;
        if end <= scroll || start >= scroll + fit {
            continue;
        }
        let hidden = scroll.saturating_sub(start);
        let first = start.max(scroll);
        let shown = end.min(scroll + fit) - first;
        let y = view.y + (first - scroll) as f32 * line;
        let block = Panel::new(
            view.x,
            y,
            (view.w - SCROLL_W - SCROLL_GAP * 2.0).max(1.0),
            shown as f32 * line,
        );
        let tone = skin.tone(cell.tone);
        scene.over_rect(block.fill(skin.panel));
        scene.over_rect(
            Panel::new(block.x, block.y, MARK_W, block.h).fill(tone.map(|c| f32::from(c) / 255.0)),
        );
        let mut runs = vec![Run::tinted(cell.label, skin.dim), Run::plain("\n")];
        for entry in &cell.lines {
            runs.push(Run::tinted(entry, tone));
            runs.push(Run::plain("\n"));
        }
        scene.over_text(
            Text::rich(
                runs,
                Panel::new(text_x, y, cols as f32 * frame.pane_column, block.h),
                size,
                skin.body,
            )
            .wrap_at(cols)
            .scrolled(hidden as f32),
        );
    }

    // Its own track, on the floating layer with the box it scrolls, through
    // the same geometry every pane's bar is drawn and dragged with.
    if total > fit {
        let track = scroll_track(box_);
        let heights = flat_heights(total);
        let back = text_geometry::scrollback_for(&heights, fit, scroll);
        if let Some((top, size)) = text_geometry::thumb(&heights, fit, back) {
            scene.over_rect(track.fill(skin.scroll_track));
            scene.over_rect(
                Panel::new(
                    track.x,
                    track.y + track.h * top,
                    track.w,
                    (track.h * size).max(8.0).min(track.h),
                )
                .fill(skin.scroll_thumb),
            );
        }
    }
}
