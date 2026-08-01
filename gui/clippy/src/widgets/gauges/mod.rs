//! The gauge vocabulary the three monitor panes and the context pane share:
//! extent, grid, and paint.

use noob_draw::{Panel, Run, Scene, Text};

#[allow(unused_imports)]
use crate::dock::View;
#[allow(unused_imports)]
use crate::monitor::Gauge;
#[allow(unused_imports)]
use crate::state::{State, Tone, TodoState};
#[allow(unused_imports)]
use crate::style::skin::Skin;
#[allow(clippy::wildcard_imports)]
use crate::view::*;



/// A monitor pane's content: one row per reading, in rows of the pane's own
/// pitch rather than of one line. See [`gauges`].
pub(crate) fn gauge_extent(frame: &Frame, panel: Panel, gauges: Vec<Gauge>) -> Option<(Vec<usize>, usize)> {
    if gauges.is_empty() {
        return None;
    }
    let grid = gauge_grid(
        &gauges,
        panel.inset(PAD),
        frame.pane_size,
        frame.pane_column,
    );
    Some((flat_heights(gauges.len()), grid.rows))
}

/// A label column, a block of dots, and the reading, laid out as three boxes
/// rather than as one padded string.
///
/// One string with the bar's room spelled as spaces was the first attempt, and
/// the readings landed on top of the bars: the spaces are the pane's column
/// width and the bar was drawn in the transcript's, which is a different
/// number. Three boxes at computed positions cannot drift apart.
///
/// The block is [`DOT_COLUMNS`] by [`DOT_ROWS`] dots in the metric's own colour,
/// filling row by row from the bottom, so a row is 25% and a dot is 1.25%. Wide
/// and short on purpose: see the constants. An unbounded reading draws no block
/// at all, where it used to draw an empty track, so most of a pane was empty
/// rectangles and the two rows that were filled read as noise. An unbounded row
/// keeps the line pitch, because a tall empty row would push the rows that do
/// have blocks off the bottom of the pane.
///
/// Twenty columns is a lot of width to ask a pane for, so the number is served
/// first and the block takes what is left. What is left can be nothing: a pane
/// dragged narrow enough that a dot would be under [`SMALL_DOT`] across draws no
/// blocks at all and every row becomes a label and a number, which is a row this
/// function already draws. That is the whole of the narrow case, and it is why
/// the readings themselves are never clipped or shrunk: a block is only ever
/// drawn in room the reading did not need.
///
/// The pane scrolls, so a reading past the bottom is reachable rather than
/// dropped. It used to stop drawing at the last row that fitted, which for the
/// hardware pane on a machine with two GPUs meant readings nothing could reach.
/// Every row is the same height ([`Grid::pitch`]) for that reason: the scroll
/// window is measured in rows of one height, and a pane whose rows differed could
/// not say how many of itself were on screen. The cost is that an unbounded row in
/// a pane that has blocks is as tall as a block row instead of one line, which is
/// a pane of evenly pitched rows rather than a pane of two pitches.
pub(crate) fn gauges(scene: &mut Scene, frame: &Frame, panel: Panel, view: View, gauges: Vec<Gauge>) {
    let skin = frame.skin;
    let content = panel.inset(PAD);

    if gauges.is_empty() {
        text_box(
            scene,
            frame,
            panel,
            frame.pane_size,
            vec![Run::tinted("sampling\u{2026}", skin.dim)],
        );
        return;
    }

    let grid = gauge_grid(&gauges, content, frame.pane_size, frame.pane_column);
    let heights = flat_heights(gauges.len());
    let scrolls = frame.scrolls;
    let window = scrolls.window(view, &heights, grid.rows);
    let (label_w, gap, dot) = (grid.label_w, grid.gap, grid.dot);
    let (block_h, pitch) = (grid.block_h, grid.pitch);
    let cell = dot + gap;
    let line = Text::line_for(frame.pane_size);

    let mut y = content.y;
    for gauge in gauges.iter().skip(window.first).take(window.count) {
        // No block in a pane with no room for one, so the row is the label and
        // the number, exactly as an unbounded reading is drawn.
        let fraction = gauge.fraction().filter(|_| grid.blocked);
        let row_h = pitch;
        let (lit, unlit, ink) = skin.gauge_slot(gauge.hue);
        scene.text(Text::rich(
            vec![Run::tinted(gauge.label, skin.dim)],
            Panel::new(
                content.x,
                y + ((row_h - line) * 0.5).floor(),
                label_w.max(1.0),
                line,
            ),
            frame.pane_size,
            skin.dim,
        ));
        // The metric's own colour, so the number and its block are one reading.
        // Nearly full is the one thing worth overriding it for: a block cannot
        // warn, because a metric whose hue is already red has nowhere to go.
        let tint = if fraction.is_some_and(|f| f > 0.85) {
            skin.bad
        } else {
            ink
        };
        let (size, at_x) = match fraction {
            Some(_) => (grid.reading, grid.read_x),
            None => (frame.pane_size, content.x + label_w),
        };
        let read_line = Text::line_for(size);
        scene.text(Text::rich(
            vec![Run::tinted(gauge.reading(), tint)],
            Panel::new(
                at_x,
                y + ((row_h - read_line) * 0.5).floor(),
                (content.x + content.w - at_x).max(1.0),
                read_line,
            ),
            size,
            tint,
        ));

        if let Some(fraction) = fraction {
            let filled = (fraction * (DOT_COLUMNS * DOT_ROWS) as f32).round() as usize;
            let top = y + ((row_h - block_h) * 0.5).floor();
            for index in 0..DOT_COLUMNS * DOT_ROWS {
                let (row, col) = (index / DOT_COLUMNS, index % DOT_COLUMNS);
                // Rows fill from the bottom, so the block reads as a level
                // rising rather than as a staircase. Every dot is drawn, lit or
                // not, which is what makes the block read as a block at 2%.
                scene.rect(
                    Panel::new(
                        content.x + label_w + col as f32 * cell,
                        top + block_h - (row + 1) as f32 * dot - row as f32 * gap,
                        dot,
                        dot,
                    )
                    .fill(if index < filled { lit } else { unlit })
                    .radius(0.5 * dot),
                );
            }
        }
        y += row_h;
    }
    scrollbar(scene, skin, panel, scrolls.thumb(view, &heights, grid.rows));
}

/// How a monitor pane's rows are sized, worked out once for the pane rather than
/// per row.
///
/// The wheel and the per-frame clamp need [`Grid::rows`] as much as the drawing
/// does, and a second copy of this arithmetic at the call site is how a pane comes
/// to scroll by a different number of rows than it drew.
struct Grid {
    /// The label column, as wide as the longest label in this pane.
    label_w: f32,
    dot: f32,
    gap: f32,
    /// Whether a block is drawn at all in this pane.
    blocked: bool,
    /// How tall the block is, or zero when it is not drawn. Its width is spent
    /// rather than carried: what a caller needs is where the reading starts,
    /// which is [`Grid::read_x`].
    block_h: f32,
    /// What every row of this pane is tall, block row or not.
    pitch: f32,
    /// The size a reading is drawn at, and where a bounded one starts.
    reading: f32,
    read_x: f32,
    /// How many rows of this pane are on screen.
    rows: usize,
}

fn gauge_grid(gauges: &[Gauge], content: Panel, size: f32, column: f32) -> Grid {
    let line = Text::line_for(size);
    // As wide as the longest label in this pane, so TOTAL TOOL CALLS is not
    // clipped and a pane of short labels does not pay for one that has none.
    let label_cols = gauges
        .iter()
        .map(|gauge| gauge.label.chars().count())
        .max()
        .unwrap_or(LABEL_COLUMNS)
        .max(LABEL_COLUMNS)
        + 1;
    let label_w = label_cols as f32 * column;
    let gap = (line * 0.12).round().max(1.0);
    // The number is served first: it gets the room its longest reading needs at
    // the pane's own size, and the block takes what is left, never more than half
    // of it and never less than a legible dot. A block that pushed the number off
    // the pane would be hiding the reading it exists to describe.
    let widest = gauges
        .iter()
        .filter(|gauge| gauge.fraction().is_some())
        .map(|gauge| gauge.reading().chars().count())
        .max()
        .unwrap_or(1)
        .max(1);
    let needed = widest as f32 * column;
    let free = (content.w - label_w - column).max(1.0);
    let room = (free - needed).max(0.0).min(free * 0.5);
    // As chunky as this pane can afford. A dot big enough to read as a block is
    // the point of the shape, but a pane of thirteen readings cannot spend the
    // same height per block as one of five. Past the floor the pane scrolls
    // instead of shrinking further, which is what item 14 asked for.
    //
    // Not clamped up to anything: a pane with no room for a legible dot is meant
    // to come out of here under [`SMALL_DOT`], which is what says no block.
    let mut dot = (line * 0.34)
        .round()
        .min((room / DOT_COLUMNS as f32 - gap).floor());
    let bounded = gauges.iter().any(|gauge| gauge.fraction().is_some());
    let tall = |dot: f32| {
        let block = dot * DOT_ROWS as f32 + gap * (DOT_ROWS - 1) as f32;
        gauges.len() as f32 * (block + 2.0 * gap).max(line)
    };
    while dot > SMALL_DOT && tall(dot) > content.h {
        dot -= 1.0;
    }
    // Whether this pane draws blocks at all. Either the dot is legible or the
    // pane is too narrow (or too short, since the loop above stops at the same
    // floor) to draw one, and then every reading is a number beside its label. A
    // pane with nothing bounded in it has no block to draw either way, and must
    // not pay a block's row height for the readings it does have.
    let blocked = bounded && dot >= SMALL_DOT;
    let (block_w, block_h) = match blocked {
        true => (
            (dot + gap) * DOT_COLUMNS as f32,
            dot * DOT_ROWS as f32 + gap * (DOT_ROWS - 1) as f32,
        ),
        false => (0.0, 0.0),
    };
    // The size of the number beside a block, which at [`BIG_READING`] of one is
    // the pane's own size and nothing else in this arithmetic bites. It is kept
    // because it is what makes a larger reading safe: capped at the room left
    // beside the block, so `1,048,576 / 2,097,152` in a pane dragged narrow comes
    // out smaller rather than clipped halfway through, which reads as a different
    // number. Floored, not rounded, because rounding up is what puts the last
    // character over the edge.
    let beside = (content.w - label_w - block_w - column).max(1.0);
    let reading = (size * BIG_READING)
        .min(size * beside / needed)
        .floor()
        .max(size);
    let pitch = (block_h + 2.0 * gap).max(Text::line_for(reading));
    Grid {
        label_w,
        dot,
        gap,
        blocked,
        block_h,
        pitch,
        reading,
        read_x: content.x + label_w + block_w + column,
        rows: (content.h / pitch).floor().max(0.0) as usize,
    }
}


/// The room the CONTEXT pane's readings get, under its header.
///
/// `None` when the pane is too short to hold even one reading under it. The
/// header itself does not scroll: it is four rows saying what this run is
/// doing and what it has asked for, and a monitor whose first rows scrolled
/// away would be a monitor with no summary.
pub(crate) fn gauge_area(panel: Panel, size: f32) -> Option<Panel> {
    let line = Text::line_for(size);
    let used = CONTEXT_HEAD as f32 * line + line * 0.5;
    if panel.h - used < line {
        return None;
    }
    Some(Panel::new(panel.x, panel.y + used, panel.w, panel.h - used))
}
