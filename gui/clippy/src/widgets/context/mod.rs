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

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::wildcard_imports)]
    use crate::view::testkit::*;
    use crate::dock::{Dock, Space};
    use crate::monitor::Monitor;
    use crate::state::State;

    /// The header of the context pane is four labelled rows: the phase, and the
    /// three counts that say what this run has asked for. They are separated the
    /// way the phase row always was, a label in the dim tint and the reading
    /// beside it, which is what the counts never had as gauges.
    ///
    /// This asserted MODEL and PATH, which were the other two rows. The strip
    /// says where the agent is working now, so a PATH row here would be the same
    /// answer twice, and the model is on the settings panel.
    #[test]
    fn the_context_header_is_the_phase_and_the_three_counts() {
        let state = busy_state();
        let mut monitor = Monitor::new();
        monitor.sample(&state);
        let mut dock = Dock::new();
        dock.reveal(View::Context);
        let out = render_with(&state, 1400.0, 900.0, &dock, &[], &monitor, None);
        let text = text_of(&out.scene);
        let body = out.layout.placed(Space::TopRight).body;
        for wanted in [
            "PHASE",
            "TOTAL REQUESTS",
            "TOTAL TOOL CALLS",
            "LAST PREFILL",
            "CONTEXT",
        ] {
            assert!(text.contains(wanted), "{wanted:?} is not in the pane: {text}");
        }
        for gone in ["MODEL", "PATH", state.model.as_str()] {
            assert!(!text.contains(gone), "{gone:?} is still in the pane: {text}");
        }
        // Every label is drawn in its own box in the dim tint, with its reading
        // in a box of its own beside it. That separation is the whole of what
        // moving these three off the gauge list bought.
        for (label, reading) in [
            ("TOTAL REQUESTS", crate::state::thousands(state.requests as u64)),
            ("TOTAL TOOL CALLS", crate::state::thousands(state.tool_calls as u64)),
            ("LAST PREFILL", crate::state::thousands(state.last_prefill)),
        ] {
            let row: Vec<&noob_draw::Text> = out
                .scene
                .texts
                .iter()
                .filter(|t| body.contains(t.at.x, t.at.y))
                .filter(|t| {
                    t.runs.iter().any(|r| r.text == label || r.text == reading)
                })
                .collect();
            assert_eq!(row.len(), 2, "{label} is not a label and a reading");
            let (name, value) = (row[0], row[1]);
            assert_eq!(name.runs[0].text, label);
            assert_eq!(value.runs[0].text, reading, "{label}");
            assert_eq!(name.runs[0].color, Some(out.skin.dim), "{label} is not dim");
            assert_eq!(value.runs[0].color, Some(out.skin.body), "{label}'s reading");
            assert!(value.at.x > name.at.x, "{label} is not beside its reading");
            assert!((value.at.y - name.at.y).abs() < 0.01, "{label} is not on one row");
        }
        // And the reading above the header is a row of its own, not a line of
        // the title strip: the phase word is drawn in the pane, not up there.
        assert!(
            out.scene.texts.iter().any(|t| {
                body.contains(t.at.x, t.at.y)
                    && t.runs.iter().any(|r| r.text.contains(state.phase.word()))
            }),
            "the phase is not drawn in the context pane"
        );
    }
    /// The calls that failed are counted beside the calls that were made.
    ///
    /// That count was the whole of the DEBUG pane's first row and the pane is
    /// gone. It rides with the total rather than taking a row of its own,
    /// because the two are one reading, and it takes the fault colour once
    /// there is anything to say.
    #[test]
    fn the_failed_calls_are_counted_beside_the_total() {
        let mut state = busy_state();
        let mut dock = Dock::new();
        dock.reveal(View::Context);
        let reading = |state: &State| {
            let out = render_with(state, 1400.0, 900.0, &dock, &[], &Monitor::new(), None);
            let body = out.layout.placed(Space::TopRight).body;
            let run = out
                .scene
                .texts
                .iter()
                .filter(|t| body.contains(t.at.x, t.at.y))
                .flat_map(|t| t.runs.iter())
                .find(|r| r.text == crate::state::thousands(state.tool_calls as u64)
                    || r.text.starts_with(&format!(
                        "{} (",
                        crate::state::thousands(state.tool_calls as u64)
                    )))
                .expect("the tool call total is drawn")
                .clone();
            (run.text.clone(), run.color, out.skin)
        };

        // Nothing has failed: the total is the whole reading, in the ordinary
        // tint. A "(0 failed)" on every window is noise.
        assert_eq!(state.failed_calls, 0);
        let (text, tint, skin) = reading(&state);
        assert_eq!(text, crate::state::thousands(state.tool_calls as u64));
        assert_eq!(tint, Some(skin.body));

        state.apply(noob_proto::Event::ToolStart {
            call_id: "boom".into(),
            name: "bash".into(),
            brief: "no".into(),
            args: serde_json::json!({"cmd": "no"}),
        });
        state.apply(noob_proto::Event::ToolEnd {
            call_id: "boom".into(),
            summary: "refused".into(),
            elapsed_ms: 1,
            error: Some(noob_proto::ToolError {
                kind: "denied".into(),
                code: None,
                message: "outside the workspace".into(),
                detail: None,
                remedy: None,
            }),
        });
        let (text, tint, skin) = reading(&state);
        assert_eq!(
            text,
            format!("{} (1 failed)", crate::state::thousands(state.tool_calls as u64))
        );
        assert_eq!(tint, Some(skin.bad), "a failure does not read as a number");
        assert_ne!(skin.bad, skin.body);
    }
    /// While a turn is running the phase reads INFERRING in the bad colour, and
    /// at rest it is READY in the ordinary body tint.
    ///
    /// The colour is the point as much as the word: it is the one reading in the
    /// window that has to be answerable from the corner of the eye, because it
    /// is what says whether anything typed now is going anywhere.
    #[test]
    fn the_phase_reads_infering_in_the_bad_colour_while_a_turn_runs() {
        let mut dock = Dock::new();
        dock.reveal(View::Context);
        let phase_run = |state: &State| {
            let out = render_with(
                state,
                1400.0,
                900.0,
                &dock,
                &[],
                &Monitor::new(),
                None,
            );
            let body = out.layout.placed(Space::TopRight).body;
            let run = out
                .scene
                .texts
                .iter()
                .filter(|text| body.contains(text.at.x, text.at.y))
                .flat_map(|text| text.runs.iter())
                .find(|run| run.text.contains("READY") || run.text.contains("INFERRING"))
                .expect("the phase is drawn in the pane")
                .clone();
            (run.text.clone(), run.color, out.skin)
        };

        let (word, tint, skin) = phase_run(&busy_state());
        assert_eq!(word, "INFERRING");
        assert_eq!(tint, Some(skin.bad), "the busy word is not the bad colour");

        let mut ready = State::new();
        ready.apply(noob_proto::Event::SessionStart {
            id: "s1".into(),
            workspace: "/tmp".into(),
            model: "laguna-s21".into(),
            resumed: false,
        });
        let (word, tint, skin) = phase_run(&ready);
        assert_eq!(word, "READY");
        assert_eq!(tint, Some(skin.body));
        assert_ne!(skin.body, skin.bad);
    }
}
