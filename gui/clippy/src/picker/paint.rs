//! The startup chooser painter.

use noob_draw::{Panel, Run, Scene, Text};

#[allow(unused_imports)]
use crate::design::{self, icons};
use crate::picker::places::{picker_indent, picker_mark_box};
use crate::picker::Row as PickerRow;
#[allow(clippy::wildcard_imports)]
use crate::picker::metrics::*;
use crate::widgets::files::ROW_ICON_COLUMNS;
#[allow(clippy::wildcard_imports)]
use crate::view::*;


/// The folder picker: the whole window until a folder is chosen.
///
/// One box in the middle of the surface, drawn with the same rectangles and the
/// same text as everything else here. No native dialog: a file chooser from the
/// desktop's toolkit would pull in dozens of crates and a portal at runtime, for
/// a window whose whole point is that it is one GPU surface.
pub(crate) fn folder_picker(scene: &mut Scene, frame: &Frame) {
    let Some(picker) = frame.picker else {
        return;
    };
    let (skin, layout) = (frame.skin, frame.layout);
    let box_ = layout.picker;
    if box_.w < 1.0 || box_.h < 1.0 {
        return;
    }
    scene.rect(panel_fill(box_, skin.panel));
    scene.rect(panel_edge(box_, skin.edge_focus));

    let size = frame.pane_size;
    let line = Text::line_for(size);
    let content = box_.inset(PAD);
    let cols = cols_of(content, frame.pane_column);
    let say = |scene: &mut Scene, runs: Vec<Run>, at: Panel, tint: [u8; 4]| {
        scene.text(Text::rich(runs, at, size, tint));
    };

    // The three buttons, on the row above the writing: the two that choose the
    // list at the left, the one that opens the row the cursor is on at the right
    // limit of the box.
    //
    // The pair carries a third face on top of the idle and the hot one every
    // button here has, and the one whose list is showing wears it: the band the
    // chosen row is drawn in, written over in the same dark ink. Two buttons
    // drawn identically are two buttons that do not say which list is in front
    // of you, and that answer used to be carried by the heading alone.
    let on_sessions = picker.on_sessions();
    for (panel, hit, icon, label) in [
        (
            layout.picker_folders,
            Hit::PickerFolders,
            icons::FOLDER,
            PICKER_FOLDERS_LABEL,
        ),
        (
            layout.picker_sessions,
            Hit::PickerSessions,
            icons::RECENT,
            PICKER_SESSIONS_LABEL,
        ),
        (
            layout.picker_open,
            Hit::PickerOpen,
            icons::CONFIRM,
            PICKER_OPEN_LABEL,
        ),
    ] {
        if panel.w < 1.0 || panel.h < 1.0 {
            continue;
        }
        let showing = match hit {
            Hit::PickerFolders => !on_sessions,
            Hit::PickerSessions => on_sessions,
            _ => false,
        };
        // The showing mode keeps its band under the pointer. Pressing it does
        // nothing, so lighting it would promise a change that never comes.
        let (face, ink) = match (showing, frame.hot == Some(hit)) {
            (true, _) => (skin.picked, skin.picked_ink),
            (false, true) => (skin.button_hot, skin.bright),
            (false, false) => (skin.button, skin.bright),
        };
        scene.rect(panel_fill(panel, face));
        scene.rect(panel_edge(panel, skin.edge_focus));
        say(
            scene,
            vec![
                Run::icon(icon.to_string(), ink),
                Run::tinted(format!(" {label}"), ink),
            ],
            Panel::new(
                panel.x + frame.pane_column,
                panel.y + PICKER_OPEN_PAD,
                (panel.w - frame.pane_column).max(1.0),
                line,
            ),
            ink,
        );
    }
    // The title, one string in both lists, and what the session list says about
    // itself beside it: how many there are, and how many files in the directory
    // could not be described.
    let writing = layout.picker_open.y + layout.picker_open.h + GAP;
    let mut head = vec![Run::tinted(PICKER_TITLE, skin.bright)];
    if let Some(note) = picker.note() {
        let room = cols.saturating_sub(PICKER_TITLE.chars().count() + 2);
        head.push(Run::tinted(format!("  {}", clip(note, room)), skin.dim));
    }
    say(
        scene,
        head,
        Panel::new(content.x, writing, content.w, line),
        skin.bright,
    );
    // The folder being listed, in full. The rows under it are names, so this is
    // the only thing on screen saying where in the tree they are, and with the
    // sessions showing it is the folder a session that never noted one would be
    // resumed in.
    say(
        scene,
        vec![Run::tinted(
            clip(&picker.at().display().to_string(), cols),
            skin.body,
        )],
        Panel::new(content.x, writing + line, content.w, line),
        skin.body,
    );
    // What has been typed, why the list is empty when it is empty for a reason,
    // or why the last press did nothing. A folder with no permission looks
    // exactly like an empty folder otherwise, and a button that silently does
    // not work looks exactly like a button that is broken.
    //
    // In a box of its own, with the surface the prompt is drawn on, the hairline
    // every panel here carries and the same cut corner. It was a line of text
    // with a funnel in front of it, which said the list had been narrowed and
    // never said that this is the thing you type into.
    let field = layout.picker_filter;
    let mut runs = vec![Run::icon(icons::SEARCH.to_string(), skin.dim), Run::plain(" ")];
    let room = cols.saturating_sub(ROW_ICON_COLUMNS + 2);
    let tint = match (picker.refused().or(picker.trouble()), picker.filter()) {
        (Some(why), _) => {
            runs.push(Run::tinted(clip(why, room), skin.bad));
            skin.bad
        }
        (None, "") => {
            runs.push(Run::tinted("type to narrow the list", skin.dim));
            skin.dim
        }
        (None, typed) => {
            runs.push(Run::tinted(clip(typed, room), skin.bright));
            skin.bright
        }
    };
    if field.w >= 1.0 && field.h >= 1.0 {
        scene.rect(panel_fill(field, skin.input));
        scene.rect(panel_edge(field, skin.edge_focus));
        say(
            scene,
            runs,
            Panel::new(
                field.x + PAD,
                field.y + PICKER_FIELD_PAD,
                (field.w - 2.0 * PAD).max(1.0),
                line,
            ),
            tint,
        );
    }

    // The row that names the columns, on the line the layout kept above the
    // list. Only the sessions are a table: a folder list is one column of names
    // and a header over it would be a word explaining the obvious.
    if picker.on_sessions() {
        let head_row = Panel::new(
            layout.picker_list.x,
            layout.picker_list.y - line,
            layout.picker_list.w,
            line,
        );
        let (at, room) = session_table(head_row, frame.pane_column);
        let names: Vec<String> = SESSION_COLUMNS
            .iter()
            .map(|(name, _)| String::from(*name))
            .chain(std::iter::once(String::from(SESSION_OPENING)))
            .collect();
        say(
            scene,
            vec![Run::tinted(session_line(&names, room), skin.dim)],
            Panel::new(at, head_row.y, (head_row.w - (at - head_row.x)).max(1.0), line),
            skin.dim,
        );
    }

    let list_cols = cols_of(layout.picker_list, frame.pane_column);
    for (index, row) in &layout.picker_rows {
        let Some(entry) = picker.row(*index) else {
            continue;
        };
        let on = *index == picker.cursor();
        if on {
            // Filled solid in the good colour, and written over in the darkest
            // ink the palette has. The quiet band the file explorer marks its
            // open row with was not enough here: the picker is a list of forty
            // folders where the only question is which one Enter opens.
            scene.rect(row.fill(skin.picked));
        }
        // Typing dims what it did not match instead of taking it away, so the
        // list you were reading is still the list in front of you. The answer
        // comes from the model, which is the same answer the arrow keys walk by:
        // a row cannot be dim here and bright to the keyboard.
        // A session whose folder has been deleted is drawn the way an
        // unreadable folder is, because it is the same thing: a row that is
        // there to be seen and cannot be opened.
        let dead = matches!(entry, PickerRow::Session(saved) if saved.gone);
        let tint = match (on, picker.matched(entry), entry) {
            (true, _, _) => skin.picked_ink,
            (false, false, _) => skin.dim,
            (false, true, PickerRow::Locked { .. }) => skin.bad,
            (false, true, _) if dead => skin.bad,
            (false, true, _) => skin.body,
        };
        let icon = match entry {
            PickerRow::Here => icons::FOLDER_OPEN,
            PickerRow::Up => icons::UP,
            PickerRow::Recent(_) => icons::RECENT,
            PickerRow::Folder { .. } => icons::FOLDER,
            PickerRow::Locked { .. } => icons::LOCKED,
            // The clock the remembered folders carry, since a saved session is
            // the same idea: something from before. The lock when it cannot be
            // opened, which is what that glyph says everywhere else here.
            PickerRow::Session(saved) => match saved.gone {
                true => icons::LOCKED,
                false => icons::RECENT,
            },
        };
        // The mark that opens and shuts the folder, drawn inside the region that
        // answers for pressing it: a hairline box with a plus in it, or with the
        // plus's upright taken away once the folder is open.
        //
        // Rectangles rather than a glyph. It was Font Awesome's filled
        // plus-square at the size of the row's text, which put a solid block at
        // the front of every folder in the list: the biggest, heaviest thing on
        // a row whose point is the folder's name. Nothing is filled here and the
        // box is well under the height of the row.
        let (indent, wide) = picker_indent(entry.depth(), frame.pane_column, list_cols);
        if let Some(open) = entry.open() {
            let hot = frame.hot == Some(Hit::PickerMark(*index));
            // Green, and the panel colour instead on the row the cursor is on,
            // where the band behind it is already that green. Under the pointer
            // it keeps its colour and doubles its weight: a second colour for a
            // hover is a mark that means two things.
            let edge = match on {
                true => skin.mark_on_band,
                false => skin.mark_edge,
            };
            let weight = match hot {
                true => 2.0,
                false => 1.0,
            };
            let square = picker_mark_box(Panel::new(row.x + indent, row.y, wide, line));
            scene.rect(square.outline(edge, weight));
            // The bars sit two pixels inside the box on every side, so the plus
            // never touches the edge round it, and both are one pixel: the box
            // is nine across and a thicker bar closes the gap up.
            let middle = ((square.w - 1.0) * 0.5).floor();
            let arm = (square.w - 4.0).max(1.0);
            scene.rect(Panel::new(square.x + 2.0, square.y + middle, arm, 1.0).fill(edge));
            if !open {
                scene.rect(Panel::new(square.x + middle, square.y + 2.0, 1.0, arm).fill(edge));
            }
        }
        let start = indent + wide;
        // A session is a table row: the glyph in the gutter, then the cells, at
        // the one x the header above the list is written at. Its own text rather
        // than one run after the glyph, because the header has no glyph and the
        // two have to line up to the pixel.
        if let PickerRow::Session(saved) = entry {
            let (at, room) = session_table(*row, frame.pane_column);
            say(
                scene,
                vec![Run::icon(icon.to_string(), tint)],
                Panel::new(row.x + start, row.y, (row.w - start).max(1.0), line),
                tint,
            );
            say(
                scene,
                vec![Run::tinted(
                    session_line(&picker.session_cells(saved), room),
                    tint,
                )],
                Panel::new(at, row.y, (row.x + row.w - at).max(1.0), line),
                tint,
            );
            continue;
        }
        let room = cols
            .saturating_sub(ROW_ICON_COLUMNS + 1 + (start / frame.pane_column.max(1.0)) as usize)
            .max(1);
        say(
            scene,
            vec![
                Run::icon(icon.to_string(), tint),
                Run::tinted(format!(" {}", clip(&picker.label(entry), room)), tint),
            ],
            Panel::new(row.x + start, row.y, (row.w - start).max(1.0), line),
            tint,
        );
    }
    scrollbar(
        scene,
        skin,
        layout.picker,
        picker.thumb(layout.picker_capacity(size)),
    );

    // The keys, spelled out, on the last line of the box. Nothing else in this
    // window needs them written down, but this is the first thing a new install
    // shows and it is the one place where there is no pane to experiment in.
    //
    // It is the whole of the foot now that the buttons are in the head, so it is
    // placed off the bottom of the box, which is where [`picker_foot`] says the
    // one line it keeps down there is.
    say(
        scene,
        vec![Run::tinted(
            clip(
                "enter opens \u{2022} right walks in \u{2022} left goes out \u{2022} esc quits",
                cols,
            ),
            skin.dim,
        )],
        Panel::new(
            content.x,
            (content.y + content.h - line).max(content.y),
            content.w,
            line,
        ),
        skin.dim,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::wildcard_imports)]
    use crate::view::testkit::*;
    use crate::config::Config;
    use crate::dock::Dock;
    use crate::menu::Menu;
    use crate::monitor::Monitor;
    use crate::picker::{Picker, Row as PickerRow};
    use crate::dock::Space;
    use crate::style::skin::Skin;
    use noob_draw::Rect;
    use crate::state::State;

    /// The window with the folder picker up, laid out and drawn off one shape,
    /// which is what makes a row land where it is drawn.
    fn render_picker(picker: &Picker, w: f32, h: f32, hot: Option<Hit>) -> Rendered {
        let dock = Dock::new();
        let state = State::new();
        let mut shape = shape(&dock, &[]);
        shape.picker = Some(picker);
        let layout = Layout::compute(w, h, &shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state: &state,
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
            clock: 0.0,
            orb_morph: None,
            drag: None,
            hot,
            trouble: None,
            esc_armed: false,
            popup_scroll: 0,
            cursor: (-100.0, -100.0),
            selection: None,
            menu: None,
            picker: Some(picker),
            settings: None,
        });
        Rendered {
            scene,
            layout,
            skin,
        }
    }
    /// The picker rendered with a menu open over it.
    fn render_picker_menu(picker: &Picker, menu: &Menu, w: f32, h: f32) -> Rendered {
        let dock = Dock::new();
        let state = State::new();
        let mut shape = shape(&dock, &[]);
        shape.picker = Some(picker);
        shape.menu = Some(menu);
        let layout = Layout::compute(w, h, &shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state: &state,
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
            clock: 0.0,
            orb_morph: None,
            drag: None,
            hot: None,
            trouble: None,
            esc_armed: false,
            popup_scroll: 0,
            cursor: (-100.0, -100.0),
            selection: None,
            menu: Some(menu),
            picker: Some(picker),
            settings: None,
        });
        Rendered {
            scene,
            layout,
            skin,
        }
    }
    /// The picker with the two sessions the swap test uses already showing.
    fn a_session_picker() -> Picker {
        let mut picker = a_picker(&["gui"], &[]);
        let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        picker.show_sessions_at(
            crate::sessions::Listing {
                sessions: vec![
                    a_saved("live", Some("/home/hec"), "carry this on", 600),
                    a_saved("older", Some("/home/hec"), "the one before", 86_400),
                ],
                skipped: Vec::new(),
            },
            now,
        );
        picker
    }
    /// One saved session, as the reader would have described it.
    fn a_saved(
        id: &str,
        at: Option<&str>,
        said: &str,
        ago: u64,
    ) -> crate::sessions::Saved {
        crate::sessions::Saved {
            id: String::from(id),
            when: std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(1_000_000_000 - ago),
            workspace: at.map(std::path::PathBuf::from),
            gone: false,
            bytes: 12_000,
            context: None,
            opening: String::from(said),
        }
    }
    /// What the picker's row at `index` says, which is the folder's name for a
    /// row that is one.
    fn said(picker: &Picker, index: usize) -> String {
        picker
            .row(index)
            .map(|row| picker.label(row))
            .unwrap_or_default()
    }
    /// The hairline box the picker draws inside a mark's region, if it drew one.
    ///
    /// Found by shape rather than by position: it is the only stroked rectangle
    /// that fits inside the region, and everything else in there is a solid bar.
    fn outline_of(out: &Rendered, mark: Panel) -> Option<Rect> {
        out.scene
            .rects
            .iter()
            .find(|rect| rect.extra()[3] > 0.0 && inside(**rect, mark))
            .copied()
    }
    /// With no folder chosen there is nothing to arrange panes around and
    /// nothing to type at, so the picker is the window: no spaces, no prompt,
    /// and it answers for every point below the title strip.
    #[test]
    fn the_window_opens_on_the_picker_instead_of_a_workspace() {
        let picker = a_picker(&["gui", "crates", "docs"], &["/home/hec/workspace/noob-cli"]);
        let out = render_picker(&picker, 1205.0, 791.0, None);
        let layout = &out.layout;
        assert!(layout.picking);
        for space in Space::ALL {
            assert!(layout.placed(space).tabs.is_empty(), "{space:?}");
            assert_eq!(layout.placed(space).body.w, 0.0, "{space:?}");
            assert_eq!(layout.placed(space).arrow_left.w, 0.0, "{space:?}");
            assert_eq!(layout.placed(space).arrow_right.w, 0.0, "{space:?}");
        }
        assert_eq!(layout.input.w, 0.0, "there is nothing to type at yet");
        assert_eq!(layout.cell(600.0, 400.0, 13.0, 8.0), None);

        // Inside the surface, under the title strip, and centred.
        let box_ = layout.picker;
        assert!(box_.y >= TITLE_H, "it starts below the strip: {box_:?}");
        assert!(box_.y + box_.h <= 791.0 && box_.x + box_.w <= 1205.0, "{box_:?}");
        let left = box_.x;
        let right = 1205.0 - (box_.x + box_.w);
        assert!((left - right).abs() <= 1.0, "off centre: {left} then {right}");

        // Every row of the list is hit where it is drawn, and the button and the
        // margin answer for themselves.
        assert_eq!(layout.picker_rows.len(), picker.rows().len());
        for (index, row) in &layout.picker_rows {
            let (x, y) = middle(*row);
            assert_eq!(layout.hit(x, y), Some(Hit::PickerRow(*index)));
            assert!(layout.picker_list.contains(x, y), "row {index} is outside the list");
        }
        let (x, y) = middle(layout.picker_open);
        assert_eq!(layout.hit(x, y), Some(Hit::PickerOpen));
        assert_eq!(
            layout.hit(box_.x + box_.w - 2.0, box_.y + 2.0),
            Some(Hit::Picker),
            "its own margin swallows a press rather than passing it on"
        );
        assert_eq!(layout.hit(2.0, 400.0), None, "and outside it there is nothing");
        // The strip is still the strip: the window can be moved and closed
        // before a folder is chosen.
        assert_eq!(layout.hit(400.0, 8.0), Some(Hit::TitleBar));
        assert_eq!(layout.hit(middle(layout.close).0, middle(layout.close).1), Some(Hit::Close));

        // What it says: the heading, the folder being listed, the remembered
        // folder, the names inside, and the button.
        let text = text_of(&out.scene);
        for wanted in [
            PICKER_TITLE,
            "/home/hec",
            "/home/hec/workspace/noob-cli",
            "gui",
            "crates",
            "..",
            PICKER_OPEN_LABEL,
            // Both ends of the line of keys: it is clipped to the width of the
            // box, so one key too many silently costs the last one, and the
            // last one is how the window is closed.
            "enter opens",
            "esc quits",
        ] {
            assert!(text.contains(wanted), "{wanted:?} is not on screen: {text}");
        }

        // The row the cursor is on is a filled accent band with the dark ink
        // written over it. Item 4: the quiet band the file explorer marks its
        // open row with said almost nothing here.
        let (index, cursor_row) = layout.picker_rows[0];
        assert_eq!(index, picker.cursor());
        assert!(
            covered(&out, cursor_row, cursor_row.h, out.skin.picked),
            "the cursor's row has no band"
        );
        // And no other row is banded, or every row would read as the one. Only
        // a full width fill counts: `skin.mark_edge` is the same accent and the
        // hairline box in front of every folder is not a band, and neither is
        // an outline stroked in the focus colour.
        let banded = out
            .scene
            .rects
            .iter()
            .filter(|rect| {
                rect.extra()[3] == 0.0
                    && rect.rgba() == out.skin.picked
                    && rect.xywh()[2] >= cursor_row.w - 0.01
            })
            .count();
        assert_eq!(banded, 1, "more than one row is banded");
        // Everything written on that band is the dark ink. Accent text on the
        // accent band is the one thing the whole palette is built to avoid.
        let ink: Vec<Option<[u8; 4]>> = out
            .scene
            .texts
            .iter()
            .filter(|text| (text.at.y - cursor_row.y).abs() < 0.01)
            .flat_map(|text| text.runs.iter().map(|run| run.color))
            .collect();
        assert!(!ink.is_empty(), "the row on the band says nothing");
        for tint in ink {
            assert_eq!(tint, Some(out.skin.picked_ink), "not the dark ink");
        }

        // Nothing hangs off the surface.
        for rect in &out.scene.rects {
            let [x, y, w, h] = rect.xywh();
            assert!(
                x >= -0.01 && y >= -0.01 && x + w <= 1205.01 && y + h <= 791.01,
                "{:?} is outside the window",
                rect.xywh()
            );
        }
    }
    /// Item E1: the button that opens the row the cursor is on sits at the right
    /// limit of the picker's head, says Open selected, carries the cut corner
    /// every panel in this window carries, sits on a surface of its own and
    /// lights up under the pointer.
    #[test]
    fn the_open_button_sits_at_the_right_limit_and_reads_as_a_button() {
        let picker = a_picker(&["gui"], &["/home/hec/workspace/noob-cli"]);
        let cold = render_picker(&picker, 1205.0, 791.0, None);
        let warm = render_picker(&picker, 1205.0, 791.0, Some(Hit::PickerOpen));
        let button = cold.layout.picker_open;

        // Its own surface idle, a stronger one hot, and neither of them the tab
        // fill it used to borrow.
        assert!(covered(&cold, button, button.h, cold.skin.button));
        assert!(covered(&warm, button, button.h, warm.skin.button_hot));
        assert!(!covered(&cold, button, button.h, cold.skin.tab_idle));
        assert!(cold.skin.button_hot[3] > cold.skin.button[3]);

        // The same 45 degree cut on the same corner as every panel, on the fill
        // and on the edge, or the fill pokes a square corner out of a cut one.
        let shaped: Vec<Rect> = cold
            .scene
            .rects
            .iter()
            .filter(|rect| {
                let [x, y, w, h] = rect.xywh();
                (x - button.x).abs() < 0.01
                    && (y - button.y).abs() < 0.01
                    && (w - button.w).abs() < 0.01
                    && (h - button.h).abs() < 0.01
            })
            .copied()
            .collect();
        assert_eq!(shaped.len(), 2, "a fill and an edge, and nothing else");
        for rect in &shaped {
            let [_, chamfer, corners, _] = rect.extra();
            assert_eq!(chamfer, CUT, "the button has no corner cut");
            assert_eq!(corners as u32, Rect::TOP_RIGHT);
        }
        assert!(
            shaped.iter().any(|rect| rect.extra()[3] > 0.0),
            "one of the two is the outline"
        );

        // At the right limit of the box's content, in its head, not at the foot
        // where it used to be beside the button that swapped the list.
        let box_ = cold.layout.picker;
        assert!(
            (button.x + button.w - (box_.x + box_.w - PAD)).abs() < 0.01,
            "{button:?} is not at the right limit of {box_:?}"
        );
        assert!(
            button.y < cold.layout.picker_filter.y,
            "{button:?} is not in the head"
        );

        // It says "Open selected" and nothing else. The folder it would open is
        // written above the list, and spelling it out here made the button as
        // wide as a path and a different width every time the cursor moved.
        let inside: String = warm
            .scene
            .texts
            .iter()
            .filter(|text| {
                text.at.x >= button.x - 0.01
                    && text.at.x + text.at.w <= button.x + button.w + 0.01
                    && text.at.y >= button.y - 0.01
                    && text.at.y + text.at.h <= button.y + button.h + 0.01
            })
            .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
            .collect();
        assert!(inside.contains(PICKER_OPEN_LABEL), "the button says {inside:?}");
        assert!(
            !inside.contains("/home/hec"),
            "the button still names a folder: {inside:?}"
        );

        // Taller than its text, which is what stops it reading as a line with a
        // box round it, and the hit region is the rectangle that was drawn.
        assert!(button.h > Text::line_for(13.0));
        let (x, y) = middle(button);
        assert_eq!(cold.layout.hit(x, y), Some(Hit::PickerOpen));
        assert_eq!(
            cold.layout.hit(button.x + button.w + 4.0, y),
            Some(Hit::Picker),
            "the gap to the button beside it is the box's own margin, not either button"
        );
    }
    /// Item E1: the picker's head is one title and three buttons. Folders and
    /// Sessions at the left choose which list is showing, and the one whose list
    /// is in front of you is filled in the band the chosen row wears, because two
    /// buttons drawn the same way say nothing about where you are.
    #[test]
    fn the_head_s_two_buttons_choose_the_list_and_say_which_one_is_showing() {
        let mut picker = a_picker(&["gui"], &[]);
        let cold = render_picker(&picker, 1205.0, 791.0, None);
        let open = cold.layout.picker_open;
        let (folders, sessions) = (cold.layout.picker_folders, cold.layout.picker_sessions);

        // Both there, the same size, side by side at the left of the head, on
        // the same row as Open and clear of it.
        assert!(folders.w > 1.0 && folders.h > 1.0, "there is no Folders button");
        assert!(sessions.w > 1.0 && sessions.h > 1.0, "there is no Sessions button");
        assert!(
            (folders.w - sessions.w).abs() < 0.01 && (folders.h - sessions.h).abs() < 0.01,
            "the pair is not one size: {folders:?} then {sessions:?}"
        );
        assert!((folders.y - open.y).abs() < 0.01 && (sessions.y - open.y).abs() < 0.01);
        assert!((folders.x - (cold.layout.picker.x + PAD)).abs() < 0.01, "{folders:?}");
        assert!(sessions.x >= folders.x + folders.w, "{folders:?} then {sessions:?}");
        assert!(open.x >= sessions.x + sessions.w, "{sessions:?} then {open:?}");

        // Each is its own target, and none of the three answers for another.
        let (fx, fy) = middle(folders);
        let (x, y) = middle(sessions);
        assert_eq!(cold.layout.hit(fx, fy), Some(Hit::PickerFolders));
        assert_eq!(cold.layout.hit(x, y), Some(Hit::PickerSessions));
        assert_eq!(
            cold.layout.hit(middle(open).0, middle(open).1),
            Some(Hit::PickerOpen)
        );

        // The folders are showing, so Folders wears the band and Sessions is a
        // plain button. The two fills are not the same colour, or the state
        // would be a state nobody can see.
        assert_ne!(cold.skin.picked, cold.skin.button);
        assert!(
            covered(&cold, folders, folders.h, cold.skin.picked),
            "the showing mode has no band"
        );
        assert!(
            covered(&cold, sessions, sessions.h, cold.skin.button),
            "the mode that is not showing wears the band"
        );
        // The band is a fill: the focus outline every one of these buttons
        // wears is the same accent, and an outline is not a band.
        assert!(!cold.scene.rects.iter().any(|rect| {
            let [x, y, w, h] = rect.xywh();
            rect.extra()[3] == 0.0
                && rect.rgba() == cold.skin.picked
                && (x - sessions.x).abs() < 0.01
                && (y - sessions.y).abs() < 0.01
                && (w - sessions.w).abs() < 0.01
                && (h - sessions.h).abs() < 0.01
        }));
        // And it is written in the ink that reads on that band.
        let ink: Vec<Option<[u8; 4]>> = cold
            .scene
            .texts
            .iter()
            .filter(|text| {
                text.at.x >= folders.x
                    && text.at.x < folders.x + folders.w
                    && text.at.y >= folders.y
                    && text.at.y < folders.y + folders.h
            })
            .flat_map(|text| text.runs.iter().map(|run| run.color))
            .collect();
        assert!(!ink.is_empty(), "the showing mode says nothing");
        for tint in ink {
            assert_eq!(tint, Some(cold.skin.picked_ink), "not the dark ink");
        }

        // The pointer lights the mode that is not showing, and nothing else.
        let warm = render_picker(&picker, 1205.0, 791.0, Some(Hit::PickerSessions));
        assert!(covered(&warm, sessions, sessions.h, warm.skin.button_hot));
        assert!(
            covered(&warm, open, open.h, warm.skin.button),
            "the pointer on one button must not light the other"
        );
        assert!(
            covered(&warm, folders, folders.h, warm.skin.picked),
            "the showing mode changed under a pointer that is not on it"
        );

        // One title, and both words are on screen at once: the head says what
        // the box is for, the pair says which list is in it.
        let text = text_of(&cold.scene);
        assert!(text.contains(PICKER_TITLE), "{text}");
        assert!(text.contains(PICKER_FOLDERS_LABEL), "{text}");
        assert!(text.contains(PICKER_SESSIONS_LABEL), "{text}");

        // Pressed, the same box lists the sessions instead: same rectangle, same
        // buttons in the same places, same title, and the band has moved.
        let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        let mut gone = a_saved("old", Some("/home/hec/deleted"), "the one before", 86_400);
        gone.gone = true;
        picker.show_sessions_at(
            crate::sessions::Listing {
                sessions: vec![
                    a_saved("live", Some("/home/hec"), "carry this on", 600),
                    gone,
                ],
                skipped: Vec::new(),
            },
            now,
        );
        let after = render_picker(&picker, 1205.0, 791.0, None);
        assert_eq!(
            (
                after.layout.picker,
                after.layout.picker_open,
                after.layout.picker_folders,
                after.layout.picker_sessions,
                after.layout.picker_filter,
            ),
            (cold.layout.picker, open, folders, sessions, cold.layout.picker_filter),
            "swapping the list moved the box"
        );
        assert_eq!(after.layout.picker_rows.len(), 2);
        assert!(
            covered(&after, sessions, sessions.h, after.skin.picked),
            "the sessions are showing and their button has no band"
        );
        assert!(
            covered(&after, folders, folders.h, after.skin.button),
            "the folder button kept the band after the list swapped"
        );
        let text = text_of(&after.scene);
        for wanted in [
            PICKER_TITLE,
            "2 saved sessions",
            "10m ago",
            "carry this on",
            "deleted (gone)",
            // Still written above the list, because it is the folder a session
            // that never noted one would be resumed in.
            "/home/hec",
            PICKER_FOLDERS_LABEL,
            PICKER_SESSIONS_LABEL,
        ] {
            assert!(text.contains(wanted), "{wanted:?} is not on screen: {text}");
        }
        // The title is the one string in both lists: neither of the two it
        // replaced is anywhere in the window.
        for gone in ["OPEN A FOLDER", "OPEN A SESSION"] {
            assert!(!text.contains(gone), "{gone:?} is still drawn");
            assert!(!text_of(&cold.scene).contains(gone), "{gone:?} is still drawn");
        }

        // The row that cannot be opened is written in the colour every other
        // thing that cannot be opened is written in.
        let (_, dead) = after.layout.picker_rows[1];
        let tints: Vec<[u8; 4]> = after
            .scene
            .texts
            .iter()
            .filter(|text| (text.at.y - dead.y).abs() < 0.01)
            .flat_map(|text| text.runs.iter().filter_map(|run| run.color))
            .collect();
        assert!(!tints.is_empty());
        assert!(
            tints.iter().all(|tint| *tint == after.skin.bad),
            "a session whose folder has gone reads like any other row"
        );

        // And all three go with the picker. A button left behind by a shape
        // change is a press that lands on something nobody can see.
        let dock = Dock::new();
        let panel = a_settings_panel(&Config::default());
        for (what, shape) in [
            ("shaded", Shape { shaded: true, ..shape(&dock, &[]) }),
            ("settings", Shape { settings: Some(&panel), ..shape(&dock, &[]) }),
        ] {
            let layout = Layout::compute(1205.0, 791.0, &shape);
            assert_eq!(layout.picker_folders.w, 0.0, "{what}");
            assert_eq!(layout.picker_sessions.w, 0.0, "{what}");
            assert_eq!(layout.picker_open.w, 0.0, "{what}");
            assert_ne!(layout.hit(fx, fy), Some(Hit::PickerFolders), "{what}");
            assert_ne!(layout.hit(x, y), Some(Hit::PickerSessions), "{what}");
        }
    }
    /// Item 3: the box does not change shape under the pointer. Walking from a
    /// folder with two entries into one with sixty used to resize and recentre
    /// the whole dialog, because its height came from the number of rows it was
    /// holding.
    #[test]
    fn the_picker_s_box_is_one_shape_whatever_the_folder_holds() {
        let short = a_picker(&["one", "two"], &[]);
        let long_names: Vec<String> = (0..60).map(|n| format!("dir{n:02}")).collect();
        let long = a_picker(
            &long_names.iter().map(String::as_str).collect::<Vec<&str>>(),
            &[],
        );
        for (w, h) in [(1205.0, 791.0), (2200.0, 1400.0), (680.0, 380.0)] {
            let a = render_picker(&short, w, h, None).layout;
            let b = render_picker(&long, w, h, None).layout;
            assert_eq!(
                (a.picker.x, a.picker.y, a.picker.w, a.picker.h),
                (b.picker.x, b.picker.y, b.picker.w, b.picker.h),
                "the box moved between two folders at {w}x{h}"
            );
            assert_eq!(
                (a.picker_list.y, a.picker_list.h),
                (b.picker_list.y, b.picker_list.h),
                "the list moved at {w}x{h}"
            );
            assert_eq!(
                (a.picker_open.x, a.picker_open.y, a.picker_open.w, a.picker_open.h),
                (b.picker_open.x, b.picker_open.y, b.picker_open.w, b.picker_open.h),
                "the button moved at {w}x{h}"
            );
            // The short folder simply leaves the bottom of its list empty, which
            // is the price of a dialog that stays put.
            assert_eq!(
                a.picker_rows.len(),
                4,
                "this folder, the way out, and the two folders in it"
            );
            assert_eq!(b.picker_rows.len(), a.picker_capacity(13.0).min(62));
            let (x, y) = middle(a.picker_open);
            assert_eq!(a.hit(x, y), Some(Hit::PickerOpen), "at {w}x{h}");
            assert_eq!(b.hit(x, y), Some(Hit::PickerOpen), "at {w}x{h}");
        }

        // And walking really does keep it still: the same picker, before and
        // after it lists a folder with a very different number of entries.
        let mut walking = Picker::open(
            Box::new(crate::picker::Fixed(
                long_names.iter().map(|s| s.to_string()).collect(),
            )),
            std::path::PathBuf::from("/home/hec"),
            Vec::new(),
        );
        let before = render_picker(&walking, 1205.0, 791.0, None).layout.picker;
        assert!(walking.step(true) && walking.walk_in());
        let after = render_picker(&walking, 1205.0, 791.0, None).layout.picker;
        assert_eq!(
            (before.x, before.y, before.w, before.h),
            (after.x, after.y, after.w, after.h)
        );
    }
    /// Item 5: typing dims the rows it did not match instead of taking them
    /// away, and the cursor only lands where the model says a match is.
    #[test]
    fn typing_in_the_picker_dims_rows_rather_than_dropping_them() {
        let mut picker = a_picker(&["gui", "crates", "docs"], &[]);
        let before = render_picker(&picker, 1205.0, 791.0, None);
        let rows = before.layout.picker_rows.len();
        assert!(picker.type_text("cra"));
        let after = render_picker(&picker, 1205.0, 791.0, None);
        assert_eq!(
            after.layout.picker_rows.len(),
            rows,
            "typing took rows out of the list"
        );

        // Every name is still on screen; the ones that did not match are drawn
        // in the dim tint rather than the body one.
        let text = text_of(&after.scene);
        for name in ["gui", "crates", "docs"] {
            assert!(text.contains(name), "{name:?} left the list: {text}");
        }
        let tint_of = |out: &Rendered, name: &str| -> Vec<Option<[u8; 4]>> {
            out.scene
                .texts
                .iter()
                .flat_map(|text| text.runs.iter())
                .filter(|run| run.text.trim() == name)
                .map(|run| run.color)
                .collect()
        };
        assert_eq!(tint_of(&after, "gui"), vec![Some(after.skin.dim)]);
        assert_eq!(tint_of(&after, "docs"), vec![Some(after.skin.dim)]);
        assert_eq!(tint_of(&before, "gui"), vec![Some(before.skin.body)]);
        // The match is where the cursor went, so it is the row on the band, and
        // what is written on a green band is the dark ink.
        assert_eq!(said(&picker, picker.cursor()), "crates");
        assert_eq!(tint_of(&after, "crates"), vec![Some(after.skin.picked_ink)]);

        // One rule: the arrows walk the matches, and a click still lands on a
        // dim row, so what the pointer can reach is a superset of what the
        // arrows stop on.
        let dim = after
            .layout
            .picker_rows
            .iter()
            .find(|(index, _)| said(&picker, *index) == "gui")
            .copied()
            .expect("the dim row is still placed");
        let (x, y) = middle(dim.1);
        assert_eq!(after.layout.hit(x, y), Some(Hit::PickerRow(dim.0)));
        assert!(picker.point_at(dim.0), "a click on a dim row selects it");
        assert_eq!(
            picker.confirm(),
            Some(crate::picker::Chosen::folder(std::path::PathBuf::from(
                "/home/hec/gui"
            )))
        );
    }
    /// Item 4: the mark in front of a folder is a region of its own inside the
    /// row, pressing it opens the folder where it stands, and what comes out is
    /// drawn one step further in than the folder it came from.
    #[test]
    fn the_mark_in_front_of_a_folder_is_its_own_target() {
        let mut picker = a_picker(&["gui", "crates"], &["/home/hec/workspace"]);
        let out = render_picker(&picker, 1205.0, 791.0, None);

        // A mark only where there is a folder to open. The remembered folder,
        // the folder being listed and the way out of it are how the list is
        // walked rather than branches of the tree.
        let marked: Vec<String> = out
            .layout
            .picker_marks
            .iter()
            .map(|(index, _)| said(&picker, *index))
            .collect();
        assert_eq!(marked, ["crates", "gui"]);

        // Each one sits inside its own row, and the row still answers for the
        // rest of itself: the press that opens a folder and the press that
        // selects it are different presses.
        for (index, mark) in &out.layout.picker_marks {
            let row = out
                .layout
                .picker_rows
                .iter()
                .find(|(at, _)| at == index)
                .map(|(_, row)| *row)
                .expect("a mark with no row under it");
            assert!(mark.w > 1.0 && (mark.h - row.h).abs() < 0.01, "{mark:?}");
            assert!(
                mark.x >= row.x && mark.x + mark.w <= row.x + row.w,
                "the mark is outside its row: {mark:?} in {row:?}"
            );
            let (x, y) = middle(*mark);
            assert_eq!(out.layout.hit(x, y), Some(Hit::PickerMark(*index)));
            assert_eq!(
                out.layout.hit(mark.x + mark.w + 2.0, y),
                Some(Hit::PickerRow(*index)),
                "the row beside the mark stopped answering"
            );
        }
        // It lights up under the pointer, so it reads as something to press.
        // The colour is the mark's own green either way and what changes is the
        // weight of the box: the old glyph swapped tint instead, which it had to,
        // because a glyph has no border to thicken.
        let (index, mark) = out.layout.picker_marks[0];
        let warm = render_picker(&picker, 1205.0, 791.0, Some(Hit::PickerMark(index)));
        assert_eq!(outline_of(&out, mark).map(|rect| rect.extra()[3]), Some(1.0));
        assert_eq!(
            outline_of(&warm, mark).map(|rect| rect.extra()[3]),
            Some(2.0),
            "the mark does not thicken under the pointer"
        );
        for at in [&out, &warm] {
            assert_eq!(
                outline_of(at, mark).map(|rect| rect.rgba()),
                Some(at.skin.mark_edge)
            );
        }

        // Pressing it puts what is inside the folder in the list under it, at a
        // deeper indent, and the mark turns over.
        assert!(picker.toggle(index));
        let after = render_picker(&picker, 1205.0, 791.0, None);
        let deeper = after
            .layout
            .picker_marks
            .iter()
            .find(|(at, _)| picker.row(*at).map(PickerRow::depth) == Some(1))
            .copied()
            .expect("nothing came out of the folder");
        assert!(
            deeper.1.x > mark.x,
            "a child is not drawn further in than its parent: {:?} then {:?}",
            mark,
            deeper.1
        );
        // A shut folder carries a plus, an open one carries the same box with
        // the upright taken out of it, and neither is a glyph: nothing is drawn
        // as text inside a mark any more.
        assert_eq!(bars_in(&out, mark), 2, "a shut folder is not a plus");
        let (_, reopened) = after
            .layout
            .picker_marks
            .iter()
            .find(|(at, _)| *at == index)
            .copied()
            .expect("the folder that was opened lost its mark");
        assert_eq!(bars_in(&after, reopened), 1, "an open folder is not a minus");
        for (at, mark) in [(&out, mark), (&after, reopened)] {
            assert!(
                !at.scene.texts.iter().any(|text| {
                    (text.at.x - mark.x).abs() < 0.01 && (text.at.y - mark.y).abs() < 0.01
                }),
                "the mark is still drawn as a glyph"
            );
        }

        // And the name beside it is still drawn inside the box, however deep it
        // sits: the indent stops before it pushes a row off the right.
        for (index, row) in &after.layout.picker_rows {
            let said = said(&picker, *index);
            assert!(
                after
                    .scene
                    .texts
                    .iter()
                    .any(|text| text.runs.iter().any(|run| run.text.trim() == said)),
                "{said:?} is not drawn"
            );
            assert!(row.x + row.w <= after.layout.picker_list.x + after.layout.picker_list.w + 0.01);
        }
    }
    /// Item A6: the mark in front of a folder is a small unfilled green box with
    /// a green plus in it. It used to be Font Awesome's filled plus-square drawn
    /// at the row's own text size, which is a solid block at the front of every
    /// folder in the list.
    #[test]
    fn the_folder_mark_is_a_small_unfilled_green_box() {
        let picker = a_picker(&["gui", "crates"], &[]);
        let out = render_picker(&picker, 1205.0, 791.0, None);
        // Not the row the cursor is on, whose mark is drawn in the ink that
        // reads on the green band rather than in the green itself.
        let (index, mark) = *out
            .layout
            .picker_marks
            .iter()
            .find(|(index, _)| *index != picker.cursor())
            .expect("no mark off the cursor's row");

        let box_ = outline_of(&out, mark).expect("the mark has no box round it");
        let [x, y, w, h] = box_.xywh();
        // Square, odd sided so the plus has a middle to sit on, and well under
        // the region it is drawn in: smaller is the whole point.
        assert_eq!(w, h, "the mark is not square");
        assert_eq!(w as i32 % 2, 1, "an even side puts the plus off centre");
        assert!(
            w <= mark.w.min(mark.h) * 0.7,
            "{w} is not smaller than the {:?} it is drawn in",
            (mark.w, mark.h)
        );
        assert!(
            (x + w * 0.5 - (mark.x + mark.w * 0.5)).abs() <= 1.0
                && (y + h * 0.5 - (mark.y + mark.h * 0.5)).abs() <= 1.0,
            "the mark is not centred in its region"
        );
        // A border and nothing behind it: an outline is a stroke, and a filled
        // rectangle of this colour anywhere in the region would be the fill the
        // glyph used to be.
        assert_eq!(box_.extra()[3], 1.0, "the box is not a hairline");
        assert_eq!(box_.rgba(), out.skin.mark_edge, "the box is not the accent");
        assert_eq!(
            out.skin.mark_edge,
            out.skin.picked,
            "the folder mark is not the colour the window picks with"
        );
        let filled = out
            .scene
            .rects
            .iter()
            .filter(|rect| {
                let [_, _, w, h] = rect.xywh();
                rect.extra()[3] == 0.0 && w > 1.0 && h > 1.0 && inside(**rect, mark)
            })
            .count();
        assert_eq!(filled, 0, "something inside the mark is filled");

        // The plus is two bars, both green, both one pixel, and both clear of
        // the border round them.
        let bars: Vec<Rect> = out
            .scene
            .rects
            .iter()
            .filter(|rect| {
                let [_, _, w, h] = rect.xywh();
                rect.extra()[3] == 0.0 && (w == 1.0 || h == 1.0) && inside(**rect, mark)
            })
            .copied()
            .collect();
        assert_eq!(bars.len(), 2, "a shut folder does not carry a plus");
        for bar in &bars {
            assert_eq!(bar.rgba(), out.skin.mark_edge);
            let [bx, by, bw, bh] = bar.xywh();
            assert!(bx > x && by > y && bx + bw < x + w && by + bh < y + h, "{bar:?} touches the box");
        }
        assert!(
            bars.iter().any(|bar| bar.xywh()[2] > 1.0) && bars.iter().any(|bar| bar.xywh()[3] > 1.0),
            "the two bars do not cross"
        );

        // On the row the cursor is on the same box is drawn in the ink that
        // reads on the band, because the band there is already this green.
        let mut picker = picker;
        assert!(picker.point_at(index), "the cursor will not go on a folder");
        let banded = render_picker(&picker, 1205.0, 791.0, None);
        let (_, on_band) = *banded
            .layout
            .picker_marks
            .iter()
            .find(|(at, _)| *at == index)
            .expect("the row the cursor moved to lost its mark");
        assert_eq!(
            outline_of(&banded, on_band).map(|rect| rect.rgba()),
            Some(banded.skin.mark_on_band),
            "the mark is green on a green band"
        );
        assert_eq!(bars_in(&banded, on_band), 2, "and it is still a plus");
    }
    /// Item A6: what is typed to narrow the list sits in a field, with the
    /// magnifier that says type here and the cut corner every other box in this
    /// window carries. It was a line of writing with a funnel in front of it.
    #[test]
    fn the_picker_s_filter_is_a_bordered_field_with_a_search_icon() {
        let mut picker = a_picker(&["gui", "crates"], &[]);
        let out = render_picker(&picker, 1205.0, 791.0, None);
        let field = out.layout.picker_filter;

        // Under the two lines of writing, above the list, the full width of the
        // box's content, and taller than the line in it.
        let line = Text::line_for(13.0);
        assert!(field.w > 1.0 && field.h > line, "{field:?} is not a field");
        assert!(
            field.y > out.layout.picker.y && field.y + field.h <= out.layout.picker_list.y + 0.01,
            "{field:?} is not between the heading and the list"
        );

        // A surface, a hairline round it, and both take the window's cut corner.
        let shaped: Vec<Rect> = out
            .scene
            .rects
            .iter()
            .filter(|rect| {
                let [x, y, w, h] = rect.xywh();
                (x - field.x).abs() < 0.01
                    && (y - field.y).abs() < 0.01
                    && (w - field.w).abs() < 0.01
                    && (h - field.h).abs() < 0.01
            })
            .copied()
            .collect();
        assert_eq!(shaped.len(), 2, "the field is not a fill and an edge");
        for rect in &shaped {
            assert_eq!(rect.extra()[1], CUT, "the field has no cut corner");
            assert_eq!(rect.extra()[2], Rect::TOP_RIGHT as f32);
        }
        assert!(shaped.iter().any(|rect| rect.rgba() == out.skin.input));
        assert!(
            shaped
                .iter()
                .any(|rect| rect.rgba() == out.skin.edge_focus && rect.extra()[3] == 1.0)
        );

        // The magnifier is inside the field, and the funnel that was there is
        // gone from the window.
        let runs: Vec<&str> = out
            .scene
            .texts
            .iter()
            .filter(|text| {
                text.at.x >= field.x
                    && text.at.y >= field.y
                    && text.at.y < field.y + field.h
            })
            .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
            .collect();
        assert!(
            runs.contains(&icons::SEARCH.to_string().as_str()),
            "the search icon is not in the field: {runs:?}"
        );
        let every: String = out
            .scene
            .texts
            .iter()
            .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
            .collect();
        assert!(!every.contains('\u{eaf1}'), "the funnel is still drawn");
        assert!(every.contains("type to narrow the list"));

        // And what is typed goes in the same field.
        assert!(picker.type_text("cra"));
        let typed = render_picker(&picker, 1205.0, 791.0, None);
        assert_eq!(typed.layout.picker_filter, field, "the field moved");
        let said: String = typed
            .scene
            .texts
            .iter()
            .filter(|text| (text.at.y - field.y - PICKER_FIELD_PAD).abs() < 0.01)
            .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
            .collect();
        assert!(said.contains("cra"), "what was typed is not in the field: {said:?}");
    }
    /// The field keeps to the room above the list, whatever the window does.
    ///
    /// Its height was its own to choose while everything under it was measured
    /// from the head the box could not give it, so in a window short enough
    /// that the head has no room the field was drawn at full height out of the
    /// bottom of the box and over the Open button. It takes the room that is
    /// there now, down to none of it, which is a picker with no field rather
    /// than a field over the list.
    #[test]
    fn the_picker_s_field_stays_out_of_its_list_in_a_short_window() {
        for height in [100.0f32, 120.0, 160.0, 200.0, 240.0, 300.0, 420.0, 791.0] {
            // Both lists: the sessions keep a row above themselves for the
            // header, so the room over the list is not the same room.
            for sessions in [false, true] {
                let picker = match sessions {
                    true => a_session_picker(),
                    false => a_picker(&["gui", "crates", "docs"], &[]),
                };
                let out = render_picker(&picker, 900.0, height, None);
                let (box_, field, list, open) = (
                    out.layout.picker,
                    out.layout.picker_filter,
                    out.layout.picker_list,
                    out.layout.picker_open,
                );
                let what = format!("{height} tall, sessions {sessions}");
                assert!(
                    list.h < 1.0 || field.y + field.h <= list.y + 0.01,
                    "{what}: the field {field:?} runs into the list {list:?}"
                );
                assert!(
                    field.y + field.h <= box_.y + box_.h + 0.01,
                    "{what}: the field {field:?} runs out of the box {box_:?}"
                );
                assert!(
                    field.h < 1.0 || field.y >= open.y + open.h - 0.01,
                    "{what}: the field {field:?} is over the Open button {open:?}"
                );
                // And nothing is drawn as a field where there is no room for
                // one: the rows of the list own that space.
                if field.h < 1.0 {
                    assert!(
                        !out.scene.rects.iter().any(|rect| {
                            let [x, y, w, _] = rect.xywh();
                            (x - field.x).abs() < 0.01
                                && (y - field.y).abs() < 0.01
                                && (w - field.w).abs() < 0.01
                        }),
                        "{what}: a field with no room is still drawn"
                    );
                }
            }
        }
        // With room to spare it is the field it always was.
        let out = render_picker(&a_picker(&["gui"], &[]), 900.0, 791.0, None);
        let field = out.layout.picker_filter;
        assert!(
            (field.h - picker_field_h(Text::line_for(13.0))).abs() < 0.01,
            "{field:?} is not the height a field asks for"
        );
    }
    /// Item E1: Open selected is one route for both lists. On the folders it
    /// opens the folder the cursor is on, on the sessions it carries the session
    /// the cursor is on, and it is the same button in the same place either way.
    ///
    /// The picker used to have four affordances for these two acts: an Open
    /// button and a Folders/Sessions swap at the foot, and an arrow back to the
    /// folders in the heading. The arrow and the foot swap are gone.
    #[test]
    fn open_selected_opens_a_folder_on_one_list_and_a_session_on_the_other() {
        // The folder list, cursor moved onto the folder inside it.
        let mut folders = a_picker(&["gui"], &[]);
        let on_folders = render_picker(&folders, 1205.0, 791.0, None);
        let button = on_folders.layout.picker_open;
        let (x, y) = middle(button);
        assert_eq!(on_folders.layout.hit(x, y), Some(Hit::PickerOpen));
        // Past this folder and past the way out of it, onto the one inside.
        assert!(folders.step(true) && folders.step(true));
        let chosen = folders.confirm().expect("Open selected chose nothing");
        assert_eq!(chosen.workspace, std::path::PathBuf::from("/home/hec/gui"));
        assert_eq!(chosen.session, None, "a folder is a fresh session");

        // The session list, in the same window: the same button, in the same
        // place, and it answers for the same point.
        let mut sessions = a_session_picker();
        let on_sessions = render_picker(&sessions, 1205.0, 791.0, None);
        assert_eq!(on_sessions.layout.picker_open, button, "the button moved");
        assert_eq!(on_sessions.layout.hit(x, y), Some(Hit::PickerOpen));
        let chosen = sessions.confirm().expect("Open selected chose nothing");
        assert_eq!(chosen.workspace, std::path::PathBuf::from("/home/hec"));
        assert_eq!(
            chosen.session.as_deref(),
            Some("live"),
            "the session under the cursor is not the one that was opened"
        );

        // Nothing that was retired is still drawn or still answers: no arrow in
        // the heading, and no second button at the foot of the box.
        let box_ = on_sessions.layout.picker;
        let foot = Panel::new(box_.x, box_.y + box_.h - picker_open_h(Text::line_for(13.0)), box_.w, picker_open_h(Text::line_for(13.0)));
        for out in [&on_folders, &on_sessions] {
            assert!(
                !text_of(&out.scene).contains('\u{ea9b}'),
                "the back arrow is still drawn"
            );
            assert!(
                !out.scene.rects.iter().any(|rect| {
                    let [rx, ry, _, _] = rect.xywh();
                    rect.rgba() == out.skin.button && foot.contains(rx + 1.0, ry + 1.0)
                }),
                "there is still a button at the foot of the box"
            );
        }
        // The one thing left down there is the line of keys, and Escape is still
        // on it.
        let keys = text_of(&on_sessions.scene);
        assert!(keys.contains("esc quits"), "{keys}");
    }
    /// Item A7: the session list is a table. A row that read
    /// "10m ago  hec  carry this on" said four things with nothing anywhere
    /// naming any of them, so every cell now sits in a column of its own under a
    /// row that says what that column is.
    #[test]
    fn the_session_list_is_a_table_under_a_row_naming_its_columns() {
        let picker = a_session_picker();
        let out = render_picker(&picker, 1205.0, 791.0, None);
        let line = Text::line_for(13.0);
        let list = out.layout.picker_list;
        let (at, _) = session_table(list, 8.0);

        // The header sits on the line the layout kept above the list, which is
        // not one of the list's rows: it names the columns, it is not one of
        // them, and pressing it must not select anything.
        let header = line_at(&out, list.y - line);
        assert!(
            out.layout
                .picker_rows
                .iter()
                .all(|(_, row)| (row.y - (list.y - line)).abs() > 0.01),
            "the header took a row of the list"
        );
        assert_eq!(
            out.layout.hit(at + 4.0, list.y - line + 2.0),
            Some(Hit::Picker),
            "the header answers as the box, not as a row"
        );

        // Each column's name starts exactly where that column starts, and the
        // last one takes whatever is left.
        let mut offset = 0;
        for (name, wide) in SESSION_COLUMNS {
            let cell: String = header.chars().skip(offset).take(wide).collect();
            assert!(
                cell.starts_with(name),
                "{name:?} does not begin column {offset}: {header:?}"
            );
            offset += wide;
        }
        assert!(
            header.chars().skip(offset).collect::<String>().starts_with(SESSION_OPENING),
            "{header:?}"
        );

        // And every row writes its cells into those same columns, at the same x
        // the header is drawn at.
        for (index, row) in &out.layout.picker_rows {
            let cells = match picker.row(*index) {
                Some(PickerRow::Session(saved)) => picker.session_cells(saved),
                other => panic!("not a session: {other:?}"),
            };
            let (row_at, _) = session_table(*row, 8.0);
            assert!((row_at - at).abs() < 0.01, "row {index} starts elsewhere");
            let text: String = out
                .scene
                .texts
                .iter()
                .filter(|text| (text.at.y - row.y).abs() < 0.01 && (text.at.x - at).abs() < 0.01)
                .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
                .collect();
            let mut offset = 0;
            for (step, (_, wide)) in SESSION_COLUMNS.iter().enumerate() {
                let cell: String = text.chars().skip(offset).take(*wide).collect();
                assert!(
                    cell.starts_with(&cells[step]),
                    "row {index} column {step} says {cell:?}, not {:?}",
                    cells[step]
                );
                offset += wide;
            }
            assert!(
                text.chars().skip(offset).collect::<String>().starts_with(&cells[4]),
                "row {index} lost what was said in it: {text:?}"
            );
        }

        // The two columns that were nowhere before: how big the transcript is,
        // and how full its context window was. Nothing has ever measured these
        // sessions, so the reading is a dash rather than a number nobody took.
        let first = line_at(&out, out.layout.picker_rows[0].1.y);
        assert!(first.contains("12 kB"), "{first:?}");
        assert!(first.contains(" - "), "{first:?}");

        // The folder list has no header at all: it is one column of names, and
        // a word over it would explain the obvious.
        let folders = a_picker(&["gui", "crates"], &[]);
        assert!(!folders.on_sessions());
        let out = render_picker(&folders, 1205.0, 791.0, None);
        let text = text_of(&out.scene);
        for name in ["when", "context", SESSION_OPENING] {
            assert!(!text.contains(name), "{name:?} is over the folder list");
        }
    }
    /// A right click on a session row opens a menu over the picker, and the
    /// picker's own drawing used to stop before the overlay: the menu was placed
    /// and it answered presses, and nothing was on screen.
    #[test]
    fn a_menu_over_the_picker_is_drawn_over_the_picker() {
        let picker = a_session_picker();
        let row = {
            let out = render_picker(&picker, 1205.0, 791.0, None);
            out.layout.picker_rows[0].1
        };
        let menu = Menu::for_session(middle(row), 0, false);
        let out = render_picker_menu(&picker, &menu, 1205.0, 791.0);

        assert!(out.layout.menu.w >= 1.0, "the menu was not placed");
        assert!(!out.scene.over_rects.is_empty(), "the menu box is not drawn");
        let rows: String = out
            .scene
            .over_texts
            .iter()
            .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
            .collect();
        for label in [
            crate::menu::Item::OpenSession.label(),
            crate::menu::Item::DeleteSession(false).label(),
        ] {
            assert!(rows.contains(label), "{label:?} is not on screen: {rows:?}");
        }

        // And it takes the press before the row it covers, which it always did.
        let (x, y) = middle(out.layout.menu_rows[1].1);
        assert_eq!(out.layout.hit(x, y), Some(Hit::MenuRow(1)));
    }
    /// Pressed once, the Delete row reads "sure?" in the colour this window
    /// gives everything that throws work away, and the box under it does not
    /// move: the second press lands on the same pixels the first one did.
    ///
    /// The wording is the settings panel's, because the panel's delete asks the
    /// same question and the two are one product.
    #[test]
    fn an_armed_delete_row_reads_sure_without_moving_the_menu() {
        let picker = a_session_picker();
        let row = {
            let out = render_picker(&picker, 1205.0, 791.0, None);
            out.layout.picker_rows[0].1
        };
        let mut menu = Menu::for_session(middle(row), 0, false);
        let before = render_picker_menu(&picker, &menu, 1205.0, 791.0);
        let (x, y) = middle(before.layout.menu_rows[1].1);

        assert!(!menu.press_delete(1), "the first press was the delete");
        let out = render_picker_menu(&picker, &menu, 1205.0, 791.0);
        let armed: Vec<&Run> = out
            .scene
            .over_texts
            .iter()
            .flat_map(|text| text.runs.iter())
            .filter(|run| run.text.contains("sure?"))
            .collect();
        assert_eq!(armed.len(), 1, "the row does not ask: {armed:?}");
        assert_eq!(armed[0].color, Some(out.skin.bad));
        let rows: String = out
            .scene
            .over_texts
            .iter()
            .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
            .collect();
        assert!(
            !rows.contains(crate::menu::Item::DeleteSession(false).label()),
            "both wordings are on screen: {rows:?}"
        );

        // The same box, the same rows, and the same press: a menu that narrowed
        // when it armed would slide out from under the pointer and cancel the
        // press it just asked for.
        assert_eq!(out.layout.menu, before.layout.menu);
        assert_eq!(out.layout.menu_rows, before.layout.menu_rows);
        assert_eq!(out.layout.hit(x, y), Some(Hit::MenuRow(1)));
    }
    /// A folder with more subfolders than the box has rows scrolls. The rows
    /// that are drawn are the rows the list is showing, and nothing is dropped
    /// off the bottom of the box.
    #[test]
    fn the_picker_s_list_scrolls_instead_of_dropping_folders() {
        let names: Vec<String> = (0..60).map(|n| format!("dir{n:02}")).collect();
        let inside: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut picker = a_picker(&inside, &[]);
        let out = render_picker(&picker, 1205.0, 791.0, None);
        let rows = out.layout.picker_capacity(13.0);
        assert!(
            (PICKER_MIN_ROWS..=PICKER_MAX_ROWS).contains(&rows),
            "{rows} rows"
        );
        assert_eq!(out.layout.picker_rows.len(), rows);
        assert_eq!(out.layout.picker_rows[0].0, 0, "anchored at the top");
        let last = out.layout.picker_rows.last().unwrap().1;
        assert!(
            last.y + last.h <= out.layout.picker_list.y + out.layout.picker_list.h + 0.01,
            "the last row hangs out of the list"
        );
        assert!(
            picker.thumb(rows).is_some(),
            "a list that does not fit says so"
        );

        // Moved down, the rows drawn are the rows the list moved to.
        assert!(picker.scroll(5, true, rows));
        let out = render_picker(&picker, 1205.0, 791.0, None);
        assert_eq!(out.layout.picker_rows[0].0, 5);
        assert_eq!(out.layout.picker_rows.len(), rows);
        for (index, row) in &out.layout.picker_rows {
            let (x, y) = middle(*row);
            assert_eq!(out.layout.hit(x, y), Some(Hit::PickerRow(*index)));
        }

        // A short list keeps the box a readable size rather than collapsing to
        // two rows, and a window too small for the whole box does not push it
        // off the surface.
        let short = render_picker(&a_picker(&["one"], &[]), 1205.0, 791.0, None);
        assert!(short.layout.picker_capacity(13.0) >= PICKER_MIN_ROWS);
        let tiny = render_picker(&picker, 680.0, 380.0, None);
        assert!(tiny.layout.picker.h <= 380.0 - TITLE_H);
        assert!(!tiny.layout.picker_rows.is_empty(), "it still lists folders");
        for rect in &tiny.scene.rects {
            let [x, y, w, h] = rect.xywh();
            assert!(x >= -0.01 && y >= -0.01 && x + w <= 680.01 && y + h <= 380.01);
        }
    }
}
