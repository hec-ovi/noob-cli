//! The files pane: the explorer list beside the open file.

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



/// The file view: the explorer column, and the open file beside it.
pub(crate) fn files(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    if state.files.is_empty() {
        scene.text(Text::rich(
            vec![Run::tinted("no files touched yet", skin.dim)],
            panel.inset(PAD),
            frame.pane_size,
            skin.dim,
        ));
        return;
    }
    if layout.file_list.w >= 1.0 {
        explorer(scene, frame, layout.file_list);
    }

    let body = layout.file_diff;
    if body.w < 1.0 || body.h < Text::line_for(frame.pane_size) + 2.0 * PAD {
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
    let (cols, chrome) = text_columns(View::Files, body, frame.pane_column);
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
        // The gutter, so a diff line says where in the file it landed. Exactly
        // `chrome` columns of it, on this row and on every row this line
        // continues onto.
        match entry.number {
            Some(number) => runs.push(Run::tinted(file_number(number, chrome), skin.comment)),
            None if !entry.text.is_empty() => runs.push(Run::plain(" ".repeat(chrome))),
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
    // Broken into rows by the same call the pane counts them with, in the
    // columns that are left once the gutter has been paid for, and the rows a
    // line continues onto are indented past that gutter. Wrapping the gutter
    // along with the text is what put every continuation row four columns out
    // from the band, the caret and the clipboard.
    scene.text(
        Text::rich(runs, content, frame.pane_size, skin.body)
            .wrap_at(cols)
            .hanging(chrome),
    );
    scrollbar(scene, skin, body, file.pane.thumb(rows, cols));
}

/// The file list down the left of the pane, one row per file the agent has
/// touched, the way an editor's explorer reads.
///
/// Flat, because the set behind it is flat: these are the files the agent has
/// opened, not a filesystem. Nothing here groups by directory or expands, and a
/// row is a file.
fn explorer(scene: &mut Scene, frame: &Frame, list: Panel) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    // The one thing between the list and the file. The pane has a single surface
    // and a single outline, so without this line the two columns read as one.
    scene.rect(list.right_edge(skin.edge));
    let line = Text::line_for(frame.pane_size);
    let cols = cols_of(list, frame.pane_column);
    for (index, row) in &layout.file_rows {
        let Some(file) = state.files.get(*index) else {
            continue;
        };
        let open = *index == state.open_file;
        if open {
            // A band across the row and a mark down its left edge, not a block
            // in a colour of its own: the pane is already a surface, and a block
            // standing on it is what made the old tabs read as buttons.
            scene.rect(row.fill(skin.strip));
            scene.rect(Panel::new(row.x, row.y, MARK_W, row.h).fill(skin.tab_accent));
        }
        // A file compaction dropped is still worth reading; it is just no longer
        // what the agent is holding, and the row says which.
        let tint = match (open, file.closed) {
            (_, true) => skin.dim,
            (true, false) => skin.bright,
            (false, false) => skin.body,
        };
        let room = cols
            .saturating_sub(ROW_ICON_COLUMNS + if file.changed { ROW_MARK_COLUMNS } else { 0 })
            .max(1);
        let mut runs = vec![
            // The type mark, so a row is recognisable before it is read.
            Run::icon(crate::design::icons::for_path(&file.path).to_string(), tint),
            Run::tinted(format!(" {}", fit_name(&file.path, room)), tint),
        ];
        if file.changed {
            runs.push(Run::tinted(" \u{2022}", skin.plus));
        }
        scene.text(Text::rich(
            runs,
            Panel::new(row.x + PAD, row.y, (row.w - 2.0 * PAD).max(1.0), line),
            frame.pane_size,
            tint,
        ));
    }
    // The list is a scroll window like any other pane, so it says how much of
    // itself is on screen the same way.
    let rows = layout.rows(list, frame.pane_size);
    scrollbar(
        scene,
        skin,
        list,
        crate::scroll::file_thumb(frame.file_scroll, state.files.len(), rows),
    );
}
