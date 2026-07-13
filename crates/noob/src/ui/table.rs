//! Bounded, width-aware rendering for the small GFM table subset recognized by
//! [`super::markdown`]. Parsing and layout live here so the streaming parser only
//! has to decide when a table starts and ends.
//!
//! The bounds limit buffering, not output. When a table reaches either bound the
//! markdown driver renders the buffered rows and streams the remainder as plain
//! source, so model output is never truncated.

use super::style::{ColorDepth, RESET, Style};
use super::theme::Theme;

/// A table is deliberately much smaller than a model response. Hitting a bound
/// degrades to readable source instead of retaining an unbounded region.
pub(super) const MAX_TABLE_BYTES: usize = 32 * 1024;
pub(super) const MAX_TABLE_ROWS: usize = 128;
pub(super) const MAX_TABLE_COLUMNS: usize = 16;

const MIN_GRID_CELL: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Alignment {
    Left,
    Center,
    Right,
}

/// A confirmed table. The source header and delimiter have already been
/// consumed; `rows` contains only body rows.
pub(super) struct Table {
    header: Vec<String>,
    align: Vec<Alignment>,
    rows: Vec<Vec<String>>,
    buffered_bytes: usize,
}

impl Table {
    /// Confirm `header` with the following GFM delimiter row. Returns `None`
    /// for lookalikes so the markdown driver can emit both lines normally.
    pub(super) fn new(header: &str, delimiter: &str) -> Option<Table> {
        let header_cells = split_row(header);
        if header_cells.is_empty()
            || header_cells.len() > MAX_TABLE_COLUMNS
            || !has_structural_pipe(header)
        {
            return None;
        }
        let delimiter_cells = split_row(delimiter);
        if delimiter_cells.len() != header_cells.len() {
            return None;
        }
        let mut align = Vec::with_capacity(delimiter_cells.len());
        for cell in &delimiter_cells {
            align.push(parse_alignment(cell)?);
        }
        Some(Table {
            header: header_cells,
            align,
            rows: Vec::new(),
            buffered_bytes: header.len() + delimiter.len(),
        })
    }

    pub(super) fn columns(&self) -> usize {
        self.header.len()
    }

    /// Whether `line` is a body row for this table. A blank line or ordinary
    /// prose ends the region. Extra cells are accepted and folded into the last
    /// column so display never drops content.
    pub(super) fn accepts_row(&self, line: &str) -> bool {
        !line.trim().is_empty() && has_structural_pipe(line) && !split_row(line).is_empty()
    }

    /// Buffer one body row. `Err(())` means the caller must flush this table and
    /// display this and following rows without table buffering.
    pub(super) fn push_row(&mut self, line: &str) -> Result<(), ()> {
        if self.rows.len() >= MAX_TABLE_ROWS
            || self.buffered_bytes.saturating_add(line.len()) > MAX_TABLE_BYTES
        {
            return Err(());
        }
        let cells = normalize_row(split_row(line), self.columns());
        self.buffered_bytes += line.len();
        self.rows.push(cells);
        Ok(())
    }

    /// Render as a wrapped grid when every column can remain useful. A narrow
    /// terminal falls back to stacked `Header: value` records.
    pub(super) fn render(&self, width: usize, theme: &Theme, depth: ColorDepth) -> String {
        let display_header: Vec<String> = self.header.iter().map(|s| display_cell(s)).collect();
        let display_rows: Vec<Vec<String>> = self
            .rows
            .iter()
            .map(|row| row.iter().map(|s| display_cell(s)).collect())
            .collect();

        let cols = display_header.len();
        // Each grid cell costs two padding spaces and one separator; the final
        // border costs one more column.
        let framing = cols.saturating_mul(3).saturating_add(1);
        let content_budget = width.saturating_sub(framing);
        if cols == 0 || content_budget < cols.saturating_mul(MIN_GRID_CELL) {
            return render_stacked(&display_header, &display_rows, width, theme, depth);
        }

        let mut natural = vec![MIN_GRID_CELL; cols];
        for (idx, cell) in display_header.iter().enumerate() {
            natural[idx] = natural[idx].max(cell_width(cell));
        }
        for row in &display_rows {
            for (idx, cell) in row.iter().enumerate().take(cols) {
                natural[idx] = natural[idx].max(cell_width(cell));
            }
        }
        let widths = fit_widths(natural, content_budget);
        render_grid(
            &display_header,
            &display_rows,
            &self.align,
            &widths,
            theme,
            depth,
        )
    }
}

/// A possible table header is held for one line while the markdown driver waits
/// for the delimiter. Escaped pipes and pipes inside code spans do not count.
pub(super) fn could_be_header(line: &str) -> bool {
    if !has_structural_pipe(line) {
        return false;
    }
    let cells = split_row(line);
    (1..=MAX_TABLE_COLUMNS).contains(&cells.len())
        && cells.iter().any(|cell| !cell.trim().is_empty())
        && !cells.iter().all(|cell| parse_alignment(cell).is_some())
}

/// Used after a buffering overflow to decide when raw table-source rows end.
pub(super) fn looks_like_body_row(line: &str, _columns: usize) -> bool {
    !line.trim().is_empty() && has_structural_pipe(line)
}

fn parse_alignment(cell: &str) -> Option<Alignment> {
    let trimmed = cell.trim();
    let left = trimmed.starts_with(':');
    let right = trimmed.ends_with(':');
    let core = trimmed.trim_matches(':');
    if core.len() < 3 || !core.bytes().all(|b| b == b'-') {
        return None;
    }
    Some(match (left, right) {
        (true, true) => Alignment::Center,
        (false, true) => Alignment::Right,
        _ => Alignment::Left,
    })
}

/// Split a GFM row. Leading/trailing pipes are decoration, escaped pipes and
/// pipes inside a backtick code span stay in the cell.
fn split_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut escaped = false;
    let mut in_code = false;
    for ch in trimmed.chars() {
        if escaped {
            cell.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => {
                escaped = true;
                cell.push(ch);
            }
            '`' => {
                in_code = !in_code;
                cell.push(ch);
            }
            '|' if !in_code => {
                cells.push(cell.trim().to_string());
                cell.clear();
            }
            _ => cell.push(ch),
        }
    }
    if escaped {
        cell.push('\\');
    }
    cells.push(cell.trim().to_string());
    if trimmed.starts_with('|') && cells.first().is_some_and(String::is_empty) {
        cells.remove(0);
    }
    if trimmed.ends_with('|') && cells.last().is_some_and(String::is_empty) {
        cells.pop();
    }
    cells
}

fn has_structural_pipe(line: &str) -> bool {
    let mut escaped = false;
    let mut in_code = false;
    for ch in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '`' => in_code = !in_code,
            '|' if !in_code => return true,
            _ => {}
        }
    }
    false
}

fn normalize_row(mut cells: Vec<String>, columns: usize) -> Vec<String> {
    if cells.len() > columns && columns > 0 {
        let extras = cells.split_off(columns - 1);
        cells.push(extras.join(" | "));
    }
    cells.resize(columns, String::new());
    cells
}

/// Remove paired inline markdown delimiters for table layout. This deliberately
/// handles only the inline subset rendered by markdown.rs and preserves an
/// unmatched marker literally.
fn display_cell(input: &str) -> String {
    let mut out = input.to_string();
    for marker in ["**", "__", "`", "*", "_"] {
        while let Some(start) = out.find(marker) {
            let from = start + marker.len();
            let Some(rel_end) = out[from..].find(marker) else {
                break;
            };
            let end = from + rel_end;
            out.replace_range(end..end + marker.len(), "");
            out.replace_range(start..start + marker.len(), "");
        }
    }
    // Cells bypass the plain-path control sanitizer, so a literal tab or other
    // control here would measure as one scalar but render as several columns and
    // desync the row. Collapse every control char (including `\t`) to a single
    // space so measurement and output agree. Runs before both.
    out.replace("\\|", "|")
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn fit_widths(mut widths: Vec<usize>, budget: usize) -> Vec<usize> {
    while widths.iter().sum::<usize>() > budget {
        let Some((idx, _)) = widths
            .iter()
            .enumerate()
            .filter(|(_, width)| **width > MIN_GRID_CELL)
            .max_by_key(|(_, width)| **width)
        else {
            break;
        };
        widths[idx] -= 1;
    }
    widths
}

fn render_grid(
    header: &[String],
    rows: &[Vec<String>],
    alignments: &[Alignment],
    widths: &[usize],
    theme: &Theme,
    depth: ColorDepth,
) -> String {
    let mut out = String::new();
    push_rule(&mut out, ('┌', '┬', '┐'), widths, theme.activity, depth);
    push_grid_row(
        &mut out,
        header,
        widths,
        alignments,
        theme.prompt,
        theme.activity,
        depth,
    );
    push_rule(&mut out, ('├', '┼', '┤'), widths, theme.activity, depth);
    for row in rows {
        push_grid_row(
            &mut out,
            row,
            widths,
            alignments,
            theme.assistant,
            theme.activity,
            depth,
        );
    }
    push_rule(&mut out, ('└', '┴', '┘'), widths, theme.activity, depth);
    out
}

fn push_rule(
    out: &mut String,
    joints: (char, char, char),
    widths: &[usize],
    style: Style,
    depth: ColorDepth,
) {
    let (left, join, right) = joints;
    let mut line = String::new();
    line.push(left);
    for (idx, width) in widths.iter().enumerate() {
        line.extend(std::iter::repeat_n('─', width + 2));
        line.push(if idx + 1 == widths.len() { right } else { join });
    }
    out.push_str(&paint(style, depth, &line));
    out.push('\n');
}

fn push_grid_row(
    out: &mut String,
    cells: &[String],
    widths: &[usize],
    alignments: &[Alignment],
    cell_style: Style,
    border_style: Style,
    depth: ColorDepth,
) {
    let wrapped: Vec<Vec<String>> = widths
        .iter()
        .enumerate()
        .map(|(idx, width)| wrap_text(cells.get(idx).map_or("", String::as_str), *width))
        .collect();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
    for row_idx in 0..height {
        out.push_str(&paint(border_style, depth, "│"));
        for (col, width) in widths.iter().enumerate() {
            let text = wrapped[col].get(row_idx).map_or("", String::as_str);
            let padded = align_text(
                text,
                *width,
                alignments.get(col).copied().unwrap_or(Alignment::Left),
            );
            out.push(' ');
            out.push_str(&paint(cell_style, depth, &padded));
            out.push(' ');
            out.push_str(&paint(border_style, depth, "│"));
        }
        out.push('\n');
    }
}

fn render_stacked(
    header: &[String],
    rows: &[Vec<String>],
    width: usize,
    theme: &Theme,
    depth: ColorDepth,
) -> String {
    let mut out = String::new();
    let value_width = width.saturating_sub(4).max(8);
    if rows.is_empty() {
        out.push_str(&paint(theme.activity, depth, "┌─ table columns"));
        out.push('\n');
        for name in header {
            out.push_str(&paint(theme.activity, depth, "│ "));
            out.push_str(&paint(theme.prompt, depth, name));
            out.push('\n');
        }
        out.push_str(&paint(theme.activity, depth, "└─"));
        out.push('\n');
        return out;
    }
    for (row_idx, row) in rows.iter().enumerate() {
        out.push_str(&paint(
            theme.activity,
            depth,
            &format!("┌─ row {}", row_idx + 1),
        ));
        out.push('\n');
        for (col, name) in header.iter().enumerate() {
            let value = row.get(col).map_or("", String::as_str);
            let label = format!("{name}: ");
            let available = value_width.saturating_sub(cell_width(&label));
            if available >= 3 {
                let lines = wrap_text(value, available);
                out.push_str(&paint(theme.activity, depth, "│ "));
                out.push_str(&paint(theme.prompt, depth, &label));
                out.push_str(&paint(theme.assistant, depth, &lines[0]));
                out.push('\n');
                for line in &lines[1..] {
                    out.push_str(&paint(theme.activity, depth, "│ "));
                    out.push_str(&" ".repeat(cell_width(&label)));
                    out.push_str(&paint(theme.assistant, depth, line));
                    out.push('\n');
                }
            } else {
                out.push_str(&paint(theme.activity, depth, "│ "));
                out.push_str(&paint(theme.prompt, depth, name));
                out.push('\n');
                for line in wrap_text(value, value_width.saturating_sub(2).max(4)) {
                    out.push_str(&paint(theme.activity, depth, "│   "));
                    out.push_str(&paint(theme.assistant, depth, &line));
                    out.push('\n');
                }
            }
        }
        out.push_str(&paint(theme.activity, depth, "└─"));
        out.push('\n');
    }
    out
}

fn align_text(text: &str, width: usize, alignment: Alignment) -> String {
    let used = cell_width(text).min(width);
    let spare = width.saturating_sub(used);
    let (left, right) = match alignment {
        Alignment::Left => (0, spare),
        Alignment::Right => (spare, 0),
        Alignment::Center => (spare / 2, spare - spare / 2),
    };
    format!("{}{}{}", " ".repeat(left), text, " ".repeat(right))
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let word_width = cell_width(word);
        let current_width = cell_width(&current);
        if word_width > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            // Hard-break by accumulated DISPLAY width, not a fixed char count, so
            // a run of 2-column glyphs cannot overflow the column.
            let mut chunk = String::new();
            let mut chunk_width = 0;
            for ch in word.chars() {
                let cw = char_width(ch);
                if chunk_width + cw > width && !chunk.is_empty() {
                    lines.push(std::mem::take(&mut chunk));
                    chunk_width = 0;
                }
                chunk.push(ch);
                chunk_width += cw;
            }
            if !chunk.is_empty() {
                lines.push(chunk);
            }
            continue;
        }
        let separator = usize::from(!current.is_empty());
        if current_width + separator + word_width > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

pub(super) fn cell_width(s: &str) -> usize {
    // The single source of truth for column sizing, wrapping, and padding. It
    // sums terminal display columns rather than Unicode scalars so a wide glyph
    // or a combining mark cannot shove a row's right border off the shared column.
    s.chars().map(char_width).sum()
}

/// Terminal display columns for one `char`: 0 for zero-width/combining marks,
/// 2 for East-Asian Wide/Fullwidth and common emoji, 1 otherwise. This is a
/// hand-rolled range check (no extra crates), covering the ranges noob actually
/// emits in tables rather than the full UAX #11 table. On ASCII it equals the
/// old scalar count, so ASCII layouts are unchanged. Shared with the dock's
/// pinned-region clamp so a wide glyph cannot wrap a region row to two physical
/// rows and desync the frame height.
pub(super) fn char_width(c: char) -> usize {
    let c = c as u32;
    // Zero-width: combining diacritics, zero-width spaces/joiner (incl. U+200D),
    // variation selectors, and the BOM.
    if matches!(c,
        0x0300..=0x036F
        | 0x200B..=0x200F
        | 0xFE00..=0xFE0F
        | 0xFEFF
    ) {
        return 0;
    }
    // Wide/Fullwidth East-Asian blocks and common emoji.
    if matches!(c,
        0x1100..=0x115F   // Hangul Jamo
        | 0x2600..=0x26FF // Miscellaneous Symbols
        | 0x2700..=0x27BF // Dingbats
        | 0x2E80..=0x303E // CJK radicals, Kangxi, punctuation
        | 0x3041..=0x33FF // Kana, CJK symbols, enclosed CJK
        | 0x3400..=0x4DBF // CJK Ext A
        | 0x4E00..=0x9FFF // CJK Unified
        | 0xA000..=0xA4CF // Yi
        | 0xAC00..=0xD7A3 // Hangul syllables
        | 0xF900..=0xFAFF // CJK compatibility ideographs
        | 0xFE30..=0xFE4F // CJK compatibility forms
        | 0xFF00..=0xFF60 // Fullwidth forms
        | 0xFFE0..=0xFFE6 // Fullwidth signs
        | 0x1F300..=0x1FAFF // Emoji (Misc symbols/pictographs .. Symbols/pictographs ext-A)
    ) {
        return 2;
    }
    1
}

fn paint(style: Style, depth: ColorDepth, text: &str) -> String {
    let open = style.sgr(depth);
    if open.is_empty() {
        text.to_string()
    } else {
        format!("{open}{text}{RESET}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(s: &str) -> String {
        let mut out = String::new();
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'[') {
                i += 2;
                while i < bytes.len() {
                    let b = bytes[i];
                    i += 1;
                    if (0x40..=0x7e).contains(&b) {
                        break;
                    }
                }
            } else {
                let ch = s[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
        out
    }

    #[test]
    fn confirms_gfm_header_and_alignment() {
        let table = Table::new("| name | score | note |", "| :--- | ---: | :---: |").unwrap();
        assert_eq!(
            table.align,
            vec![Alignment::Left, Alignment::Right, Alignment::Center]
        );
        assert!(table.accepts_row("| Ada | 10 | ok |"));
        assert!(!table.accepts_row("ordinary prose"));
        assert!(Table::new("not a table", "---").is_none());
        assert!(
            Table::new("a | b", "-- | ---").is_none(),
            "delimiter needs three dashes"
        );
    }

    #[test]
    fn escaped_and_code_span_pipes_stay_inside_cells() {
        assert_eq!(
            split_row(r"| a\|b | `x|y` | z |"),
            vec![r"a\|b", "`x|y`", "z"]
        );
        assert!(!has_structural_pipe(r"only \| escaped"));
        assert!(!has_structural_pipe("`only | code`"));
    }

    #[test]
    fn extra_cells_fold_into_the_last_column_without_loss() {
        let row = normalize_row(split_row("a | b | c | d"), 2);
        assert_eq!(row, vec!["a", "b | c | d"]);
    }

    #[test]
    fn wide_table_is_a_wrapped_grid_with_balanced_styles() {
        let mut table = Table::new("name | description", ":--- | ---:").unwrap();
        table
            .push_row("alpha | a description that wraps cleanly into several words")
            .unwrap();
        table.push_row("beta | short").unwrap();
        let rendered = table.render(48, &Theme::matrix(), ColorDepth::Truecolor);
        let stripped = plain(&rendered);
        assert!(stripped.contains('┌') && stripped.contains('┬') && stripped.contains('┘'));
        assert!(stripped.contains("alpha"));
        assert!(stripped.contains("description"));
        assert!(rendered.matches("\x1b[").count() > 4);
        assert_eq!(
            rendered.matches("\x1b[0m").count() * 2,
            rendered.matches("\x1b[").count()
        );
        for line in stripped.lines() {
            assert!(line.chars().count() <= 48, "line escaped width: {line:?}");
        }
    }

    #[test]
    fn narrow_table_falls_back_to_stacked_records() {
        let mut table = Table::new("name | description | state", "--- | --- | ---").unwrap();
        table
            .push_row("alpha | long descriptive words | ready")
            .unwrap();
        let rendered = plain(&table.render(18, &Theme::matrix(), ColorDepth::Truecolor));
        assert!(rendered.contains("row 1"));
        assert!(rendered.contains("name:"));
        assert!(rendered.contains("description"));
        assert!(
            !rendered.contains('┬'),
            "narrow mode must not squeeze a grid"
        );
    }

    #[test]
    fn table_buffer_bounds_reject_before_adding_and_do_not_truncate_existing_rows() {
        let mut table = Table::new("a | b", "--- | ---").unwrap();
        for i in 0..MAX_TABLE_ROWS {
            table.push_row(&format!("{i} | value")).unwrap();
        }
        assert!(
            table
                .push_row("overflow | still belongs to caller")
                .is_err()
        );
        assert_eq!(table.rows.len(), MAX_TABLE_ROWS);
        let shown = plain(&table.render(40, &Theme::matrix(), ColorDepth::None));
        assert!(shown.contains(&(MAX_TABLE_ROWS - 1).to_string()));
    }

    #[test]
    fn inline_markers_are_removed_only_when_paired() {
        assert_eq!(display_cell("**bold** and `code`"), "bold and code");
        assert_eq!(display_cell("an * unmatched"), "an * unmatched");
    }

    #[test]
    fn wide_glyph_rows_keep_right_borders_aligned() {
        // OLD behavior: cell_width counted Unicode scalars, so a 2-column glyph
        // like 世 padded as if it were 1 column and that row's right │ landed one
        // cell left of the ASCII rows, giving a ragged right border. With
        // display-width padding every rendered line is the same terminal width.
        let mut table = Table::new("name | note", "--- | ---").unwrap();
        table.push_row("ascii | plain").unwrap();
        table.push_row("cjk | 世界").unwrap(); // two 2-column glyphs
        table.push_row("emoji | 🚀 go").unwrap(); // one 2-column emoji
        table.push_row("combine | e\u{0301}").unwrap(); // e + combining acute = 1 col
        let stripped = plain(&table.render(40, &Theme::matrix(), ColorDepth::None));
        let widths: Vec<usize> = stripped.lines().map(cell_width).collect();
        assert!(widths.len() > 4);
        for (line, w) in stripped.lines().zip(&widths) {
            assert_eq!(*w, widths[0], "ragged right border on line {line:?}");
        }
    }

    #[test]
    fn tab_in_cell_does_not_desync_the_row() {
        // A literal tab used to be emitted verbatim: it counts as one scalar for
        // padding but renders as several columns, shoving the right border out.
        // It is now collapsed to a space so measurement and output agree.
        let mut table = Table::new("k | v", "--- | ---").unwrap();
        table.push_row("plain | ab").unwrap();
        table.push_row("tabbed | a\tb").unwrap();
        let stripped = plain(&table.render(40, &Theme::matrix(), ColorDepth::None));
        let widths: Vec<usize> = stripped.lines().map(cell_width).collect();
        assert!(
            widths.iter().all(|w| *w == widths[0]),
            "rows desynced: {widths:?}"
        );
        assert!(
            !stripped.contains('\t'),
            "tab must be sanitized out of cells"
        );
    }

    #[test]
    fn char_width_matches_declared_ranges() {
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('世'), 2);
        assert_eq!(char_width('🚀'), 2);
        assert_eq!(char_width('\u{0301}'), 0); // combining acute
        assert_eq!(char_width('\u{200D}'), 0); // zero-width joiner
        assert_eq!(cell_width("e\u{0301}"), 1);
        assert_eq!(cell_width("世界"), 4);
    }
}
