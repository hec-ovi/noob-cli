//! The activity pane: one line per tool call, with its popup anchor.

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
    // The row under the pointer lights up when it belongs to a call: the
    // band under the glyphs is what says these rows press, before anything
    // is pressed. Only while no popup covers the pane.
    let (cx, cy) = frame.cursor;
    if state.open_call.is_none() && panel.inset(PAD).contains(cx, cy) {
        let inset = panel.inset(PAD);
        let line = Text::line_for(frame.pane_size);
        let row = ((cy - inset.y) / line).floor().max(0.0) as usize;
        if let Some((absolute, _)) = state.activity.spot_in(rows, cols, row, 0)
            && state.call_at_line(absolute).is_some()
            && let Some((first, tall)) = state.activity.band_of(rows, cols, absolute)
        {
            scene.rect(
                Panel::new(
                    inset.x,
                    inset.y + first as f32 * line,
                    (inset.w - SCROLL_W - SCROLL_GAP * 2.0).max(1.0),
                    tall as f32 * line,
                )
                .fill(skin.hot),
            );
        }
    }
    let mut runs = Vec::new();
    for line in state.activity.visible(rows, cols) {
        // The clock column in front of a row is the subordinate part of it and
        // is drawn in the dim tone: what the eye is looking down the list for
        // is the tag and the subject, and a time in the call's own color would
        // be competing with them for the row.
        let (clock, rest) = line.split_gutter();
        if !clock.is_empty() {
            runs.push(Run::tinted(clock, skin.dim));
        }
        runs.push(Run::tinted(rest, skin.tone(line.tone)));
        runs.push(Run::plain("\n"));
    }
    scene.text(
        Text::rich(runs, panel.inset(PAD), frame.pane_size, frame.skin.body)
            .scrolled(state.activity.window(rows, cols).skip as f32)
            .wrap_at(cols),
    );
    scrollbar(scene, skin, panel, state.activity.thumb(rows, cols));
}
