//! A Markdown table, laid out for the box it is drawn in.
//!
//! The model writes tables, and a transcript that prints `| a | b |` as it came
//! wraps every row mid-cell into something no reader can line up. This turns a
//! block of pipe rows into the columns it describes.
//!
//! One source row lays out as one string, which is what lets the pane keep
//! counting lines the way it always has: the string carries the newlines the
//! cells wrapped at, and `text-geometry` counts a newline as the end of a row,
//! so the rows a table row is drawn as and the rows it is measured in are the
//! same rows.
//!
//! Width decides the shape. While every column can hold something useful the
//! block is a grid with its cells wrapped inside their columns. Under that it
//! is a list: one `Header: value` line per cell, wrapping as prose. Nothing is
//! ever cut, because a table in a narrow panel is still the answer to
//! something.

use crate::markdown;

/// Below this a column holds fragments rather than words, and the grid is worth
/// less than the list. A table of short cells still gets its grid in a narrow
/// panel, because it is measured on what its cells actually need first.
const MIN_CELL: usize = 12;
/// A wider table than this is not a table any reader is following.
pub const MAX_COLUMNS: usize = 12;
/// And a longer one is a listing. The rows past it are shown as the model
/// wrote them: measuring a block has to walk it, and a walk that grows with a
/// pane full of pipes is a frame that gets slower the longer a session runs.
pub const MAX_ROWS: usize = 200;

const GAP: &str = " │ ";
const RULE: &str = "─┼─";

/// Which line of a table block a source line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The row naming the columns.
    Head,
    /// The `|---|:--:|` row under it, drawn as the rule it stands for.
    Rule,
    /// One row of the body.
    Body,
}

/// Where the cells of a column sit in their width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Align {
    Left,
    Center,
    Right,
}

/// Whether `line` is shaped like a row of a table: pipes with cells between
/// them. A pipe inside a code span or escaped with a backslash is text.
pub fn is_row(line: &str) -> bool {
    !cells(line).is_empty()
}

/// Whether `line` is the `|---|:--:|` row that confirms the table.
pub fn is_rule(line: &str) -> bool {
    let cells = cells(line);
    !cells.is_empty() && cells.iter().all(|cell| align(cell).is_some())
}

/// Whether `head` opens a table that `rule` confirms.
pub fn opens(head: &str, rule: &str) -> bool {
    let head = cells(head);
    !head.is_empty()
        && head.len() <= MAX_COLUMNS
        && !head.iter().all(|cell| align(cell).is_some())
        && cells(rule).len() == head.len()
        && is_rule(rule)
}

/// The block laid out for a box `cols` wide: one drawn string per source line,
/// in the order they were given.
///
/// `source` starts at the head row and its rule; everything after them is body.
pub fn layout(source: &[&str], cols: usize) -> Vec<String> {
    let head: Vec<String> = cells(source.first().copied().unwrap_or_default())
        .iter()
        .map(|cell| markdown::inline_shown(cell))
        .collect();
    let width = head.len();
    let rows: Vec<Vec<String>> = source
        .iter()
        .skip(2)
        .map(|line| {
            let mut row: Vec<String> = cells(line)
                .iter()
                .map(|cell| markdown::inline_shown(cell))
                .collect();
            // A row with more cells than the head has keeps them, folded into
            // the last column: the model wrote them, so they are shown.
            if row.len() > width {
                let extra = row.split_off(width.max(1));
                if let Some(last) = row.last_mut() {
                    for cell in extra {
                        last.push(' ');
                        last.push_str(&cell);
                    }
                }
            }
            row.resize(width, String::new());
            row
        })
        .collect();
    let aligns: Vec<Align> = cells(source.get(1).copied().unwrap_or_default())
        .iter()
        .map(|cell| align(cell).unwrap_or(Align::Left))
        .collect();

    match widths(&head, &rows, cols) {
        Some(widths) => grid(source, &head, &rows, &aligns, &widths),
        None => list(source, &head, &rows, cols),
    }
}

/// The width of each column, or `None` when the box cannot hold a grid.
fn widths(head: &[String], rows: &[Vec<String>], cols: usize) -> Option<Vec<usize>> {
    let count = head.len();
    let framing = GAP.chars().count() * count.saturating_sub(1);
    let budget = cols.checked_sub(framing)?;
    if count == 0 {
        return None;
    }

    // What every column would take if nothing were in its way.
    let mut widths: Vec<usize> = head.iter().map(|cell| chars(cell)).collect();
    for row in rows {
        for (column, cell) in row.iter().enumerate() {
            widths[column] = widths[column].max(chars(cell));
        }
    }
    let mut over = widths.iter().sum::<usize>().saturating_sub(budget);
    // A grid nothing fits in is worse than a list. Asked after the naturals,
    // so a table of short cells keeps its columns in a narrow panel and only a
    // table of prose gives them up.
    if over > 0 && budget < count * MIN_CELL {
        return None;
    }
    // Otherwise the widest column gives up a character at a time, so what is
    // taken comes off the prose and not off the labels beside it.
    while over > 0 {
        let widest = widths
            .iter()
            .enumerate()
            .filter(|(_, width)| **width > MIN_CELL)
            .max_by_key(|(column, width)| (**width, std::cmp::Reverse(*column)))
            .map(|(column, _)| column)?;
        widths[widest] -= 1;
        over -= 1;
    }
    Some(widths)
}

/// The grid: cells wrapped inside their columns, one string per source line.
fn grid(
    source: &[&str],
    head: &[String],
    rows: &[Vec<String>],
    aligns: &[Align],
    widths: &[usize],
) -> Vec<String> {
    let mut out = Vec::with_capacity(source.len());
    out.push(row(head, aligns, widths));
    out.push(
        widths
            .iter()
            .map(|width| "─".repeat(*width))
            .collect::<Vec<String>>()
            .join(RULE),
    );
    for cells in rows {
        out.push(row(cells, aligns, widths));
    }
    out.truncate(source.len());
    out
}

/// One row of the grid: every cell wrapped in its column, the lines of the
/// cells laid beside each other.
fn row(cells: &[String], aligns: &[Align], widths: &[usize]) -> String {
    let wrapped: Vec<Vec<String>> = cells
        .iter()
        .enumerate()
        .map(|(column, cell)| broken(cell, widths[column]))
        .collect();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let mut lines = Vec::with_capacity(height);
    for line in 0..height {
        let mut text = String::new();
        for (column, cell) in wrapped.iter().enumerate() {
            if column > 0 {
                text.push_str(GAP);
            }
            let part = cell.get(line).map_or("", String::as_str);
            let align = aligns.get(column).copied().unwrap_or(Align::Left);
            pad(&mut text, part, widths[column], align);
        }
        // The blanks that pad the last column are room nobody can see, and a
        // row that ends in them is wider than the panel and wraps.
        lines.push(text.trim_end().to_string());
    }
    lines.join("\n")
}

/// The list: one `Header: value` line per cell, for a panel too narrow to hold
/// a column anything fits in.
fn list(source: &[&str], head: &[String], rows: &[Vec<String>], cols: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(source.len());
    // The head names columns that are not there any more; the rule under it is
    // still a rule, and it is what says a block starts here.
    out.push(String::new());
    out.push("─".repeat(cols.min(24)));
    let last = rows.len().saturating_sub(1);
    for (index, cells) in rows.iter().enumerate() {
        let mut lines: Vec<String> = cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| !cell.trim().is_empty())
            .map(|(column, cell)| match head.get(column) {
                Some(name) if !name.trim().is_empty() => format!("{name}: {cell}"),
                _ => cell.clone(),
            })
            .collect();
        // A blank row between records, so a reader can see where one ends.
        if index < last {
            lines.push(String::new());
        }
        out.push(lines.join("\n"));
    }
    out.truncate(source.len());
    out
}

/// The lines one cell takes inside a column `width` wide.
fn broken(cell: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = cell.trim().chars().collect();
    // Broken by the one wrap rule, so a cell breaks between words the way the
    // pane around it does.
    text_geometry::rows_in(cell.trim(), width, text_geometry::Break::Word)
        .iter()
        .map(|row| chars[row.start..row.end].iter().collect())
        .collect()
}

/// `text` in a column `width` wide, blanks either side of it as the column asks.
fn pad(out: &mut String, text: &str, width: usize, align: Align) {
    let room = width.saturating_sub(chars(text));
    let before = match align {
        Align::Left => 0,
        Align::Center => room / 2,
        Align::Right => room,
    };
    for _ in 0..before {
        out.push(' ');
    }
    out.push_str(text);
    for _ in 0..room - before {
        out.push(' ');
    }
}

/// The cells of one source row, or nothing when the line is not a row.
///
/// A row is pipes with something between them. The outer pipes are optional,
/// the way GitHub writes them, and a pipe inside a code span or behind a
/// backslash belongs to the text.
fn cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut code = false;
    let mut escaped = false;
    let mut split = false;
    for ch in trimmed.chars() {
        match ch {
            _ if escaped => {
                escaped = false;
                cell.push(ch);
            }
            '\\' => escaped = true,
            '`' => {
                code = !code;
                cell.push(ch);
            }
            '|' if !code => {
                cells.push(cell.trim().to_string());
                cell = String::new();
                split = true;
            }
            _ => cell.push(ch),
        }
    }
    cells.push(cell.trim().to_string());
    if !split || code {
        return Vec::new();
    }
    // `| a | b |` splits into an empty cell at each end. They are the border,
    // not a column.
    if cells.first().is_some_and(String::is_empty) {
        cells.remove(0);
    }
    if cells.last().is_some_and(String::is_empty) {
        cells.pop();
    }
    match cells.iter().all(String::is_empty) {
        true => Vec::new(),
        false => cells,
    }
}

/// How a `---`, `:--`, `--:` or `:-:` cell wants its column aligned.
fn align(cell: &str) -> Option<Align> {
    let cell = cell.trim();
    let body = cell.trim_start_matches(':').trim_end_matches(':');
    if body.is_empty() || !body.chars().all(|ch| ch == '-') {
        return None;
    }
    Some(match (cell.starts_with(':'), cell.ends_with(':')) {
        (true, true) => Align::Center,
        (false, true) => Align::Right,
        _ => Align::Left,
    })
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOOLS: [&str; 4] = [
        "| Tool | What it does |",
        "|---|---|",
        "| websearch | Search the web, fetch pages as Markdown, find papers |",
        "| skill | Load a specialized skill to guide your actions |",
    ];

    /// Every row of a laid-out block is one string, and every one of its lines
    /// is inside the box. A line wider than the panel is the whole defect: it
    /// wraps where the panel says rather than where the columns are.
    fn assert_inside(laid: &[String], cols: usize) {
        for line in laid {
            for row in line.split('\n') {
                assert!(
                    chars(row) <= cols,
                    "{row:?} is {} wide in a box of {cols}",
                    chars(row)
                );
            }
        }
    }

    #[test]
    fn a_head_and_its_rule_open_a_table_and_prose_does_not() {
        assert!(opens(TOOLS[0], TOOLS[1]));
        assert!(opens("a | b", "--- | ---"), "the outer pipes are optional");
        assert!(!opens("a | b", "not a rule"));
        assert!(!opens("no pipes here", "|---|"));
        assert!(!opens("| a | b |", "|---|---|---|"), "the rule must match");
        assert!(!is_row("a `b | c` d"), "a pipe inside code is text");
        assert!(!is_row("2 \\| 3"), "and so is an escaped one");
        assert!(is_row(TOOLS[2]));
        assert!(!is_row("plain prose"));
        assert!(is_rule("| :--- | ---: | :-: |"));
    }

    #[test]
    fn a_table_that_fits_is_a_grid_with_its_columns_lined_up() {
        let laid = layout(&TOOLS, 90);
        assert_inside(&laid, 90);
        assert_eq!(laid.len(), TOOLS.len(), "one line out per line in");
        assert!(laid[0].starts_with("Tool"), "{:?}", laid[0]);
        assert!(laid[1].contains('┼'), "the rule is drawn: {:?}", laid[1]);
        // The separator stands in the same column on every row.
        let at = |line: &str| line.find('│').expect("a separator");
        assert_eq!(at(&laid[0]), at(&laid[2]));
        assert_eq!(at(&laid[2]), at(&laid[3]));
        assert!(laid[2].contains("websearch"));
        assert!(!laid[2].contains('|'), "the source pipes are gone");
    }

    #[test]
    fn a_cell_too_wide_for_its_column_wraps_inside_it() {
        let laid = layout(&TOOLS, 48);
        assert_inside(&laid, 48);
        let websearch: Vec<&str> = laid[2].split('\n').collect();
        assert!(websearch.len() > 1, "the description wraps: {:?}", laid[2]);
        // The rows under the first keep the column, so the wrapped text stays
        // under the text it belongs to rather than under the name.
        let at = laid[0].find('│').expect("a separator");
        for row in &websearch {
            assert_eq!(row.chars().nth(at), Some('│'), "{row:?}");
        }
        // And what the model wrote is all still there.
        let read: String = websearch
            .iter()
            .map(|row| row[row.find('│').expect("a separator")..].trim_matches(['│', ' ']))
            .collect::<Vec<&str>>()
            .join(" ");
        assert_eq!(read, "Search the web, fetch pages as Markdown, find papers");
    }

    /// The list is prose: its lines are as long as the sentence is, and the
    /// pane wraps them the way it wraps any other line.
    #[test]
    fn a_panel_too_narrow_for_a_grid_gets_the_cells_as_a_list() {
        let laid = layout(&TOOLS, 20);
        assert_eq!(laid.len(), TOOLS.len());
        assert!(!laid[2].contains('│'), "no columns left: {:?}", laid[2]);
        assert!(laid[2].starts_with("Tool: websearch"), "{:?}", laid[2]);
        assert!(laid[2].contains("\nWhat it does: Search the web"), "{:?}", laid[2]);
        assert!(laid[2].ends_with('\n'), "a gap before the next record");
        assert!(!laid[3].ends_with('\n'), "and none after the last");
    }

    #[test]
    fn the_marks_inside_a_cell_are_rendered_like_any_other_text() {
        let source = ["| Tool | Note |", "|---|---|", "| **bash** | runs `ls` |"];
        let laid = layout(&source, 60);
        assert!(laid[2].contains("bash"), "{:?}", laid[2]);
        assert!(!laid[2].contains('*') && !laid[2].contains('`'), "{:?}", laid[2]);
    }

    #[test]
    fn the_rule_says_where_the_cells_sit_in_their_columns() {
        let source = ["| a | b | c |", "|:---|:---:|---:|", "| x | y | z |"];
        let laid = layout(&source, 60);
        let row = &laid[2];
        let cells: Vec<&str> = row.split('│').collect();
        assert!(cells[0].starts_with('x'), "left: {row:?}");
        assert!(cells[1].starts_with(' ') && cells[1].trim() == "y", "centre: {row:?}");
        assert!(cells[2].ends_with('z'), "right: {row:?}");
    }

    /// A model writes ragged tables. A row with a cell missing is padded and a
    /// row with one too many keeps it, because it was written to be read.
    #[test]
    fn a_ragged_row_neither_panics_nor_loses_a_cell() {
        let source = [
            "| a | b |",
            "|---|---|",
            "| only one |",
            "| one | two | three |",
            "||",
        ];
        let laid = layout(&source, 60);
        assert_eq!(laid.len(), source.len());
        assert!(laid[3].contains("two three"), "{:?}", laid[3]);
        assert_inside(&laid, 60);
    }

    #[test]
    fn a_box_mid_resize_lays_out_rather_than_dividing_by_zero() {
        for cols in [0, 1, 2, 5] {
            let laid = layout(&TOOLS, cols);
            assert_eq!(laid.len(), TOOLS.len(), "at {cols}");
        }
    }
}

