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

/// Columns the file view spends on its line-number gutter, on every row.
pub(crate) const GUTTER: usize = 4;
/// Columns a row of the file explorer spends before its name: the type icon and
/// the space after it.
pub(crate) const ROW_ICON_COLUMNS: usize = 2;
/// Columns a row spends on the changed mark, when it carries one.
pub(crate) const ROW_MARK_COLUMNS: usize = 2;
/// The widest the explorer column gets, however long the names in it are. Past
/// this the list is spending the pane on directory prefixes nobody is reading.
pub(crate) const LIST_MAX_COLUMNS: usize = 20;
/// The narrowest it gets: an icon and enough characters to tell two names apart.
pub(crate) const LIST_MIN_COLUMNS: usize = 9;
/// What the file keeps whatever the list wants: the line-number gutter and
/// enough code beside it to read a line. The file view usually lives in the
/// right-hand column, which is about 35 columns wide in a window at its minimum
/// size, so a list sized to its own content alone would leave the thing being
/// looked at unreadable. At that size this floor wins and the list goes below
/// [`LIST_MIN_COLUMNS`], because the file is what is being read.
pub(crate) const DIFF_MIN_COLUMNS: usize = GUTTER + 20;


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

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::wildcard_imports)]
    use crate::view::testkit::*;
    use crate::config::Config;
    use crate::dock::{Dock, Space};
    use crate::monitor::Monitor;
    use crate::state::{State, Tone};

    /// A file, open, holding lines long enough that the pane has to wrap them,
    /// each with the line number a file's rows carry.
    fn a_wrapped_file(lines: &[(u32, &str)]) -> (State, Vec<String>) {
        let paths = ["src/main.rs"];
        let mut state = touched(&paths);
        for (number, text) in lines {
            state.files[0]
                .pane
                .push(crate::state::Line::new(*text, Tone::Body).at(*number));
        }
        (state, labels(&paths))
    }

    /// A state that has touched every named file, in order, with the last one
    /// open. The paths are what the agent would have sent, so `short_name` and
    /// the type icons are exercised rather than bypassed.
    fn touched(paths: &[&str]) -> State {
        let mut state = State::new();
        for path in paths {
            state.apply(noob_proto::Event::FileEdit {
                path: (*path).into(),
                span: noob_proto::Span {
                    start: 1,
                    end: 1,
                    kind: None,
                    name: None,
                },
                before: "was".into(),
                after: "is".into(),
                call_id: None,
            });
        }
        state
    }
    fn labels(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| short_name(p)).collect()
    }

    /// The list runs down the pane, one row per file, not across it. This was a
    /// horizontal strip of tabs and the direct instruction was "vertical, like
    /// in visual studio code".
    #[test]
    fn the_file_list_is_a_column_with_one_row_per_file() {
        let paths = ["src/calc.py", "README.md", "src/main.rs"];
        let state = touched(&paths);
        let names = labels(&paths);
        let out = render(
            &state,
            1400.0,
            900.0,
            &a_dock_showing(View::Files),
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        assert_eq!(out.layout.file_rows.len(), 3);
        let line = Text::line_for(13.0);
        let mut last: Option<Panel> = None;
        for (step, (index, row)) in out.layout.file_rows.iter().enumerate() {
            assert_eq!(*index, step, "the rows are the files, in order");
            assert!((row.h - line).abs() < 0.01, "a row is one line tall: {row:?}");
            if let Some(above) = last {
                assert!((row.x - above.x).abs() < 0.01, "the rows are a column");
                assert!(
                    (row.y - (above.y + line)).abs() < 0.01,
                    "row {step} does not sit under the one before it"
                );
            }
            last = Some(*row);
        }
        // The list is on the left and the file is beside it, not under it.
        let (list, diff) = (out.layout.file_list, out.layout.file_diff);
        assert!(list.w > 1.0 && diff.w > 1.0);
        assert!((list.x + list.w - diff.x).abs() < 0.01, "{list:?} {diff:?}");
        assert!((list.y - diff.y).abs() < 0.01, "the two columns start level");
        // Every name is there, and the type icon in front of it.
        let text = text_of(&out.scene);
        for name in &names {
            assert!(text.contains(name.as_str()), "{name} is not in the list: {text}");
        }
        for path in paths {
            let icon = crate::design::icons::for_path(path).to_string();
            assert!(text.contains(&icon), "{path} has no type icon");
        }
    }
    /// One row is marked, and it is the open file's. A band and an accent down
    /// the left edge rather than a block in another colour: a block standing on
    /// the pane's own surface is what made the old tabs read as buttons.
    #[test]
    fn the_open_file_s_row_is_the_marked_one() {
        let paths = ["src/calc.py", "src/main.rs"];
        let mut state = touched(&paths);
        state.open_file = 0;
        let names = labels(&paths);
        let out = render(
            &state,
            1400.0,
            900.0,
            &Dock::new(),
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        for (index, row) in &out.layout.file_rows {
            let open = *index == state.open_file;
            assert_eq!(
                covered(&out, *row, row.h, out.skin.strip),
                open,
                "row {index} and its band disagree about being open"
            );
            let mark = topped(&out, *row, row.h, out.skin.tab_accent);
            assert_eq!(
                mark.is_some(),
                open,
                "row {index} and its mark disagree about being open"
            );
            if let Some(mark) = mark {
                assert!((mark.xywh()[2] - MARK_W).abs() < 0.01, "{:?}", mark.xywh());
            }
            let label = out
                .scene
                .texts
                .iter()
                .find(|text| row.contains(text.at.x + 1.0, text.at.y + 1.0))
                .unwrap_or_else(|| panic!("row {index} has no label"));
            assert_eq!(
                label.color,
                if open { out.skin.bright } else { out.skin.body },
                "row {index} is not tinted for being open or not"
            );
        }
    }
    /// A list longer than the pane shows a screenful, scrolls to the rest, and
    /// says so with a thumb. The window comes from text-geometry, so the rows
    /// drawn and the rows a scroll position names cannot disagree.
    #[test]
    fn a_list_longer_than_the_pane_scrolls_instead_of_dropping_files() {
        let paths: Vec<String> = (0..40).map(|n| format!("src/file{n}.rs")).collect();
        let borrowed: Vec<&str> = paths.iter().map(String::as_str).collect();
        let mut state = touched(&borrowed);
        state.open_file = 0;
        let names = labels(&borrowed);
        let short: Vec<&str> = names.iter().map(String::as_str).collect();

        let dock = a_dock_showing(View::Files);
        let out = render(&state, 1400.0, 900.0, &dock, &short);
        let shown = out.layout.file_rows.len();
        assert!(shown > 4, "only {shown} rows fit");
        assert!(shown < paths.len(), "all {shown} rows fit, nothing to scroll");
        assert_eq!(out.layout.file_rows[0].0, 0, "the top of the list");

        // Scrolled down, the same rows carry later files, and none of them are
        // drawn outside the column.
        let scrolled = Layout::compute(1400.0, 900.0, &scrolled_shape(&dock, &short, 5));
        assert_eq!(scrolled.file_rows.len(), shown);
        assert_eq!(scrolled.file_rows[0].0, 5);
        for (_, row) in &scrolled.file_rows {
            assert!(
                row.y >= scrolled.file_list.y - 0.01
                    && row.y + row.h <= scrolled.file_list.y + scrolled.file_list.h + 0.01,
                "{row:?} is outside {:?}",
                scrolled.file_list
            );
        }
        // Past the end clamps to the last screenful rather than to nothing.
        let far = Layout::compute(1400.0, 900.0, &scrolled_shape(&dock, &short, 999));
        assert_eq!(far.file_rows.len(), shown);
        assert_eq!(far.file_rows.last().unwrap().0, paths.len() - 1);

        // And the list carries a thumb, because it does not all fit.
        let rows = out.layout.rows(out.layout.file_list, 13.0);
        assert!(
            crate::scroll::file_thumb(0, state.files.len(), rows).is_some(),
            "no thumb on a long list"
        );
        assert!(
            crate::scroll::file_thumb(0, 0, rows).is_none(),
            "a thumb with nothing to scroll"
        );
    }
    /// The file is the thing being looked at, so it keeps its floor whatever the
    /// list wants. The pane is narrow: at the smallest window the layout allows,
    /// the file view lives in the right-hand column.
    #[test]
    fn the_file_keeps_room_to_be_read_beside_the_list() {
        let paths = ["src/averyverylongfilename.rs", "src/other.rs"];
        let state = touched(&paths);
        let names = labels(&paths);
        let short: Vec<&str> = names.iter().map(String::as_str).collect();
        for (w, h) in [(680.0, 380.0), (900.0, 700.0), (2200.0, 1400.0)] {
            let out = render(&state, w, h, &a_dock_showing(View::Files), &short);
            let (list, diff) = (out.layout.file_list, out.layout.file_diff);
            assert!(list.w > 1.0, "no list at {w}x{h}");
            assert!(
                cols_of(diff, 8.0) >= DIFF_MIN_COLUMNS,
                "the file has {} columns at {w}x{h}, under the {DIFF_MIN_COLUMNS} floor",
                cols_of(diff, 8.0)
            );
            assert!(
                cols_of(list, 8.0) <= LIST_MAX_COLUMNS,
                "the list is {} columns wide at {w}x{h}",
                cols_of(list, 8.0)
            );
        }
    }
    /// Where a character is, in the file view, is measured from the file's own
    /// box and not from the pane. The list is not text to be selected: a drag
    /// starting on a row is a drag on the list, and hit testing the whole pane
    /// would put every click in the file a list's width away from the glyph
    /// under it.
    #[test]
    fn the_file_s_text_is_measured_from_its_own_column() {
        let paths = ["src/calc.py", "src/main.rs"];
        let state = touched(&paths);
        let names = labels(&paths);
        let out = render(
            &state,
            1400.0,
            900.0,
            &a_dock_showing(View::Files),
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let space = Space::TopLeft;
        assert_eq!(out.layout.content(space), out.layout.file_diff);
        // The other spaces are unchanged: their content is their whole body.
        assert_eq!(
            out.layout.content(Space::TopRight),
            out.layout.placed(Space::TopRight).body
        );

        let diff = out.layout.file_diff.inset(PAD);
        let (row, at) = (3usize, 6usize);
        let line = Text::line_for(13.0);
        let cell = out.layout.cell(
            diff.x + at as f32 * 8.0,
            diff.y + row as f32 * line + 2.0,
            13.0,
            8.0,
        );
        assert_eq!(cell, Some((space, row, at)), "measured from the wrong box");

        // And a point on a row of the list is no cell at all.
        let (_, first) = out.layout.file_rows[0];
        assert_eq!(
            out.layout
                .cell(first.x + 4.0, first.y + first.h * 0.5, 13.0, 8.0),
            None
        );
    }
    /// The gutter in front of a file's text is chrome, not text. One place says
    /// how many columns it takes, so the wrapping the file is drawn with and the
    /// column a click resolves to cannot drift apart, which is what put file
    /// selection four characters along.
    #[test]
    fn a_file_s_line_numbers_are_not_part_of_its_line() {
        let box_ = Panel::new(0.0, 0.0, 8.0 * 40.0 + 2.0 * PAD, 100.0);
        assert_eq!(text_columns(View::Files, box_, 8.0), (40 - GUTTER, GUTTER));
        for view in View::ALL.into_iter().filter(|v| *v != View::Files) {
            assert_eq!(text_columns(view, box_, 8.0), (40, 0), "{view:?}");
        }
        // A box narrower than the gutter still wraps in at least one column.
        let sliver = Panel::new(0.0, 0.0, 8.0 + 2.0 * PAD, 100.0);
        assert_eq!(text_columns(View::Files, sliver, 8.0).0, 1);
    }
    /// The rows a file line wraps onto start under its text, not under its line
    /// number.
    ///
    /// The gutter is four columns the text never gets, and it is written once,
    /// on the first row of the line. The rows under it used to start at the
    /// left edge of the box, so they held four characters more than the
    /// arithmetic that counts the rows, places the caret and draws the band
    /// budgets for, and everything below the first row of a wrapped file line
    /// was four columns out. Every row is the same width now: the gutter, then
    /// exactly the characters the pane says that row is showing.
    #[test]
    fn a_wrapped_file_line_continues_under_its_own_text() {
        let long = "let total = numbers.iter().filter(|n| **n > 0).map(|n| n * 2).sum::<i64>(); \
                    // and a comment on the end of it with plenty of blanks to break at";
        let (state, names) = a_wrapped_file(&[(7, long), (8, "fn main() {}"), (9, "")]);
        let out = render(
            &state,
            1400.0,
            900.0,
            &a_dock_showing(View::Files),
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let body = out.layout.file_diff;
        let (cols, chrome) = text_columns(View::Files, body, 8.0);
        let pane = &state.files[0].pane;
        let rows = out.layout.rows(body, 13.0);
        let window = pane.window(rows, cols);
        assert_eq!(window.skip, 0, "the file is meant to fit in the pane");

        let text = out
            .scene
            .texts
            .iter()
            .find(|text| text.at == body.inset(PAD) && text.wrap_cols.is_some())
            .expect("the file draws its text");
        assert_eq!(text.wrap_cols, Some(cols), "the box wraps in the columns the pane counts");
        assert_eq!(text.wrap_indent, chrome, "the box does not keep the gutter clear");

        // The rows the renderer will lay out, which is what the reader sees.
        let laid: String =
            noob_draw::Run::wrapped_under(&text.runs, cols, text.wrap_break, text.wrap_indent)
                .iter()
                .map(|run| run.text.as_str())
                .collect();
        let drawn: Vec<Vec<char>> = laid.split('\n').map(|row| row.chars().collect()).collect();

        let mut wrapped = 0;
        let mut previous = None;
        for (row, on_screen) in drawn.iter().enumerate().take(rows) {
            let Some((line, start)) = pane.spot_in(rows, cols, row, 0) else {
                break;
            };
            let (same, end) = pane
                .spot_in(rows, cols, row, cols + 9)
                .expect("the row a moment ago is still a row");
            assert_eq!(same, line, "row {row} lands on two different lines");
            let source = pane.line(line).expect("a row of a line the pane holds");
            let shown: Vec<char> = source.text.chars().take(end).skip(start).collect();

            let (gutter, after) = on_screen.split_at(chrome.min(on_screen.len()));
            assert_eq!(after, shown, "screen row {row} is not the characters the pane says");
            assert!(
                on_screen.len() <= chrome + cols,
                "screen row {row} is wider than the box"
            );
            match previous == Some(line) {
                // A row that continues a line is blank where the number was.
                true => {
                    assert!(
                        gutter.iter().all(|ch| *ch == ' ') && gutter.len() == chrome,
                        "row {row} continues line {line} under {gutter:?} instead of the text"
                    );
                    wrapped += 1;
                }
                // The first row of a line carries the number, once. A line the
                // file did not number is blank there, and an empty line with
                // no number has nothing on it at all.
                false => {
                    let head: String = gutter.iter().collect();
                    let want = match (source.number, source.text.is_empty()) {
                        (Some(number), _) => file_number(number, chrome),
                        (None, false) => " ".repeat(chrome),
                        (None, true) => String::new(),
                    };
                    assert_eq!(head, want, "row {row} of line {line} has the wrong gutter");
                }
            }
            previous = Some(line);
        }
        assert!(wrapped >= 2, "only {wrapped} rows continued a wrapped line");
    }
    /// A number wider than the gutter is still exactly the gutter, because the
    /// width of the gutter is what every row of the line is indented by.
    #[test]
    fn a_line_number_is_written_in_exactly_the_columns_the_gutter_has() {
        assert_eq!(file_number(7, GUTTER), "007 ");
        assert_eq!(file_number(120, GUTTER), "120 ");
        // Past three digits the blank goes, and past four the number says it
        // was cut rather than reading as another line's number.
        assert_eq!(file_number(1204, GUTTER), "1204");
        assert_eq!(file_number(12040, GUTTER), "120\u{2026}");
        for number in [1u32, 9, 10, 999, 1000, 9999, 10_000, 999_999] {
            assert_eq!(
                file_number(number, GUTTER).chars().count(),
                GUTTER,
                "line {number} does not fill the gutter"
            );
        }
    }
    /// The band over a wrapped file line covers the glyphs on every row of it,
    /// including the rows that continue it.
    ///
    /// The band was measured in the full width of the box while the text was
    /// laid out in the width the gutter leaves, so it started four columns left
    /// of the first character it was highlighting and, on a continuation row,
    /// covered the indent instead of the text.
    #[test]
    fn the_band_over_a_wrapped_file_line_covers_the_glyphs() {
        let long = "let total = numbers.iter().filter(|n| **n > 0).map(|n| n * 2).sum::<i64>(); \
                    // and a comment on the end of it with plenty of blanks to break at";
        let (state, names) = a_wrapped_file(&[(7, long)]);
        let files = names.iter().map(String::as_str).collect::<Vec<_>>();
        let line = state.files[0].pane.last() - 1;
        let chars = long.chars().count();
        let selection = {
            let mut selection =
                crate::select::Selection::new(crate::select::Where::Pane(View::Files), crate::select::Spot::new(line, 0));
            selection.extend(crate::select::Spot::new(line, chars));
            selection
        };

        let dock = a_dock_showing(View::Files);
        let shape = shape(&dock, &files);
        let layout = Layout::compute(1400.0, 900.0, &shape);
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
            selection: Some(selection),
            menu: None,
            picker: None,
            settings: None,
        });

        let body = layout.file_diff;
        let content = body.inset(PAD);
        let (cols, chrome) = text_columns(View::Files, body, 8.0);
        let pane = &state.files[0].pane;
        let rows = layout.rows(body, 13.0);
        let (top, height) = pane.band_of(rows, cols, line).expect("the line is on screen");
        let spans = pane.rows_of_line(line, cols);
        assert!(height > 1, "the line under test does not wrap");

        let mut bands: Vec<[f32; 4]> = scene
            .rects
            .iter()
            .filter(|rect| rect.rgba() == skin.select)
            .map(|rect| rect.xywh())
            .collect();
        bands.sort_by(|a, b| a[1].partial_cmp(&b[1]).unwrap());
        assert_eq!(bands.len(), height, "one band per row of the line: {bands:?}");

        let line_h = Text::line_for(13.0);
        for (i, band) in bands.iter().enumerate() {
            let span = spans[i];
            // Past the gutter on the first row and past the indent under it on
            // the rest, and exactly as wide as the characters drawn there.
            assert!(
                (band[0] - (content.x + chrome as f32 * 8.0)).abs() < 0.01,
                "row {i} of the band starts at {} and the text at {}",
                band[0],
                content.x + chrome as f32 * 8.0
            );
            assert!(
                (band[2] - span.len() as f32 * 8.0).abs() < 0.01,
                "row {i} of the band is {} wide over {} characters",
                band[2],
                span.len()
            );
            assert!(
                (band[1] - (content.y + (top + i) as f32 * line_h)).abs() < 0.01,
                "row {i} of the band is on the wrong row"
            );
        }
    }
    #[test]
    fn the_file_strip_says_so_when_there_are_no_files() {
        let text = text_of(&render(&State::new(), 1200.0, 800.0, &a_dock_showing(View::Files), &[]).scene);
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
        let out = render(&state, 1400.0, 900.0, &a_dock_showing(View::Files), &["calc.py"]);
        let colors: Vec<Option<[u8; 4]>> = out
            .scene
            .texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| r.color))
            .collect();
        assert!(colors.contains(&Some(out.skin.string)), "the string is tinted");
        assert!(colors.contains(&Some(out.skin.comment)), "the comment is tinted");
    }
    #[test]
    fn a_changed_file_is_marked_in_its_tab() {
        let text = text_of(&render(&busy_state(), 1400.0, 900.0, &a_dock_showing(View::Files), &["calc.py"]).scene);
        assert!(text.contains("calc.py \u{2022}"), "{text}");
    }
}
