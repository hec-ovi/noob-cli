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
    // The call row under the pointer turns red and grows a bullet: the list
    // says these rows press by marking the one the hand is on, with no band
    // behind it.
    let lit = hovered_call_row(frame, panel, rows, cols);
    let mut runs = Vec::new();
    for (step, line) in state.activity.visible(rows, cols).iter().enumerate() {
        let absolute = state.activity.showing_from(rows, cols) + step;
        let picked = lit == Some(absolute);
        let ink = match picked {
            true => skin.bad,
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
        // The bullet stands in the last space of the clock column, between the
        // reading and the tag. Drawn over a character the row already has
        // rather than inserted before one: a row that grew a column under the
        // pointer would be a row whose every later column disagreed with what
        // a press there resolves to.
        let bullet = picked && head.ends_with(' ');
        let head = match bullet {
            true => head[..head.len() - 1].to_string(),
            false => head,
        };
        if !head.is_empty() {
            runs.push(Run::tinted(head, skin.dim));
        }
        if bullet {
            runs.push(Run::tinted("\u{2022}", skin.bad));
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

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::wildcard_imports)]
    use crate::view::testkit::*;
    
    
    
    use crate::state::Tone;

    /// The time on an activity row is drawn in the dim tone, and the tag and
    /// the subject keep the call's own color.
    ///
    /// Item 42 put the clock on the row. Drawn in the call's color it would be
    /// eight characters shouting as loudly as the thing the row is about, which
    /// is what "does not fight the subject" rules out. The two runs together
    /// are still exactly the stored line, so nothing the pane measures moved.
    /// The row under the pointer reads as picked: its subject turns red and a
    /// bullet stands between the clock and the tag. The bullet takes a space
    /// the row already had, so the row is exactly as wide as it was and a
    /// press still lands on the character under the hand.
    #[test]
    fn the_row_under_the_pointer_turns_red_and_grows_a_bullet() {
        let mut state = busy_state();
        state.day_zero = Some(14 * 3600 + 30 * 60);
        state.apply_at(
            noob_proto::Event::ToolStart {
                call_id: "c9".into(),
                name: "bash".into(),
                brief: "cargo build".into(),
                args: serde_json::json!({"cmd": "cargo build --release"}),
            },
            Some(9.0),
        );
        let dock = a_dock_showing(View::Activity);
        let cold = render(&state, 1400.0, 900.0, &dock, &[]);
        let row_of = |out: &Rendered| {
            let text = out
                .scene
                .texts
                .iter()
                .find(|text| text.runs.iter().any(|run| run.text.starts_with("14:30:09")))
                .expect("the activity pane draws the row");
            let at = text
                .runs
                .iter()
                .position(|run| run.text.starts_with("14:30:09"))
                .expect("the reading is its own run");
            text.runs[at..].to_vec()
        };
        let idle = row_of(&cold);
        assert!(
            !idle.iter().any(|run| run.text == "\u{2022}"),
            "an untouched row wears the bullet"
        );

        // The pointer on that row: it is the last one the pane holds.
        let space = crate::dock::Space::ALL
            .into_iter()
            .find(|space| dock.slot(*space).active() == Some(View::Activity))
            .expect("the activity pane is in the window");
        let layout = crate::view::Layout::compute(1400.0, 900.0, &shape(&dock, &[]));
        let panel = layout.placed(space).body.inset(crate::view::PAD);
        let line = noob_draw::Text::line_for(13.0);
        let rows = layout.rows(layout.placed(space).body, 13.0);
        let cols = cols_of(layout.placed(space).body, 8.0);
        let last = state.activity.last() - 1;
        let row = (0..rows)
            .find(|row| {
                state.activity.spot_in(rows, cols, *row, 0).map(|(l, _)| l) == Some(last)
            })
            .expect("the row is on screen");
        let hot = render_hovered(
            &state,
            1400.0,
            900.0,
            &dock,
            &[],
            (panel.x + 4.0, panel.y + row as f32 * line + line / 2.0),
        );
        let lit = row_of(&hot);
        let bullet = lit
            .iter()
            .position(|run| run.text == "\u{2022}")
            .expect("the row under the pointer wears a bullet");
        assert_eq!(lit[bullet].color, Some(hot.skin.bad), "the bullet is red");
        assert_eq!(lit[bullet + 1].color, Some(hot.skin.bad), "the subject is red");
        // Same characters, same width: the bullet took a space, it did not add
        // a column.
        let width = |runs: &[noob_draw::Run]| -> usize {
            runs.iter()
                .take_while(|run| !run.text.contains('\n'))
                .map(|run| run.text.chars().count())
                .sum()
        };
        assert_eq!(width(&idle), width(&lit), "the row changed width under the pointer");
    }

    #[test]
    fn the_clock_on_an_activity_row_is_drawn_dim_and_the_subject_is_not() {
        let mut state = busy_state();
        state.day_zero = Some(14 * 3600 + 30 * 60);
        state.apply_at(
            noob_proto::Event::ToolStart {
                call_id: "c9".into(),
                name: "bash".into(),
                brief: "cargo build".into(),
                args: serde_json::json!({"cmd": "cargo build --release"}),
            },
            Some(9.0),
        );
        let out = render(&state, 1400.0, 900.0, &a_dock_showing(View::Activity), &[]);
        let text = out
            .scene
            .texts
            .iter()
            .find(|text| text.runs.iter().any(|run| run.text.starts_with("14:30:09")))
            .expect("the activity pane draws the row's clock");
        let at = text
            .runs
            .iter()
            .position(|run| run.text.starts_with("14:30:09"))
            .expect("the reading is its own run");
        let (clock, rest) = (&text.runs[at], &text.runs[at + 1]);
        assert_eq!(clock.text, "14:30:09  ");
        assert_eq!(clock.color, Some(out.skin.dim));
        assert!(rest.text.contains("bash"), "{:?}", rest.text);
        assert!(rest.text.contains("cargo build --release"), "{:?}", rest.text);
        assert_eq!(
            rest.color,
            Some(out.skin.tone(Tone::Call(crate::state::Kind::Bash))),
            "the subject is drawn in the call's own color"
        );
        assert_ne!(rest.color, clock.color);
        // The row on screen is the row the pane holds, character for character:
        // the split is two runs of one line, not a line with something added.
        let held = state
            .activity
            .line(state.activity.last() - 1)
            .expect("the row is still there");
        assert_eq!(format!("{}{}", clock.text, rest.text), held.text);
    }
}
