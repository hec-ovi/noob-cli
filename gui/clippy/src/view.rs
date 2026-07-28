//! Layout, and turning state into a scene.
//!
//! One surface carved into panes, never several OS windows. The window has no
//! system chrome, so the title bar, the close box and the resize edges are all
//! rectangles here and hit regions in [`Layout`].
//!
//! Every pane's text is wrapped and clipped to the same content box, so a long
//! line in one pane can never reach into its neighbour. That is a property of
//! `Panel::inset`, and this module's only job is to never take those two
//! numbers from different places.

use noob_draw::{Panel, Run, Scene, Text};

use crate::skin::Skin;
use crate::state::{State, Stream};

pub const TITLE_H: f32 = 30.0;
pub const INPUT_H: f32 = 34.0;
pub const STATUS_H: f32 = 24.0;
pub const RESIZE_EDGE: f32 = 6.0;
const GAP: f32 = 6.0;
const PAD: f32 = 9.0;
const BODY_SIZE: f32 = 14.0;
const PANE_SIZE: f32 = 13.0;
const SMALL_SIZE: f32 = 12.0;

/// Where everything is this frame. Built from the window size alone, so hit
/// testing and drawing can never disagree about it.
pub struct Layout {
    pub title: Panel,
    pub close: Panel,
    pub talk: Panel,
    pub shell: Panel,
    pub tools: Panel,
    pub code: Panel,
    pub input: Panel,
    pub status: Panel,
}

impl Layout {
    pub fn compute(width: f32, height: f32) -> Layout {
        let whole = Panel::new(0.0, 0.0, width, height);
        let (title, rest) = whole.split_top(TITLE_H);
        let (rest, status) = rest.split_bottom(STATUS_H);
        let (body, input) = rest.split_bottom(INPUT_H);

        let body = body.inset(GAP);
        // The conversation gets the wider half; the three activity streams
        // share the other. A code view is worth more room than a tool log, so
        // it takes half of that column.
        let (talk, right) = body.split_left((body.w * 0.54).floor() - GAP * 0.5);
        let right = Panel::new(right.x + GAP, right.y, right.w - GAP, right.h);
        let (shell, rest) = right.split_top((right.h * 0.26).floor());
        let (tools, code) = rest.split_top((rest.h * 0.32).floor());

        Layout {
            title,
            close: Panel::new(width - 26.0, 8.0, 14.0, 14.0),
            talk,
            shell: Panel::new(shell.x, shell.y, shell.w, shell.h - GAP),
            tools: Panel::new(tools.x, tools.y, tools.w, tools.h - GAP),
            code,
            input: input.inset(GAP),
            status,
        }
    }

    pub fn of(&self, stream: Stream) -> Panel {
        match stream {
            Stream::Talk => self.talk,
            Stream::Shell => self.shell,
            Stream::Tools => self.tools,
            Stream::Code => self.code,
        }
    }

    /// The pane under a point, if any. Used for click-to-focus and for
    /// routing the scroll wheel to what the pointer is over.
    pub fn pane_at(&self, x: f32, y: f32) -> Option<Stream> {
        [Stream::Talk, Stream::Shell, Stream::Tools, Stream::Code]
            .into_iter()
            .find(|stream| self.of(*stream).contains(x, y))
    }

    /// Rows a pane can show, which is what scrolling is measured in.
    pub fn rows(&self, stream: Stream) -> usize {
        let size = if stream == Stream::Talk {
            BODY_SIZE
        } else {
            PANE_SIZE
        };
        // The header line is inside the content box and is not scrollback.
        Text::rows_for(size, self.of(stream).inset(PAD).h).saturating_sub(1)
    }
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
    pub skin: &'a Skin,
    pub layout: &'a Layout,
    pub focus: Stream,
    pub input: &'a str,
    pub caret: usize,
    pub column: f32,
    /// Shown in the title bar when the agent could not be reached.
    pub trouble: Option<&'a str>,
}

pub fn build(frame: &Frame) -> Scene {
    let mut scene = Scene::default();
    let skin = frame.skin;
    let layout = frame.layout;
    let state = frame.state;
    let width = layout.title.w;
    let height = layout.status.y + layout.status.h;

    scene.rect(Panel::new(0.0, 0.0, width, height).fill(skin.backdrop));
    scene.rect(layout.title.fill(skin.bar));
    scene.rect(layout.close.fill(skin.caret));

    // Title bar: who we are, what we are talking to, and where.
    let title_box = Panel::new(12.0, 6.0, (width - 12.0 - 40.0).max(1.0), TITLE_H - 6.0);
    let mut title = vec![Run::tinted("NO0B ▸ CLIppy", skin.bright)];
    if let Some(trouble) = frame.trouble {
        title.push(Run::tinted(format!("   {trouble}"), skin.bad));
    } else {
        title.push(Run::tinted(
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
    scene.text(Text::rich(title, title_box, SMALL_SIZE, skin.title));

    pane(&mut scene, frame, Stream::Talk);
    pane(&mut scene, frame, Stream::Shell);
    pane(&mut scene, frame, Stream::Tools);
    pane(&mut scene, frame, Stream::Code);

    // The input line, with its own caret because there is no system text field.
    scene.rect(layout.input.fill(skin.input));
    scene.rect(layout.input.top_edge(skin.edge_focus));
    let input_box = layout.input.inset(PAD);
    let prompt = if state.busy { "…" } else { "›" };
    scene.text(Text::rich(
        vec![
            Run::tinted(format!("{prompt} "), skin.dim),
            Run::tinted(frame.input, skin.bright),
        ],
        input_box,
        BODY_SIZE,
        skin.bright,
    ));
    let caret_x = input_box.x + (frame.caret as f32 + 2.0) * frame.column;
    if caret_x < input_box.x + input_box.w {
        scene.rect(
            Panel::new(caret_x, input_box.y + 1.0, 2.0, BODY_SIZE * 1.15).fill(skin.caret),
        );
    }

    // Status: the session budget, as a gauge and as numbers.
    scene.rect(layout.status.fill(skin.bar));
    let gauge = Panel::new(0.0, layout.status.y, width, 2.0);
    scene.rect(gauge.fill(skin.gauge_track));
    let used = state.context_fraction();
    if used > 0.0 {
        scene.rect(Panel::new(0.0, gauge.y, width * used, 2.0).fill(skin.gauge));
    }
    // Derived from the bar rather than nudged into place: a box that is moved
    // without being shrunk runs past an edge, which is the same mistake that
    // once put one pane's text into the next one's.
    let line = (SMALL_SIZE * 1.42).round();
    let status_box = layout
        .status
        .inset(((layout.status.h - line) * 0.5).max(0.0))
        .inset(0.0);
    scene.text(Text::rich(
        vec![
            Run::tinted(format!("{:<12}", state.status), skin.bright),
            Run::tinted(state.budget_line(), skin.title),
        ],
        Panel::new(
            status_box.x + 12.0,
            status_box.y,
            (status_box.w - 24.0).max(1.0),
            status_box.h,
        ),
        SMALL_SIZE,
        skin.title,
    ));

    scene
}

fn pane(scene: &mut Scene, frame: &Frame, stream: Stream) {
    let skin = frame.skin;
    let panel = frame.layout.of(stream);
    let focused = frame.focus == stream;
    let state = frame.state;

    scene.rect(panel.fill(if stream == Stream::Talk {
        skin.panel
    } else {
        skin.panel_thin
    }));
    scene.rect(panel.top_edge(if focused { skin.edge_focus } else { skin.edge }));

    let size = if stream == Stream::Talk {
        BODY_SIZE
    } else {
        PANE_SIZE
    };
    let content = panel.inset(PAD);
    let pane = state.pane(stream);
    let rows = frame.layout.rows(stream);

    // Header: the pane's name, plus what it is currently about.
    let subject = match stream {
        Stream::Code => state.focus.clone().unwrap_or_default(),
        Stream::Talk if state.turn > 0 => format!("turn {}", state.turn),
        _ => String::new(),
    };
    let mut runs = vec![Run::tinted(
        format!("{:<6}", pane.title.to_uppercase()),
        if focused { skin.bright } else { skin.dim },
    )];
    if !subject.is_empty() {
        runs.push(Run::tinted(subject, skin.dim));
    }
    if pane.scrollback > 0 {
        runs.push(Run::tinted(
            format!("   ↑{} of {}", pane.scrollback, pane.len()),
            skin.good,
        ));
    }
    runs.push(Run::plain("\n"));

    let syntax = state
        .focus
        .as_deref()
        .map(crate::syntax::for_path)
        .unwrap_or(crate::syntax::Syntax::None);
    for line in pane.visible(rows) {
        let base = skin.tone(line.tone);
        // Only the code pane is source, and only its unchanged and added lines
        // are worth tokenizing: a removed line reads as removed first.
        let tokenize = stream == Stream::Code
            && matches!(line.tone, crate::state::Tone::Plus | crate::state::Tone::Body);
        if tokenize {
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
    scene.text(Text::rich(runs, content, size, skin.body));
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

    /// Panes tile the body: no gaps a click falls through, no overlap where
    /// two panes both claim a pixel.
    #[test]
    fn the_three_activity_panes_stack_without_overlapping() {
        let layout = Layout::compute(1200.0, 800.0);
        assert!(layout.shell.y + layout.shell.h <= layout.tools.y);
        assert!(layout.tools.y + layout.tools.h <= layout.code.y);
        assert!(layout.talk.x + layout.talk.w <= layout.shell.x);
        // The right column is one column.
        assert_eq!(layout.shell.x, layout.tools.x);
        assert_eq!(layout.shell.x, layout.code.x);
        assert_eq!(layout.shell.w, layout.code.w);
    }

    #[test]
    fn every_pane_stays_inside_the_window() {
        for (w, h) in [(1200.0, 800.0), (640.0, 400.0), (2560.0, 1440.0)] {
            let layout = Layout::compute(w, h);
            for stream in [Stream::Talk, Stream::Shell, Stream::Tools, Stream::Code] {
                let panel = layout.of(stream);
                assert!(panel.x >= 0.0 && panel.y >= TITLE_H, "{stream:?} at {w}x{h}");
                assert!(panel.x + panel.w <= w + 0.01, "{stream:?} at {w}x{h}");
                assert!(
                    panel.y + panel.h <= h - INPUT_H - STATUS_H + 0.01,
                    "{stream:?} at {w}x{h}"
                );
            }
        }
    }

    /// A window dragged down to nothing must not produce a negative panel that
    /// wraps text at a nonsense width.
    #[test]
    fn a_tiny_window_still_produces_usable_panels() {
        let layout = Layout::compute(120.0, 90.0);
        for stream in [Stream::Talk, Stream::Shell, Stream::Tools, Stream::Code] {
            let content = layout.of(stream).inset(PAD);
            assert!(content.w >= 1.0 && content.h >= 1.0, "{stream:?}");
        }
        assert!(layout.input.w >= 1.0);
    }

    #[test]
    fn hit_testing_finds_the_pane_under_a_point() {
        let layout = Layout::compute(1200.0, 800.0);
        for stream in [Stream::Talk, Stream::Shell, Stream::Tools, Stream::Code] {
            let panel = layout.of(stream);
            let inside = (panel.x + panel.w * 0.5, panel.y + panel.h * 0.5);
            assert_eq!(layout.pane_at(inside.0, inside.1), Some(stream), "{stream:?}");
        }
        // The title bar belongs to no pane.
        assert_eq!(layout.pane_at(600.0, 10.0), None);
    }

    #[test]
    fn the_resize_edges_are_the_border_and_nothing_else() {
        use winit::window::ResizeDirection as Dir;
        assert_eq!(edge(0.0, 0.0, 800.0, 600.0), Some(Dir::NorthWest));
        assert_eq!(edge(799.0, 599.0, 800.0, 600.0), Some(Dir::SouthEast));
        assert_eq!(edge(400.0, 2.0, 800.0, 600.0), Some(Dir::North));
        assert_eq!(edge(400.0, 300.0, 800.0, 600.0), None);
    }

    /// Rows are what scrolling counts, so they must match the space the text
    /// actually gets rather than the panel it sits in.
    #[test]
    fn rows_fit_inside_the_content_box() {
        let layout = Layout::compute(1200.0, 800.0);
        for stream in [Stream::Talk, Stream::Shell, Stream::Tools, Stream::Code] {
            let content = layout.of(stream).inset(PAD);
            let size = if stream == Stream::Talk {
                BODY_SIZE
            } else {
                PANE_SIZE
            };
            let rows = layout.rows(stream);
            let used = (rows + 1) as f32 * (size * 1.42).round();
            assert!(used <= content.h + 0.01, "{stream:?}: {used} in {}", content.h);
        }
    }

    #[test]
    fn a_deep_workspace_shows_its_last_two_segments() {
        assert_eq!(short_path("/home/hec/workspace/noob-cli"), "workspace/noob-cli");
        assert_eq!(short_path("noob-cli"), "noob-cli");
        assert_eq!(short_path(""), "");
    }

    // ---- the scene itself, without a GPU -------------------------------

    fn scene_of(state: &State, width: f32, height: f32) -> (Scene, Layout) {
        let layout = Layout::compute(width, height);
        let skin = Skin::matrix();
        let scene = build(&Frame {
            state,
            skin: &skin,
            layout: &layout,
            focus: Stream::Talk,
            input: "type here",
            caret: 4,
            column: 8.0,
            trouble: None,
        });
        (scene, layout)
    }

    /// Everything the panes hold, as one string, for asserting that content
    /// reached the scene at all.
    fn rendered(scene: &Scene) -> String {
        scene
            .texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| r.text.as_str()))
            .collect()
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
            call_id: Some("c2".into()),
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

    /// The four streams all reach the screen, each in its own pane. This is the
    /// end of the path that starts at a frame on the wire.
    #[test]
    fn every_stream_reaches_the_scene() {
        let state = busy_state();
        let (scene, _) = scene_of(&state, 1200.0, 800.0);
        let text = rendered(&scene);
        assert!(text.contains("looking at it now"), "talk missing");
        assert!(text.contains("cargo test --workspace"), "shell missing");
        assert!(text.contains("return a + b"), "code missing");
        assert!(text.contains("laguna-s21"), "the model is named");
        assert!(text.contains("1,816 / 65,536"), "the budget is shown");
        assert!(text.contains("type here"), "the input line is shown");
    }

    /// The rule the whole layout rests on: a pane's text box is inside its
    /// panel, so text is wrapped and clipped to the same rectangle and can
    /// never reach a neighbour.
    #[test]
    fn no_text_box_escapes_the_window() {
        let state = busy_state();
        for (w, h) in [(1200.0, 800.0), (700.0, 420.0), (2560.0, 1440.0)] {
            let (scene, _) = scene_of(&state, w, h);
            for text in &scene.texts {
                assert!(text.at.x >= 0.0 && text.at.y >= 0.0, "{:?} at {w}x{h}", text.at);
                assert!(
                    text.at.x + text.at.w <= w + 0.01,
                    "{:?} runs past {w}",
                    text.at
                );
                assert!(
                    text.at.y + text.at.h <= h + 0.01,
                    "{:?} runs past {h}",
                    text.at
                );
                assert!(text.at.w >= 1.0 && text.at.h >= 1.0, "{:?}", text.at);
            }
        }
    }

    /// A code line is colored by its syntax; a shell line is not. The extension
    /// is the only signal, and it is the harness's, never the model's.
    #[test]
    fn the_code_pane_is_syntax_colored_and_the_others_are_not() {
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
        let (scene, layout) = scene_of(&state, 1200.0, 800.0);
        let skin = Skin::matrix();
        let code = scene
            .texts
            .iter()
            .find(|t| t.at.x == layout.code.inset(PAD).x && t.at.y == layout.code.inset(PAD).y)
            .expect("the code pane has a text box");
        let colors: Vec<Option<[u8; 4]>> = code.runs.iter().map(|r| r.color).collect();
        assert!(colors.contains(&Some(skin.string)), "the string is tinted");
        assert!(colors.contains(&Some(skin.comment)), "the comment is tinted");
    }

    /// Nothing outside the window: every rectangle is inside the surface, so a
    /// resize can never leave one drawing over the desktop.
    #[test]
    fn every_rectangle_is_inside_the_surface() {
        let state = busy_state();
        for (w, h) in [(1200.0, 800.0), (320.0, 240.0)] {
            let (scene, _) = scene_of(&state, w, h);
            assert!(!scene.rects.is_empty());
            for rect in &scene.rects {
                let [x, y, rw, rh] = rect.xywh();
                assert!(x >= 0.0 && y >= 0.0, "{rect:?} at {w}x{h}");
                assert!(x + rw <= w + 0.01 && y + rh <= h + 0.01, "{rect:?} at {w}x{h}");
            }
        }
    }

    /// A caret past the right edge is not drawn rather than drawn on top of the
    /// pane beside it.
    #[test]
    fn a_caret_past_the_edge_is_dropped() {
        let state = State::new();
        let layout = Layout::compute(600.0, 500.0);
        let skin = Skin::matrix();
        let count = |caret: usize| {
            build(&Frame {
                state: &state,
                skin: &skin,
                layout: &layout,
                focus: Stream::Talk,
                input: "",
                caret,
                column: 8.0,
                trouble: None,
            })
            .rects
            .len()
        };
        assert_eq!(count(0) - 1, count(10_000), "the caret rect is gone");
    }

    /// A failure to start the agent has to be visible in the window, because
    /// there is no terminal for it to be printed to.
    #[test]
    fn trouble_replaces_the_title_detail() {
        let state = State::new();
        let layout = Layout::compute(900.0, 600.0);
        let skin = Skin::matrix();
        let scene = build(&Frame {
            state: &state,
            skin: &skin,
            layout: &layout,
            focus: Stream::Talk,
            input: "",
            caret: 0,
            column: 8.0,
            trouble: Some("cannot start \"noob\": not found"),
        });
        let text = rendered(&scene);
        assert!(text.contains("cannot start"), "{text}");
    }
}
