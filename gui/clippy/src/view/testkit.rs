//! The scene-test rig: build a window, render it, read what was drawn.
//!
//! A child of `view`, so it reaches view's own privates, and `pub(crate)`, so
//! every box's tests can build a scene without borrowing another box's test
//! file to do it. `#[cfg(test)]` at the declaration: none of this ships.

#[allow(clippy::wildcard_imports)]
use super::*;
use crate::config::Config;
use crate::monitor::Monitor;
use crate::state::State;
use noob_draw::Rect;

pub(crate) struct Rendered {
    pub(crate) scene: Scene,
    pub(crate) layout: Layout,
    pub(crate) skin: Skin,
}
/// A shape at a chosen column and pane size, for the tests that measure a
/// block against the room its pane has.
pub(crate) fn sized_shape<'a>(dock: &'a Dock, column: f32, pane_column: f32) -> Shape<'a> {
    Shape {
        column,
        pane_column,
        ..shape(dock, &[])
    }
}

pub(crate) fn render(state: &State, w: f32, h: f32, dock: &Dock, files: &[&str]) -> Rendered {
    render_with(state, w, h, dock, files, &Monitor::new(), None)
}

/// The same window with the pointer somewhere in it, for the states a widget
/// only wears under the hand.
pub(crate) fn render_hovered(
    state: &State,
    w: f32,
    h: f32,
    dock: &Dock,
    files: &[&str],
    cursor: (f32, f32),
) -> Rendered {
    let shape = shape(dock, files);
    let layout = Layout::compute(w, h, &shape);
    let skin = Skin::from(&Config::default());
    let monitor = Monitor::new();
    let scene = build(&Frame {
        state,
        scrolls: &crate::scroll::Scrolls::default(),
        file_scroll: 0,
        monitor: &monitor,
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
        cursor,
        selection: None,
        menu: None,
        picker: None,
        settings: None,
    });
    Rendered { scene, layout, skin }
}
pub(crate) fn render_with(
    state: &State,
    w: f32,
    h: f32,
    dock: &Dock,
    files: &[&str],
    monitor: &Monitor,
    drag: Option<Drag>,
) -> Rendered {
    render_scrolled(
        state,
        &crate::scroll::Scrolls::default(),
        w,
        h,
        dock,
        files,
        monitor,
        drag,
    )
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_scrolled(
    state: &State,
    scrolls: &crate::scroll::Scrolls,
    w: f32,
    h: f32,
    dock: &Dock,
    files: &[&str],
    monitor: &Monitor,
    drag: Option<Drag>,
) -> Rendered {
    render_skinned(
        state,
        scrolls,
        w,
        h,
        dock,
        files,
        monitor,
        drag,
        Skin::from(&Config::default()),
    )
}
/// The same scene under a palette of the caller's choosing, for the tests
/// that render one of the other themes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_skinned(
    state: &State,
    scrolls: &crate::scroll::Scrolls,
    w: f32,
    h: f32,
    dock: &Dock,
    files: &[&str],
    monitor: &Monitor,
    drag: Option<Drag>,
    skin: Skin,
) -> Rendered {
    let shape = shape(dock, files);
    let layout = Layout::compute(w, h, &shape);
    let scene = build(&Frame {
        state,
        scrolls,
        file_scroll: 0,
        monitor,
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
        drag,
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
pub(crate) fn shape<'a>(dock: &'a Dock, files: &[&str]) -> Shape<'a> {
    scrolled_shape(dock, files, 0)
}
/// The same, with the explorer list scrolled `first` rows down.
pub(crate) fn scrolled_shape<'a>(dock: &'a Dock, files: &[&str], first: usize) -> Shape<'a> {
    Shape {
        shaded: false,
        dock,
        menu: None,
        picker: None,
        settings: None,
        file_labels: files.iter().map(|f| f.to_string()).collect(),
        file_first: first,
        agent_tab: None,
        column: 8.0,
        menu_column: 7.0,
        pane_size: 13.0,
        pane_column: 8.0,
        input_h: INPUT_H,
        left_width: [crate::config::LEFT_WIDTH; 2],
        top_height: [crate::config::TOP_HEIGHT; 2],
        settings_rail: crate::config::SETTINGS_RAIL,
        popup: None,
    }
}
/// The same with both halves of the grid breaking in the same place, which
/// is the window nobody has dragged one half away from the other.
pub(crate) fn split_shape(dock: &Dock, left_width: f32, top_height: f32) -> Shape<'_> {
    halves_shape(dock, [left_width; 2], [top_height; 2])
}
/// The same with each half put where the test wants it.
pub(crate) fn halves_shape(dock: &Dock, left_width: [f32; 2], top_height: [f32; 2]) -> Shape<'_> {
    Shape {
        left_width,
        top_height,
        ..shape(dock, &[])
    }
}
pub(crate) fn busy_state() -> State {
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
/// A window whose every list is longer than any pane can hold: forty todos,
/// twelve children with news each, and thirty calls that failed.
pub(crate) fn crowded_state() -> State {
    let mut state = busy_state();
    let todos: Vec<serde_json::Value> = (0..40)
        .map(|i| serde_json::json!({"content": format!("step {i:02}"), "status": "pending"}))
        .collect();
    state.apply(noob_proto::Event::ToolStart {
        call_id: "plan-2".into(),
        name: "plan".into(),
        brief: "40 items".into(),
        args: serde_json::json!({"todos": todos}),
    });
    for i in 0..24 {
        state.apply(noob_proto::Event::AgentSpawn {
            agent_id: format!("kid-{i:02}"),
            prompt: format!("child {i:02} is reading"),
            tools: "read".into(),
        });
        state.apply(noob_proto::Event::AgentOutput {
            agent_id: format!("kid-{i:02}"),
            line: format!("news {i:02}"),
        });
    }
    for i in 0..30 {
        let id = format!("bad-{i:02}");
        state.apply(noob_proto::Event::ToolStart {
            call_id: id.clone(),
            name: "bash".into(),
            brief: format!("call {i:02}"),
            args: serde_json::json!({"cmd": "no"}),
        });
        state.apply(noob_proto::Event::ToolEnd {
            call_id: id,
            summary: "no".into(),
            elapsed_ms: 1,
            error: Some(noob_proto::ToolError {
                kind: "denied".into(),
                code: None,
                message: format!("boom {i:02}"),
                detail: None,
                remedy: None,
            }),
        });
    }
    state
}
/// A prompt holding `text` with the caret at `at`.
pub(crate) fn typed_prompt(text: &str, at: usize) -> crate::prompt::Prompt {
    let mut prompt = crate::prompt::Prompt::default();
    prompt.insert(text);
    prompt.place(at);
    prompt
}
/// A monitor that has read this state twice, which is what the two token
/// panes need before they report a rate.
pub(crate) fn sampled(state: &State) -> Monitor {
    let mut monitor = Monitor::new();
    monitor.sample(state);
    monitor.sample(state);
    monitor
}
/// An agent with one of everything, so every section has rows of its own to
/// draw rather than a note saying it is empty.
pub(crate) fn an_agent() -> crate::agent::Agent {
    crate::agent::Agent {
        env_path: Some(std::path::PathBuf::from("/home/hec/.config/noob/.env")),
        env_exists: true,
        env: vec![
            (
                String::from(crate::agent::ENDPOINT),
                String::from("http://localhost:8080/v1"),
            ),
            (String::from("NOOB_CTX"), String::from("262144")),
        ],
        skills_at: Some(std::path::PathBuf::from("/home/hec/.config/noob/skills")),
        skills: vec![crate::agent::Skill {
            dir: String::from("coding"),
            name: String::from("coding"),
            about: String::from("Changing code that already exists."),
            repo: Some(String::from("https://github.com/someone/coding")),
            path: std::path::PathBuf::from("/home/hec/.config/noob/skills/coding"),
            on: true,
            doc: vec![
                String::from("# Changing code"),
                String::new(),
                String::from("Read the file before writing it."),
            ],
        }],
        // One configured server, so the section that lists them has an
        // entry the way the fixture's skills directory has a skill.
        mcp: crate::agent::Mcp {
            global: Some(std::path::PathBuf::from("/home/hec/.config/noob/mcp.json")),
            project: None,
            any_file: true,
            servers: vec![crate::agent::Server {
                name: String::from("docs"),
                how: String::from("http://localhost:9000/mcp"),
                project: false,
                on: true,
                entry: String::from("{ \"url\": \"http://localhost:9000/mcp\" }"),
            }],
            trouble: Vec::new(),
        },
        // Where the global AGENTS.md would go, with nothing in it: the
        // machine this fixture stands for has never written one, so the
        // block shows the shipped prompt.
        instructions: crate::agent::Instructions {
            path: Some(std::path::PathBuf::from("/home/hec/.config/noob/AGENTS.md")),
            body: Vec::new(),
            capped: false,
        },
        sessions: crate::sessions::Listing {
            sessions: vec![crate::sessions::Saved {
                id: String::from("abc"),
                when: std::time::SystemTime::UNIX_EPOCH,
                workspace: Some(std::path::PathBuf::from("/home/hec/workspace/noob-cli")),
                gone: false,
                bytes: 4_096,
                context: None,
                opening: String::from("rebuild the settings panel"),
            }],
            skipped: Vec::new(),
        },
        ..crate::agent::Agent::default()
    }
}
pub(crate) fn text_of(scene: &Scene) -> String {
    scene
        .texts
        .iter()
        .flat_map(|t| t.runs.iter().map(|r| r.text.as_str()))
        .collect()
}
pub(crate) fn middle(panel: Panel) -> (f32, f32) {
    (panel.x + panel.w * 0.5, panel.y + panel.h * 0.5)
}
/// Whether a rectangle of this colour is drawn exactly over `box_`, at
/// `height` from its top.
pub(crate) fn covered(out: &Rendered, box_: Panel, height: f32, want: [f32; 4]) -> bool {
    out.scene.rects.iter().any(|rect| {
        let [x, y, w, h] = rect.xywh();
        (x - box_.x).abs() < 0.01
            && (y - box_.y).abs() < 0.01
            && (w - box_.w).abs() < 0.01
            && (h - height).abs() < 0.01
            && rect.rgba() == want
    })
}
/// The rectangle of this colour drawn at the top-left of `box_`, whatever
/// its width. What an accent line stopping short of the cut needs, since
/// [`covered`] insists on the full width.
pub(crate) fn topped(out: &Rendered, box_: Panel, height: f32, want: [f32; 4]) -> Option<Rect> {
    out.scene
        .rects
        .iter()
        .find(|rect| {
            let [x, y, _, h] = rect.xywh();
            (x - box_.x).abs() < 0.01
                && (y - box_.y).abs() < 0.01
                && (h - height).abs() < 0.01
                && rect.rgba() == want
        })
        .copied()
}
/// Every cell that has tabs in it, which is not every cell of the grid: the
/// window opens with the one under the conversation empty, and an empty cell
/// has no strip and no body because its room went to its neighbour.
pub(crate) fn occupied(dock: &Dock) -> Vec<Space> {
    Space::ALL
        .into_iter()
        .filter(|space| !dock.slot(*space).is_empty())
        .collect()
}
/// How wide and how tall a space is, strip and body together.
pub(crate) fn box_of(layout: &Layout, space: Space) -> (f32, f32) {
    let placed = layout.placed(space);
    (
        placed.strip.w,
        placed.body.y + placed.body.h - placed.strip.y,
    )
}
/// Everything drawn on one line of the panel, as one string.
pub(crate) fn line_of(out: &Rendered, x: f32, y: f32) -> String {
    out.scene
        .texts
        .iter()
        .filter(|text| (text.at.x - x).abs() < 0.51 && (text.at.y - y).abs() < 0.51)
        .flat_map(|text| text.runs.iter())
        .map(|run| run.text.as_str())
        .collect()
}
/// Every scrollbar the panel drew: the tracks, and the thumbs that stand in
/// them. Matched on the two colours nothing else in this window is painted
/// in.
pub(crate) fn bars_of(out: &Rendered) -> (Vec<Panel>, Vec<Panel>) {
    let of = |want: [f32; 4]| -> Vec<Panel> {
        out.scene
            .rects
            .iter()
            .filter(|rect| rect.rgba() == want)
            .map(|rect| {
                let [x, y, w, h] = rect.xywh();
                Panel::new(x, y, w, h)
            })
            .collect()
    };
    (of(out.skin.scroll_track), of(out.skin.scroll_thumb))
}
/// The scrollbar drawn in one space: its track and its thumb, or nothing when
/// the pane's content fits and it drew no bar.
pub(crate) fn bar_in(out: &Rendered, space: Space) -> Option<([f32; 4], [f32; 4])> {
    let body = out.layout.placed(space).body;
    let of = |want: [f32; 4]| {
        out.scene
            .rects
            .iter()
            .find(|r| r.rgba() == want && body.contains(r.xywh()[0], r.xywh()[1]))
            .map(|r| r.xywh())
    };
    Some((of(out.skin.scroll_track)?, of(out.skin.scroll_thumb)?))
}
pub(crate) fn inside(rect: Rect, box_: Panel) -> bool {
    let [x, y, w, h] = rect.xywh();
    x >= box_.x - 0.01
        && y >= box_.y - 0.01
        && x + w <= box_.x + box_.w + 0.01
        && y + h <= box_.y + box_.h + 0.01
}
/// Whether the whole of `inner` is inside `outer`.
pub(crate) fn within(inner: Panel, outer: Panel) -> bool {
    inner.x >= outer.x - 0.01
        && inner.y >= outer.y - 0.01
        && inner.x + inner.w <= outer.x + outer.w + 0.01
        && inner.y + inner.h <= outer.y + outer.h + 0.01
}
/// Where a view is showing and how tall its content is there, taken from the
/// pane's own extent so a test drives the arithmetic the wheel drives.
pub(crate) fn measured(
    state: &State,
    w: f32,
    h: f32,
    dock: &Dock,
    monitor: &Monitor,
    view: View,
) -> (Space, Vec<usize>, usize) {
    let space = Space::ALL
        .into_iter()
        .find(|space| dock.slot(*space).active() == Some(view))
        .expect("the view is showing somewhere");
    let shape = shape(dock, &[]);
    let layout = Layout::compute(w, h, &shape);
    let skin = Skin::from(&Config::default());
    let frame = Frame {
        state,
        scrolls: &crate::scroll::Scrolls::default(),
        file_scroll: 0,
        monitor,
        dock,
        skin: &skin,
        layout: &layout,
        prompt: &crate::prompt::Prompt::default(),
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
    };
    let (heights, rows) = scroll_extent(&frame, view, layout.placed(space).body)
        .expect("the view reports an extent");
    (space, heights, rows)
}
/// Every dot drawn in one space, lit or unlit, found by fill: a dot is a few
/// pixels square, which no size filter can tell from a hairline.
pub(crate) fn dots_in(out: &Rendered, space: Space) -> Vec<[f32; 4]> {
    let body = out.layout.placed(space).body;
    let hues: Vec<[f32; 4]> = out
        .skin
        .gauges
        .iter()
        .chain(out.skin.gauges_unlit.iter())
        .copied()
        .collect();
    out.scene
        .rects
        .iter()
        .filter(|r| hues.contains(&r.rgba()) && body.contains(r.xywh()[0], r.xywh()[1]))
        .map(|r| r.xywh())
        .collect()
}
/// Every dot of the orb in a scene.
///
/// A rectangle a few pixels across inside the title strip is one. It used to
/// be "a rectangle in the strip with a corner radius", which stopped finding
/// the resting orb the day its dots became squares; size is what both states
/// share. Nothing else drawn up there is small: the strip's own fill and the
/// context gauge run the width of the window, and a window button is thirty
/// pixels wide.
pub(crate) fn discs_of(scene: &Scene) -> Vec<&Rect> {
    scene
        .rects
        .iter()
        .filter(|rect| {
            let [_, y, w, h] = rect.xywh();
            w <= 4.0 && y + h <= TITLE_H
        })
        .collect()
}

pub(crate) fn a_dock_showing(view: crate::dock::View) -> Dock {
    let mut dock = Dock::new();
    let space = dock
        .space_of(view)
        .unwrap_or_else(|| panic!("{view:?} is not in the arrangement"));
    dock.slot_mut(space).show(view);
    dock
}

/// One frame at a given moment on the orb's clock, with the orb settled at
/// whichever formation the state names.
pub(crate) fn render_at(state: &State, clock: f32) -> Rendered {
    render_moving(state, clock, None)
}

/// The same with the orb partway through the move between its two
/// formations, which is what the window draws for [`ORB_MORPH`] either side
/// of a turn.
pub(crate) fn render_moving(state: &State, clock: f32, orb_morph: Option<f32>) -> Rendered {
    let dock = Dock::new();
    let shape = shape(&dock, &[]);
    let layout = Layout::compute(1400.0, 900.0, &shape);
    let skin = Skin::from(&Config::default());
    let scene = build(&Frame {
        state,
        scrolls: &crate::scroll::Scrolls::default(),
        file_scroll: 0,
        monitor: &Monitor::new(),
        dock: &dock,
        skin: &skin,
        layout: &layout,
        prompt: &crate::prompt::Prompt::default(),
        column: 8.0,
        pane_column: 8.0,
        body_size: 14.0,
        pane_size: 13.0,
        clock,
        orb_morph,
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
/// Whether any text in this list has a glyph box overlapping the panel.
pub(crate) fn text_over(texts: &[Text], panel: Panel) -> bool {
    texts.iter().any(|text| {
        text.at.x < panel.x + panel.w
            && panel.x < text.at.x + text.at.w
            && text.at.y < panel.y + panel.h
            && panel.y < text.at.y + text.at.h
    })
}

/// How many one pixel bars are drawn in that box: two for a plus, one for
/// the minus an open folder carries.
pub(crate) fn bars_in(out: &Rendered, mark: Panel) -> usize {
    out.scene
        .rects
        .iter()
        .filter(|rect| {
            let [_, _, w, h] = rect.xywh();
            rect.extra()[3] == 0.0 && (w == 1.0 || h == 1.0) && inside(**rect, mark)
        })
        .count()
}

pub(crate) fn a_settings_panel(config: &Config) -> Settings {
    Settings::open(
        config,
        Some(std::path::Path::new("/home/hec/.config/noob/no0b.conf")),
        an_agent(),
    )
}

/// Everything drawn at this line, left to right, as one string.
pub(crate) fn line_at(out: &Rendered, y: f32) -> String {
    let mut texts: Vec<&noob_draw::Text> = out
        .scene
        .texts
        .iter()
        .filter(|text| (text.at.y - y).abs() < 0.01)
        .collect();
    texts.sort_by(|a, b| a.at.x.total_cmp(&b.at.x));
    texts
        .iter()
        .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
        .collect()
}

pub(crate) fn a_picker(inside: &[&str], recents: &[&str]) -> Picker {
    Picker::open(
        Box::new(crate::picker::Fixed(
            inside.iter().map(|s| s.to_string()).collect(),
        )),
        std::path::PathBuf::from("/home/hec"),
        recents.iter().map(std::path::PathBuf::from).collect(),
    )
}

/// left column, custom alone on the right.
pub(crate) fn option_name(side: crate::settings::Side, option: usize) -> &'static str {
    match side {
        crate::settings::Side::Left => crate::config::THEMES[option],
        _ => "custom",
    }
}

/// space a few, which is not a strip that overflows.
pub(crate) fn a_crowded_dock(space: Space) -> Dock {
    let mut dock = Dock::new();
    for view in View::ALL {
        // The conversation stays where it is, so the grid keeps its
        // columns: a window with one space in it gives that space the whole
        // width, and a strip that wide fits every tab there is.
        if view != View::Output && dock.space_of(view).is_some() {
            dock.move_view(view, space);
        }
    }
    dock.slot_mut(space).show_at(0);
    dock
}
