//! The activity pane: one clipped row per tool call, with its popup anchor.

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

pub(crate) fn activity(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    let rows = frame.layout.rows(panel, frame.pane_size);
    let cols = cols_of(panel, frame.pane_column);
    // The call row under the pointer brightens: the list says these rows
    // press by lighting the one the hand is on, with no band behind it.
    let lit = hovered_call_row(frame, panel, rows, cols);
    let mut runs = Vec::new();
    for (step, line) in state.activity.visible(rows, cols).iter().enumerate() {
        let absolute = state.activity.showing_from(rows, cols) + step;
        let ink = match lit == Some(absolute) {
            true => skin.bright,
            false => skin.tone(line.tone),
        };
        // A clipped pane: each entry is the row the wrap would have drawn
        // first, with an ellipsis after it when the entry goes on. The span
        // is the pane's own (`rows_of_line`), so what is drawn is what a
        // press or a selection there resolves to.
        let span = state
            .activity
            .rows_of_line(absolute, cols)
            .first()
            .copied()
            .unwrap_or(text_geometry::Row { start: 0, end: 0 });
        let whole = line.shown().chars().count();
        // The clock column in front of a row is the subordinate part of it
        // and is drawn in the dim tone: the eye reads the list by tag and
        // subject, and a time in the call's own color would compete.
        let gutter = line.gutter.min(span.end);
        let head: String = line.shown().chars().take(gutter).collect();
        let rest: String = line.shown().chars().take(span.end).skip(gutter).collect();
        if !head.is_empty() {
            runs.push(Run::tinted(head, skin.dim));
        }
        runs.push(Run::tinted(rest, ink));
        if span.end < whole && span.end < cols {
            runs.push(Run::tinted("\u{2026}", skin.dim));
        }
        runs.push(Run::plain("\n"));
    }
    scene.text(
        Text::rich(runs, panel.inset(PAD), frame.pane_size, frame.skin.body)
            .scrolled(state.activity.window(rows, cols).skip as f32)
            .wrap_at(cols),
    );
    scrollbar(scene, skin, panel, state.activity.thumb(rows, cols));
}

/// The absolute line of the call row under the pointer, or nothing: over
/// empty space, over a row no call owns, or while the popup covers the pane.
fn hovered_call_row(frame: &Frame, panel: Panel, rows: usize, cols: usize) -> Option<usize> {
    if frame.state.open_call.is_some() {
        return None;
    }
    let (cx, cy) = frame.cursor;
    let inset = panel.inset(PAD);
    if !inset.contains(cx, cy) {
        return None;
    }
    let line = Text::line_for(frame.pane_size);
    let row = ((cy - inset.y) / line).floor().max(0.0) as usize;
    let (absolute, _) = frame.state.activity.spot_in(rows, cols, row, 0)?;
    frame.state.call_at_line(absolute).map(|_| absolute)
}
