//! Layout, hit regions, and turning state into a scene.
//!
//! One surface carved into panes, never several OS windows. The window has no
//! system chrome, so the title bar, its three buttons, the tab strips, the
//! scrollbars and the resize edges are all rectangles here and hit regions in
//! [`Layout`]. Drawing and hit testing take the same numbers from the same
//! place, which is the only way they can never disagree.
//!
//! The window has two shapes. Open, it is a conversation beside three panes.
//! Shaded, it is one strip carrying [`State::headline`] and nothing else, the
//! way Winamp collapsed to its title. Double-click the bar to go between them.

use noob_draw::{Panel, Run, Scene, Text};

use crate::monitor::Monitor;
use crate::skin::Skin;
use crate::state::{State, TodoState, Tone};

pub const TITLE_H: f32 = 30.0;
pub const INPUT_H: f32 = 36.0;
pub const STATUS_H: f32 = 24.0;
pub const TAB_H: f32 = 22.0;
pub const RESIZE_EDGE: f32 = 6.0;
const GAP: f32 = 6.0;
const PAD: f32 = 9.0;
const SMALL: f32 = 12.0;
const SCROLL_W: f32 = 4.0;
const BUTTON_W: f32 = 26.0;

/// Which tab of the upper right group is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Activity,
    Plan,
    Agents,
    Monitor,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Activity, Tab::Plan, Tab::Agents, Tab::Monitor];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Activity => "ACTIVITY",
            Tab::Plan => "PLAN",
            Tab::Agents => "AGENTS",
            Tab::Monitor => "MONITOR",
        }
    }

    pub fn next(self) -> Tab {
        let at = Tab::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Tab::ALL[(at + 1) % Tab::ALL.len()]
    }
}

/// Something the pointer can land on. Returned by [`Layout::hit`] so every
/// click is resolved in one place instead of in a chain of `if` in the event
/// handler.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    TitleBar,
    Minimize,
    Maximize,
    Close,
    Tab(Tab),
    /// Collapse or expand the upper right group.
    FoldTop,
    FoldFiles,
    File(usize),
    Talk,
    Group,
    Files,
    Input,
}

/// Where everything is this frame. Built from the window size and a little
/// view state, so nothing else has to recompute it.
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub shaded: bool,

    pub title: Panel,
    pub minimize: Panel,
    pub maximize: Panel,
    pub close: Panel,

    pub talk: Panel,
    pub tabs: Vec<(Tab, Panel)>,
    pub fold_top: Panel,
    pub group: Panel,
    pub file_tabs: Vec<(usize, Panel)>,
    pub fold_files: Panel,
    pub files: Panel,
    pub input: Panel,
    pub status: Panel,
}

/// What the layout needs to know beyond the window size.
pub struct Shape {
    pub shaded: bool,
    /// Tab strips stay; their content collapses away.
    pub fold_top: bool,
    pub fold_files: bool,
    /// One label per file tab, in order. Which one is selected is a drawing
    /// decision, not a layout one, so it is not here.
    pub file_labels: Vec<String>,
    pub column: f32,
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
            let nowhere = Panel::new(0.0, 0.0, 0.0, 0.0);
            return Layout {
                width,
                height,
                shaded: true,
                title,
                minimize: buttons[0],
                maximize: buttons[1],
                close: buttons[2],
                talk: nowhere,
                tabs: Vec::new(),
                fold_top: nowhere,
                group: nowhere,
                file_tabs: Vec::new(),
                fold_files: nowhere,
                files: nowhere,
                input: nowhere,
                status: nowhere,
            };
        }

        let (rest, status) = rest.split_bottom(STATUS_H.min(rest.h));
        let (body, input) = rest.split_bottom(INPUT_H.min(rest.h));
        let body = body.inset(GAP);
        let (talk, right) = body.split_left((body.w * 0.54).floor() - GAP * 0.5);
        let right = Panel::new(right.x + GAP, right.y, (right.w - GAP).max(1.0), right.h);

        // Each group is a tab strip plus its content. A folded group is its
        // strip alone, and the room it gives up goes to the other one.
        let top_h = match (shape.fold_top, shape.fold_files) {
            (true, _) => TAB_H,
            (false, true) => (right.h - TAB_H - GAP).max(TAB_H),
            (false, false) => ((right.h - GAP) * 0.42).max(TAB_H).floor(),
        };
        let (top, lower) = right.split_top(top_h.min(right.h));
        let lower = Panel::new(lower.x, lower.y + GAP, lower.w, (lower.h - GAP).max(0.0));

        let (top_strip, group) = top.split_top(TAB_H.min(top.h));
        let (files_strip, files) = lower.split_top(TAB_H.min(lower.h));

        let fold_top = Panel::new(top_strip.x + top_strip.w - TAB_H, top_strip.y, TAB_H, TAB_H);
        let fold_files = Panel::new(
            files_strip.x + files_strip.w - TAB_H,
            files_strip.y,
            TAB_H,
            TAB_H,
        );

        let tabs = strip(
            Panel::new(top_strip.x, top_strip.y, (top_strip.w - TAB_H).max(1.0), TAB_H),
            Tab::ALL.iter().map(|t| t.label().len()),
            shape.column,
        )
        .into_iter()
        .enumerate()
        .map(|(i, panel)| (Tab::ALL[i], panel))
        .collect();

        let file_tabs = strip(
            Panel::new(
                files_strip.x,
                files_strip.y,
                (files_strip.w - TAB_H).max(1.0),
                TAB_H,
            ),
            shape.file_labels.iter().map(|label| label.chars().count()),
            shape.column,
        )
        .into_iter()
        .enumerate()
        .collect();

        Layout {
            width,
            height,
            shaded: false,
            title,
            minimize: buttons[0],
            maximize: buttons[1],
            close: buttons[2],
            talk,
            tabs,
            fold_top,
            group: if shape.fold_top {
                Panel::new(group.x, group.y, group.w, 0.0)
            } else {
                group
            },
            file_tabs,
            fold_files,
            files: if shape.fold_files {
                Panel::new(files.x, files.y, files.w, 0.0)
            } else {
                files
            },
            input: input.inset(GAP),
            status,
        }
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
        if self.fold_top.contains(x, y) {
            return Some(Hit::FoldTop);
        }
        if self.fold_files.contains(x, y) {
            return Some(Hit::FoldFiles);
        }
        for (tab, panel) in &self.tabs {
            if panel.contains(x, y) {
                return Some(Hit::Tab(*tab));
            }
        }
        for (index, panel) in &self.file_tabs {
            if panel.contains(x, y) {
                return Some(Hit::File(*index));
            }
        }
        for (panel, hit) in [
            (self.talk, Hit::Talk),
            (self.group, Hit::Group),
            (self.files, Hit::Files),
            (self.input, Hit::Input),
        ] {
            if panel.contains(x, y) {
                return Some(hit);
            }
        }
        None
    }

    /// Rows a panel can show. The header line is content, not scrollback.
    pub fn rows(&self, panel: Panel, size: f32) -> usize {
        Text::rows_for(size, panel.inset(PAD).h)
    }
}

/// Lay tabs left to right at the width their labels need, dropping any that do
/// not fit rather than squeezing them into unreadable slivers.
fn strip(bar: Panel, widths: impl Iterator<Item = usize>, column: f32) -> Vec<Panel> {
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

pub struct Frame<'a> {
    pub state: &'a State,
    pub monitor: &'a Monitor,
    pub skin: &'a Skin,
    pub layout: &'a Layout,
    pub tab: Tab,
    pub fold_top: bool,
    pub fold_files: bool,
    pub input: &'a str,
    pub caret: usize,
    pub column: f32,
    pub body_size: f32,
    pub pane_size: f32,
    /// The GPU capability report and the settings path: facts about this
    /// machine, which belong beside the readings and not in the activity log.
    pub reports: &'a [String],
    /// What the pointer is over, for the button highlight.
    pub hot: Option<Hit>,
    /// Shown in the title bar when the agent could not be reached.
    pub trouble: Option<&'a str>,
}

pub fn build(frame: &Frame) -> Scene {
    let mut scene = Scene::default();
    let skin = frame.skin;
    let layout = frame.layout;
    let state = frame.state;

    scene.rect(Panel::new(0.0, 0.0, layout.width, layout.height).fill(skin.backdrop));
    title_bar(&mut scene, frame);

    if layout.shaded {
        return scene;
    }

    let talk_rows = layout.rows(layout.talk, frame.body_size).saturating_sub(1);
    pane(&mut scene, frame, layout.talk, frame.body_size, |runs| {
        let subject = if state.turn > 0 {
            format!("turn {}", state.turn)
        } else {
            String::new()
        };
        header(runs, "TALK", &subject, skin);
        // A window that starts inside a fenced block has to know it is looking
        // at code, so the state is carried in from the lines above it.
        let mut fence = state.talk.fence_before(talk_rows);
        for line in state.talk.visible(talk_rows) {
            match line.tone {
                // Only the model's prose is Markdown. What the human typed and
                // what the harness noted are shown as written.
                Tone::Body => crate::markdown::line(&line.text, &mut fence, skin, runs),
                tone => runs.push(Run::tinted(&line.text, skin.tone(tone))),
            }
            runs.push(Run::plain("\n"));
        }
    });
    scrollbar(&mut scene, skin, layout.talk, state.talk.thumb(talk_rows));

    tab_strip(
        &mut scene,
        frame,
        &layout
            .tabs
            .iter()
            .map(|(tab, panel)| (tab.label().to_string(), *tab == frame.tab, false, *panel))
            .collect::<Vec<_>>(),
        layout.fold_top,
        frame.fold_top,
    );
    if !frame.fold_top {
        group_pane(&mut scene, frame);
    }

    // Indexed through `get`: the layout is built from labels handed in, and a
    // caller whose labels are one frame ahead of its state must not panic the
    // window.
    let file_labels: Vec<(String, bool, bool, Panel)> = layout
        .file_tabs
        .iter()
        .filter_map(|(index, panel)| {
            let file = state.files.get(*index)?;
            Some((
                short_name(&file.path),
                *index == state.open_file,
                file.changed,
                *panel,
            ))
        })
        .collect();
    tab_strip(
        &mut scene,
        frame,
        &file_labels,
        layout.fold_files,
        frame.fold_files,
    );
    if !frame.fold_files {
        files_pane(&mut scene, frame);
    }

    input_row(&mut scene, frame);
    status_bar(&mut scene, frame);
    scene
}

fn title_bar(scene: &mut Scene, frame: &Frame) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    scene.rect(layout.title.fill(skin.bar));

    // ASCII, at the body size, centred by measured column width. The first
    // version used \u{2715} and \u{25a1} and drew nothing on a font that has
    // neither, which reads as three broken buttons.
    let mark = frame.body_size;
    for (panel, glyph, hit, tint) in [
        (layout.minimize, "_", Hit::Minimize, skin.hot),
        (layout.maximize, "[]", Hit::Maximize, skin.hot),
        (layout.close, "X", Hit::Close, skin.close_hot),
    ] {
        let lit = frame.hot == Some(hit);
        if lit {
            scene.rect(panel.fill(tint));
        }
        let width = glyph.chars().count() as f32 * frame.column;
        let centred = Panel::new(
            panel.x + ((panel.w - width) * 0.5).floor(),
            panel.y,
            width.max(1.0),
            panel.h,
        )
        .row(0.0, Text::line_for(mark));
        scene.text(Text::rich(
            vec![Run::tinted(glyph, if lit { skin.bright } else { skin.title })],
            centred,
            mark,
            skin.bright,
        ));
    }

    let room = (layout.width - BUTTON_W * 3.0 - 12.0).max(1.0);
    let mut runs = vec![Run::tinted("NO0B \u{25b8} CLIppy", skin.bright)];
    if let Some(trouble) = frame.trouble {
        runs.push(Run::tinted(format!("   {trouble}"), skin.bad));
    } else if layout.shaded {
        // Shaded, this strip is the whole window, so it carries the one thing
        // worth knowing rather than the model name and the path.
        runs.push(Run::tinted(format!("   {}", state.headline()), skin.good));
    } else {
        runs.push(Run::tinted(
            format!(
                "   {}   {}{}",
                if state.model.is_empty() {
                    "…"
                } else {
                    &state.model
                },
                short_path(&state.workspace),
                if state.resumed { "   resumed" } else { "" },
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

fn tab_strip(
    scene: &mut Scene,
    frame: &Frame,
    tabs: &[(String, bool, bool, Panel)],
    fold: Panel,
    folded: bool,
) {
    let skin = frame.skin;
    if tabs.is_empty() && fold.w == 0.0 {
        return;
    }
    let bar = Panel::new(
        tabs.first().map_or(fold.x, |(_, _, _, p)| p.x),
        fold.y,
        (fold.x + fold.w) - tabs.first().map_or(fold.x, |(_, _, _, p)| p.x),
        fold.h,
    );
    scene.rect(bar.fill(skin.strip));
    scene.rect(bar.bottom_edge(skin.edge));
    for (label, active, changed, panel) in tabs {
        if *active {
            scene.rect(panel.fill(skin.panel));
            scene.rect(panel.top_edge(skin.edge_focus));
        } else {
            scene.rect(panel.left_edge(skin.edge));
        }
        let color = if *active { skin.bright } else { skin.dim };
        let mut runs = vec![Run::tinted(label.as_str(), color)];
        if *changed {
            runs.push(Run::tinted("\u{2022}", skin.plus));
        }
        scene.text(Text::rich(
            runs,
            panel.row(SMALL * 0.6, Text::line_for(SMALL)),
            SMALL,
            color,
        ));
    }
    scene.text(Text::rich(
        vec![Run::tinted(
            if folded { "\u{25b8}" } else { "\u{25be}" },
            skin.dim,
        )],
        fold.row(0.0, Text::line_for(SMALL)),
        SMALL,
        skin.dim,
    ));
}

fn group_pane(scene: &mut Scene, frame: &Frame) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    let panel = layout.group;
    let rows = layout.rows(panel, frame.pane_size);
    pane(scene, frame, panel, frame.pane_size, |runs| match frame.tab {
        Tab::Activity => {
            for line in state.activity.visible(rows) {
                runs.push(Run::tinted(&line.text, skin.tone(line.tone)));
                runs.push(Run::plain("\n"));
            }
        }
        Tab::Plan => {
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
        }
        Tab::Agents => {
            if state.agents.is_empty() {
                runs.push(Run::tinted("no sub-agents this session", skin.dim));
            }
            for agent in &state.agents {
                runs.push(Run::tinted(
                    format!("{:<9}{:<9}", agent.label, agent.state),
                    skin.tone(agent.tone),
                ));
                runs.push(Run::tinted(clip(&agent.brief, 200), skin.dim));
                runs.push(Run::plain("\n"));
            }
        }
        Tab::Monitor => monitor_text(runs, frame),
    });
    match frame.tab {
        Tab::Activity => scrollbar(scene, skin, panel, state.activity.thumb(rows)),
        Tab::Monitor => monitor_bars(scene, frame, panel),
        _ => {}
    }
}

/// The labelled rows of the monitor. The bars themselves are rectangles drawn
/// over this text, so a proportion is a shape rather than a row of `#`.
fn monitor_text(runs: &mut Vec<Run>, frame: &Frame) {
    let skin = frame.skin;
    if frame.monitor.gauges.is_empty() {
        runs.push(Run::tinted("sampling…", skin.dim));
        runs.push(Run::plain("\n"));
    }
    for gauge in &frame.monitor.gauges {
        runs.push(Run::tinted(format!("{:<8}", gauge.label), skin.dim));
        // The bar's room, kept as spaces so the reading lands after it.
        runs.push(Run::plain(" ".repeat(BAR_COLUMNS)));
        runs.push(Run::tinted(
            format!("  {}", gauge.reading()),
            if gauge.fraction().is_some_and(|f| f > 0.85) {
                skin.bad
            } else {
                skin.body
            },
        ));
        runs.push(Run::plain("\n"));
    }
    runs.push(Run::plain("\n"));
    for note in frame.monitor.notes.iter().chain(frame.reports.iter()) {
        runs.push(Run::tinted(note.as_str(), skin.dim));
        runs.push(Run::plain("\n"));
    }
}

/// One radeontop-style bar per gauge, and a btop-style history behind it. Both
/// out of rectangles: a proportion drawn as a shape reads at a glance, and a
/// row of block characters depends on a font having them.
fn monitor_bars(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let skin = frame.skin;
    let content = panel.inset(PAD);
    let line = Text::line_for(frame.pane_size);
    let bar_x = content.x + 8.0 * frame.column;
    let bar_w = (BAR_COLUMNS as f32 * frame.column).min((content.w - 8.0 * frame.column).max(1.0));
    for (row, gauge) in frame.monitor.gauges.iter().enumerate() {
        let y = content.y + row as f32 * line;
        if y + line > content.y + content.h {
            break;
        }
        let track = Panel::new(bar_x, y + line * 0.28, bar_w, line * 0.44);
        // The history first, behind the bar: the past is context, not content.
        let series = frame.monitor.history(gauge.key);
        if series.len() > 1 {
            let step = (track.w / series.len() as f32).max(1.0);
            for (i, point) in series.iter().enumerate() {
                let height = (track.h * point).max(1.0);
                scene.rect(
                    Panel::new(
                        track.x + i as f32 * step,
                        track.y + track.h - height,
                        step.max(1.0),
                        height,
                    )
                    .fill(skin.scroll_track),
                );
            }
        } else {
            scene.rect(track.fill(skin.gauge_track));
        }
        if let Some(fraction) = gauge.fraction() {
            let full = fraction > 0.85;
            scene.rect(
                Panel::new(track.x, track.y, (track.w * fraction).max(1.0), track.h)
                    .fill(if full { skin.close_hot } else { skin.gauge }),
            );
        }
    }
}

const BAR_COLUMNS: usize = 24;

fn files_pane(scene: &mut Scene, frame: &Frame) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    let panel = layout.files;
    let rows = layout.rows(panel, frame.pane_size);
    let open = state.files.get(state.open_file);
    pane(scene, frame, panel, frame.pane_size, |runs| {
        let Some(file) = open else {
            runs.push(Run::tinted("no files touched yet", skin.dim));
            return;
        };
        let syntax = crate::syntax::for_path(&file.path);
        for line in file.pane.visible(rows) {
            let base = skin.tone(line.tone);
            // A removed line reads as removed first, so only what is there now
            // is tokenized.
            let source = matches!(line.tone, Tone::Plus | Tone::Body);
            if source {
                let (marker, rest) = line.text.split_at(line.text.len().min(2));
                runs.push(Run::tinted(marker, base));
                for (text, token) in crate::syntax::scan(rest, syntax) {
                    runs.push(Run::tinted(text, skin.token(token).unwrap_or(base)));
                }
            } else {
                runs.push(Run::tinted(&line.text, base));
            }
            runs.push(Run::plain("\n"));
        }
    });
    if let Some(file) = open {
        scrollbar(scene, skin, panel, file.pane.thumb(rows));
    }
}

/// A pane: its fill, its edge, and one text box built by `body`.
fn pane(
    scene: &mut Scene,
    frame: &Frame,
    panel: Panel,
    size: f32,
    body: impl FnOnce(&mut Vec<Run>),
) {
    if panel.h < 2.0 {
        return;
    }
    scene.rect(panel.fill(frame.skin.panel));
    let mut runs = Vec::new();
    body(&mut runs);
    scene.text(Text::rich(
        runs,
        panel.inset(PAD),
        size,
        frame.skin.body,
    ));
}

fn header(runs: &mut Vec<Run>, title: &str, subject: &str, skin: &Skin) {
    runs.push(Run::tinted(format!("{title:<6}"), skin.dim));
    if !subject.is_empty() {
        runs.push(Run::tinted(subject, skin.dim));
    }
    runs.push(Run::plain("\n"));
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
    scene.rect(layout.input.fill(skin.input));
    scene.rect(layout.input.top_edge(skin.edge_focus));
    // One centred line, not a margin: insetting this bar leaves a box too short
    // to hold the text, which draws and then clips to nothing.
    let box_ = layout.input.row(PAD, Text::line_for(frame.body_size));
    let prompt = if state.phase.busy() { "\u{2026}" } else { "\u{203a}" };
    scene.text(Text::rich(
        vec![
            Run::tinted(format!("{prompt} "), skin.dim),
            Run::tinted(frame.input, skin.bright),
        ],
        box_,
        frame.body_size,
        skin.bright,
    ));
    // Two columns for the prompt, then one per character typed.
    let caret_x = box_.x + (frame.caret as f32 + 2.0) * frame.column;
    if caret_x < box_.x + box_.w {
        scene.rect(Panel::new(caret_x, box_.y, 2.0, box_.h).fill(skin.caret));
    }
}

fn status_bar(scene: &mut Scene, frame: &Frame) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    scene.rect(layout.status.fill(skin.bar));
    let gauge = Panel::new(0.0, layout.status.y, layout.width, 2.0);
    scene.rect(gauge.fill(skin.gauge_track));
    let used = state.context_fraction();
    if used > 0.0 {
        scene.rect(Panel::new(0.0, gauge.y, layout.width * used, 2.0).fill(skin.gauge));
    }
    scene.text(Text::rich(
        vec![
            Run::tinted(format!("{:<12}", state.phase.word().to_lowercase()), skin.bright),
            Run::tinted(state.budget_line(), skin.title),
        ],
        layout.status.row(12.0, Text::line_for(SMALL)),
        SMALL,
        skin.title,
    ));
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

    fn shape(files: &[&str]) -> Shape {
        Shape {
            shaded: false,
            fold_top: false,
            fold_files: false,
            file_labels: files.iter().map(|f| f.to_string()).collect(),
            column: 8.0,
        }
    }

    fn layout(w: f32, h: f32) -> Layout {
        Layout::compute(w, h, &shape(&["calc.py", "tools.md"]))
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
            brief: "3 items".into(),
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

    fn scene_of(state: &State, w: f32, h: f32, shape: &Shape) -> (Scene, Layout, Skin) {
        scene_on(state, w, h, shape, Tab::Activity)
    }

    fn scene_on(
        state: &State,
        w: f32,
        h: f32,
        shape: &Shape,
        tab: Tab,
    ) -> (Scene, Layout, Skin) {
        let layout = Layout::compute(w, h, shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state,
            monitor: &Monitor::new(),
            skin: &skin,
            layout: &layout,
            tab,
            fold_top: shape.fold_top,
            fold_files: shape.fold_files,
            input: "type here",
            caret: 4,
            column: shape.column,
            body_size: 14.0,
            pane_size: 13.0,
            reports: &[],
            hot: None,
            trouble: None,
        });
        (scene, layout, skin)
    }

    fn rendered(scene: &Scene) -> String {
        scene
            .texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| r.text.as_str()))
            .collect()
    }

    #[test]
    fn the_groups_stack_without_overlapping_and_stay_in_the_window() {
        for (w, h) in [(1200.0, 800.0), (700.0, 460.0), (2560.0, 1440.0)] {
            let layout = layout(w, h);
            assert!(layout.talk.x + layout.talk.w <= layout.group.x);
            assert!(layout.group.y + layout.group.h <= layout.files.y);
            for panel in [layout.talk, layout.group, layout.files, layout.input] {
                assert!(panel.x >= 0.0 && panel.y >= 0.0, "{panel:?} at {w}x{h}");
                assert!(panel.x + panel.w <= w + 0.01, "{panel:?} at {w}x{h}");
                assert!(panel.y + panel.h <= h + 0.01, "{panel:?} at {w}x{h}");
            }
        }
    }

    /// Every click resolves in one place, so what a region looks like and what
    /// it does can never come apart.
    #[test]
    fn every_region_is_hit_where_it_is_drawn() {
        let layout = layout(1200.0, 800.0);
        let middle = |p: Panel| (p.x + p.w * 0.5, p.y + p.h * 0.5);
        let cases: Vec<(Panel, Hit)> = vec![
            (layout.close, Hit::Close),
            (layout.maximize, Hit::Maximize),
            (layout.minimize, Hit::Minimize),
            (layout.fold_top, Hit::FoldTop),
            (layout.fold_files, Hit::FoldFiles),
            (layout.talk, Hit::Talk),
            (layout.group, Hit::Group),
            (layout.files, Hit::Files),
            (layout.input, Hit::Input),
        ];
        for (panel, expected) in cases {
            let (x, y) = middle(panel);
            assert_eq!(layout.hit(x, y), Some(expected), "{expected:?} {panel:?}");
        }
        for (tab, panel) in &layout.tabs {
            let (x, y) = middle(*panel);
            assert_eq!(layout.hit(x, y), Some(Hit::Tab(*tab)));
        }
        for (index, panel) in &layout.file_tabs {
            let (x, y) = middle(*panel);
            assert_eq!(layout.hit(x, y), Some(Hit::File(*index)));
        }
    }

    /// The buttons sit on the title bar and must win against it, or the window
    /// drags instead of closing.
    #[test]
    fn the_buttons_win_against_the_title_bar_they_sit_on() {
        let layout = layout(1200.0, 800.0);
        assert!(layout.title.contains(layout.close.x + 1.0, 10.0));
        assert_eq!(layout.hit(layout.close.x + 1.0, 10.0), Some(Hit::Close));
        assert_eq!(layout.hit(200.0, 10.0), Some(Hit::TitleBar));
    }

    /// Shaded, the window is one strip. Every other region has to be gone, or
    /// a click lands on a pane that is not on screen.
    #[test]
    fn shading_leaves_the_bar_and_nothing_else() {
        let mut shape = shape(&["a.rs"]);
        shape.shaded = true;
        let layout = Layout::compute(1200.0, 800.0, &shape);
        assert!(layout.shaded);
        assert!(layout.tabs.is_empty() && layout.file_tabs.is_empty());
        for panel in [layout.talk, layout.group, layout.files, layout.input] {
            assert_eq!(panel.w, 0.0);
            assert_eq!(panel.h, 0.0);
        }
        // Below the strip there is nothing to hit.
        assert_eq!(layout.hit(600.0, 400.0), None);
        assert_eq!(layout.hit(600.0, 10.0), Some(Hit::TitleBar));
        // And the strip carries the headline rather than the model and path.
        let (scene, _, _) = scene_of(&busy_state(), 1200.0, 800.0, &shape);
        let text = rendered(&scene);
        assert!(text.contains("WORKING") || text.contains("THINKING"), "{text}");
        assert!(!text.contains("looking at it now"), "no pane content");
    }

    /// Folding a group gives its room to the other one rather than leaving a
    /// hole where it was.
    #[test]
    fn folding_a_group_gives_its_room_away() {
        let open = Layout::compute(1200.0, 800.0, &shape(&["a.rs"]));
        let mut folded = shape(&["a.rs"]);
        folded.fold_top = true;
        let folded = Layout::compute(1200.0, 800.0, &folded);
        assert_eq!(folded.group.h, 0.0, "the folded group has no content");
        assert!(folded.files.h > open.files.h, "the other group grew");
        assert!(folded.fold_top.w > 0.0, "its strip is still there to unfold");
    }

    /// Every text box must be able to hold at least one line of its own size.
    /// A box shorter than that draws the text and clips every pixel of it,
    /// which reads as the interface being broken.
    #[test]
    fn no_text_box_is_too_small_to_show_its_text() {
        let state = busy_state();
        for (w, h) in [(1200.0, 800.0), (700.0, 420.0), (420.0, 300.0)] {
            for (tab, fold_top, fold_files) in [
                (Tab::Activity, false, false),
                (Tab::Plan, false, false),
                (Tab::Agents, true, false),
                (Tab::Activity, false, true),
            ] {
                let mut shape = shape(&["calc.py"]);
                shape.fold_top = fold_top;
                shape.fold_files = fold_files;
                let (scene, _, _) = scene_on(&state, w, h, &shape, tab);
                for text in &scene.texts {
                    assert!(text.at.w >= 1.0, "{:?} at {w}x{h}", text.at);
                    assert!(
                        text.at.h >= Text::line_for(text.size),
                        "{:?} cannot hold one {}pt line at {w}x{h}",
                        text.at,
                        text.size
                    );
                    assert!(text.at.x >= 0.0 && text.at.y >= 0.0, "{:?}", text.at);
                    assert!(text.at.x + text.at.w <= w + 0.01, "{:?}", text.at);
                    assert!(text.at.y + text.at.h <= h + 0.01, "{:?}", text.at);
                }
            }
        }
    }

    #[test]
    fn every_rectangle_is_inside_the_surface() {
        let state = busy_state();
        for (w, h) in [(1200.0, 800.0), (320.0, 240.0)] {
            let (scene, _, _) = scene_of(&state, w, h, &shape(&["a.rs"]));
            assert!(!scene.rects.is_empty());
            for rect in &scene.rects {
                let [x, y, rw, rh] = rect.xywh();
                assert!(x >= 0.0 && y >= 0.0, "{rect:?} at {w}x{h}");
                assert!(x + rw <= w + 0.01 && y + rh <= h + 0.01, "{rect:?} at {w}x{h}");
            }
        }
    }

    /// Each tab shows its own thing and not the others'.
    #[test]
    fn each_tab_shows_its_own_content() {
        let state = busy_state();
        let shape = shape(&["calc.py"]);

        let text = rendered(&scene_on(&state, 1400.0, 900.0, &shape, Tab::Activity).0);
        assert!(text.contains("cargo test --workspace"), "{text}");

        let text = rendered(&scene_on(&state, 1400.0, 900.0, &shape, Tab::Plan).0);
        assert!(text.contains("[x] read it"), "{text}");
        assert!(text.contains("[>] fix it"), "{text}");
        assert!(!text.contains("cargo test --workspace"), "activity leaked");

        let text = rendered(&scene_on(&state, 1400.0, 900.0, &shape, Tab::Agents).0);
        assert!(text.contains("search the web"), "{text}");
        assert!(text.contains("running"), "{text}");
    }

    /// The conversation, the file diff and the budget are always on screen,
    /// whichever tab is up.
    #[test]
    fn the_conversation_and_the_file_are_always_visible() {
        let state = busy_state();
        for tab in Tab::ALL {
            let shape = shape(&["calc.py"]);
            let text = rendered(&scene_on(&state, 1400.0, 900.0, &shape, tab).0);
            assert!(text.contains("looking at it now"), "{tab:?}");
            assert!(text.contains("return a + b"), "{tab:?}");
            assert!(text.contains("1,816 / 65,536"), "{tab:?}");
            assert!(text.contains("type here"), "{tab:?}");
        }
    }

    /// A file the agent wrote is marked in its tab, so a glance says which of
    /// them it changed rather than only read.
    #[test]
    fn a_changed_file_is_marked_in_its_tab() {
        let state = busy_state();
        let text = rendered(&scene_of(&state, 1400.0, 900.0, &shape(&["calc.py"])).0);
        assert!(text.contains("calc.py\u{2022}"), "{text}");
    }

    #[test]
    fn the_code_pane_is_syntax_colored() {
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
        let (scene, layout, skin) = scene_of(&state, 1400.0, 900.0, &shape(&["calc.py"]));
        let content = layout.files.inset(PAD);
        let code = scene
            .texts
            .iter()
            .find(|t| t.at.x == content.x && t.at.y == content.y)
            .expect("the file pane has a text box");
        let colors: Vec<Option<[u8; 4]>> = code.runs.iter().map(|r| r.color).collect();
        assert!(colors.contains(&Some(skin.string)), "the string is tinted");
        assert!(colors.contains(&Some(skin.comment)), "the comment is tinted");
    }

    /// A tab strip too narrow for its tabs drops the ones that do not fit
    /// rather than drawing slivers nobody can read or click.
    #[test]
    fn tabs_that_do_not_fit_are_dropped_not_squeezed() {
        let many: Vec<&str> = vec!["averyverylongfilename.rs"; 30];
        let layout = Layout::compute(900.0, 700.0, &shape(&many));
        assert!(layout.file_tabs.len() < many.len(), "some were dropped");
        for (_, panel) in &layout.file_tabs {
            assert!(panel.w > 20.0, "no slivers: {panel:?}");
            assert!(
                panel.x + panel.w <= layout.fold_files.x + 0.01,
                "a tab ran into the fold control"
            );
        }
    }

    #[test]
    fn a_scrollbar_appears_only_when_there_is_something_to_scroll() {
        let mut state = State::new();
        let short = scene_of(&state, 1200.0, 800.0, &shape(&[])).0.rects.len();
        for n in 0..500 {
            state.apply(noob_proto::Event::TextDelta {
                d: format!("line {n}\n"),
            });
        }
        let long = scene_of(&state, 1200.0, 800.0, &shape(&[])).0.rects.len();
        assert_eq!(long, short + 2, "a track and a thumb appeared");
    }

    #[test]
    fn a_caret_past_the_edge_is_dropped() {
        let state = State::new();
        let layout = layout(600.0, 500.0);
        let skin = Skin::default();
        let count = |caret: usize| {
            build(&Frame {
                state: &state,
                monitor: &Monitor::new(),
                skin: &skin,
                layout: &layout,
                tab: Tab::Activity,
                fold_top: false,
                fold_files: false,
                input: "",
                caret,
                column: 8.0,
                body_size: 14.0,
                pane_size: 13.0,
                reports: &[],
                hot: None,
                trouble: None,
            })
            .rects
            .len()
        };
        assert_eq!(count(0) - 1, count(10_000), "the caret rect is gone");
    }

    #[test]
    fn trouble_replaces_the_title_detail() {
        let state = State::new();
        let layout = layout(900.0, 600.0);
        let skin = Skin::default();
        let scene = build(&Frame {
            state: &state,
            monitor: &Monitor::new(),
            skin: &skin,
            layout: &layout,
            tab: Tab::Activity,
            fold_top: false,
            fold_files: false,
            input: "",
            caret: 0,
            column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            reports: &[],
            hot: None,
            trouble: Some("cannot start \"noob\": not found"),
        });
        assert!(rendered(&scene).contains("cannot start"));
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
        assert_eq!(short_name("a/b/index.ts"), "b/index.ts");
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
