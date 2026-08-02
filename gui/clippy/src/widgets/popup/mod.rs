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

/// The extra room the popup keeps inside its box and inside each block. Named
/// for the popup rather than `PAD`, which is the window's own margin arriving
/// on the wildcard import above.
pub(crate) const POPUP_PAD: f32 = 10.0;
/// The tone bar's columns: the bar itself and the air after it.
const BAR_COLS: usize = 2;

/// How many text columns the popup's blocks wrap in, from the same box the
/// drawing uses, so the wheel, the track and the glyphs agree.
fn block_cols(box_: Panel, column: f32) -> usize {
    let inside = box_.inset(POPUP_PAD);
    let text = inside.w - BAR_COLS as f32 * column - SCROLL_W - SCROLL_GAP * 2.0 - POPUP_PAD;
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
    let inside = box_.inset(POPUP_PAD);
    Panel::new(
        inside.x,
        inside.y + line * 2.0,
        inside.w,
        (inside.h - line * 2.0).max(1.0),
    )
}

/// The popup's blocks flattened to the lines they are drawn as: each cell's
/// label, its lines, and a blank between blocks. What a selection resolves
/// and copies against, with the same wrap arithmetic the painter draws by.
pub(crate) fn document(call: &Call) -> crate::state::Pane {
    let mut doc = crate::state::Pane::new(4096);
    for (step, cell) in call.cells().iter().enumerate() {
        if step > 0 {
            doc.say("", Tone::Dim);
        }
        doc.say(cell.label, Tone::Dim);
        for line in &cell.lines {
            doc.say(line.clone(), cell.tone);
        }
    }
    doc
}

/// The character of the popup's document under a point, for a selection.
pub(crate) fn spot_at(frame: &Frame, x: f32, y: f32) -> Option<crate::select::Spot> {
    let call = frame.state.popped()?;
    let box_ = frame.layout.call_popup;
    if box_.w < 1.0 || box_.h < 1.0 {
        return None;
    }
    let line = Text::line_for(frame.pane_size);
    let view = viewport(box_, line);
    if !view.contains(x, y.max(view.y)) {
        return None;
    }
    let cols = block_cols(box_, frame.pane_column);
    let fit = ((view.h / line).floor() as usize).max(1);
    let total = content_rows(call, cols);
    let scroll = frame.popup_scroll.min(total.saturating_sub(fit));
    let mut doc = document(call);
    doc.anchor_first(scroll, fit, cols);
    let row = ((y - view.y) / line).floor().max(0.0) as usize;
    let text_x = view.x + BAR_COLS as f32 * frame.pane_column;
    let at = (((x - text_x) / frame.pane_column.max(1.0)).floor().max(0.0)) as usize;
    let (line, column) = doc.spot_in(fit, cols, row, at)?;
    Some(crate::select::Spot::new(line, column))
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
    let inside = box_.inset(POPUP_PAD);

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

    // The selection's bands, over the blocks and under nothing: the popup
    // is the floating layer's last word. The same document the copy reads,
    // anchored to the same window the blocks were drawn in.
    if let Some(selection) = frame.selection
        && selection.at == crate::select::Where::CallPopup
        && !selection.is_empty()
    {
        let mut doc = document(call);
        doc.anchor_first(scroll, fit, cols);
        for r in 0..fit {
            let Some((line_at, wrapped)) = doc.spot_row(fit, cols, r) else {
                continue;
            };
            let len = doc
                .line(line_at)
                .map_or(0, |l| l.shown().chars().count());
            let Some((from, to)) = selection.columns_on(line_at, len) else {
                continue;
            };
            let Some(span) = doc.rows_of_line(line_at, cols).get(wrapped).copied() else {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::wildcard_imports)]
    use crate::view::testkit::*;
    use crate::config::Config;
    use crate::dock::Dock;
    use crate::monitor::Monitor;
    use noob_draw::Rect;

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
            popup_scroll: 0,
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
            args: serde_json::json!({"cmd": "cargo build"}),
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

    /// The document is the blocks in drawing order, and a selection over it
    /// copies the lines between its ends: what the popup shows is what a
    /// drag inside it takes to the clipboard.
    #[test]
    fn the_document_is_the_blocks_in_order_and_a_selection_copies_it() {
        let state = a_failed_call();
        let call = state.call(0).expect("the record");
        let doc = document(call);
        let all: Vec<String> = (0..doc.last())
            .filter_map(|at| doc.line(at).map(|l| l.text.clone()))
            .collect();
        let labels: Vec<&String> = all
            .iter()
            .filter(|l| ["INVOKED", "WHEN", "GENERATED", "RETURNED", "DETAIL"].contains(&l.as_str()))
            .collect();
        assert!(labels.len() >= 4, "{all:?}");
        let detail_at = all
            .iter()
            .position(|l| l == "DETAIL")
            .expect("the failure block");
        let mut selection = crate::select::Selection::new(
            crate::select::Where::CallPopup,
            crate::select::Spot::new(detail_at, 0),
        );
        selection.extend(crate::select::Spot::new(detail_at + 2, 99));
        let copied = selection.text(&document(call));
        assert!(copied.contains("exit_status 127"), "{copied}");
        assert!(copied.contains("cargo: command not found"), "{copied}");
    }

    /// The popup is on the floating layer, it carries the four things it
    /// promises, and shutting it takes the whole thing off the screen.
    ///
    /// On the overlay for the reason the menu is: the renderer paints a layer's
    /// rectangles in one pass and its glyphs in a later one, so a box pushed onto
    /// the base layer lands under the pane text it is covering, however late it
    /// was pushed.
    #[test]
    fn the_activity_popup_is_painted_over_the_window_and_closes_off_it() {
        let mut state = busy_state();
        state.apply(noob_proto::Event::ToolEnd {
            call_id: "c1".into(),
            summary: "bash cargo test (2.0s, exit 101)".into(),
            elapsed_ms: 2000,
            error: Some(noob_proto::ToolError {
                kind: "exit_status".into(),
                code: Some(101),
                message: "1 test failed".into(),
                detail: Some("thread 'a' panicked at src/lib.rs:9".into()),
                remedy: Some("run it again with --nocapture".into()),
            }),
        });
        let dock = Dock::new();

        // Shut, there is no box and nothing on the overlay.
        let shut = render_popup(&state, 1400.0, 900.0, &dock);
        assert!(shut.layout.call_popup.w < 1.0);
        assert!(shut.scene.over_rects.is_empty(), "something is floating already");

        state.open_call = state.call_at_line(0);
        assert!(state.open_call.is_some(), "the first row is the bash call");
        let out = render_popup(&state, 1400.0, 900.0, &dock);
        let box_ = out.layout.call_popup;
        assert!(box_.w >= 1.0 && box_.h >= 1.0);

        // The condition that makes the overlay the point: there is pane text
        // under it.
        assert!(
            text_over(&out.scene.texts, box_),
            "nothing is written under the popup, so this proves nothing"
        );
        let surface = |rects: &[Rect]| {
            rects
                .iter()
                .any(|r| r.xywh() == [box_.x, box_.y, box_.w, box_.h] && r.extra()[3] == 0.0)
        };
        assert!(surface(&out.scene.over_rects), "the popup has no surface");
        assert!(
            !surface(&out.scene.rects),
            "the popup's surface is in the base layer, under every glyph"
        );

        // Everything on the overlay is the popup's, and every cell it promised
        // is written.
        let floating: String = out
            .scene
            .over_texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| r.text.as_str()))
            .collect();
        for want in ["INVOKED", "GENERATED", "RETURNED", "DETAIL", "WHEN"] {
            assert!(floating.contains(want), "no {want} cell: {floating}");
        }
        assert!(floating.contains("cargo test --workspace"), "{floating}");
        assert!(floating.contains("exit_status 101"), "{floating}");
        assert!(floating.contains("panicked at src/lib.rs:9"), "{floating}");
        assert!(floating.contains("run it again with --nocapture"), "{floating}");

        for text in &out.scene.over_texts {
            assert!(
                text.at.x >= box_.x - 0.01 && text.at.y >= box_.y - 0.01,
                "{:?} is on the overlay but is not the popup",
                text.at
            );
        }
    }
}
