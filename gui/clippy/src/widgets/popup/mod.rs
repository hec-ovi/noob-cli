//! The call popup: one activity row opened out, full panel.
//!
//! A metadata header (when, which turn, how long, the tool, the thing it
//! reached for, the file when there is one), then the remaining room split
//! into two halves: what the model generated, and what came back. Each half
//! scrolls on its own and holds its own selection, the way the OUTPUT pane
//! does, so a long command and a long result are read side by side rather
//! than in one stack.

use noob_draw::{Panel, Run, Scene, Text};

#[allow(unused_imports)]
use crate::dock::View;
#[allow(unused_imports)]
use crate::monitor::Gauge;
#[allow(unused_imports)]
use crate::state::{Call, State, Tone, TodoState};
#[allow(unused_imports)]
use crate::style::skin::Skin;
#[allow(clippy::wildcard_imports)]
use crate::view::*;

/// The extra room the popup keeps inside its box and inside each half. Named
/// for the popup rather than `PAD`, which is the window's own margin arriving
/// on the wildcard import above.
pub(crate) const POPUP_PAD: f32 = 10.0;
/// The tone bar's columns: the bar itself and the air after it.
const BAR_COLS: usize = 2;

/// The two scrolled halves under the header. Everything per-half is indexed
/// by [`Half::index`], the selection included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Half {
    /// What the model generated: the call's arguments, as lines.
    Call,
    /// What came back: summary, output, and the failure when there is one.
    Result,
}

impl Half {
    pub const BOTH: [Half; 2] = [Half::Call, Half::Result];

    pub fn index(self) -> usize {
        match self {
            Half::Call => 0,
            Half::Result => 1,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Half::Call => "CALL",
            Half::Result => "RESULT",
        }
    }
}

/// How many text columns a half's lines wrap in, from the same box the
/// drawing uses, so the wheel, the track and the glyphs agree.
fn half_cols(box_: Panel, column: f32) -> usize {
    let inside = box_.inset(POPUP_PAD);
    let text = inside.w - BAR_COLS as f32 * column - SCROLL_W - SCROLL_GAP * 2.0 - POPUP_PAD;
    ((text / column.max(1.0)).floor() as usize).max(8)
}

/// One half's document: the lines a selection over it resolves and copies
/// against, with the tones they are drawn in.
pub(crate) fn half_document(call: &Call, half: Half) -> crate::state::Pane {
    let mut doc = crate::state::Pane::new(4096);
    match half {
        Half::Call => {
            for line in call.call_lines() {
                doc.say(line, Tone::Body);
            }
        }
        Half::Result => {
            for (line, tone) in call.result_lines() {
                doc.say(line, tone);
            }
        }
    }
    doc
}

/// How many rows the header takes: the heading, its metadata lines, and the
/// blank that holds the halves off it.
fn header_rows(state: &State, call: &Call) -> usize {
    2 + state.popup_header(call).len()
}

/// Where one half sits: its label row and the viewport under it. The two
/// split what the header leaves evenly, and each keeps one row for its label.
fn half_box(box_: Panel, line: f32, header: usize, half: Half) -> Panel {
    let inside = box_.inset(POPUP_PAD);
    let top = inside.y + header as f32 * line;
    let room = ((inside.h - header as f32 * line) / 2.0).max(line * 2.0);
    Panel::new(
        inside.x,
        top + half.index() as f32 * room,
        inside.w,
        room,
    )
}

/// The half under a point, for the wheel and for a press.
pub(crate) fn half_at(frame: &Frame, y: f32) -> Option<Half> {
    let (state, call) = (frame.state, frame.state.popped()?);
    let box_ = frame.layout.call_popup;
    if box_.w < 1.0 || box_.h < 1.0 {
        return None;
    }
    let line = Text::line_for(frame.pane_size);
    let header = header_rows(state, call);
    Half::BOTH
        .into_iter()
        .find(|half| {
            let at = half_box(box_, line, header, *half);
            y >= at.y && y < at.y + at.h
        })
}

/// One half's scrollable extent: its content rows and the rows its viewport
/// shows. What the wheel, the dragged track and the per-frame clamp all
/// measure with. `None` while no popup is up.
pub(crate) fn scroll_geometry(frame: &Frame, half: Half) -> Option<(usize, usize)> {
    let call = frame.state.popped()?;
    let box_ = frame.layout.call_popup;
    if box_.w < 1.0 || box_.h < 1.0 {
        return None;
    }
    let line = Text::line_for(frame.pane_size);
    let at = half_box(box_, line, header_rows(frame.state, call), half);
    let fit = (((at.h - line) / line).floor() as usize).max(1);
    let cols = half_cols(box_, frame.pane_column);
    let doc = half_document(call, half);
    let total: usize = (0..)
        .map_while(|n| {
            let rows = doc.rows_of_line(n, cols);
            (!rows.is_empty()).then_some(rows.len())
        })
        .sum();
    Some((total, fit))
}

/// The character of one half's document under a point, for a selection. The
/// half is the caller's: a drag that started in one half stays a selection
/// of that half however far the pointer wanders.
pub(crate) fn spot_at(frame: &Frame, half: Half, x: f32, y: f32) -> Option<crate::select::Spot> {
    let call = frame.state.popped()?;
    let box_ = frame.layout.call_popup;
    if box_.w < 1.0 || box_.h < 1.0 {
        return None;
    }
    let line = Text::line_for(frame.pane_size);
    let at = half_box(box_, line, header_rows(frame.state, call), half);
    let view = Panel::new(at.x, at.y + line, at.w, (at.h - line).max(1.0));
    let cols = half_cols(box_, frame.pane_column);
    let fit = ((view.h / line).floor() as usize).max(1);
    let mut doc = half_document(call, half);
    let (total, _) = scroll_geometry(frame, half)?;
    let scroll = frame.popup_scroll[half.index()].min(total.saturating_sub(fit));
    doc.anchor_first(scroll, fit, cols);
    let row = ((y.max(view.y) - view.y) / line).floor().max(0.0) as usize;
    let text_x = view.x + BAR_COLS as f32 * frame.pane_column;
    let column = (((x - text_x) / frame.pane_column.max(1.0)).floor().max(0.0)) as usize;
    let (line_at, spot) = doc.spot_in(fit, cols, row.min(fit.saturating_sub(1)), column)?;
    Some(crate::select::Spot::new(line_at, spot))
}

/// The popup itself: the heading and close mark, the metadata lines, then
/// the two halves. On the floating layer for the reason the menu is: a box
/// on the base layer is painted before every glyph in the window and comes
/// out under the pane text.
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
    let inside = box_.inset(POPUP_PAD);

    // The heading: the tool, its target and how it stands, with the close
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

    // The metadata, dim, one fact per line, never scrolled: it is the head
    // of the thing, not part of either half.
    let header = frame.state.popup_header(call);
    let mut runs = Vec::new();
    for fact in &header {
        runs.push(Run::tinted(fact, skin.dim));
        runs.push(Run::plain("\n"));
    }
    scene.over_text(Text::rich(
        runs,
        Panel::new(
            inside.x,
            inside.y + line,
            inside.w,
            header.len() as f32 * line,
        ),
        size,
        skin.dim,
    ));

    // The two halves: a labelled bar, a scrolled viewport, its own thumb,
    // and its own selection bands.
    let rows_of_header = header_rows(frame.state, call);
    let cols = half_cols(box_, frame.pane_column);
    for half in Half::BOTH {
        let at = half_box(box_, line, rows_of_header, half);
        let view = Panel::new(at.x, at.y + line, at.w, (at.h - line).max(1.0));
        let fit = ((view.h / line).floor() as usize).max(1);
        let mut doc = half_document(call, half);
        let heights: Vec<usize> = (0..)
            .map_while(|n| {
                let rows = doc.rows_of_line(n, cols);
                (!rows.is_empty()).then_some(rows.len())
            })
            .collect();
        let total: usize = heights.iter().sum();
        let scroll = frame.popup_scroll[half.index()].min(total.saturating_sub(fit));

        // The label row, with the tone bar the blocks used to carry.
        let tone = match half {
            Half::Call => skin.tone(Tone::Body),
            Half::Result => match call.error.is_some() {
                true => skin.tone(Tone::Bad),
                false => skin.tone(Tone::Body),
            },
        };
        scene.over_rect(
            Panel::new(at.x, at.y, MARK_W, line).fill(tone.map(|c| f32::from(c) / 255.0)),
        );
        scene.over_text(Text::rich(
            vec![Run::tinted(half.label(), skin.dim)],
            Panel::new(
                at.x + BAR_COLS as f32 * frame.pane_column,
                at.y,
                at.w,
                line,
            ),
            size,
            skin.dim,
        ));

        // The lines, in the window the scroll offset names, wrapped in the
        // same columns everything above measured: the pane's own window
        // machinery, anchored top-first the way the pane lends it out.
        let text_x = view.x + BAR_COLS as f32 * frame.pane_column;
        doc.anchor_first(scroll, fit, cols);
        let window = doc.window(fit, cols);
        let mut body = Vec::new();
        for l in doc.visible(fit, cols) {
            body.push(Run::tinted(l.shown(), skin.tone(l.tone)));
            body.push(Run::plain("\n"));
        }
        if !body.is_empty() {
            scene.over_text(
                Text::rich(
                    body,
                    Panel::new(text_x, view.y, cols as f32 * frame.pane_column, view.h),
                    size,
                    skin.body,
                )
                .wrap_at(cols)
                .scrolled(window.skip as f32),
            );
        }

        // The selection's bands, over the half and under nothing.
        let wanted = match half {
            Half::Call => crate::select::Where::CallPopup,
            Half::Result => crate::select::Where::CallResult,
        };
        if let Some(selection) = frame.selection
            && selection.at == wanted
            && !selection.is_empty()
        {
            let mut banded = half_document(call, half);
            banded.anchor_first(scroll, fit, cols);
            for r in 0..fit {
                let Some((line_at, wrapped)) = banded.spot_row(fit, cols, r) else {
                    continue;
                };
                let len = banded
                    .line(line_at)
                    .map_or(0, |l| l.shown().chars().count());
                let Some((from, to)) = selection.columns_on(line_at, len) else {
                    continue;
                };
                let Some(span) = banded.rows_of_line(line_at, cols).get(wrapped).copied() else {
                    continue;
                };
                let (a, b) = (from.max(span.start), to.min(span.end));
                if a >= b {
                    continue;
                }
                scene.over_rect(
                    Panel::new(
                        text_x + (a - span.start) as f32 * frame.pane_column,
                        view.y + r as f32 * line,
                        (b - a) as f32 * frame.pane_column,
                        line,
                    )
                    .fill(skin.select),
                );
            }
        }

        // Its own track, beside its own viewport.
        if total > fit {
            let track = Panel::new(
                at.x + at.w - SCROLL_W - SCROLL_GAP,
                view.y,
                SCROLL_W,
                view.h,
            );
            let back = text_geometry::scrollback_for(&heights, fit, scroll);
            if let Some((top, size_)) = text_geometry::thumb(&heights, fit, back) {
                scene.over_rect(track.fill(skin.scroll_track));
                scene.over_rect(
                    Panel::new(
                        track.x,
                        track.y + track.h * top,
                        track.w,
                        (track.h * size_).max(8.0).min(track.h),
                    )
                    .fill(skin.scroll_thumb),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::wildcard_imports)]
    use crate::view::testkit::*;
    use crate::config::Config;
    use crate::dock::Dock;
    use crate::monitor::Monitor;

    /// The same window with one activity row opened out.
    fn render_popup(state: &State, w: f32, h: f32, dock: &Dock) -> Rendered {
        let shape = Shape {
            popup: state.popped(),
            ..shape(dock, &[])
        };
        let layout = Layout::compute(w, h, &shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state,
            scrolls: &crate::scroll::Scrolls::default(),
            file_scroll: 0,
            monitor: &Monitor::new(),
            dock,
            skin: &skin,
            layout: &layout,
            prompt: &typed_prompt("type here", 4),
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
            popup_scroll: [0, 0],
            cursor: (-100.0, -100.0),
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        });
        Rendered {
            scene,
            layout,
            skin,
        }
    }

    fn a_failed_call() -> crate::state::State {
        let mut state = crate::state::State::new();
        state.apply(noob_proto::Event::ToolStart {
            call_id: "b".into(),
            name: "bash".into(),
            brief: "cargo build".into(),
            args: serde_json::json!({"cmd": "cd x\ncargo build\necho done"}),
        });
        state.apply(noob_proto::Event::ToolEnd {
            call_id: "b".into(),
            summary: "bash cargo build (exit 127)".into(),
            elapsed_ms: 9,
            error: Some(noob_proto::ToolError {
                kind: "exit_status".into(),
                code: Some(127),
                message: "cargo: command not found".into(),
                detail: None,
                remedy: None,
            }),
        });
        state
    }

    /// The call half shows the command as the lines the model wrote it in,
    /// never as one string with `\n` spelled out; the result half is the
    /// failure's story; and a selection over one half copies that half.
    #[test]
    fn the_halves_read_as_lines_and_a_selection_copies_its_own_half() {
        let state = a_failed_call();
        let call = state.call(0).expect("the record");

        let call_doc = half_document(call, Half::Call);
        let lines: Vec<String> = (0..call_doc.last())
            .filter_map(|at| call_doc.line(at).map(|l| l.text.clone()))
            .collect();
        assert!(lines.contains(&String::from("cargo build")), "{lines:?}");
        assert!(
            !lines.iter().any(|l| l.contains("\\n")),
            "a newline is a line, not two characters: {lines:?}"
        );

        let mut selection = crate::select::Selection::new(
            crate::select::Where::CallResult,
            crate::select::Spot::new(0, 0),
        );
        selection.extend(crate::select::Spot::new(9, 99));
        let copied = selection.text(&half_document(call, Half::Result));
        assert!(copied.contains("exit_status 127"), "{copied}");
        assert!(copied.contains("cargo: command not found"), "{copied}");
        assert!(
            !copied.contains("cd x"),
            "the result selection stays out of the call half: {copied}"
        );
    }

    /// The popup is on the floating layer: a metadata header, then the CALL
    /// and RESULT halves, each labelled. Shutting it takes it off the screen.
    #[test]
    fn the_popup_is_a_header_over_two_labelled_halves() {
        let mut state = a_failed_call();
        state.day_zero = Some(10 * 3600);

        let dock = Dock::new();
        let shut = render_popup(&state, 1400.0, 900.0, &dock);
        assert!(shut.layout.call_popup.w < 1.0);
        assert!(shut.scene.over_rects.is_empty(), "something is floating already");

        state.open_call = state.call_at_line(0);
        assert!(state.open_call.is_some(), "the first row is the bash call");
        let out = render_popup(&state, 1400.0, 900.0, &dock);
        let box_ = out.layout.call_popup;
        assert!(box_.w >= 1.0 && box_.h >= 1.0);

        let floating: String = out
            .scene
            .over_texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| r.text.as_str()))
            .collect();
        for want in [
            "CALL",
            "RESULT",
            "turn",
            "tool: bash",
            "cargo build",
            "exit_status 127",
        ] {
            assert!(floating.contains(want), "no {want:?} on the popup: {floating}");
        }

        for text in &out.scene.over_texts {
            assert!(
                text.at.x >= box_.x - 0.01 && text.at.y >= box_.y - 0.01,
                "{:?} is on the overlay but is not the popup",
                text.at
            );
        }
    }
}
