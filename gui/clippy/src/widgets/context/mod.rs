//! The context pane: what the run holds now, as gauges.

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
use crate::widgets::LABEL_COLUMNS;

/// Rows the CONTEXT pane spends on its header before its readings start: the
/// phase, then the three counts that say what this run has asked for. They stay
/// put while the readings under them scroll.
pub(crate) const CONTEXT_HEAD: usize = 4;


/// The CONTEXT pane: what phase this run is in, what it has asked for, and how
/// full it is.
///
/// The header is four rows with labels beside them, in the separation the phase
/// row has had since the title strip was cut back. The three under the phase are
/// counts that were readings in the list below until they were the three most
/// worth reading first, so they came up here: a labelled row is easier to find
/// than a dot block. The model and the workspace were up here too and are not
/// any more, because the strip says the path again and the model is on the
/// settings panel. The readings under the header are [`Monitor::context`], named
/// for this pane.
pub(crate) fn context(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    let content = panel.inset(PAD);
    let line = Text::line_for(frame.pane_size);
    // The failed count rides with the total rather than getting a row of its
    // own: the two are one reading, and a pane that failed nothing still says
    // so. It is where the DEBUG pane's count went when that pane was removed.
    let calls = match state.failed_calls {
        0 => crate::state::thousands(state.tool_calls as u64),
        failed => format!(
            "{} ({} failed)",
            crate::state::thousands(state.tool_calls as u64),
            crate::state::thousands(failed as u64)
        ),
    };
    let rows: [(&str, String, [u8; 4]); CONTEXT_HEAD] = [
        (
            "PHASE",
            match state.resumed {
                true => format!("{} (resumed)", state.phase.word()),
                false => state.phase.word().to_string(),
            },
            // The bad colour while a turn is running, which is the one tint in
            // the palette that pulls the eye off whatever it was reading. It is
            // not a fault: it is the reading that says the machine has the turn
            // and anything you type is queued behind it.
            if state.phase.busy() {
                skin.bad
            } else {
                skin.body
            },
        ),
        (
            "TOTAL REQUESTS",
            crate::state::thousands(state.requests as u64),
            skin.body,
        ),
        (
            "TOTAL TOOL CALLS",
            calls,
            // In the fault colour once something has failed, which is how the
            // pane it came from read its own count. A run with nothing wrong
            // with it reads the same as every other row.
            match state.failed_calls {
                0 => skin.body,
                _ => skin.bad,
            },
        ),
        (
            "LAST PREFILL",
            crate::state::thousands(state.last_prefill),
            skin.body,
        ),
    ];
    // As wide as the longest label, the way the readings below size theirs.
    // Fixed at ten columns, "TOTAL TOOL CALLS" ran into its own number.
    let label_cols = rows
        .iter()
        .map(|(label, _, _)| label.chars().count())
        .max()
        .unwrap_or(LABEL_COLUMNS)
        .max(LABEL_COLUMNS)
        + 1;
    let label_w = label_cols as f32 * frame.pane_column;
    for (index, (label, value, tint)) in rows.iter().enumerate() {
        let y = content.y + index as f32 * line;
        scene.text(Text::rich(
            vec![Run::tinted(*label, skin.dim)],
            Panel::new(content.x, y, label_w.max(1.0), line),
            frame.pane_size,
            skin.dim,
        ));
        // Clipped, not wrapped: the rows are at fixed heights, so a long value
        // that wrapped would have its second row cut off by its own box.
        let room = cols_of(panel, frame.pane_column).saturating_sub(label_cols + 1);
        let text = match value.is_empty() {
            true => String::from("\u{2014}"),
            false => clip(value, room.max(1)),
        };
        scene.text(Text::rich(
            vec![Run::tinted(text, *tint)],
            Panel::new(
                content.x + label_w,
                y,
                (content.w - label_w).max(1.0),
                line,
            ),
            frame.pane_size,
            *tint,
        ));
    }
    // The readings start under the header, in the room that is left.
    let Some(below) = crate::widgets::gauges::gauge_area(panel, frame.pane_size) else {
        return;
    };
    crate::widgets::gauges::gauges(scene, frame, below, View::Context, frame.monitor.context());
}
