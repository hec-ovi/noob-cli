//! What the window's own chrome draws: the title strip, a tab, a pane's
//! surface, the prompt row, and the marks a drag leaves behind.
//!
//! Split from `view` so that measuring the window, answering a press and
//! painting it are three files rather than one. Everything here takes a
//! [`Frame`] and writes into a [`Scene`].

use noob_draw::{Panel, Rect, Run, Scene, Text};

use crate::design::icons;
use crate::dock::{Space, View};
use crate::style::skin::Skin;
#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn title_bar(scene: &mut Scene, frame: &Frame) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    // Open, the bar is a strip across the top. Shaded, it is the whole surface,
    // with the strip's contents drawn at the top of it.
    //
    // Asking for a 30 pixel window is a request, not an instruction: a
    // compositor is free to hand back a taller surface, and this one does unless
    // the window is maximized. Everything under the strip was then cleared to
    // transparent, which composites as black, so shading drew a green strip on a
    // black block. Filling the whole surface in the bar's own colour makes a
    // surface that stays tall read as a green bar, and one that does shrink is
    // pixel for pixel what it was.
    //
    // The bar colour rather than a new one, so the opacity setting still reaches
    // it, and one rectangle rather than a full-surface one under the strip's own:
    // two translucent fills over each other would leave the top 30 pixels more
    // solid than the rest of the bar.
    let surface = if layout.shaded {
        Panel::new(0.0, 0.0, layout.width, layout.height.max(layout.title.h))
    } else {
        layout.title
    };
    scene.rect(surface.fill(skin.bar));

    // How full the context is, as a hairline along the bottom of the strip.
    // It was a bar of its own at the foot of the window; two pixels at the top
    // of the window says the same thing and costs no rows.
    let gauge = Panel::new(0.0, layout.title.y + layout.title.h - 2.0, layout.width, 2.0);
    scene.rect(gauge.fill(skin.gauge_track));
    let used = state.context_fraction();
    if used > 0.0 {
        scene.rect(Panel::new(0.0, gauge.y, layout.width * used, 2.0).fill(skin.gauge));
    }

    // The orb, in the square the strip keeps for it: turning while there is a
    // turn to animate, one frozen dimmer square of dots otherwise, and on its
    // way between the two while a turn is starting or ending. The base layer is
    // enough for it, unlike the menu, because [`ORB_W`] is reserved and no glyph
    // in the window starts inside it, so there is nothing here for a disc to be
    // painted under. It also costs a draw call fewer that way, and there are
    // 516 of these a frame.
    let block = Panel::new(
        layout.title.x,
        layout.title.y,
        ORB_W.min(layout.title.w),
        layout.title.h,
    );
    for disc in crate::orb::discs(block, frame.clock, frame.morph(), skin) {
        scene.rect(disc);
    }

    // These were three hand-drawn rectangles, because the Unicode glyphs the
    // first version asked for were not on this machine and a missing glyph
    // draws as nothing. The symbol font ships in the binary now, so they are
    // the same marks every other window on the desktop uses.
    for (panel, hit, tint, glyph, quiet) in [
        (layout.minimize, Hit::Minimize, skin.hot, crate::design::icons::MINIMIZE, true),
        (layout.maximize, Hit::Maximize, skin.hot, crate::design::icons::MAXIMIZE, true),
        (layout.close, Hit::Close, skin.close_hot, crate::design::icons::CLOSE, false),
    ] {
        let lit = frame.hot == Some(hit);
        if lit {
            scene.rect(panel.fill(tint));
        }
        // Close reads at full strength because it is the one that cannot be
        // undone; the other two sit back until the pointer is on them.
        let ink = match (lit, quiet) {
            (true, _) => skin.bright,
            (false, true) => skin.dim,
            (false, false) => skin.title,
        };
        // The box runs to the button's right edge rather than being sized to
        // one estimated glyph. A box exactly one guessed advance wide clipped
        // these: the maximize mark lost all but its left edge and close all but
        // one arm of its cross.
        let left = ((panel.w - SMALL * 0.6) * 0.5).max(0.0).floor();
        let row = strip_row(panel);
        scene.text(
            Text::rich(
                vec![Run::icon(glyph.to_string(), ink)],
                Panel::new(row.x + left, row.y, (row.w - left).max(1.0), row.h),
                SMALL,
                ink,
            )
            // The mark's own line box, capped at the room the row turned out to
            // have. Left at a full line in a shorter row, the glyph is laid out
            // below the box and clipped away: the whole mark lost to keep the
            // two pixels of air under it.
            .line_height(row.h),
        );
    }

    // The name, then the marker, and nothing else at full strength. It read
    // "NO0B \u{25b8} CLIppy" while the window had two names; it has one.
    let room = (layout.width - BUTTON_W * 3.0 - ORB_W - 12.0).max(1.0);
    let mut runs = vec![
        Run::tinted("NO0B \u{25b8}", skin.bright),
        // Which release this is. At the text tint, not the dim one: dim is the
        // faintest thing the palette has, and the version is the answer to the
        // first question anyone asks about a build.
        //
        // The commit used to follow it, out of a build.rs stamp. It is gone:
        // seven characters of hex is not something anyone reads off a title,
        // and the room they took now says where the agent is working.
        Run::tinted(format!(" {VERSION}"), skin.title),
    ];
    // Then the folder this session is in, after the same marker that separates
    // the name from the version. Clipped by column against the room the strip
    // actually has, because the strip is one box with no ellipsis of its own
    // and a deep path would be cut mid-glyph instead of shortened. Before
    // SessionStart there is no folder, and a marker with nothing after it is
    // worse than no marker.
    if !state.workspace.is_empty() {
        // One estimated advance, the guess the rest of this strip measures with.
        let taken = "NO0B \u{25b8}".chars().count() + VERSION.chars().count() + 4;
        let space = columns_in(room, SMALL * 0.6).saturating_sub(taken);
        if space > 1 {
            runs.push(Run::tinted(" \u{25b8}", skin.bright));
            runs.push(Run::tinted(
                format!(" {}", clip(&short_path(&state.workspace), space)),
                skin.title,
            ));
        }
    }
    // Open, the strip says which build this is and where it is working. The
    // phase, the model and the token budget were readings squeezed into a title
    // with no room to label them; they belong in the monitors, which have both.
    // Trouble stays because it is the one thing that makes the rest of the
    // window meaningless.
    if let Some(trouble) = frame.trouble {
        runs.push(Run::tinted(format!("   {trouble}"), skin.bad));
    } else if layout.shaded {
        // Shaded, this strip is the whole window, so it carries the one thing
        // worth knowing while there is nowhere else to read it. In the bad
        // colour while a turn is running, the same as the phase reads in the
        // pane it comes from: the word and the orb beside it then say the same
        // thing, which is the whole job of a strip this small.
        let tint = match state.phase.busy() {
            true => skin.bad,
            false => skin.good,
        };
        runs.push(Run::tinted(format!("   {}", state.headline()), tint));
    }
    let row = strip_row(Panel::new(ORB_W, layout.title.y, room, layout.title.h));
    scene.text(Text::rich(runs, row, SMALL, skin.title).line_height(row.h));
}
/// One tab of a strip, before its label goes on.
///
/// A tab is not a button. Both states carry the pane's own surface and the same
/// cut corner the pane has, so the tab reads as the top of the pane; what says
/// which one is showing is weight. The showing tab is that surface at full
/// strength with an accent line along its top, the rest are the same colour at
/// a lower alpha. A filled block over a filled strip is what made these look
/// like a row of buttons.
///
/// One green for every view, not a hue each. Nine hues on nine tabs is a
/// harlequin strip, and it was answering a question nobody asked: which pane
/// this is is written on it, and all the line has to say is which one you are
/// looking at.
///
/// `Skin::tab` is exactly `Skin::panel`, and the showing tab sits flush on the
/// pane, so the two composite to one surface with nothing between them. That is
/// the other half of losing the line under the strip ([`pane_edges`]): a step in
/// colour where the line was is the same complaint as the line.
pub(crate) fn tab_block(scene: &mut Scene, skin: &Skin, tab: Panel, active: bool) {
    let cut = cut_of(tab);
    scene.rect(
        tab.fill(if active { skin.tab } else { skin.tab_idle })
            .chamfer(cut, Rect::TOP_RIGHT),
    );
    if !active {
        return;
    }
    // Stopped where the cut starts. Run to the full width and the last pixels
    // of the line hang in a corner that is not there any more.
    scene.rect(
        Panel::new(tab.x, tab.y, (tab.w - cut).max(1.0), ACCENT_H.min(tab.h))
            .fill(skin.tab_accent),
    );
    // And picked up again there, so the accent turns the corner instead of
    // stopping in mid air: down the cut, then on down the right edge. The
    // diagonal is the heavy one ([`CUT_EDGE_H`]) and the two sides are
    // hairlines, because the diagonal is the mark that says what shape a tab is
    // and the sides only have to say where it ends.
    let thin = TAB_EDGE_H.min(tab.h);
    cut_line(scene, tab, skin.tab_accent, CUT_EDGE_H.min(tab.h));
    scene.rect(
        Panel::new(
            tab.x + tab.w - thin,
            tab.y + cut,
            thin,
            (tab.h - cut).max(0.0),
        )
        .fill(skin.tab_accent),
    );
    // The left side runs the whole height, since nothing stops it: the accent
    // starts at the same x and there is no bottom border for it to meet.
    scene.rect(Panel::new(tab.x, tab.y, thin, tab.h).fill(skin.tab_accent));
    // And no foot. A line on the tab's last row is a line at the pane's top
    // edge, which is the rule under the strip that item 12 took away; the tab
    // and its pane are one surface and nothing is drawn across the seam.
}
/// The two arrows at the right end of a strip that holds more tabs than it can
/// show, and nothing at all on one that fits.
///
/// Glyphs and no box. The strip itself is not a surface (see [`space_pane`]), and
/// a filled block at that end of it would sit square over the cut corner of the
/// pane below, which is the stray corner the strip's own fill was taken away for.
/// The direction that has nowhere left to go is dimmed instead of hidden, so the
/// pair does not move under the pointer at either end of the walk.
pub(crate) fn strip_arrows(scene: &mut Scene, frame: &Frame, space: Space) {
    let placed = frame.layout.placed(space);
    if placed.arrow_left.w < 1.0 {
        return;
    }
    let slot = frame.dock.slot(space);
    // Live while there is another tab that way at all, which is what an arrow
    // walks to. Not whether the strip itself can still move: at the end of the
    // strip the last few tabs are all on screen together, and the arrow still
    // steps the showing tab through them.
    let at = slot.active_index().unwrap_or(0);
    let line = Text::line_for(SMALL);
    for (panel, glyph, live) in [
        (placed.arrow_left, icons::TABS_LEFT, at > 0),
        (
            placed.arrow_right,
            icons::TABS_RIGHT,
            at + 1 < slot.views.len(),
        ),
    ] {
        let ink = if live {
            frame.skin.bright
        } else {
            frame.skin.dim
        };
        // The box runs to the arrow's right edge rather than being sized to one
        // guessed advance, the way the window buttons do it: a box exactly one
        // estimated advance wide clips the glyph in it.
        let left = ((panel.w - SMALL * 0.6) * 0.5).max(0.0).floor();
        scene.text(Text::rich(
            vec![Run::icon(glyph.to_string(), ink)],
            Panel::new(
                panel.x + left,
                panel.y + ((panel.h - line) * 0.5).max(0.0).floor(),
                panel.w - left,
                line,
            ),
            SMALL,
            ink,
        ));
    }
}
pub(crate) fn space_pane(scene: &mut Scene, frame: &Frame, space: Space) {
    let skin = frame.skin;
    let placed = frame.layout.placed(space);
    let slot = frame.dock.slot(space);
    if placed.strip.w < 1.0 {
        return;
    }

    // The strip itself is not drawn. It is the window, not a toolbar, and the
    // tabs standing in it are the only thing up here. Its fill and the hairline
    // along its foot were both square, so they ran past the cut corner of the
    // pane below and left a stray stroke there. Nothing spans the strip now, and
    // nothing runs along the pane's top edge either: the tab and the pane are one
    // surface, which is what item 12 asked for.
    for (view, panel) in &placed.tabs {
        let active = slot.active() == Some(*view);
        let lifted = frame.drag.is_some_and(|drag| drag.view == *view);
        tab_block(scene, skin, *panel, active);
        // Not showing reads as not showing. This was the title tint, as strong
        // as the showing tab's, which left the fill to carry the whole
        // difference and is why the fill had to be so heavy.
        let color = if active && !lifted {
            skin.bright
        } else {
            skin.dim
        };
        scene.text(Text::rich(
            vec![Run::tinted(
                tab_label(*view, frame.state.shown_agent),
                color,
            )],
            panel.row(SMALL * 0.6, Text::line_for(SMALL)),
            SMALL,
            color,
        ));
    }
    strip_arrows(scene, frame, space);
    if slot.folded || placed.body.h < 2.0 {
        return;
    }
    let panel = placed.body;
    scene.rect(panel_fill(panel, skin.panel));
    // Three sides, not four. The missing one is the top, which was the line under
    // the tabs; see [`pane_edges`]. The fill still carries the cut, so the corner
    // is unchanged.
    //
    // The same three edges whether or not a drop would land here. A space being
    // dragged onto used to be lit in `edge_focus` instead, and once the top edge
    // went the lit outline no longer closed around the pane, so it read as a
    // pane with a coloured left side rather than as a target. What says a drop
    // lands here now is a box over the whole space; see [`drop_target`].
    pane_edges(scene, panel, skin.edge);

    // Banded in the box the text is actually in, which is the whole body for
    // every pane but the file one: the file view spends its left column on the
    // explorer, and banding the body there put the highlight a list's width off
    // the glyphs it was meant to cover.
    selection_band(scene, frame, frame.layout.content(space), slot.active());

    match slot.active() {
        None => {}
        Some(View::Output) => crate::widgets::output::output(scene, frame, panel),
        Some(View::Activity) => crate::widgets::activity::activity(scene, frame, panel),
        Some(View::Plan) => crate::widgets::plan::plan(scene, frame, panel),
        Some(View::Agents) => crate::widgets::agents::agents(scene, frame, panel),
        Some(View::Agent) => crate::widgets::agent::agent(scene, frame, panel),
        Some(View::Hardware) => {
            crate::widgets::gauges::gauges(scene, frame, panel, View::Hardware, frame.monitor.hardware())
        }
        // The monitor's lists are named for the panes they feed, so a reading in
        // the wrong pane is a rename away from being obvious rather than two
        // files away.
        Some(View::Context) => crate::widgets::context::context(scene, frame, panel),
        Some(View::Session) => crate::widgets::gauges::gauges(scene, frame, panel, View::Session, frame.monitor.session()),
        Some(View::Files) => crate::widgets::files::files(scene, frame, panel),
    }
}
pub(crate) fn input_row(scene: &mut Scene, frame: &Frame) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    scene.rect(panel_fill(layout.input, skin.input));
    scene.rect(panel_edge(layout.input, skin.edge_focus));
    let line = Text::line_for(frame.body_size);
    let box_ = input_box(layout.input, line);
    let columns = columns_in(box_.w, frame.column);
    // What the box can hold and how much of the prompt is above it. Everything
    // below is drawn from `top`, which is the first row's y whether or not that
    // row is on screen, so the rows that are on screen land where the caret
    // arithmetic and the click arithmetic both say they do.
    let skip = prompt_skip(frame.prompt.caret(), columns, rows_in(box_, line));
    let top = box_.y - skip as f32 * line;
    // Under the glyphs, like the band in a pane, so selected text stays
    // readable rather than being painted over.
    if let Some((from, to)) = frame.prompt.selection() {
        let mut at = from + PROMPT_COLUMNS;
        let end = to + PROMPT_COLUMNS;
        while at < end {
            let row = at / columns;
            // One rectangle per visual row: a selection that wrapped is not
            // one rectangle, it is a run on each row it crosses.
            let stop = end.min((row + 1) * columns);
            let band = Panel::new(
                box_.x + (at % columns) as f32 * frame.column,
                top + row as f32 * line,
                (stop - at) as f32 * frame.column,
                line,
            );
            if band.y >= box_.y - 0.5 && band.y + band.h <= box_.y + box_.h + 0.5 {
                scene.rect(band.fill(skin.select));
            }
            at = stop;
        }
    }
    // The marker slot, two columns of it. At rest it is the prompt's chevron and
    // a space. While a turn runs it is two blanks, and the three dots that used
    // to be an ellipsis glyph are drawn into it below as rectangles: one shaped
    // box has one baseline, so a glyph cannot be lifted off it on its own.
    let busy = state.phase.busy();
    let marker = if busy { "  " } else { "\u{203a} " };
    let mut runs = vec![
        Run::tinted(marker.to_string(), skin.dim),
        Run::tinted(frame.prompt.text(), skin.bright),
    ];
    // Armed, the empty prompt says what the second ESC does, in the colour
    // that means stop. It sits where the eye already is: on the line the
    // first ESC was aimed at.
    if frame.esc_armed {
        runs.push(Run::tinted("press ESC again to cancel", skin.bad));
    }
    scene.text(
        Text::rich(runs, box_, frame.body_size, skin.bright)
        // Broken on the column the caret is placed by, so counting columns
        // lands on the glyph that is really there. This is the one box in the
        // window that is not wrapped at blanks: a row that ended early would
        // put the caret a word away from the character it is on, since
        // everything here is `row * columns + column`. The panes wrap at
        // blanks, and their rows are counted the same way they are drawn.
        .break_at(columns)
        // The rows above the window are paid for and not drawn, the way a pane
        // showing the tail of a long stream is. Without this a prompt longer
        // than its allowance goes on being typed into a box that shows only its
        // first rows, which is a setting that appears to do nothing.
        .scrolled(skip as f32),
    );
    // The dots, on the first row of the box, in the marker's own two columns.
    // Round, because they stand in for three round glyphs and because the orb in
    // the strip is round for exactly as long as they are on screen. Pushed
    // before the caret so the caret is still the last thin rectangle in the row,
    // which is how the tests find it.
    // Skipped once the first row has scrolled off the top: the marker's two
    // blank columns went with it, and three dots over somebody's text is not a
    // marker, it is three dots in the way.
    if busy && skip == 0 {
        let span = 3.0 * PROMPT_DOT + 2.0 * PROMPT_DOT_GAP;
        let slack = (PROMPT_COLUMNS as f32 * frame.column - span).max(0.0) * 0.5;
        let rest = box_.y + (line - PROMPT_DOT) * 0.5;
        for (index, lift) in prompt_wave(frame.clock, busy).into_iter().enumerate() {
            let dot = Panel::new(
                box_.x + slack + index as f32 * (PROMPT_DOT + PROMPT_DOT_GAP),
                rest - lift,
                PROMPT_DOT,
                PROMPT_DOT,
            );
            scene.rect(dot.fill(skin.caret).radius(PROMPT_DOT * 0.5));
        }
    }
    let at = frame.prompt.caret() + PROMPT_COLUMNS;
    let (row, column) = (at / columns, at % columns);
    let caret = Panel::new(
        box_.x + column as f32 * frame.column,
        top + row as f32 * line,
        2.0,
        line,
    );
    // Always true, since the box is scrolled to the caret's row; the check is
    // what makes that a fact rather than an assumption.
    if caret.y >= box_.y - 0.5 && caret.y + caret.h <= box_.y + box_.h + 0.5 {
        scene.rect(caret.fill(skin.caret));
    }
}
/// How many of the prompt's rows have scrolled off the top of its box.
///
/// As few as it takes to keep the caret's row inside the box: nothing while the
/// prompt fits, and then one row per row typed past the allowance, so what you
/// are typing is on screen and what you typed earlier is above it. Drawing and
/// hit testing both come through here, or a click would land a row off on
/// anything that had scrolled.
pub(crate) fn prompt_skip(caret: usize, columns: usize, rows: usize) -> usize {
    let row = (caret + PROMPT_COLUMNS) / columns.max(1);
    row.saturating_sub(rows.max(1) - 1)
}
/// How far each of the prompt's three dots is off its rest line, in pixels.
///
/// The wave as asked for: one dot up while the other two are down, then the next
/// one, and around again. Stepped rather than a sine, because three dots
/// bouncing smoothly read as a wobble and three dots taking turns read as a
/// wave.
///
/// Level whenever nothing is running, and that is not a nicety: the window holds
/// a redraw deadline only while a turn does, so a lift that moved at rest would
/// be a frame that never arrives and a dot stuck wherever the last redraw left
/// it.
pub(crate) fn prompt_wave(clock: f32, busy: bool) -> [f32; 3] {
    let mut lift = [0.0; 3];
    if busy {
        lift[(clock.max(0.0) / PROMPT_DOT_STEP) as usize % 3] = PROMPT_DOT_LIFT;
    }
    lift
}
/// The box the prompt's text is drawn in, inside the strip the layout gave it.
///
/// Top-aligned so the first line does not move as the prompt grows. Drawing
/// and hit testing both take it from here, which is the only way a click can
/// land on the column the glyph is actually in.
pub(crate) fn input_box(input: Panel, line: f32) -> Panel {
    Panel::new(
        input.x + PAD,
        input.y + INPUT_PAD,
        (input.w - 2.0 * PAD).max(1.0),
        (input.h - 2.0 * INPUT_PAD).max(line),
    )
}
/// The tab under the pointer while it is being dragged, so the drag has
/// something following it and the drop has somewhere to be aimed.
///
/// On the floating layer, like the box under it: a tab in the air is the most
/// floating thing there is, and in the base layer its own box was painted before
/// every glyph in the window, so it slid under the text of whatever pane it
/// crossed.
pub(crate) fn dragging(scene: &mut Scene, frame: &Frame) {
    let Some(drag) = frame.drag else {
        return;
    };
    let skin = frame.skin;
    let label = tab_label(drag.view, frame.state.shown_agent);
    let w = (label.chars().count() as f32 + 3.0) * frame.column;
    let ghost = Panel::new(drag.at.0 - w * 0.5, drag.at.1 - TAB_H * 0.5, w, TAB_H);
    // Out of the window, letting go closes the widget, so the tab in the air says
    // so: its edge and its label go to the bad colour, and there is no green box
    // anywhere on screen because there is no space to land in. The pointer says
    // the same thing (`main`'s `cursor_for`), and neither is enough on its own:
    // the cursor is 20 pixels of somebody else's theme and the ghost is the thing
    // being carried.
    let out = drag.landing == Landing::Out;
    let (edge, ink) = match out {
        true => (skin.drop_out, skin.bad),
        false => (skin.edge_focus, skin.bright),
    };
    scene.over_rect(ghost.fill(skin.bar));
    scene.over_rect(ghost.outline(edge, 1.0));
    scene.over_text(Text::rich(
        vec![Run::tinted(label, ink)],
        ghost.row(SMALL * 0.6, Text::line_for(SMALL)),
        SMALL,
        ink,
    ));
}
/// What a drop would do, drawn over the room it would take: a translucent green
/// box, and a caret in the gap between the two tabs the tab would land between.
///
/// On the floating layer, so it covers the pane rather than being painted under
/// the pane's own text the way a base-layer rectangle is (see [`overlay`]). A
/// wash under the glyphs is exactly the feedback item 17 said it could not see.
///
/// The box is the room the pane would have after the drop, which is one cell of
/// the grid or two, so a drop between two cells shows the pair before the button
/// comes up. It is taken by making the move on a copy of the dock and asking that
/// copy which cells the pane would cover: the box and the move come off one
/// answer rather than two, so they cannot promise different things.
///
/// A drop on a tab strip is the exception: it names a place among tabs and does
/// not move anything on the grid, so the box is the pane that is already drawn
/// there. Folded, that pane is its strip and nothing else, which is all there is
/// of it to point at.
///
/// The caret is only drawn for a drop that names a place, which is a drop on a
/// tab strip. In the body of a pane there is no gap being aimed at: the tab goes
/// to the end of the space, and a caret standing between two tabs would promise
/// a position the drop does not name.
pub(crate) fn drop_target(scene: &mut Scene, frame: &Frame) {
    let Some(drag) = frame.drag else {
        return;
    };
    let (skin, layout) = (frame.skin, frame.layout);
    let box_ = match drag.landing {
        Landing::In(space, Some(_)) => {
            let placed = layout.placed(space);
            Panel::new(
                placed.strip.x,
                placed.strip.y,
                placed.strip.w,
                placed.strip.h + placed.body.h,
            )
        }
        Landing::In(..) | Landing::Span(..) => drop_room(layout, frame.dock, drag),
        Landing::Out | Landing::Nowhere => return,
    };
    if box_.w < 1.0 || box_.h < 1.0 {
        return;
    }
    // The same cut corner every panel in the window has, so the box lies on the
    // pane instead of squaring off its top right corner.
    scene.over_rect(box_.fill(skin.drop_target).chamfer(cut_of(box_), Rect::TOP_RIGHT));
    let Landing::In(space, Some(at)) = drag.landing else {
        return;
    };
    let placed = layout.placed(space);
    let x = layout
        .insertion_gap(space, at)
        .min(placed.strip.x + placed.strip.w - CARET_W);
    scene.over_rect(Panel::new(x, placed.strip.y, CARET_W, placed.strip.h).fill(skin.drop_mark));
}
/// The room a pane would have once this drop had happened.
///
/// The move is made on a copy of the dock, so the cells it answers with are the
/// cells the real move would give it, spans and emptied neighbours included.
pub(crate) fn drop_room(layout: &Layout, dock: &Dock, drag: Drag) -> Panel {
    let mut after = dock.clone();
    match drag.landing {
        Landing::In(space, None) => {
            after.move_view(drag.view, space);
        }
        Landing::Span(a, b) => {
            after.span_view(drag.view, a, b);
        }
        _ => return nowhere(),
    }
    let Some(head) = after.space_of(drag.view) else {
        return nowhere();
    };
    let cover = after.cover();
    Space::ALL
        .into_iter()
        .filter(|cell| cover[cell.index()] == Some(head))
        .fold(nowhere(), |box_, cell| around(box_, layout.grid[cell.index()]))
}
