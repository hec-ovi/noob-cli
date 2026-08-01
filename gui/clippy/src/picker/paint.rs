//! The startup chooser painter.

use noob_draw::{Panel, Run, Scene, Text};

#[allow(unused_imports)]
use crate::design::{self, icons};
use crate::picker::places::{picker_indent, picker_mark_box};
use crate::picker::Row as PickerRow;
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
