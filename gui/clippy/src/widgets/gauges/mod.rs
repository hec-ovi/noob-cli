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
use crate::widgets::context::CONTEXT_HEAD;
use crate::widgets::LABEL_COLUMNS;


/// A gauge is a block of dots: twenty across and four down is 0 to 100 percent,
/// so one row is 25 percent and one dot is 1.25.
///
/// Wide and short, which is the shape the panes were asked for. Eight by five
/// was the shape before and it stood the hardware pane on end: six readings each
/// five rows tall is a column of stacks, tall and narrow, in a pane that has
/// width to spare and no height. Twenty dots to a row also puts a usable
/// resolution on one row, a dot being a percent and a quarter, so a reading
/// climbing under load moves dot by dot instead of in fifths of a row.
pub(crate) const DOT_COLUMNS: usize = 20;
pub(crate) const DOT_ROWS: usize = 4;

/// How much larger the number beside a block is than the label. One: the same
/// size as every other glyph in the window.
///
/// It was one and a half, and at that size the readings were the loudest thing on
/// screen, which is not what a monitor is for. The metric's own tint is what says
/// a number is the thing being read. The arithmetic that caps it against the room
/// beside the block is kept, so raising this again cannot put a reading over the
/// edge of a narrow pane.
pub(crate) const BIG_READING: f32 = 1.0;

/// The smallest a dot shrinks to, across or down, when a pane has more readings
/// than room. Below this the block stops reading as a block, so it is not drawn:
/// too tall for its rows and they scroll off, too narrow for its columns and the
/// pane draws numbers alone. A reading that scrolled off is true and a number
/// with no block is true; a smear is not.
pub(crate) const SMALL_DOT: f32 = 4.0;

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

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::wildcard_imports)]
    use crate::view::testkit::*;
    use crate::dock::{Dock, Space};
    use crate::config::Config;
    use crate::monitor::Monitor;
    use crate::state::State;

    /// The bug this replaced: the bar's room was spelled as spaces in the
    /// pane's font while the bar itself was drawn in the transcript's column
    /// width, so the readings landed on top of the bars.
    ///
    /// The bar is a block of dots now, so the thing the reading has to clear is
    /// every dot of it. Found by fill rather than by size: a dot is a few pixels
    /// square, which no size filter can tell from a hairline.
    #[test]
    fn a_monitor_reading_never_lands_on_its_block() {
        let mut state = State::new();
        state.apply(noob_proto::Event::UsageReport {
            usage: noob_proto::Usage {
                prompt: 5869,
                cached_prompt: 5348,
                completion: 40,
                context_total: 65536,
            },
        });
        let mut monitor = Monitor::new();
        monitor.sample(&state);
        monitor.sample(&state);

        let mut dock = Dock::new();
        dock.reveal(View::Context);
        // Deliberately mismatched: the transcript's columns are wider than the
        // pane's, which is the situation that produced the overlap.
        for (column, pane_column) in [(8.4, 7.8), (7.8, 8.4), (8.0, 8.0)] {
            let shape = sized_shape(&dock, column, pane_column);
            let layout = Layout::compute(1400.0, 900.0, &shape);
            let skin = Skin::from(&Config::default());
            let scene = build(&Frame {
                state: &state,
                scrolls: &crate::scroll::Scrolls::default(),
                file_scroll: 0,
                monitor: &monitor,
                dock: &dock,
                skin: &skin,
                layout: &layout,
                prompt: &crate::prompt::Prompt::default(),
                column,
                pane_column,
                body_size: 14.0,
                pane_size: 13.0,
                clock: 0.0,
                orb_morph: None,
                drag: None,
                hot: None,
                trouble: None,
                esc_armed: false,
            popup_scroll: 0,
            cursor: (-100.0, -100.0),
                selection: None,
                menu: None,
                picker: None,
            settings: None,
            });
            let body = layout.placed(Space::TopRight).body;
            let hues: Vec<[f32; 4]> = skin
                .gauges
                .iter()
                .chain(skin.gauges_unlit.iter())
                .copied()
                .collect();
            let dots: Vec<[f32; 4]> = scene
                .rects
                .iter()
                .filter(|r| hues.contains(&r.rgba()) && body.contains(r.xywh()[0], r.xywh()[1]))
                .map(|r| r.xywh())
                .collect();
            assert!(!dots.is_empty(), "no dots were drawn");
            for [_, _, w, h] in &dots {
                assert_eq!(w, h, "a dot is square so its radius rounds it off");
            }
            let block_right = dots.iter().map(|[x, _, w, _]| x + w).fold(0.0f32, f32::max);
            let reading = scene
                .texts
                .iter()
                .find(|t| {
                    body.contains(t.at.x, t.at.y)
                        && t.runs.iter().any(|r| r.text.contains('/'))
                })
                .expect("the bounded reading is on screen");
            assert!(
                reading.at.x >= block_right,
                "reading at {} overlaps a block ending at {block_right} ({column}/{pane_column})",
                reading.at.x
            );
        }
    }
    /// Twenty dots across and four down, so a row is 25% and a dot is 1.25%. 525
    /// of 1000 tokens is 52.5%, which is two whole rows and two dots of a third,
    /// filling from the bottom the way a level meter does. Every dot is drawn
    /// either way, so the block reads as a block rather than as a scatter.
    ///
    /// This asserted eight across and five down, and before that ten columns of
    /// four in one shared gauge colour. The shape is the width and height of the
    /// block: five rows of eight stood the panes on end, which is what item 13
    /// reported, and the same forty dots wide and short is the same reading in a
    /// shape a pane has room for.
    #[test]
    fn a_gauge_is_a_block_of_dots_in_the_metric_s_own_colour() {
        let mut state = State::new();
        state.apply(noob_proto::Event::UsageReport {
            usage: noob_proto::Usage {
                prompt: 525,
                cached_prompt: 0,
                completion: 0,
                context_total: 1000,
            },
        });
        let mut monitor = Monitor::new();
        monitor.sample(&state);

        let mut dock = Dock::new();
        dock.reveal(View::Context);
        let shape = shape(&dock, &[]);
        let layout = Layout::compute(1400.0, 900.0, &shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state: &state,
            scrolls: &crate::scroll::Scrolls::default(),
            file_scroll: 0,
            monitor: &monitor,
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock: 0.0,
            orb_morph: None,
            drag: None,
            hot: None,
            trouble: None,
            esc_armed: false,
            popup_scroll: 0,
            cursor: (-100.0, -100.0),
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        });

        // CONTEXT is the only bounded reading in this pane with anything in it,
        // and its hue is nobody else's, so filtering by that colour isolates the
        // one block under test.
        let context = monitor
            .context()
            .into_iter()
            .find(|gauge| gauge.key == "context")
            .expect("the context reading");
        let (lit, unlit, ink) = skin.gauge_slot(context.hue);
        let body = layout.placed(Space::TopRight).body;
        let of = |color: [f32; 4]| -> Vec<[f32; 4]> {
            scene
                .rects
                .iter()
                .filter(|r| r.rgba() == color && body.contains(r.xywh()[0], r.xywh()[1]))
                .map(|r| r.xywh())
                .collect()
        };
        let dots = of(lit);
        assert_eq!((DOT_COLUMNS, DOT_ROWS), (20, 4), "the shape item 13 asked for");
        assert_eq!(dots.len(), 42, "52.5% of 80 dots");
        assert_eq!(
            of(unlit).len(),
            DOT_COLUMNS * DOT_ROWS - 42,
            "the rest of the block is still drawn, faintly"
        );

        // Rows, not columns: 42 dots is two full rows of twenty and two of a
        // third, and the part-filled row is the top one.
        let mut rows: Vec<f32> = dots.iter().map(|[_, y, _, _]| *y).collect();
        rows.sort_by(f32::total_cmp);
        rows.dedup();
        assert_eq!(rows.len(), 3);
        let across = |y: f32| dots.iter().filter(|[_, dy, _, _]| *dy == y).count();
        assert_eq!(
            rows.iter().map(|y| across(*y)).collect::<Vec<_>>(),
            vec![2, DOT_COLUMNS, DOT_COLUMNS],
            "the part-filled row is at the top"
        );
        // Evenly pitched, or the block reads as a random scatter.
        let pitch = rows[1] - rows[0];
        for pair in rows.windows(2) {
            assert!((pair[1] - pair[0] - pitch).abs() < 0.01, "{rows:?}");
        }

        // Wider than it is tall, which is the whole of the shape complaint: the
        // block used to be a stack.
        let left = dots.iter().map(|[x, _, _, _]| *x).fold(f32::MAX, f32::min);
        let right = dots.iter().map(|[x, _, w, _]| x + w).fold(0.0f32, f32::max);
        let top = rows[0];
        let foot = of(lit)
            .iter()
            .chain(of(unlit).iter())
            .map(|[_, y, _, h]| y + h)
            .fold(0.0f32, f32::max);
        assert!(
            right - left > 2.0 * (foot - top),
            "the block is {} by {}",
            right - left,
            foot - top
        );

        // The number is the metric's colour and the pane's own size. It was one
        // and a half times the pane size, which read as the loudest thing in the
        // window; the tint is what says it is the reading.
        let reading = scene
            .texts
            .iter()
            .find(|t| t.runs.iter().any(|r| r.text.contains("525 / 1,000")))
            .expect("the context reading is written out");
        assert_eq!(reading.runs[0].color, Some(ink));
        assert_eq!(reading.size, 13.0, "the reading is not the pane size");

        // And an unbounded reading draws no block at all: no track, no dots, and
        // the number where the block would have started.
        let calls = scene
            .texts
            .iter()
            .find(|t| t.runs.iter().any(|r| r.text == "TOTAL TOOL CALLS"))
            .expect("an unbounded row");
        let row = Panel::new(body.x, calls.at.y, body.w, calls.at.h);
        assert!(
            !scene
                .rects
                .iter()
                .any(|r| row.contains(r.xywh()[0], r.xywh()[1] + 0.5 * r.xywh()[3])),
            "something was drawn on the row of an unbounded reading"
        );
    }
    /// The reading is never squeezed: it is the pane's own size at every width,
    /// and it always fits the box it was given. What gives instead is the block,
    /// which is drawn in the room the reading did not need.
    ///
    /// This asserted that a narrow pane drew the number smaller, which was the
    /// answer while the number was one and a half times the pane size and could
    /// afford to lose the difference. At the pane size there is nothing to give
    /// back, so twenty columns of dots go instead.
    #[test]
    fn the_reading_keeps_its_size_and_the_block_gives_way() {
        let mut state = State::new();
        state.apply(noob_proto::Event::UsageReport {
            usage: noob_proto::Usage {
                prompt: 1_048_576,
                cached_prompt: 0,
                completion: 0,
                context_total: 2_097_152,
            },
        });
        let mut monitor = Monitor::new();
        monitor.sample(&state);
        let mut dock = Dock::new();
        dock.reveal(View::Context);

        let mut blocks = Vec::new();
        for width in [1600.0, 760.0] {
            let out = render_with(&state, width, 900.0, &dock, &[], &monitor, None);
            let reading = out
                .scene
                .texts
                .iter()
                .find(|t| t.runs.iter().any(|r| r.text.contains("1,048,576 /")))
                .expect("the context reading is on screen");
            // The box it was given has to hold it: a monospace column at this
            // size is the pane's column scaled by the size it is drawn at.
            let chars = reading
                .runs
                .iter()
                .map(|r| r.text.chars().count())
                .sum::<usize>() as f32;
            let column = 8.0 * reading.size / 13.0;
            assert!(
                chars * column <= reading.at.w + 0.01,
                "{width}: {chars} columns of {column} do not fit {}",
                reading.at.w
            );
            assert_eq!(reading.size, 13.0, "{width}: not the pane size");
            blocks.push(dots_in(&out, Space::TopRight).len());
        }
        assert_eq!(blocks[0], DOT_COLUMNS * DOT_ROWS, "a whole block fits at 1600");
        assert_eq!(
            blocks[1], 0,
            "the narrow pane drew {} dots rather than none",
            blocks[1]
        );
    }
    /// A pane with no room for a legible block draws none of it, and says so by
    /// drawing the reading where an unbounded one goes. The alternative is twenty
    /// dots two pixels wide, which is a texture rather than a level, in the room
    /// the number needed to be read at all.
    ///
    /// Every reading is still on the pane either way. Losing the block is not
    /// losing the number.
    #[test]
    fn a_pane_too_narrow_for_a_block_draws_no_block_and_keeps_the_numbers() {
        let state = busy_state();
        let mut monitor = Monitor::new();
        monitor.sample(&state);
        monitor.sample(&state);
        let mut dock = Dock::new();
        dock.reveal(View::Context);

        // Wide enough for a block, and narrow enough that a dot would be under
        // SMALL_DOT across. 680 is the window's own minimum size.
        let wide = render_with(&state, 1400.0, 900.0, &dock, &[], &monitor, None);
        assert_eq!(
            dots_in(&wide, Space::TopRight).len(),
            DOT_COLUMNS * DOT_ROWS,
            "one whole block at 1400"
        );
        for [_, _, w, _] in dots_in(&wide, Space::TopRight) {
            assert!(w >= SMALL_DOT, "a {w} pixel dot is a smear");
        }

        let narrow = render_with(&state, 680.0, 500.0, &dock, &[], &monitor, None);
        assert!(
            dots_in(&narrow, Space::TopRight).is_empty(),
            "a block was drawn in a pane with no room for one"
        );
        let text = text_of(&narrow.scene);
        for label in ["CONTEXT", "TOTAL REQUESTS", "LAST PREFILL"] {
            assert!(text.contains(label), "{label} left the narrow pane: {text}");
        }
        assert!(text.contains("1,816 / 65,536"), "the fill still reads: {text}");
    }
}
