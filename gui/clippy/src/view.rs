//! Layout, hit regions, and turning state into a scene.
//!
//! One surface carved into three spaces, never several OS windows. The window
//! has no system chrome, so the title bar, its three buttons, the tab strips,
//! the scrollbars and the resize edges are all rectangles here and hit regions
//! in [`Layout`]. Drawing and hit testing take the same numbers from the same
//! place, which is the only way they can never disagree.
//!
//! Every view is a tab in one of the three spaces and can be dragged into
//! another; [`crate::dock`] owns that arrangement, and this module only asks it
//! where things are.
//!
//! The window has two shapes. Open, it is three spaces. Shaded, it is one strip
//! carrying [`State::headline`] and nothing else, the way Winamp collapsed to
//! its title. Double-click the bar to go between them.

use noob_draw::{Panel, Rect, Run, Scene, Text};

use crate::dock::{Dock, Space, View};
use crate::monitor::{Gauge, Monitor};
use crate::skin::Skin;
use crate::state::{State, TodoState, Tone};

pub const TITLE_H: f32 = 30.0;
pub const INPUT_H: f32 = 36.0;
pub const TAB_H: f32 = 22.0;
pub const RESIZE_EDGE: f32 = 6.0;
const GAP: f32 = 6.0;
const PAD: f32 = 9.0;
/// Columns the file view spends on its line-number gutter, on every row.
const GUTTER: usize = 4;
const SMALL: f32 = 12.0;
const SCROLL_W: f32 = 4.0;
const BUTTON_W: f32 = 26.0;
const LABEL_COLUMNS: usize = 9;
/// A gauge bar is a grid of dots: ten columns is 0 to 100 percent, so one
/// column is 10 percent and one of its four dots is 2.5 percent.
const DOT_COLUMNS: usize = 10;
const DOT_ROWS: usize = 4;
const PROMPT_COLUMNS: usize = 2;
const INPUT_PAD: f32 = 6.0;
/// How far the 45 degree cut reaches along each edge of a panel's top-right
/// corner. One corner, so the shape reads as a mark rather than as a rounded
/// box, and always the same corner so two panels side by side still line up.
const CUT: f32 = 10.0;

/// Something the pointer can land on. Returned by [`Layout::hit`] so every
/// click is resolved in one place instead of in a chain of `if` in the event
/// handler.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    TitleBar,
    Minimize,
    Maximize,
    Close,
    /// A view's tab, in the space it currently lives in.
    Tab(View, Space),
    /// The body of a space: where a dragged tab lands.
    Body(Space),
    /// One of the file view's inner tabs, and the space it is showing in. The
    /// space is carried so a tab dropped over the file strip still lands
    /// somewhere: a drop target is a place on screen, not a widget.
    File(usize, Space),
    Input,
}

impl Hit {
    /// The space a drop here would move a view into.
    pub fn space(self) -> Option<Space> {
        match self {
            Hit::Tab(_, space)
            | Hit::Body(space)
            | Hit::File(_, space) => Some(space),
            _ => None,
        }
    }
}

/// Where one space is, and where its tabs are.
pub struct Placed {
    pub strip: Panel,
    pub body: Panel,
    pub tabs: Vec<(View, Panel)>,
}

/// Where everything is this frame. Built from the window size and the dock, so
/// nothing else has to recompute it.
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub shaded: bool,

    pub title: Panel,
    pub minimize: Panel,
    pub maximize: Panel,
    pub close: Panel,

    /// One per [`Space`], in `Space::ALL` order.
    pub spaces: [Placed; 3],
    /// The file view's inner tabs, and the space they are drawn in.
    pub file_tabs: Vec<(usize, Panel)>,
    pub files_in: Option<Space>,
    pub input: Panel,
}

/// What the layout needs beyond the window size.
pub struct Shape<'a> {
    pub shaded: bool,
    pub dock: &'a Dock,
    /// One label per file tab, in order.
    pub file_labels: Vec<String>,
    pub column: f32,
    /// How tall the prompt is. It grows with what has been typed, so it is an
    /// input to the layout rather than a constant.
    pub input_h: f32,
}

fn nowhere() -> Panel {
    Panel::new(0.0, 0.0, 0.0, 0.0)
}

fn empty_placed() -> Placed {
    Placed {
        strip: nowhere(),
        body: nowhere(),
        tabs: Vec::new(),
    }
}

impl Layout {
    pub fn compute(width: f32, height: f32, shape: &Shape) -> Layout {
        let whole = Panel::new(0.0, 0.0, width, height);
        let (title, rest) = whole.split_top(TITLE_H.min(height));
        let buttons = [
            Panel::new(width - BUTTON_W * 3.0, 0.0, BUTTON_W, TITLE_H),
            Panel::new(width - BUTTON_W * 2.0, 0.0, BUTTON_W, TITLE_H),
            Panel::new(width - BUTTON_W, 0.0, BUTTON_W, TITLE_H),
        ];

        if shape.shaded {
            // One strip and nothing else. Every other region collapses to
            // nothing so a stale hit region cannot survive the shape change.
            return Layout {
                width,
                height,
                shaded: true,
                title,
                minimize: buttons[0],
                maximize: buttons[1],
                close: buttons[2],
                spaces: [empty_placed(), empty_placed(), empty_placed()],
                file_tabs: Vec::new(),
                files_in: None,
                input: nowhere(),
            };
        }

        let (body, input) = rest.split_bottom(shape.input_h.max(INPUT_H).min(rest.h));
        let body = body.inset(GAP);

        // An empty space gives its room away rather than leaving a hole.
        let has = |space: Space| !shape.dock.slot(space).is_empty();
        let (left, right) = if has(Space::Left) && (has(Space::TopRight) || has(Space::BottomRight))
        {
            let split = (body.w * 0.54).floor() - GAP * 0.5;
            let (left, right) = body.split_left(split);
            (
                left,
                Panel::new(right.x + GAP, right.y, (right.w - GAP).max(1.0), right.h),
            )
        } else if has(Space::Left) {
            (body, nowhere())
        } else {
            (nowhere(), body)
        };

        let folded = |space: Space| shape.dock.slot(space).folded;
        let (top, bottom) = match (has(Space::TopRight), has(Space::BottomRight)) {
            (false, false) => (nowhere(), nowhere()),
            (true, false) => (right, nowhere()),
            (false, true) => (nowhere(), right),
            (true, true) => {
                let top_h = match (folded(Space::TopRight), folded(Space::BottomRight)) {
                    (true, _) => TAB_H,
                    (false, true) => (right.h - TAB_H - GAP).max(TAB_H),
                    (false, false) => ((right.h - GAP) * 0.46).max(TAB_H).floor(),
                };
                let (top, lower) = right.split_top(top_h.min(right.h));
                (
                    top,
                    Panel::new(lower.x, lower.y + GAP, lower.w, (lower.h - GAP).max(0.0)),
                )
            }
        };

        let place = |space: Space, area: Panel| -> Placed {
            if area.w < 1.0 || area.h < 1.0 {
                return empty_placed();
            }
            let (strip, rest) = area.split_top(TAB_H.min(area.h));
            let room = strip;
            let slot = shape.dock.slot(space);
            let tabs = strip_tabs(
                room,
                slot.views.iter().map(|v| v.label().chars().count()),
                shape.column,
            )
            .into_iter()
            .enumerate()
            .map(|(i, panel)| (slot.views[i], panel))
            .collect();
            Placed {
                strip,
                body: if slot.folded {
                    Panel::new(rest.x, rest.y, rest.w, 0.0)
                } else {
                    rest
                },
                tabs,
            }
        };

        let spaces = [
            place(Space::Left, left),
            place(Space::TopRight, top),
            place(Space::BottomRight, bottom),
        ];

        // The file view's inner tabs live along the top of whichever space is
        // showing it.
        let files_in = shape.dock.space_of(View::Files).filter(|space| {
            shape.dock.slot(*space).active() == Some(View::Files)
                && !shape.dock.slot(*space).folded
        });
        let file_tabs = match shape.dock.space_of(View::Files) {
            Some(space)
                if shape.dock.slot(space).active() == Some(View::Files)
                    && !shape.dock.slot(space).folded =>
            {
                let body = &spaces[Space::ALL.iter().position(|s| *s == space).unwrap()].body;
                if body.h > TAB_H * 2.0 {
                    let bar = Panel::new(body.x, body.y, body.w, TAB_H);
                    strip_tabs(
                        bar,
                        shape.file_labels.iter().map(|l| l.chars().count() + 1),
                        shape.column,
                    )
                    .into_iter()
                    .enumerate()
                    .collect()
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        };

        Layout {
            width,
            height,
            shaded: false,
            title,
            minimize: buttons[0],
            maximize: buttons[1],
            close: buttons[2],
            spaces,
            file_tabs,
            files_in,
            input: input.inset(GAP),
        }
    }

    pub fn placed(&self, space: Space) -> &Placed {
        &self.spaces[Space::ALL.iter().position(|s| *s == space).unwrap()]
    }

    /// What is under a point. One place, so a click and the thing it appears to
    /// land on can never come apart.
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        for (panel, hit) in [
            (self.close, Hit::Close),
            (self.maximize, Hit::Maximize),
            (self.minimize, Hit::Minimize),
        ] {
            if panel.contains(x, y) {
                return Some(hit);
            }
        }
        if self.title.contains(x, y) {
            return Some(Hit::TitleBar);
        }
        if self.shaded {
            return None;
        }
        for space in Space::ALL {
            let placed = self.placed(space);
            for (view, panel) in &placed.tabs {
                if panel.contains(x, y) {
                    return Some(Hit::Tab(*view, space));
                }
            }
        }
        if let Some(space) = self.files_in {
            for (index, panel) in &self.file_tabs {
                if panel.contains(x, y) {
                    return Some(Hit::File(*index, space));
                }
            }
        }
        for space in Space::ALL {
            let placed = self.placed(space);
            if placed.body.contains(x, y) || placed.strip.contains(x, y) {
                return Some(Hit::Body(space));
            }
        }
        if self.input.contains(x, y) {
            return Some(Hit::Input);
        }
        None
    }

    /// Rows a panel can show. The header line is content, not scrollback.
    pub fn rows(&self, panel: Panel, size: f32) -> usize {
        Text::rows_for(size, panel.inset(PAD).h)
    }

    /// Which pane the pointer is over, and which character cell of it.
    ///
    /// Arithmetic rather than a layout query, which is what a monospace grid
    /// buys: the renderer never has to be asked where a glyph landed. The
    /// column is rounded to the nearest boundary rather than floored, so
    /// pressing on the right half of a character puts the caret after it, the
    /// way a text cursor behaves everywhere else.
    pub fn cell(&self, x: f32, y: f32, size: f32, column: f32) -> Option<(Space, usize, usize)> {
        if self.shaded || column <= 0.0 {
            return None;
        }
        let line = Text::line_for(size);
        for space in Space::ALL {
            let body = self.placed(space).body.inset(PAD);
            if !body.contains(x, y) {
                continue;
            }
            let row = ((y - body.y) / line).floor().max(0.0) as usize;
            let at = (((x - body.x) / column).round().max(0.0)) as usize;
            return Some((space, row, at));
        }
        None
    }

    /// Where a click in the prompt puts the caret, as a character offset into
    /// the typed text.
    ///
    /// The inverse of the arithmetic [`input_row`] draws the caret with, off
    /// the same box, so the caret lands under the pointer instead of near it.
    /// A click past the end of the text lands at the end, which is why `chars`
    /// is passed in.
    pub fn input_caret(&self, x: f32, y: f32, size: f32, column: f32, chars: usize) -> usize {
        if column <= 0.0 {
            return chars;
        }
        let line = Text::line_for(size);
        let box_ = input_box(self.input, line);
        let columns = columns_in(box_.w, column);
        let row = ((y - box_.y) / line).floor().max(0.0) as usize;
        // Rounded, not floored, so pressing on the right half of a character
        // puts the caret after it, the way a text cursor behaves everywhere.
        let at = ((((x - box_.x) / column).round().max(0.0)) as usize).min(columns);
        // The marker in front of the text owns the first columns of the first
        // row, so a click on it means the start of the text.
        (row * columns + at).saturating_sub(PROMPT_COLUMNS).min(chars)
    }
}

/// Lay tabs left to right at the width their labels need, dropping any that do
/// not fit rather than squeezing them into unreadable slivers.
fn strip_tabs(bar: Panel, widths: impl Iterator<Item = usize>, column: f32) -> Vec<Panel> {
    let mut out = Vec::new();
    let mut x = bar.x;
    for chars in widths {
        let w = (chars as f32 + 3.0) * column;
        if x + w > bar.x + bar.w {
            break;
        }
        out.push(Panel::new(x, bar.y, w, bar.h));
        x += w;
    }
    out
}

/// Which edge, if any, a point is on. An undecorated window loses the window
/// manager's resize handles, so these are ours to provide.
pub fn edge(x: f32, y: f32, width: f32, height: f32) -> Option<winit::window::ResizeDirection> {
    use winit::window::ResizeDirection as Dir;
    let left = x <= RESIZE_EDGE;
    let right = x >= width - RESIZE_EDGE;
    let top = y <= RESIZE_EDGE;
    let bottom = y >= height - RESIZE_EDGE;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(Dir::NorthWest),
        (_, true, true, _) => Some(Dir::NorthEast),
        (true, _, _, true) => Some(Dir::SouthWest),
        (_, true, _, true) => Some(Dir::SouthEast),
        (true, ..) => Some(Dir::West),
        (_, true, ..) => Some(Dir::East),
        (_, _, true, _) => Some(Dir::North),
        (_, _, _, true) => Some(Dir::South),
        _ => None,
    }
}

/// A tab being dragged, and where it would land if it were dropped now.
#[derive(Clone, Copy, Debug)]
pub struct Drag {
    pub view: View,
    pub at: (f32, f32),
    pub onto: Option<Space>,
}

pub struct Frame<'a> {
    pub state: &'a State,
    pub monitor: &'a Monitor,
    pub dock: &'a Dock,
    pub skin: &'a Skin,
    pub layout: &'a Layout,
    /// What has been typed, where the caret is, and what is selected in it.
    pub prompt: &'a crate::prompt::Prompt,
    pub column: f32,
    /// The column width at `pane_size`. The panes are a different size from
    /// the transcript, so anything that lines text up with a rectangle has to
    /// use this one.
    pub pane_column: f32,
    pub body_size: f32,
    pub pane_size: f32,
    pub drag: Option<Drag>,
    /// What the pointer is over, for the button highlight.
    pub hot: Option<Hit>,
    /// Shown in the title bar when the agent could not be reached.
    pub trouble: Option<&'a str>,
    /// A drag over one of the text panes, drawn as a band under the glyphs.
    pub selection: Option<crate::select::Selection>,
    /// The avatar clip and how far into it we are. None when the settings
    /// turned it off or the named clip could not be read.
    pub avatar: Option<(&'a crate::avatar::Avatar, u64)>,
}

impl Frame<'_> {
    /// The font size and column width a view is actually drawn with.
    ///
    /// Talk uses the transcript size and every other pane the smaller one.
    /// Measuring a pane with the wrong one of the two is what put the
    /// selection band and the hit test off the glyphs they were describing,
    /// so nothing may reach for `body_size` or `pane_size` directly when the
    /// view is a variable.
    pub fn metrics_of(&self, view: View) -> (f32, f32) {
        match view {
            View::Talk => (self.body_size, self.column),
            _ => (self.pane_size, self.pane_column),
        }
    }
}

pub fn build(frame: &Frame) -> Scene {
    let mut scene = Scene::default();
    let layout = frame.layout;

    scene.rect(Panel::new(0.0, 0.0, layout.width, layout.height).fill(frame.skin.backdrop));
    title_bar(&mut scene, frame);
    if layout.shaded {
        return scene;
    }

    for space in Space::ALL {
        space_pane(&mut scene, frame, space);
    }
    input_row(&mut scene, frame);
    dragging(&mut scene, frame);
    scene
}

fn title_bar(scene: &mut Scene, frame: &Frame) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    scene.rect(layout.title.fill(skin.bar));

    // How full the context is, as a hairline along the bottom of the strip.
    // It was a bar of its own at the foot of the window; two pixels at the top
    // of the window says the same thing and costs no rows.
    let gauge = Panel::new(0.0, layout.title.y + layout.title.h - 2.0, layout.width, 2.0);
    scene.rect(gauge.fill(skin.gauge_track));
    let used = state.context_fraction();
    if used > 0.0 {
        scene.rect(Panel::new(0.0, gauge.y, layout.width * used, 2.0).fill(skin.gauge));
    }

    // These were three hand-drawn rectangles, because the Unicode glyphs the
    // first version asked for were not on this machine and a missing glyph
    // draws as nothing. The symbol font ships in the binary now, so they are
    // the same marks every other window on the desktop uses.
    let line = Text::line_for(SMALL);
    for (panel, hit, tint, glyph, quiet) in [
        (layout.minimize, Hit::Minimize, skin.hot, crate::icons::MINIMIZE, true),
        (layout.maximize, Hit::Maximize, skin.hot, crate::icons::MAXIMIZE, true),
        (layout.close, Hit::Close, skin.close_hot, crate::icons::CLOSE, false),
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

    let room = (layout.width - BUTTON_W * 3.0 - 12.0).max(1.0);
    let mut runs = vec![
        Run::tinted("NO0B \u{25b8} CLIppy", skin.bright),
        // Which build this is. Stamped by build.rs from the commit, because a
        // crate version cannot tell two test builds apart.
        Run::tinted(format!(" {}", env!("CLIPPY_BUILD")), skin.dim),
    ];
    if let Some(trouble) = frame.trouble {
        runs.push(Run::tinted(format!("   {trouble}"), skin.bad));
    } else if layout.shaded {
        // Shaded, this strip is the whole window, so it carries the one thing
        // worth knowing rather than the model name and the path.
        runs.push(Run::tinted(format!("   {}", state.headline()), skin.good));
    } else {
        // The phase and the token budget used to live in a bar along the
        // bottom. They are the same two facts wherever they are drawn, and
        // here they cost no rows. Ordered so that a narrow window loses the
        // path before it loses what the agent is doing.
        runs.push(Run::tinted(
            format!("   {}", state.phase.word().to_lowercase()),
            skin.bright,
        ));
        runs.push(Run::tinted(
            format!(
                "   {}   {}{}   {}",
                if state.model.is_empty() {
                    "…"
                } else {
                    &state.model
                },
                short_path(&state.workspace),
                if state.resumed { "   resumed" } else { "" },
                state.budget_line(),
            ),
            skin.title,
        ));
    }
    scene.text(Text::rich(
        runs,
        Panel::new(0.0, 0.0, room, TITLE_H).row(12.0, Text::line_for(SMALL)),
        SMALL,
        skin.title,
    ));
}

/// The body of a panel: the fill, cut corner and all.
///
/// The cut lives on the fill as well as on the outline because they are the
/// same shape twice. A square fill under a cut outline shows a triangle of the
/// wrong colour poking out of the corner.
fn panel_fill(panel: Panel, rgba: [f32; 4]) -> Rect {
    panel.fill(rgba).chamfer(CUT, Rect::TOP_RIGHT)
}

/// Its hairline border, as one rectangle. Four of them could not follow the
/// cut.
fn panel_edge(panel: Panel, rgba: [f32; 4]) -> Rect {
    panel_fill(panel, rgba).stroke(1.0)
}

fn space_pane(scene: &mut Scene, frame: &Frame, space: Space) {
    let skin = frame.skin;
    let placed = frame.layout.placed(space);
    let slot = frame.dock.slot(space);
    if placed.strip.w < 1.0 {
        return;
    }

    // The strip. A space being dragged onto is lit along its whole edge, so a
    // drop target is a place rather than a guess.
    let target = frame.drag.is_some_and(|drag| drag.onto == Some(space));
    scene.rect(placed.strip.fill(skin.strip));
    scene.rect(placed.strip.bottom_edge(skin.edge));
    for (view, panel) in &placed.tabs {
        let active = slot.active() == Some(*view);
        let lifted = frame.drag.is_some_and(|drag| drag.view == *view);
        if active {
            scene.rect(panel.fill(skin.panel));
            scene.rect(panel.top_edge(skin.edge_focus));
        } else {
            scene.rect(panel.left_edge(skin.edge));
        }
        let color = match (lifted, active) {
            (true, _) => skin.dim,
            (_, true) => skin.bright,
            _ => skin.title,
        };
        scene.text(Text::rich(
            vec![Run::tinted(view.label(), color)],
            panel.row(SMALL * 0.6, Text::line_for(SMALL)),
            SMALL,
            color,
        ));
    }
    if slot.folded || placed.body.h < 2.0 {
        return;
    }
    let panel = placed.body;
    scene.rect(panel_fill(panel, skin.panel));
    scene.rect(panel_edge(
        panel,
        if target { skin.edge_focus } else { skin.edge },
    ));

    selection_band(scene, frame, panel, slot.active());

    match slot.active() {
        None => {}
        Some(View::Talk) => talk(scene, frame, panel),
        Some(View::Activity) => activity(scene, frame, panel),
        Some(View::Plan) => plan(scene, frame, panel),
        Some(View::Agents) => agents(scene, frame, panel),
        Some(View::Hardware) => gauges(scene, frame, panel, frame.monitor.hardware()),
        Some(View::Llm) => gauges(scene, frame, panel, frame.monitor.llm()),
        Some(View::Files) => files(scene, frame, panel),
        Some(View::Avatar) => avatar(scene, frame, panel),
    }
}

/// The avatar, centred in its panel.
///
/// Centred rather than pinned to a corner because it is the one view whose
/// content does not grow: everything else is a list that starts at the top and
/// runs down, and this is a picture.
fn avatar(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let skin = frame.skin;
    let Some((clip, at_ms)) = frame.avatar else {
        text_box(
            scene,
            frame,
            panel,
            frame.pane_size,
            vec![Run::tinted("avatar off", skin.dim)],
        );
        return;
    };
    let lines = clip.frame_at(at_ms);
    let content = panel.inset(PAD);
    let line_h = Text::line_for(frame.pane_size);
    let (w, h) = (
        clip.cols as f32 * frame.pane_column,
        lines.len() as f32 * line_h,
    );
    // A clip larger than the space it is in is drawn from the top left rather
    // than off both edges, which is what a negative offset would do.
    let box_ = Panel::new(
        content.x + ((content.w - w) / 2.0).max(0.0),
        content.y + ((content.h - h) / 2.0).max(0.0),
        w.min(content.w).max(1.0),
        h.min(content.h).max(line_h),
    );
    let mut runs = Vec::new();
    for line in lines {
        runs.push(Run::tinted(line, skin.body));
        runs.push(Run::plain("\n"));
    }
    scene.text(Text::rich(runs, box_, frame.pane_size, skin.body));
}

/// The band behind selected text, drawn before the glyphs go over it.
///
/// One rectangle per visible line of the selection rather than one for the
/// whole block, because the first and last lines start and stop mid-line and a
/// single rectangle would cover text that is not selected.
fn selection_band(scene: &mut Scene, frame: &Frame, panel: Panel, showing: Option<View>) {
    let (Some(selection), Some(view)) = (frame.selection, showing) else {
        return;
    };
    if selection.view != view || selection.is_empty() {
        return;
    }
    let Some(pane) = frame.state.pane_of(view) else {
        return;
    };
    // The pane's own size, not the pane size for everything: Talk is drawn at
    // the transcript size, and banding it at the smaller one is what put the
    // highlight off the glyphs it was supposed to cover.
    let (size, column) = frame.metrics_of(view);
    let content = panel.inset(PAD);
    let rows = frame.layout.rows(panel, size);
    let cols = cols_of(panel, column);
    let line_h = Text::line_for(size);
    let window = pane.window(rows, cols);
    let first = pane.showing_from(rows, cols);
    for step in 0..window.count {
        let number = first + step;
        let Some(line) = pane.line(number) else {
            continue;
        };
        let chars = line.text.chars().count();
        let Some((from, to)) = selection.columns_on(number, chars) else {
            continue;
        };
        let Some((top, height)) = pane.band_of(rows, cols, number) else {
            continue;
        };
        // A wrapped line needs one rectangle per visual row, each covering only
        // the part of the selection that lands on that row. The first line in
        // the window may start partway down, which is what `skip` records.
        let from_row = if step == 0 { window.skip } else { 0 };
        for i in 0..height {
            let wrapped = from_row + i;
            let row_start = wrapped * cols;
            let row_end = (row_start + cols).min(chars.max(row_start));
            let a = from.max(row_start);
            let b = to.min(row_end);
            if a >= b {
                continue;
            }
            let x = content.x + (a - row_start) as f32 * column;
            let width = ((b - a) as f32 * column).min(content.x + content.w - x);
            let y = content.y + (top + i) as f32 * line_h;
            if width <= 0.0 || y + line_h > content.y + content.h {
                continue;
            }
            scene.rect(Panel::new(x, y, width, line_h).fill(frame.skin.select));
        }
    }
}

fn text_box(scene: &mut Scene, frame: &Frame, panel: Panel, size: f32, runs: Vec<Run>) {
    scene.text(Text::rich(runs, panel.inset(PAD), size, frame.skin.body));
}

fn talk(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    let rows = frame.layout.rows(panel, frame.body_size);
    let cols = cols_of(panel, frame.column);
    let mut runs = Vec::new();
    // A window that starts inside a fenced block has to know it is looking at
    // code, so the state is carried in from the lines above it.
    let mut fence = state.talk.fence_before(rows, cols);
    for line in state.talk.visible(rows, cols) {
        match line.tone {
            // Only the model's prose is Markdown. What the human typed and
            // what the harness noted are shown as written.
            Tone::Body => crate::markdown::line(&line.text, &mut fence, skin, &mut runs),
            tone => runs.push(Run::tinted(&line.text, skin.tone(tone))),
        }
        runs.push(Run::plain("\n"));
    }
    // The window may start partway down a wrapped line rather than dropping
    // it, so the shaped buffer is scrolled by the rows that sit above.
    scene.text(
        Text::rich(runs, panel.inset(PAD), frame.body_size, frame.skin.body)
            .scrolled(state.talk.window(rows, cols).skip as f32),
    );
    scrollbar(scene, skin, panel, state.talk.thumb(rows, cols));
}

fn activity(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    let rows = frame.layout.rows(panel, frame.pane_size);
    let cols = cols_of(panel, frame.pane_column);
    let mut runs = Vec::new();
    for line in state.activity.visible(rows, cols) {
        runs.push(Run::tinted(&line.text, skin.tone(line.tone)));
        runs.push(Run::plain("\n"));
    }
    scene.text(
        Text::rich(runs, panel.inset(PAD), frame.pane_size, frame.skin.body)
            .scrolled(state.activity.window(rows, cols).skip as f32),
    );
    scrollbar(scene, skin, panel, state.activity.thumb(rows, cols));
}

fn plan(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    let mut runs = Vec::new();
    if state.plan.is_empty() {
        runs.push(Run::tinted("no plan yet", skin.dim));
    }
    for todo in &state.plan {
        let (mark, color) = match todo.state {
            TodoState::Done => ("[x] ", skin.good),
            TodoState::Active => ("[>] ", skin.bright),
            TodoState::Pending => ("[ ] ", skin.dim),
        };
        runs.push(Run::tinted(mark, color));
        runs.push(Run::tinted(&todo.text, color));
        runs.push(Run::plain("\n"));
    }
    text_box(scene, frame, panel, frame.pane_size, runs);
}

/// The fleet: one child per row, and under each the last thing it said.
///
/// A row alone is a name and a word, which for eight children at once tells
/// you nothing about any of them. The second line is where the news is: while
/// a child runs it is that child's own output, and once it ends it is the
/// reason it ended.
fn agents(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    let mut runs = Vec::new();
    if state.agents.is_empty() {
        runs.push(Run::tinted("no sub-agents this session", skin.dim));
    }
    for agent in &state.agents {
        runs.push(Run::tinted(format!("{:<9}", agent.label), skin.dim));
        runs.push(Run::tinted(
            format!("{:<10}", agent.state),
            skin.tone(agent.tone),
        ));
        // The tool set says whether this child can change anything, which is
        // the one thing about a detached child worth knowing at a glance.
        if !agent.tools.is_empty() {
            runs.push(Run::tinted(format!("{:<10}", agent.tools), skin.dim));
        }
        runs.push(Run::tinted(clip(&agent.brief, 300), skin.body));
        runs.push(Run::plain("\n"));
        if !agent.last.is_empty() {
            runs.push(Run::tinted(format!("           {}", clip(&agent.last, 300)), skin.dim));
            runs.push(Run::plain("\n"));
        }
    }
    text_box(scene, frame, panel, frame.pane_size, runs);
}

/// A label column, a bar, and a reading, laid out as three boxes rather than as
/// one padded string.
///
/// One string with the bar's room spelled as spaces was the first attempt, and
/// the readings landed on top of the bars: the spaces are the pane's column
/// width and the bar was drawn in the transcript's, which is a different
/// number. Three boxes at computed positions cannot drift apart.
///
/// The bar reads as [`DOT_COLUMNS`] columns of [`DOT_ROWS`] dots, so a column
/// is 10% and a dot is 2.5%. Four stacked dots do not fit the half-line track
/// a solid bar used, so a bounded row is taller than a line of text and its
/// label and reading are centred against the grid. Unbounded readings keep the
/// line pitch: they draw no dots, and a tall empty row for them would push the
/// rows that do have dots off the bottom of the pane.
fn gauges(scene: &mut Scene, frame: &Frame, panel: Panel, gauges: Vec<Gauge>) {
    let skin = frame.skin;
    let content = panel.inset(PAD);
    let column = frame.pane_column;
    let line = Text::line_for(frame.pane_size);

    if gauges.is_empty() {
        text_box(
            scene,
            frame,
            panel,
            frame.pane_size,
            vec![Run::tinted("sampling…", skin.dim)],
        );
        return;
    }

    let label_w = LABEL_COLUMNS as f32 * column;
    let room = (content.w - label_w - 2.0 * column).max(1.0);
    let gap = (line * 0.16).round().max(1.0);
    // The dot is as big as the line height asks for, unless ten of them with
    // their gaps would not fit the room left beside the label.
    let dot = (line * 0.28)
        .round()
        .min((room / DOT_COLUMNS as f32 - gap).floor())
        .max(2.0);
    let cell = dot + gap;
    let bar_w = cell * DOT_COLUMNS as f32;
    let grid_h = dot * DOT_ROWS as f32 + gap * (DOT_ROWS - 1) as f32;
    // A bounded row is the grid plus a gap above and below it.
    let pitch = grid_h + 2.0 * gap;
    let read_x = content.x + label_w + bar_w + column;

    let mut y = content.y;
    for gauge in &gauges {
        let fraction = gauge.fraction();
        let row_h = if fraction.is_some() { pitch } else { line };
        if y + row_h > content.y + content.h {
            break;
        }
        let text_y = y + ((row_h - line) * 0.5).floor();
        scene.text(Text::rich(
            vec![Run::tinted(gauge.label, skin.dim)],
            Panel::new(content.x, text_y, label_w.max(1.0), line),
            frame.pane_size,
            skin.dim,
        ));
        scene.text(Text::rich(
            vec![Run::tinted(
                gauge.reading(),
                if fraction.is_some_and(|f| f > 0.85) {
                    skin.bad
                } else {
                    skin.body
                },
            )],
            Panel::new(
                read_x,
                text_y,
                (content.x + content.w - read_x).max(1.0),
                line,
            ),
            frame.pane_size,
            skin.body,
        ));

        let track = Panel::new(
            content.x + label_w,
            y + gap,
            bar_w,
            (row_h - 2.0 * gap).max(3.0),
        );
        scene.rect(track.fill(skin.gauge_track));
        // The history first, behind the dots: the past is context, not content.
        let series = frame.monitor.history(gauge.key);
        if series.len() > 1 {
            let step = track.w / series.len() as f32;
            for (i, point) in series.iter().enumerate() {
                let height = (track.h * point).max(1.0);
                scene.rect(
                    Panel::new(
                        track.x + i as f32 * step,
                        track.y + track.h - height,
                        step.max(1.0),
                        height,
                    )
                    .fill(skin.scroll_thumb),
                );
            }
        }
        if let Some(fraction) = fraction {
            let tint = if fraction > 0.9 {
                skin.close_hot
            } else {
                skin.gauge
            };
            let lit = (fraction * (DOT_COLUMNS * DOT_ROWS) as f32).round() as usize;
            for index in 0..lit {
                let (col, level) = (index / DOT_ROWS, index % DOT_ROWS);
                // Columns fill bottom up, so a part-filled column reads as a
                // level and not as a hole in the row above it.
                scene.rect(
                    Panel::new(
                        track.x + col as f32 * cell + 0.5 * gap,
                        track.y + grid_h - (level + 1) as f32 * dot - level as f32 * gap,
                        dot,
                        dot,
                    )
                    .fill(tint)
                    .radius(0.5 * dot),
                );
            }
        }
        y += row_h;
    }
}

fn files(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    // The inner tab strip, one per file, along the top of this space's body.
    let strip = Panel::new(panel.x, panel.y, panel.w, TAB_H);
    scene.rect(strip.fill(skin.strip));
    scene.rect(strip.bottom_edge(skin.edge));
    if layout.file_tabs.is_empty() {
        scene.text(Text::rich(
            vec![Run::tinted("no files touched yet", skin.dim)],
            strip.row(SMALL * 0.6, Text::line_for(SMALL)),
            SMALL,
            skin.dim,
        ));
    }
    for (index, tab) in &layout.file_tabs {
        let Some(file) = state.files.get(*index) else {
            continue;
        };
        let active = *index == state.open_file;
        if active {
            scene.rect(tab.fill(skin.panel));
            scene.rect(tab.top_edge(skin.edge_focus));
        }
        // A file compaction dropped is still worth reading; it is just no
        // longer what the agent is holding, and the tab says which.
        let color = match (active, file.closed) {
            (_, true) => skin.dim,
            (true, false) => skin.bright,
            (false, false) => skin.title,
        };
        let mut runs = vec![
            // The type mark, so a tab is recognisable before it is read.
            Run::icon(crate::icons::for_path(&file.path).to_string(), color),
            Run::tinted(format!(" {}", short_name(&file.path)), color),
        ];
        if file.changed {
            runs.push(Run::tinted(" \u{2022}", skin.plus));
        }
        scene.text(Text::rich(
            runs,
            tab.row(SMALL * 0.6, Text::line_for(SMALL)),
            SMALL,
            color,
        ));
    }

    let body = Panel::new(
        panel.x,
        panel.y + TAB_H,
        panel.w,
        (panel.h - TAB_H).max(1.0),
    );
    if body.h < Text::line_for(frame.pane_size) + 2.0 * PAD {
        return;
    }
    let rows = layout.rows(body, frame.pane_size);
    let Some(file) = state.files.get(state.open_file) else {
        return;
    };

    // A band behind every block header, drawn before the text. Without it a
    // `write lines 17-17` reads as a line of the file rather than as the mark
    // between two of them.
    let content = body.inset(PAD);
    let line = Text::line_for(frame.pane_size);
    // Every row carries a four column gutter, so the text wraps in what is
    // left rather than in the full width of the box.
    let cols = cols_of(body, frame.pane_column).saturating_sub(GUTTER).max(1);
    let first = file.pane.showing_from(rows, cols);
    let shown = file.pane.visible(rows, cols);
    for (step, entry) in shown.iter().enumerate() {
        if !matches!(entry.tone, Tone::Call(_)) {
            continue;
        }
        // A header that wraps gets a band as tall as it actually is, taken
        // from the same arithmetic the text is laid out with.
        let Some((top, height)) = file.pane.band_of(rows, cols, first + step) else {
            continue;
        };
        let y = content.y + top as f32 * line;
        let tall = height as f32 * line;
        if y + tall > content.y + content.h {
            break;
        }
        scene.rect(Panel::new(body.x + 1.0, y, (body.w - 2.0).max(1.0), tall).fill(skin.strip));
    }

    let syntax = crate::syntax::for_path(&file.path);
    let mut runs = Vec::new();
    for entry in &shown {
        let base = skin.tone(entry.tone);
        // The gutter, so a diff line says where in the file it landed.
        match entry.number {
            Some(number) => runs.push(Run::tinted(format!("{number:03} "), skin.comment)),
            None if !entry.text.is_empty() => runs.push(Run::plain("    ")),
            None => {}
        }
        // A removed line reads as removed first, so only what is there now is
        // tokenized.
        if matches!(entry.tone, Tone::Plus | Tone::Body) {
            let (marker, rest) = entry.text.split_at(entry.text.len().min(2));
            runs.push(Run::tinted(marker, base));
            for (text, token) in crate::syntax::scan(rest, syntax) {
                runs.push(Run::tinted(text, skin.token(token).unwrap_or(base)));
            }
        } else {
            runs.push(Run::tinted(&entry.text, base));
        }
        runs.push(Run::plain("\n"));
    }
    scene.text(Text::rich(runs, content, frame.pane_size, skin.body));
    scrollbar(scene, skin, body, file.pane.thumb(rows, cols));
}

/// The tab under the pointer while it is being dragged, so the drag has
/// something following it and the drop has somewhere to be aimed.
fn dragging(scene: &mut Scene, frame: &Frame) {
    let Some(drag) = frame.drag else {
        return;
    };
    let skin = frame.skin;
    let label = drag.view.label();
    let w = (label.chars().count() as f32 + 3.0) * frame.column;
    let ghost = Panel::new(drag.at.0 - w * 0.5, drag.at.1 - TAB_H * 0.5, w, TAB_H);
    scene.rect(ghost.fill(skin.bar));
    scene.rect(ghost.outline(skin.edge_focus, 1.0));
    scene.text(Text::rich(
        vec![Run::tinted(label, skin.bright)],
        ghost.row(SMALL * 0.6, Text::line_for(SMALL)),
        SMALL,
        skin.bright,
    ));
}

/// The bar down the right edge of a pane. Absent when everything fits, because
/// a scrollbar that is always full length says nothing.
fn scrollbar(scene: &mut Scene, skin: &Skin, panel: Panel, thumb: Option<(f32, f32)>) {
    let Some((top, size)) = thumb else {
        return;
    };
    let track = Panel::new(
        panel.x + panel.w - SCROLL_W - 2.0,
        panel.y + 3.0,
        SCROLL_W,
        (panel.h - 6.0).max(1.0),
    );
    scene.rect(track.fill(skin.scroll_track));
    scene.rect(
        Panel::new(
            track.x,
            track.y + track.h * top,
            track.w,
            (track.h * size).max(8.0).min(track.h),
        )
        .fill(skin.scroll_thumb),
    );
}

fn input_row(scene: &mut Scene, frame: &Frame) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    scene.rect(panel_fill(layout.input, skin.input));
    scene.rect(panel_edge(layout.input, skin.edge_focus));
    let line = Text::line_for(frame.body_size);
    let box_ = input_box(layout.input, line);
    let columns = columns_in(box_.w, frame.column);
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
                box_.y + row as f32 * line,
                (stop - at) as f32 * frame.column,
                line,
            );
            if band.y + band.h <= box_.y + box_.h + 0.5 {
                scene.rect(band.fill(skin.select));
            }
            at = stop;
        }
    }
    let marker = if state.phase.busy() { "\u{2026}" } else { "\u{203a}" };
    scene.text(
        Text::rich(
            vec![
                Run::tinted(format!("{marker} "), skin.dim),
                Run::tinted(frame.prompt.text(), skin.bright),
            ],
            box_,
            frame.body_size,
            skin.bright,
        )
        // Wrap by glyph, so counting columns lands the caret where the glyph
        // actually is. Word wrap would put it a word away on every long line.
        .wrap_anywhere(),
    );
    let at = frame.prompt.caret() + PROMPT_COLUMNS;
    let (row, column) = (at / columns, at % columns);
    let caret = Panel::new(
        box_.x + column as f32 * frame.column,
        box_.y + row as f32 * line,
        2.0,
        line,
    );
    if caret.y + caret.h <= box_.y + box_.h + 0.5 {
        scene.rect(caret.fill(skin.caret));
    }
}


/// The box the prompt's text is drawn in, inside the strip the layout gave it.
///
/// Top-aligned so the first line does not move as the prompt grows. Drawing
/// and hit testing both take it from here, which is the only way a click can
/// land on the column the glyph is actually in.
fn input_box(input: Panel, line: f32) -> Panel {
    Panel::new(
        input.x + PAD,
        input.y + INPUT_PAD,
        (input.w - 2.0 * PAD).max(1.0),
        (input.h - 2.0 * INPUT_PAD).max(line),
    )
}

/// How many characters fit across a box of this width.
fn columns_in(width: f32, column: f32) -> usize {
    ((width / column.max(1.0)).floor() as usize).max(1)
}

/// How many characters fit across a panel's content box.
///
/// The one place a pane's width becomes a column count. Wrapping, hit testing
/// and the selection band all have to agree on this number, so they all ask
/// here rather than each dividing by the column width themselves.
pub fn cols_of(panel: Panel, column: f32) -> usize {
    columns_in(panel.inset(PAD).w, column)
}

/// How tall the prompt has to be to hold `chars` characters.
///
/// Grows a line at a time up to `max_rows`, then scrolls inside itself. A
/// prompt that grows without limit eventually eats the conversation it is
/// about, and how much of the window that is worth is a matter of taste, which
/// is why the ceiling is a setting.
pub fn input_height(width: f32, column: f32, chars: usize, line: f32, max_rows: usize) -> f32 {
    let inner = (width - 2.0 * GAP - 2.0 * PAD).max(column);
    let columns = columns_in(inner, column);
    let rows = (chars + PROMPT_COLUMNS + 1)
        .div_ceil(columns)
        .clamp(1, max_rows.max(1));
    // The strip, not the box inside it: the layout insets this by `GAP` before
    // the prompt gets it, and forgetting that cost the last row of a full one.
    (rows as f32 * line + 2.0 * INPUT_PAD + 2.0 * GAP).max(INPUT_H)
}

fn clip(text: &str, chars: usize) -> String {
    let mut out: String = text.chars().take(chars).collect();
    if text.chars().count() > chars {
        out.push('\u{2026}');
    }
    out
}

/// The file name, and enough of its parent to tell two `mod.rs` apart.
pub fn short_name(path: &str) -> String {
    let parts: Vec<&str> = path.rsplit('/').take(2).collect();
    match parts.as_slice() {
        [] => String::new(),
        [name] => (*name).to_string(),
        [name, parent, ..] if *name == "mod.rs" || *name == "index.ts" || *name == "__init__.py" => {
            format!("{parent}/{name}")
        }
        [name, ..] => (*name).to_string(),
    }
}

/// A path shortened to its tail, so a deep workspace does not push the model
/// name off the title bar.
fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.rsplit('/').take(2).collect();
    match parts.len() {
        0 => String::new(),
        1 => parts[0].to_string(),
        _ => format!("{}/{}", parts[1], parts[0]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn shape<'a>(dock: &'a Dock, files: &[&str]) -> Shape<'a> {
        Shape {
            shaded: false,
            dock,
            file_labels: files.iter().map(|f| f.to_string()).collect(),
            column: 8.0,
            input_h: INPUT_H,
        }
    }

    /// A prompt holding `text` with the caret at `at`.
    fn typed_prompt(text: &str, at: usize) -> crate::prompt::Prompt {
        let mut prompt = crate::prompt::Prompt::default();
        prompt.insert(text);
        prompt.place(at);
        prompt
    }

    fn busy_state() -> State {
        let mut state = State::new();
        state.apply(noob_proto::Event::SessionStart {
            id: "s1".into(),
            workspace: "/home/hec/workspace/noob-cli".into(),
            model: "laguna-s21".into(),
            resumed: false,
        });
        state.apply(noob_proto::Event::TurnStart { turn: 1 });
        state.apply(noob_proto::Event::TextDelta {
            d: "looking at it now".into(),
        });
        state.apply(noob_proto::Event::ToolStart {
            call_id: "c1".into(),
            name: "bash".into(),
            brief: "cargo test".into(),
            args: serde_json::json!({"cmd": "cargo test --workspace"}),
        });
        state.apply(noob_proto::Event::ToolStart {
            call_id: "c2".into(),
            name: "plan".into(),
            brief: "2 items".into(),
            args: serde_json::json!({"todos": [
                {"content": "read it", "status": "completed"},
                {"content": "fix it", "status": "in_progress"},
            ]}),
        });
        state.apply(noob_proto::Event::ToolStart {
            call_id: "c3".into(),
            name: "subagent".into(),
            brief: "research".into(),
            args: serde_json::json!({"prompt": "search the web"}),
        });
        // The admission above is the parent asking; the child's own frames are
        // what the fleet is drawn from.
        state.apply(noob_proto::Event::AgentSpawn {
            agent_id: "agent-1".into(),
            prompt: "search the web".into(),
            tools: "web".into(),
        });
        state.apply(noob_proto::Event::AgentStateChanged {
            agent_id: "agent-1".into(),
            state: noob_proto::AgentState::Running,
            detail: None,
        });
        state.apply(noob_proto::Event::AgentOutput {
            agent_id: "agent-1".into(),
            line: "* websearch search".into(),
        });
        state.apply(noob_proto::Event::FileEdit {
            path: "src/calc.py".into(),
            span: noob_proto::Span {
                start: 2,
                end: 2,
                kind: None,
                name: None,
            },
            before: "    return a - b".into(),
            after: "    return a + b".into(),
            call_id: Some("c4".into()),
        });
        state.apply(noob_proto::Event::UsageReport {
            usage: noob_proto::Usage {
                prompt: 1816,
                cached_prompt: 1200,
                completion: 42,
                context_total: 65536,
            },
        });
        state
    }

    struct Rendered {
        scene: Scene,
        layout: Layout,
        skin: Skin,
    }

    fn render(state: &State, w: f32, h: f32, dock: &Dock, files: &[&str]) -> Rendered {
        render_with(state, w, h, dock, files, &Monitor::new(), None)
    }

    /// The window has to say which build it is, or a tester cannot tell two of
    /// them apart. The crate version alone cannot: it does not move between
    /// commits, so `build.rs` stamps the commit into it.
    #[test]
    fn the_title_bar_names_the_build() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &[]);
        let text = text_of(&out.scene);
        assert!(text.contains("CLIppy"), "{text}");
        assert!(
            text.contains(env!("CLIPPY_BUILD")),
            "the build stamp {:?} is not on screen: {text}",
            env!("CLIPPY_BUILD")
        );
        assert!(
            env!("CLIPPY_BUILD").starts_with(env!("CARGO_PKG_VERSION")),
            "the stamp has to start with the version, got {:?}",
            env!("CLIPPY_BUILD")
        );
    }

    /// The bar along the bottom is gone, and the two readings it carried are
    /// still on screen. Removing it silently would have lost the token budget.
    #[test]
    fn nothing_is_drawn_along_the_bottom_and_its_readings_moved_up() {
        let (w, h) = (1400.0, 900.0);
        let out = render(&busy_state(), w, h, &Dock::new(), &[]);
        let text = text_of(&out.scene);
        assert!(
            text.contains("context"),
            "the token budget has to survive the bar it used to live in: {text}"
        );

        // The input row now runs to the bottom of the window. It used to stop
        // 24 pixels short, and those pixels were the bar.
        let floor = out.layout.input.y + out.layout.input.h;
        // Only the window's own bottom margin is left, not a reserved strip.
        assert!(
            h - floor <= GAP + 0.01,
            "the input row stops {} short of the bottom, more than the {GAP}px margin, \
             so something is still reserved down there",
            h - floor
        );
    }

    /// The context gauge moved to the bottom edge of the title strip. It is two
    /// pixels either way; what matters is that it is still drawn and still
    /// scales with how full the context is.
    #[test]
    fn the_context_gauge_is_a_hairline_under_the_title_strip() {
        let mut state = busy_state();
        state.context = Some(crate::state::ContextFill {
            used: 4_000,
            total: 16_000,
            compact_at: 12_000,
        });
        let out = render(&state, 1400.0, 900.0, &Dock::new(), &[]);
        let edge = out.layout.title.y + out.layout.title.h - 2.0;
        let hairlines: Vec<[f32; 4]> = out
            .scene
            .rects
            .iter()
            .map(|r| r.xywh())
            .filter(|[_, y, _, h]| (*y - edge).abs() < 0.01 && (*h - 2.0).abs() < 0.01)
            .collect();
        assert!(
            hairlines.len() >= 2,
            "expected a track and a fill on the strip's bottom edge, got {hairlines:?}"
        );
        let fill = hairlines.iter().map(|[_, _, w, _]| *w).fold(f32::INFINITY, f32::min);
        let track = hairlines.iter().map(|[_, _, w, _]| *w).fold(0.0f32, f32::max);
        assert!(fill > 0.0 && fill < track, "the fill has to be part of the track: {hairlines:?}");
    }

    /// The arrow at the end of each tab strip is gone. Clicking the tab already
    /// showing still collapses its space, so nothing was lost with it, and the
    /// square it occupied is now available to tabs.
    #[test]
    fn no_control_sits_at_the_end_of_a_tab_strip() {
        let dock = Dock::new();
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &[]);
        for space in Space::ALL {
            let strip = out.layout.placed(space).strip;
            if strip.w < 1.0 {
                continue;
            }
            let (x, y) = (strip.x + strip.w - TAB_H * 0.5, strip.y + strip.h * 0.5);
            // Whatever is under the square the arrow used to occupy, it is
            // not a control of its own: a strip resolves only to its tabs now.
            let hit = out.layout.hit(x, y);
            assert!(
                matches!(hit, None | Some(Hit::Tab(..)) | Some(Hit::Body(_)) | Some(Hit::TitleBar)),
                "{space:?} still has a control at the end of its strip: {hit:?}"
            );
        }
    }

    fn render_with(
        state: &State,
        w: f32,
        h: f32,
        dock: &Dock,
        files: &[&str],
        monitor: &Monitor,
        drag: Option<Drag>,
    ) -> Rendered {
        let shape = shape(dock, files);
        let layout = Layout::compute(w, h, &shape);
        let skin = Skin::from(&Config::default());
        // The clip that ships, at a real point in its loop, so the avatar view
        // is exercised at its actual size rather than as a placeholder.
        let clip = crate::avatar::Avatar::built_in();
        let scene = build(&Frame {
            state,
            monitor,
            dock,
            skin: &skin,
            layout: &layout,
            prompt: &typed_prompt("type here", 4),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            drag,
            hot: None,
            trouble: None,
            selection: None,
            avatar: clip.as_ref().map(|clip| (clip, 700)),
        });
        Rendered {
            scene,
            layout,
            skin,
        }
    }

    fn text_of(scene: &Scene) -> String {
        scene
            .texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| r.text.as_str()))
            .collect()
    }

    #[test]
    fn the_default_arrangement_puts_every_space_on_screen() {
        let dock = Dock::new();
        for (w, h) in [(1200.0, 800.0), (700.0, 460.0), (2200.0, 1400.0)] {
            let out = render(&busy_state(), w, h, &dock, &["a.rs"]);
            for space in Space::ALL {
                let placed = out.layout.placed(space);
                assert!(placed.strip.w > 1.0, "{space:?} at {w}x{h}");
                assert!(placed.strip.x >= 0.0 && placed.strip.y >= TITLE_H - 0.1);
                assert!(
                    placed.body.x + placed.body.w <= w + 0.01,
                    "{space:?} {:?} at {w}x{h}",
                    placed.body
                );
                assert!(placed.body.y + placed.body.h <= h + 0.01, "{space:?}");
            }
        }
    }

    /// Every click resolves in one place, so what a region looks like and what
    /// it does can never come apart.
    #[test]
    fn every_tab_is_hit_where_it_is_drawn() {
        let dock = Dock::new();
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &["a.rs", "b.md"]);
        let middle = |p: Panel| (p.x + p.w * 0.5, p.y + p.h * 0.5);
        for space in Space::ALL {
            let placed = out.layout.placed(space);
            for (view, panel) in &placed.tabs {
                let (x, y) = middle(*panel);
                assert_eq!(
                    out.layout.hit(x, y),
                    Some(Hit::Tab(*view, space)),
                    "{view:?} in {space:?}"
                );
            }
        }
        for (index, panel) in &out.layout.file_tabs {
            let (x, y) = middle(*panel);
            let hit = out.layout.hit(x, y).expect("a file tab");
            assert_eq!(hit, Hit::File(*index, Space::BottomRight));
            // And it still names a space, so a tab dropped here lands.
            assert_eq!(hit.space(), Some(Space::BottomRight));
        }
        for (panel, hit) in [
            (out.layout.close, Hit::Close),
            (out.layout.maximize, Hit::Maximize),
            (out.layout.minimize, Hit::Minimize),
            (out.layout.input, Hit::Input),
        ] {
            let (x, y) = middle(panel);
            assert_eq!(out.layout.hit(x, y), Some(hit));
        }
    }

    /// A drop lands somewhere. Every point inside a space's body or its strip
    /// names that space, or a drag can be released over nothing.
    #[test]
    fn every_point_in_a_space_names_that_space_for_a_drop() {
        let dock = Dock::new();
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &["a.rs"]);
        for space in Space::ALL {
            let placed = out.layout.placed(space);
            for point in [
                (placed.body.x + 4.0, placed.body.y + 4.0),
                (
                    placed.body.x + placed.body.w - 4.0,
                    placed.body.y + placed.body.h - 4.0,
                ),
                (placed.strip.x + placed.strip.w - TAB_H - 4.0, placed.strip.y + 4.0),
            ] {
                let hit = out.layout.hit(point.0, point.1).expect("a hit");
                assert_eq!(hit.space(), Some(space), "{point:?} in {space:?}: {hit:?}");
            }
        }
    }

    /// The arrangement drives the layout: a view dragged elsewhere is drawn
    /// elsewhere, and its old space keeps working.
    #[test]
    fn a_moved_view_is_drawn_in_its_new_space() {
        let mut dock = Dock::new();
        dock.move_view(View::Llm, Space::Left);
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &["a.rs"]);
        let left: Vec<View> = out
            .layout
            .placed(Space::Left)
            .tabs
            .iter()
            .map(|(v, _)| *v)
            .collect();
        assert!(left.contains(&View::Llm), "{left:?}");
        assert!(left.contains(&View::Talk), "{left:?}");
        let top: Vec<View> = out
            .layout
            .placed(Space::TopRight)
            .tabs
            .iter()
            .map(|(v, _)| *v)
            .collect();
        assert!(!top.contains(&View::Llm), "{top:?}");
    }

    /// An emptied space gives its room away rather than leaving a hole.
    #[test]
    fn an_empty_space_gives_its_room_to_its_neighbour() {
        let full = Dock::new();
        let mut emptied = Dock::new();
        for view in [
            View::Activity,
            View::Plan,
            View::Agents,
            View::Hardware,
            View::Llm,
        ] {
            emptied.move_view(view, Space::BottomRight);
        }
        let a = render(&busy_state(), 1200.0, 800.0, &full, &["a.rs"]);
        let b = render(&busy_state(), 1200.0, 800.0, &emptied, &["a.rs"]);
        assert_eq!(b.layout.placed(Space::TopRight).strip.w, 0.0);
        assert!(
            b.layout.placed(Space::BottomRight).body.h
                > a.layout.placed(Space::BottomRight).body.h,
            "the other space grew"
        );
    }

    /// With nothing on the left, the right column takes the whole width rather
    /// than leaving half the window empty.
    #[test]
    fn an_empty_left_column_hands_the_width_over() {
        let mut dock = Dock::new();
        dock.move_view(View::Talk, Space::TopRight);
        let out = render(&busy_state(), 1200.0, 800.0, &dock, &[]);
        assert_eq!(out.layout.placed(Space::Left).strip.w, 0.0);
        let top = out.layout.placed(Space::TopRight);
        assert!(top.body.w > 1000.0, "{:?}", top.body);
    }

    /// A drag has to be visible, and its target has to be named, or a drop is
    /// a guess.
    #[test]
    fn a_dragged_tab_follows_the_pointer_and_lights_its_target() {
        let dock = Dock::new();
        let plain = render(&busy_state(), 1200.0, 800.0, &dock, &["a.rs"]);
        let dragging = render_with(
            &busy_state(),
            1200.0,
            800.0,
            &dock,
            &["a.rs"],
            &Monitor::new(),
            Some(Drag {
                view: View::Activity,
                at: (400.0, 500.0),
                onto: Some(Space::Left),
            }),
        );
        assert!(
            dragging.scene.rects.len() > plain.scene.rects.len(),
            "the ghost is drawn"
        );
        // The ghost is where the pointer is.
        let ghost = dragging
            .scene
            .rects
            .iter()
            .map(|r| r.xywh())
            .find(|[x, y, w, h]| {
                *x < 400.0 && *x + *w > 400.0 && *y < 500.0 && *y + *h > 500.0 && *h <= TAB_H + 1.0
            });
        assert!(ghost.is_some(), "no ghost under the pointer");
        // The target space is outlined in the focus colour.
        let target = dragging.layout.placed(Space::Left).body;
        let lit = dragging.scene.rects.iter().any(|r| {
            let [x, y, w, h] = r.xywh();
            (w <= 1.5 || h <= 1.5) && x >= target.x - 0.5 && y >= target.y - 0.5
        });
        assert!(lit, "the target is not outlined");
        let _ = dragging.skin;
    }

    /// The band has to land on the text it selects. This is the geometry that
    /// can be silently wrong: the selection model is right, the copy is right,
    /// and the highlight sits a line off.
    #[test]
    fn the_selection_band_covers_the_rows_it_selects() {
        let mut state = busy_state();
        // Three known lines at the end of the conversation.
        for text in ["alpha alpha", "beta beta", "gamma gamma"] {
            state.talk.say(text, Tone::Body);
        }
        let last = state.talk.last() - 1;
        let mut selection =
            crate::select::Selection::new(View::Talk, crate::select::Spot::new(last - 2, 6));
        selection.extend(crate::select::Spot::new(last, 5));
        state.selection = Some(selection);

        let dock = Dock::new();
        let shape = shape(&dock, &["a.rs"]);
        let layout = Layout::compute(1400.0, 900.0, &shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state: &state,
            monitor: &Monitor::new(),
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            drag: None,
            hot: None,
            trouble: None,
            selection: Some(selection),
            avatar: None,
        });

        let body = layout.placed(Space::Left).body.inset(PAD);
        let bands: Vec<[f32; 4]> = scene
            .rects
            .iter()
            .filter(|r| r.rgba() == skin.select)
            .map(|r| r.xywh())
            .collect();
        // One per selected line, no more: a single rectangle over the block
        // would cover text on the first and last lines that is not selected.
        assert_eq!(bands.len(), 3, "{bands:?}");
        // Consecutive rows, top to bottom, each one line tall.
        //
        // At the size Talk is *drawn* with, not the pane size. This assertion
        // used to read `line_for(13.0)` while the transcript rendered at 14.0,
        // so it passed while the highlight sat a growing fraction of a row
        // above the glyphs it was supposed to cover.
        let line = Text::line_for(14.0);
        let mut ys: Vec<f32> = bands.iter().map(|b| b[1]).collect();
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for pair in ys.windows(2) {
            assert!((pair[1] - pair[0] - line).abs() < 0.01, "{ys:?}");
        }
        for band in &bands {
            assert!((band[3] - line).abs() < 0.01, "band is {} tall", band[3]);
            assert!(band[0] >= body.x - 0.01, "{band:?} starts left of the pane");
            assert!(
                band[0] + band[2] <= body.x + body.w + 0.01,
                "{band:?} runs past the pane"
            );
        }
        // The first line starts six columns in, the last starts at the edge.
        let first = bands
            .iter()
            .min_by(|a, b| a[1].partial_cmp(&b[1]).unwrap())
            .unwrap();
        assert!((first[0] - (body.x + 6.0 * 8.0)).abs() < 0.01, "{first:?}");
    }

    /// A selection in a pane that is not on screen must not paint anything.
    #[test]
    fn a_selection_in_a_hidden_pane_draws_nothing() {
        let mut state = busy_state();
        state.activity.say("something to select", Tone::Body);
        let last = state.activity.last() - 1;
        let mut selection =
            crate::select::Selection::new(View::Activity, crate::select::Spot::new(last, 0));
        selection.extend(crate::select::Spot::new(last, 9));
        state.selection = Some(selection);

        // Fold every space away, so nothing is showing at all.
        let mut dock = Dock::new();
        for space in Space::ALL {
            dock.slot_mut(space).folded = true;
        }
        let shape = shape(&dock, &["a.rs"]);
        let layout = Layout::compute(1400.0, 900.0, &shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state: &state,
            monitor: &Monitor::new(),
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            drag: None,
            hot: None,
            trouble: None,
            selection: Some(selection),
            avatar: None,
        });
        assert!(!scene.rects.iter().any(|r| r.rgba() == skin.select));
    }

    /// The avatar draws, moves, and stays inside its panel. It is the one
    /// view whose content is a fixed-size picture rather than a list, so it is
    /// also the one that can overflow a small pane instead of scrolling.
    #[test]
    fn the_avatar_animates_inside_its_own_panel() {
        let state = busy_state();
        let mut dock = Dock::new();
        dock.reveal(View::Avatar);
        let body = |w: f32, h: f32| {
            let out = render(&state, w, h, &dock, &["a.rs"]);
            let panel = out.layout.placed(Space::BottomRight).body;
            let drawn: Vec<String> = out
                .scene
                .texts
                .iter()
                .filter(|t| {
                    t.at.x >= panel.x - 0.5
                        && t.at.y >= panel.y - 0.5
                        && t.at.x + t.at.w <= panel.x + panel.w + 0.5
                        && t.at.y + t.at.h <= panel.y + panel.h + 0.5
                })
                .map(|t| t.runs.iter().map(|r| r.text.as_str()).collect::<String>())
                .collect();
            drawn.join("")
        };
        let wide = body(1400.0, 900.0);
        assert!(wide.contains('#') || wide.contains('%'), "nothing drawn: {wide:?}");
        // It is not a placeholder: the clip is playing.
        assert!(!wide.contains("avatar off"), "{wide:?}");
        // And it still fits when the pane is small, which is where a
        // fixed-size picture would otherwise run past its panel.
        assert!(!body(700.0, 400.0).is_empty());
    }

    /// Every text box must be able to hold at least one line of its own size.
    /// A box shorter than that draws the text and clips every pixel of it,
    /// which reads as the interface being broken.
    #[test]
    fn no_text_box_is_too_small_to_show_its_text() {
        let state = busy_state();
        let mut monitor = Monitor::new();
        monitor.sample(&state);
        monitor.sample(&state);
        for (w, h) in [(1400.0, 900.0), (900.0, 520.0), (700.0, 400.0)] {
            for view in View::ALL {
                let mut dock = Dock::new();
                dock.reveal(view);
                let out = render_with(&state, w, h, &dock, &["calc.py"], &monitor, None);
                for text in &out.scene.texts {
                    assert!(text.at.w >= 1.0, "{view:?} {:?} at {w}x{h}", text.at);
                    assert!(
                        text.at.h >= Text::line_for(text.size),
                        "{view:?} {:?} cannot hold one {}pt line at {w}x{h}",
                        text.at,
                        text.size
                    );
                    assert!(text.at.x >= 0.0 && text.at.y >= 0.0, "{:?}", text.at);
                    assert!(text.at.x + text.at.w <= w + 0.01, "{view:?} {:?}", text.at);
                    assert!(text.at.y + text.at.h <= h + 0.01, "{view:?} {:?}", text.at);
                }
            }
        }
    }

    #[test]
    fn every_rectangle_is_inside_the_surface() {
        let state = busy_state();
        let dock = Dock::new();
        for (w, h) in [(1400.0, 900.0), (320.0, 240.0)] {
            let out = render(&state, w, h, &dock, &["a.rs"]);
            assert!(!out.scene.rects.is_empty());
            for rect in &out.scene.rects {
                let [x, y, rw, rh] = rect.xywh();
                assert!(x >= 0.0 && y >= 0.0, "{rect:?} at {w}x{h}");
                assert!(
                    x + rw <= w + 0.01 && y + rh <= h + 0.01,
                    "{rect:?} at {w}x{h}"
                );
            }
        }
    }

    /// Each view shows its own thing and not another's.
    #[test]
    fn each_view_shows_its_own_content() {
        let state = busy_state();
        let mut monitor = Monitor::new();
        monitor.sample(&state);
        monitor.sample(&state);
        let seen = |view: View| {
            let mut dock = Dock::new();
            dock.reveal(view);
            text_of(&render_with(&state, 1400.0, 900.0, &dock, &["calc.py"], &monitor, None).scene)
        };
        assert!(seen(View::Activity).contains("cargo test --workspace"));
        let plan = seen(View::Plan);
        assert!(plan.contains("[x] read it"), "{plan}");
        assert!(!plan.contains("cargo test --workspace"), "activity leaked");
        assert!(seen(View::Agents).contains("search the web"));
        assert!(seen(View::Files).contains("return a + b"));
        // The two monitors are two different lists.
        let hardware = seen(View::Hardware);
        let llm = seen(View::Llm);
        assert!(hardware.contains("CPU") || hardware.contains("RAM"), "{hardware}");
        assert!(llm.contains("TOTAL PRE"), "{llm}");
        assert!(!llm.contains("CPU"), "hardware leaked into the LLM view: {llm}");
        assert!(!hardware.contains("DECODE"), "the reverse: {hardware}");
    }

    /// The conversation and the budget are on screen whichever tab is up,
    /// because they are in a different space.
    #[test]
    fn the_conversation_stays_visible_whatever_the_other_space_shows() {
        let state = busy_state();
        for view in [View::Activity, View::Plan, View::Agents] {
            let mut dock = Dock::new();
            dock.reveal(view);
            let text = text_of(&render(&state, 1400.0, 900.0, &dock, &["calc.py"]).scene);
            assert!(text.contains("looking at it now"), "{view:?}");
            assert!(text.contains("1,816 / 65,536"), "{view:?}");
            assert!(text.contains("type here"), "{view:?}");
        }
    }

    #[test]
    fn a_changed_file_is_marked_in_its_tab() {
        let text = text_of(&render(&busy_state(), 1400.0, 900.0, &Dock::new(), &["calc.py"]).scene);
        assert!(text.contains("calc.py \u{2022}"), "{text}");
    }

    #[test]
    fn the_file_strip_says_so_when_there_are_no_files() {
        let text = text_of(&render(&State::new(), 1200.0, 800.0, &Dock::new(), &[]).scene);
        assert!(text.contains("no files touched yet"), "{text}");
    }

    #[test]
    fn the_files_view_is_syntax_colored() {
        let mut state = State::new();
        state.apply(noob_proto::Event::FileEdit {
            path: "calc.py".into(),
            span: noob_proto::Span {
                start: 1,
                end: 1,
                kind: None,
                name: None,
            },
            before: String::new(),
            after: "x = \"hello\"  # a note".into(),
            call_id: None,
        });
        let out = render(&state, 1400.0, 900.0, &Dock::new(), &["calc.py"]);
        let colors: Vec<Option<[u8; 4]>> = out
            .scene
            .texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| r.color))
            .collect();
        assert!(colors.contains(&Some(out.skin.string)), "the string is tinted");
        assert!(colors.contains(&Some(out.skin.comment)), "the comment is tinted");
    }

    /// The bug this replaced: the bar's room was spelled as spaces in the
    /// pane's font while the bar itself was drawn in the transcript's column
    /// width, so the readings landed on top of the bars.
    ///
    /// The bar is a grid of dots now, so the thing the reading has to clear is
    /// the track and every dot on it. Found by fill rather than by size: a dot
    /// is a few pixels square, which no size filter can tell from a hairline.
    #[test]
    fn a_monitor_reading_never_lands_on_its_bar() {
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
        dock.reveal(View::Llm);
        // Deliberately mismatched: the transcript's columns are wider than the
        // pane's, which is the situation that produced the overlap.
        for (column, pane_column) in [(8.4, 7.8), (7.8, 8.4), (8.0, 8.0)] {
            let shape = Shape {
                shaded: false,
                dock: &dock,
                file_labels: vec![],
                column,
                input_h: INPUT_H,
            };
            let layout = Layout::compute(1400.0, 900.0, &shape);
            let skin = Skin::from(&Config::default());
            let scene = build(&Frame {
                state: &state,
                monitor: &monitor,
                dock: &dock,
                skin: &skin,
                layout: &layout,
                prompt: &crate::prompt::Prompt::default(),
                column,
                pane_column,
                body_size: 14.0,
                pane_size: 13.0,
                drag: None,
                hot: None,
                trouble: None,
            selection: None,
            avatar: None,
            });
            let body = layout.placed(Space::TopRight).body;
            let bars: Vec<[f32; 4]> = scene
                .rects
                .iter()
                .filter(|r| {
                    [skin.gauge_track, skin.gauge, skin.close_hot].contains(&r.rgba())
                        && body.contains(r.xywh()[0], r.xywh()[1])
                })
                .map(|r| r.xywh())
                .collect();
            let dots = bars.iter().filter(|[_, _, w, h]| w == h).count();
            assert!(dots > 0, "no dots were drawn");
            let bar_right = bars
                .iter()
                .map(|[x, _, w, _]| x + w)
                .fold(0.0f32, f32::max);
            assert!(bar_right > body.x, "no bars were drawn");
            let reading = scene
                .texts
                .iter()
                .find(|t| {
                    body.contains(t.at.x, t.at.y)
                        && t.runs
                            .iter()
                            .any(|r| r.text.contains('/') || r.text.contains("tok"))
                })
                .expect("a reading is on screen");
            assert!(
                reading.at.x >= bar_right,
                "reading at {} overlaps a bar ending at {bar_right} ({column}/{pane_column})",
                reading.at.x
            );
        }
    }

    /// Ten columns of four dots, so a column is 10% and a dot is 2.5%. 525 of
    /// 1000 tokens is 52.5%, which is five whole columns and one dot of a
    /// sixth, and the part-filled column fills from the bottom the way a level
    /// meter does.
    #[test]
    fn a_gauge_is_ten_columns_of_four_dots() {
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
        dock.reveal(View::Llm);
        let shape = Shape {
            shaded: false,
            dock: &dock,
            file_labels: vec![],
            column: 8.0,
            input_h: INPUT_H,
        };
        let layout = Layout::compute(1400.0, 900.0, &shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state: &state,
            monitor: &monitor,
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            drag: None,
            hot: None,
            trouble: None,
            selection: None,
            avatar: None,
        });

        // CONTEXT is the only bounded reading with anything in it: CACHED is
        // zero of 525 and every other row is unbounded, so every lit dot in
        // this pane belongs to the one gauge under test.
        let body = layout.placed(Space::TopRight).body;
        let dots: Vec<[f32; 4]> = scene
            .rects
            .iter()
            .filter(|r| r.rgba() == skin.gauge && body.contains(r.xywh()[0], r.xywh()[1]))
            .map(|r| r.xywh())
            .collect();
        assert_eq!(dots.len(), 21, "52.5% is five full columns plus one dot");
        for [_, _, w, h] in &dots {
            assert_eq!(w, h, "a dot is drawn square so its radius rounds it off");
        }

        let mut columns: Vec<f32> = dots.iter().map(|[x, _, _, _]| *x).collect();
        columns.sort_by(f32::total_cmp);
        columns.dedup();
        assert_eq!(columns.len(), 6, "five whole columns and a part-filled one");
        let height = |x: f32| dots.iter().filter(|[dx, _, _, _]| *dx == x).count();
        assert_eq!(
            columns.iter().map(|x| height(*x)).collect::<Vec<_>>(),
            vec![4, 4, 4, 4, 4, 1]
        );
        // Evenly pitched, or the grid reads as a random scatter.
        let pitch = columns[1] - columns[0];
        for pair in columns.windows(2) {
            assert!((pair[1] - pair[0] - pitch).abs() < 0.01, "{columns:?}");
        }

        let floor = dots.iter().map(|[_, y, _, _]| *y).fold(f32::MIN, f32::max);
        let lone = dots
            .iter()
            .find(|[x, _, _, _]| *x == columns[5])
            .expect("the part-filled column");
        assert_eq!(lone[1], floor, "a part-filled column fills from the bottom");

        // The row had to grow: four stacked dots do not fit a line of text.
        let top = dots.iter().map(|[_, y, _, _]| *y).fold(f32::MAX, f32::min);
        assert!(
            floor + dots[0][3] - top > Text::line_for(13.0),
            "the dot grid is taller than the line pitch it replaced"
        );
    }

    /// A frame that is nothing but a prompt: the strip it landed in, its
    /// layout, and the scene, at the default 14pt body size.
    fn render_prompt(
        prompt: &crate::prompt::Prompt,
        max_rows: usize,
    ) -> (Panel, Layout, Scene) {
        let state = State::new();
        let dock = Dock::new();
        let skin = Skin::from(&Config::default());
        let mut shape = shape(&dock, &[]);
        shape.input_h = input_height(1200.0, 8.0, prompt.len(), Text::line_for(14.0), max_rows);
        let layout = Layout::compute(1200.0, 800.0, &shape);
        let scene = build(&Frame {
            state: &state,
            monitor: &Monitor::new(),
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt,
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            drag: None,
            hot: None,
            trouble: None,
            selection: None,
            avatar: None,
        });
        (layout.input, layout, scene)
    }

    /// The prompt grows with what has been typed, and the caret follows the
    /// wrap rather than running off the end of the first line.
    #[test]
    fn the_prompt_grows_and_the_caret_stays_inside_it() {
        let line = Text::line_for(14.0);
        let short = typed_prompt("short", 5);
        let long = typed_prompt(&"x".repeat(600), 600);
        let (one, ..) = render_prompt(&short, 8);
        let (many, _, scene) = render_prompt(&long, 8);
        assert!(many.h > one.h, "the prompt grew: {} then {}", one.h, many.h);
        assert!(many.h <= 8.0 * line + 30.0, "and stopped growing: {}", many.h);
        let caret = scene
            .rects
            .iter()
            .map(|r| r.xywh())
            .rfind(|[x, y, w, _]| *w <= 3.0 && many.contains(*x, *y))
            .expect("the caret is drawn");
        assert!(
            caret[1] + caret[3] <= many.y + many.h + 0.5,
            "the caret left the prompt: {caret:?} in {many:?}"
        );
        assert!(caret[1] > many.y, "and it is not still on the first row");
    }

    /// How tall it is allowed to get is a setting, not a constant. Two rows
    /// and twenty rows are both a window somebody wants.
    #[test]
    fn the_prompt_stops_growing_at_the_configured_row_count() {
        let line = Text::line_for(14.0);
        // More than twenty rows of it, so the ceiling is what stops it.
        let long = typed_prompt(&"x".repeat(3000), 3000);
        let (two, ..) = render_prompt(&long, 2);
        let (twenty, ..) = render_prompt(&long, 20);
        assert!(twenty.h > two.h, "{} is not taller than {}", twenty.h, two.h);
        // The strip holds that many rows and the padding around them.
        assert!((two.h - (2.0 * line + 2.0 * INPUT_PAD)).abs() < 0.01, "{}", two.h);
        assert!(
            (twenty.h - (20.0 * line + 2.0 * INPUT_PAD)).abs() < 0.01,
            "{}",
            twenty.h
        );
        // A ceiling nobody typed up to still leaves the prompt one row.
        let (empty, ..) = render_prompt(&crate::prompt::Prompt::default(), 20);
        assert!((empty.h - (line + 2.0 * INPUT_PAD)).abs() < 0.01, "{}", empty.h);
    }

    /// A click lands on the character it is over, on any row of a wrapped
    /// prompt. This is the arithmetic that can be silently wrong: the caret is
    /// drawn from it, so an inverse that disagrees puts the caret elsewhere.
    #[test]
    fn a_click_in_the_prompt_lands_on_the_character_under_it() {
        let typed = "0123456789".repeat(50);
        let prompt = typed_prompt(&typed, 0);
        let (strip, layout, scene) = render_prompt(&prompt, 8);
        let line = Text::line_for(14.0);
        let box_ = input_box(strip, line);
        let columns = columns_in(box_.w, 8.0);
        for at in [0usize, 1, 7, columns, columns + 3, columns * 2 + 9] {
            let column = (at + PROMPT_COLUMNS) % columns;
            let row = (at + PROMPT_COLUMNS) / columns;
            // The middle of that cell, which is where a pointer would be.
            let x = box_.x + column as f32 * 8.0 + 3.0;
            let y = box_.y + row as f32 * line + line * 0.5;
            assert_eq!(
                layout.input_caret(x, y, 14.0, 8.0, prompt.len()),
                at,
                "row {row} column {column}"
            );
        }
        // Past the end of the text, and past the end of a row.
        let below = box_.y + box_.h - 1.0;
        assert_eq!(
            layout.input_caret(box_.x + box_.w - 1.0, below, 14.0, 8.0, prompt.len()),
            prompt.len()
        );
        // And the caret the click asks for is where the frame draws it.
        let mut moved = typed_prompt(&typed, 0);
        moved.place(columns + 3);
        let (_, _, after) = render_prompt(&moved, 8);
        let caret = |scene: &Scene| {
            scene
                .rects
                .iter()
                .map(|r| r.xywh())
                .rfind(|[x, y, w, _]| *w <= 3.0 && strip.contains(*x, *y))
                .expect("the caret is drawn")
        };
        assert_ne!(caret(&scene), caret(&after));
        let placed = caret(&after);
        assert_eq!(
            layout.input_caret(placed[0] + 1.0, placed[1] + 1.0, 14.0, 8.0, moved.len()),
            columns + 3
        );
    }

    /// Select-all bands every row the text covers, and nothing outside the
    /// prompt. A selection you cannot see is a selection you delete by
    /// accident.
    #[test]
    fn the_prompt_bands_what_it_has_selected() {
        let mut prompt = typed_prompt(&"y".repeat(400), 0);
        prompt.select_all();
        let (strip, _, scene) = render_prompt(&prompt, 8);
        let skin = Skin::from(&Config::default());
        let bands: Vec<[f32; 4]> = scene
            .rects
            .iter()
            .filter(|r| r.rgba() == skin.select)
            .map(|r| r.xywh())
            .collect();
        let line = Text::line_for(14.0);
        let box_ = input_box(strip, line);
        let columns = columns_in(box_.w, 8.0);
        assert_eq!(bands.len(), (400 + PROMPT_COLUMNS).div_ceil(columns));
        for band in &bands {
            assert!(band[1] >= box_.y - 0.01, "{band:?} is above the prompt");
            assert!(
                band[1] + band[3] <= box_.y + box_.h + 0.5,
                "{band:?} runs below the prompt"
            );
            assert!(
                band[0] + band[2] <= box_.x + box_.w + 0.01,
                "{band:?} runs past the right edge"
            );
        }
        // The first row starts after the marker, not at the left edge.
        let first = bands
            .iter()
            .min_by(|a, b| a[1].total_cmp(&b[1]))
            .expect("a first row");
        assert!((first[0] - (box_.x + PROMPT_COLUMNS as f32 * 8.0)).abs() < 0.01);
        // Nothing selected is nothing banded.
        let (_, _, plain) = render_prompt(&typed_prompt("hello", 5), 8);
        assert!(!plain.rects.iter().any(|r| r.rgba() == skin.select));
    }

    /// Two dark panels side by side over a busy desktop read as one region
    /// with a gap in it. The border is what tells them apart.
    ///
    /// One stroked rectangle covering the whole panel, not four 1px edges
    /// around it: a square outline cannot follow the cut corner, and it used to
    /// cost five rectangles per pane instead of two.
    #[test]
    fn every_space_is_drawn_with_a_border() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &["a.rs"]);
        for space in Space::ALL {
            let panel = out.layout.placed(space).body;
            let over_panel: Vec<_> = out
                .scene
                .rects
                .iter()
                .filter(|r| r.xywh() == [panel.x, panel.y, panel.w, panel.h])
                .collect();
            let strokes: Vec<_> = over_panel
                .iter()
                .filter(|r| r.extra()[3] > 0.0)
                .collect();
            assert_eq!(strokes.len(), 1, "{space:?} is not bordered by one rect");
            assert_eq!(strokes[0].extra()[3], 1.0, "a hairline, not a slab");
            // The fill under it is a second rectangle, and no more than that.
            assert_eq!(over_panel.len(), 2, "{space:?} costs more than fill plus edge");
        }
    }

    /// The cut corner, on the fill and on the border alike. A square fill under
    /// a cut border leaves a triangle of panel colour outside its own edge.
    #[test]
    fn a_panel_is_cut_on_its_top_right_corner_only() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &["a.rs"]);
        let boxes: Vec<Panel> = Space::ALL
            .iter()
            .map(|space| out.layout.placed(*space).body)
            .chain(std::iter::once(out.layout.input))
            .collect();
        for panel in boxes {
            let shaped: Vec<_> = out
                .scene
                .rects
                .iter()
                .filter(|r| r.xywh() == [panel.x, panel.y, panel.w, panel.h])
                .collect();
            assert_eq!(shaped.len(), 2, "{panel:?} is not a fill plus an edge");
            for rect in shaped {
                let [_, chamfer, corners, _] = rect.extra();
                assert_eq!(chamfer, CUT, "{rect:?} is not cut");
                assert_eq!(corners, Rect::TOP_RIGHT as f32, "{rect:?} cuts elsewhere");
            }
        }
    }

    /// Shaded, the window is one strip. Every other region has to be gone, or
    /// a click lands on a pane that is not on screen.
    #[test]
    fn shading_leaves_the_bar_and_nothing_else() {
        let dock = Dock::new();
        let mut shape = shape(&dock, &["a.rs"]);
        shape.shaded = true;
        let layout = Layout::compute(1200.0, 800.0, &shape);
        assert!(layout.shaded);
        for space in Space::ALL {
            assert_eq!(layout.placed(space).body.h, 0.0);
            assert!(layout.placed(space).tabs.is_empty());
        }
        assert_eq!(layout.hit(600.0, 400.0), None);
        assert_eq!(layout.hit(600.0, 10.0), Some(Hit::TitleBar));

        let skin = Skin::from(&Config::default());
        let state = busy_state();
        let scene = build(&Frame {
            state: &state,
            monitor: &Monitor::new(),
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            drag: None,
            hot: None,
            trouble: None,
            selection: None,
            avatar: None,
        });
        let text = text_of(&scene);
        assert!(text.contains("WORKING") || text.contains("THINKING"), "{text}");
        assert!(!text.contains("looking at it now"), "no pane content");
    }

    #[test]
    fn the_buttons_win_against_the_title_bar_they_sit_on() {
        let out = render(&State::new(), 1200.0, 800.0, &Dock::new(), &[]);
        assert!(out.layout.title.contains(out.layout.close.x + 1.0, 10.0));
        assert_eq!(
            out.layout.hit(out.layout.close.x + 1.0, 10.0),
            Some(Hit::Close)
        );
        assert_eq!(out.layout.hit(200.0, 10.0), Some(Hit::TitleBar));
    }

    #[test]
    fn tabs_that_do_not_fit_are_dropped_not_squeezed() {
        let dock = Dock::new();
        let many: Vec<&str> = vec!["averyverylongfilename.rs"; 30];
        let out = render(&busy_state(), 900.0, 700.0, &dock, &many);
        assert!(out.layout.file_tabs.len() < many.len(), "some were dropped");
        for (_, panel) in &out.layout.file_tabs {
            assert!(panel.w > 20.0, "no slivers: {panel:?}");
        }
    }

    #[test]
    fn the_resize_edges_are_the_border_and_nothing_else() {
        use winit::window::ResizeDirection as Dir;
        assert_eq!(edge(0.0, 0.0, 800.0, 600.0), Some(Dir::NorthWest));
        assert_eq!(edge(799.0, 599.0, 800.0, 600.0), Some(Dir::SouthEast));
        assert_eq!(edge(400.0, 300.0, 800.0, 600.0), None);
    }

    #[test]
    fn a_file_tab_says_enough_to_tell_two_of_them_apart() {
        assert_eq!(short_name("src/calc.py"), "calc.py");
        assert_eq!(short_name("crates/noob/src/mod.rs"), "src/mod.rs");
        assert_eq!(short_name("README"), "README");
    }

    #[test]
    fn a_deep_workspace_shows_its_last_two_segments() {
        assert_eq!(
            short_path("/home/hec/workspace/noob-cli"),
            "workspace/noob-cli"
        );
        assert_eq!(short_path("noob-cli"), "noob-cli");
    }
}
