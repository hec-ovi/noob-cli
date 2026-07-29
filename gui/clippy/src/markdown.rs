//! The model writes Markdown. This renders it instead of showing the marks.
//!
//! A local model answers in headings, bold, bullets and fenced code whether or
//! not anything asked it to, so a transcript that prints `**read**` and
//! ```` ```python ```` verbatim is showing its working rather than its answer.
//!
//! Deliberately a formatter and not a parser. It works one line at a time, with
//! one bit of carried state (whether a fence is open), because a transcript is
//! rendered from a scrolling window and re-parsing six thousand lines to draw
//! forty is the wrong shape. Everything it does not recognise passes through
//! unchanged, which is the correct rendering for prose.

use noob_draw::Run;

use crate::skin::Skin;
use crate::syntax;

/// A fence that is open, and the language it named.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Fence(pub Option<String>);

impl Fence {
    /// Whether a block is open here.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn open(&self) -> bool {
        self.0.is_some()
    }

    /// Toggle on a fence line, returning whether this line was one.
    fn toggle(&mut self, line: &str) -> bool {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("```") && !trimmed.starts_with("~~~") {
            return false;
        }
        self.0 = match self.0 {
            Some(_) => None,
            None => Some(trimmed[3..].trim().to_ascii_lowercase()),
        };
        true
    }
}

/// Where a fence stands after `lines`, so a window that starts mid-block knows
/// it is looking at code.
pub fn fence_after<'a>(lines: impl Iterator<Item = &'a str>) -> Fence {
    let mut fence = Fence::default();
    for line in lines {
        fence.toggle(line);
    }
    fence
}

/// Whether a line is a row of a pipe table.
///
/// A leading pipe is required rather than merely allowed. Prose containing a
/// pipe is far more common than a table written without one, and guessing wrong
/// turns a sentence into a one-column table.
pub fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.len() > 1
}

/// The cells of a row, trimmed, with the outer pipes discarded.
fn cells(line: &str) -> Vec<&str> {
    let t = line.trim();
    let inner = t.strip_prefix('|').unwrap_or(t);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner.split('|').map(str::trim).collect()
}

/// Whether a row is the `|---|:--:|` rule under a header rather than content.
fn is_rule(line: &str) -> bool {
    let cells = cells(line);
    !cells.is_empty()
        && cells.iter().all(|c| {
            !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':' || ch == ' ')
        })
}

/// A table's shape: how wide each column has to be to hold its widest cell.
///
/// Computed over the whole block, including rows above the visible window, so
/// the columns do not jump about while a table is scrolled through.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Table {
    widths: Vec<usize>,
}

impl Table {
    /// Measure a block. Rule rows are skipped: their dashes say nothing about
    /// how wide the content is.
    pub fn of<'a>(rows: impl Iterator<Item = &'a str>) -> Table {
        let mut widths: Vec<usize> = Vec::new();
        for row in rows.filter(|r| !is_rule(r)) {
            for (i, cell) in cells(row).into_iter().enumerate() {
                let width = cell.chars().count();
                match widths.get_mut(i) {
                    Some(at) => *at = (*at).max(width),
                    None => widths.push(width),
                }
            }
        }
        Table { widths }
    }

    pub fn is_empty(&self) -> bool {
        self.widths.is_empty()
    }

    /// How many columns the whole table needs, edges included.
    ///
    /// `"| a | b |"` is two columns of content plus `"| "`, `" | "` and `" |"`,
    /// so three separators of two, three and two columns.
    pub fn width(&self) -> usize {
        if self.widths.is_empty() {
            return 0;
        }
        let content: usize = self.widths.iter().sum();
        // Two for the opening edge, two for the closing one, three between
        // every adjacent pair of columns.
        content + 4 + 3 * (self.widths.len() - 1)
    }

    /// Draw one row of the table, box-drawn and column aligned.
    ///
    /// A rule row becomes the box's own divider, so the rendering uses exactly
    /// the lines the source had: adding a top or bottom border would put a row
    /// on screen that no source line corresponds to, and every selection and
    /// scroll position is anchored to source lines.
    ///
    /// The row is a viewport onto the full table rather than a truncation of
    /// it: `skip` columns are scrolled off to the left and `room` columns are
    /// visible. A table is never wrapped, because a wrapped table is not a
    /// table, so scrolling sideways is the only way to read a wide one. An edge
    /// with more table beyond it is marked, or there would be nothing to say
    /// that scrolling is possible.
    pub fn row(&self, line: &str, skip: usize, room: usize, skin: &Skin, out: &mut Vec<Run>) {
        if room == 0 || self.widths.is_empty() {
            return;
        }
        let total = self.width();
        let skip = skip.min(total.saturating_sub(room));
        let more_left = skip > 0;
        let more_right = total > skip + room;
        let lead = usize::from(more_left);
        let span = room.saturating_sub(lead + usize::from(more_right));

        if more_left {
            out.push(Run::tinted("\u{2039}", skin.markup));
        }
        // The window into the row, in absolute table columns.
        let (from, to) = (skip + lead, skip + lead + span);
        let rule = is_rule(line);
        let cells = cells(line);
        let mut at = 0;

        let mut clipped = |text: &str, tint: [u8; 4], at: &mut usize| {
            let start = *at;
            let width = text.chars().count();
            *at += width;
            let a = from.max(start);
            let b = to.min(*at);
            if a < b {
                out.push(Run::tinted(
                    text.chars().skip(a - start).take(b - a).collect::<String>(),
                    tint,
                ));
            }
        };

        for (i, width) in self.widths.iter().enumerate() {
            let edge = match (i, rule) {
                (0, true) => "\u{251c}\u{2500}",
                (0, false) => "\u{2502} ",
                (_, true) => "\u{2500}\u{253c}\u{2500}",
                (_, false) => " \u{2502} ",
            };
            clipped(edge, skin.markup, &mut at);
            let body = match (rule, cells.get(i)) {
                (true, _) => "\u{2500}".repeat(*width),
                (false, Some(cell)) => {
                    let pad = width.saturating_sub(cell.chars().count());
                    format!("{cell}{}", " ".repeat(pad))
                }
                // A row with fewer cells than the widest one still has to fill
                // its columns, or the closing edge lands mid table.
                (false, None) => " ".repeat(*width),
            };
            clipped(&body, if rule { skin.markup } else { skin.body }, &mut at);
        }
        clipped(
            if rule { "\u{2500}\u{2524}" } else { " \u{2502}" },
            skin.markup,
            &mut at,
        );
        if more_right {
            out.push(Run::tinted("\u{203a}", skin.markup));
        }
    }
}

/// Append one line as runs, updating the fence state.
pub fn line(text: &str, fence: &mut Fence, skin: &Skin, out: &mut Vec<Run>) {
    if fence.toggle(text) {
        // The fence itself is structure, not content: show the language it
        // named rather than the backticks.
        let label = match &fence.0 {
            Some(lang) if !lang.is_empty() => format!("── {lang} "),
            Some(_) => String::from("── code "),
            None => String::from("──"),
        };
        out.push(Run::tinted(label, skin.comment));
        return;
    }
    if let Some(lang) = &fence.0 {
        let syntax = syntax::for_language(lang);
        for (fragment, token) in syntax::scan(text, syntax) {
            out.push(Run::tinted(fragment, skin.token(token).unwrap_or(skin.body)));
        }
        return;
    }

    let trimmed = text.trim_start();
    let indent = &text[..text.len() - trimmed.len()];

    // A heading: the hashes are the mark, not the text.
    if let Some(rest) = trimmed.strip_prefix('#') {
        let level = 1 + rest.chars().take_while(|c| *c == '#').count();
        let title = rest.trim_start_matches('#').trim_start();
        if !title.is_empty() || level > 1 {
            out.push(Run::tinted(indent, skin.body));
            out.push(Run::tinted(
                if level <= 2 { "▌ " } else { "  " },
                skin.markup,
            ));
            inline(title, if level <= 2 { skin.bright } else { skin.markup }, skin, out);
            return;
        }
    }
    // A horizontal rule, drawn rather than spelled.
    if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-' || c == '=' || c == '*') {
        out.push(Run::tinted(
            "─".repeat(trimmed.len().min(60)),
            skin.comment,
        ));
        return;
    }
    // A quote.
    if let Some(rest) = trimmed.strip_prefix("> ") {
        out.push(Run::tinted(format!("{indent}│ "), skin.comment));
        inline(rest, skin.dim, skin, out);
        return;
    }
    // A bullet or a numbered item: the marker is structure.
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            out.push(Run::tinted(format!("{indent}• "), skin.markup));
            inline(rest, skin.body, skin, out);
            return;
        }
    }
    if let Some((number, rest)) = numbered(trimmed) {
        out.push(Run::tinted(format!("{indent}{number} "), skin.markup));
        inline(rest, skin.body, skin, out);
        return;
    }

    out.push(Run::tinted(indent, skin.body));
    inline(trimmed, skin.body, skin, out);
}

/// `1. text` or `12) text`, returning the marker and the rest.
fn numbered(line: &str) -> Option<(&str, &str)> {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 3 {
        return None;
    }
    let after = &line[digits..];
    let rest = after.strip_prefix(". ").or_else(|| after.strip_prefix(") "))?;
    Some((&line[..digits + 1], rest))
}

/// `**bold**`, `*emphasis*` and `` `code` `` inside one line.
///
/// Scanned rather than parsed, so an unmatched marker is text. That matters:
/// prose is full of lone asterisks and stray backticks, and a parser that
/// insists on pairing them swallows the rest of the paragraph.
fn inline(text: &str, base: [u8; 4], skin: &Skin, out: &mut Vec<Run>) {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let at = |k: usize| chars.get(k).map_or(text.len(), |(byte, _)| *byte);
    let mut k = 0;
    let mut plain_from = 0;

    macro_rules! flush {
        ($end:expr) => {
            if $end > plain_from {
                out.push(Run::tinted(&text[plain_from..$end], base));
            }
        };
    }

    while k < chars.len() {
        let (byte, c) = chars[k];
        let rest = &text[byte..];
        // Longest marker first, or `**bold**` reads as two emphases.
        for (marker, color) in [
            ("**", skin.bright),
            ("`", skin.string),
            ("__", skin.bright),
            ("*", skin.markup),
        ] {
            if !rest.starts_with(marker) {
                continue;
            }
            let width = marker.chars().count();
            let Some(close) = find(&chars, text, k + width, marker) else {
                continue;
            };
            // Tight, the way CommonMark requires: the opener is followed by a
            // non-space and the closer preceded by one. Without this, `a * b *
            // c` reads as an emphasis and comes out as `a  b  c`, and prose is
            // full of multiplication signs and bullets in the middle of lines.
            let opens = chars.get(k + width).is_some_and(|(_, c)| !c.is_whitespace());
            let closes = close > 0 && chars[close - 1].1 != ' ';
            if !opens || !closes {
                continue;
            }
            flush!(byte);
            let inner = &text[at(k + width)..at(close)];
            if inner.is_empty() {
                continue;
            }
            out.push(Run::tinted(inner, color));
            k = close + width;
            plain_from = at(k);
            break;
        }
        if plain_from <= byte {
            k += 1;
        }
        let _ = c;
    }
    flush!(text.len());
    if out.is_empty() {
        out.push(Run::tinted(text, base));
    }
}

/// The char index where `marker` next occurs at or after `from`.
fn find(chars: &[(usize, char)], text: &str, from: usize, marker: &str) -> Option<usize> {
    (from..chars.len()).find(|k| text[chars[*k].0..].starts_with(marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn skin() -> Skin {
        Skin::from(&Config::default())
    }

    fn render(text: &str) -> (String, Vec<Run>) {
        let skin = skin();
        let mut fence = Fence::default();
        let mut runs = Vec::new();
        for line in text.lines() {
            self::line(line, &mut fence, &skin, &mut runs);
            runs.push(Run::plain("\n"));
        }
        let flat: String = runs.iter().map(|r| r.text.as_str()).collect();
        (flat, runs)
    }

    /// Render a whole table block the way the transcript does: measure it once,
    /// then draw each source line.
    fn table(src: &str, room: usize) -> String {
        table_from(src, 0, room)
    }

    /// The same, scrolled `skip` columns to the right.
    fn table_from(src: &str, skip: usize, room: usize) -> String {
        let skin = skin();
        let table = Table::of(src.lines());
        let mut out = String::new();
        for line in src.lines() {
            let mut runs = Vec::new();
            table.row(line, skip, room, &skin, &mut runs);
            out.push_str(&runs.iter().map(|r| r.text.as_str()).collect::<String>());
            out.push('\n');
        }
        out
    }

    const SRC: &str = "\
| Name | Type |
| --- | --- |
| talk | pane |
| llm | monitor |";

    /// A table used to reach the screen as raw pipes and a row of dashes. It is
    /// box drawn and column aligned now, and every column is the same width on
    /// every row.
    #[test]
    fn a_table_is_box_drawn_and_its_columns_line_up() {
        let out = table(SRC, 60);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 4, "one drawn row per source row: {out}");
        assert!(lines[0].starts_with('\u{2502}'), "{out}");
        assert!(lines[0].contains("Name"), "{out}");
        assert!(!out.contains('|'), "no raw pipe survives: {out}");
        assert!(!out.contains("---"), "no dashed rule survives: {out}");
        // Every row is exactly as wide as every other, which is what "lines up"
        // means and what the old rendering could not do.
        let widths: Vec<usize> = lines.iter().map(|l| l.chars().count()).collect();
        assert!(
            widths.windows(2).all(|p| p[0] == p[1]),
            "rows are {widths:?} wide: {out}"
        );
    }

    /// The `|---|` row becomes the box's divider rather than a line of dashes,
    /// and no row is added or removed: a selection is anchored to source lines,
    /// so a border of its own would put the highlight on the wrong text.
    #[test]
    fn the_rule_row_becomes_the_divider_and_no_row_is_invented() {
        let out = table(SRC, 60);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), SRC.lines().count());
        assert!(
            lines[1].starts_with('\u{251c}') && lines[1].contains('\u{253c}'),
            "the second row is the divider: {out}"
        );
        assert!(lines[1].ends_with('\u{2524}'), "{out}");
    }

    /// Column widths come from the widest cell anywhere in the block, not from
    /// the header, or a long value would push its column out of alignment.
    #[test]
    fn a_column_is_as_wide_as_its_widest_cell_anywhere() {
        let t = Table::of(SRC.lines());
        // "monitor" is seven characters and is the widest in column two.
        assert_eq!(t.widths, vec![4, 7]);
    }

    /// A table wider than the pane is a viewport onto the whole thing, not a
    /// truncation of it, and both edges say when there is more beyond them.
    /// Cutting the data off was the first version and it lost columns.
    #[test]
    fn a_wide_table_scrolls_sideways_instead_of_losing_its_columns() {
        let room = 14;
        let at_left = table_from(SRC, 0, room);
        for line in at_left.lines() {
            assert!(
                line.chars().count() <= room,
                "{line:?} is {} wide in a {room} column pane",
                line.chars().count()
            );
        }
        assert!(at_left.contains('\u{203a}'), "more to the right is marked: {at_left}");
        assert!(!at_left.contains('\u{2039}'), "nothing off to the left yet: {at_left}");
        assert!(at_left.contains("Name"), "{at_left}");

        // Scrolled right, the far column is reachable and the left edge now
        // says there is something behind it.
        let table = Table::of(SRC.lines());
        let scrolled = table_from(SRC, table.width() - room, room);
        assert!(scrolled.contains('\u{2039}'), "{scrolled}");
        assert!(!scrolled.contains('\u{203a}'), "the right edge is reached: {scrolled}");
        assert!(
            scrolled.contains("monitor"),
            "the column that was cut off is now readable: {scrolled}"
        );
    }

    /// Scrolling past the right edge is not possible: the viewport clamps, so
    /// the wheel cannot push the table off into blank space.
    #[test]
    fn scrolling_a_table_stops_at_its_right_edge() {
        let room = 14;
        let table = Table::of(SRC.lines());
        let at_edge = table_from(SRC, table.width() - room, room);
        let far_past = table_from(SRC, 9_999, room);
        assert_eq!(far_past, at_edge, "asking for more stops at the edge");
    }

    /// The declared width has to match what is actually drawn, or the clamp
    /// stops in the wrong place and the last column stays unreachable.
    #[test]
    fn the_declared_width_is_what_a_row_actually_draws() {
        let table = Table::of(SRC.lines());
        let full = table_from(SRC, 0, 500);
        for line in full.lines() {
            assert_eq!(
                line.chars().count(),
                table.width(),
                "{line:?} against a declared width of {}",
                table.width()
            );
        }
    }

    /// A row with fewer cells than the widest still has to close its box, or
    /// the table looks torn.
    #[test]
    fn a_ragged_row_still_fills_and_closes_its_columns() {
        let src = "| a | b |\n| --- | --- |\n| only |";
        let out = table(src, 60);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[2].ends_with('\u{2502}'), "{out}");
        let widths: Vec<usize> = lines.iter().map(|l| l.chars().count()).collect();
        assert!(widths.windows(2).all(|p| p[0] == p[1]), "{widths:?}: {out}");
    }

    /// Prose is not a table. A sentence with a pipe in it is far more common
    /// than a table written without a leading one, so the leading pipe is
    /// required and this must pass through untouched.
    #[test]
    fn a_sentence_containing_a_pipe_is_not_a_table() {
        assert!(!is_table_row("run a | b to pipe it"));
        assert!(!is_table_row(""));
        assert!(!is_table_row("|"));
        assert!(is_table_row("| a | b |"));
        assert!(is_table_row("  | indented | row |"));
        let (text, _) = render("use `ls | wc` to count\n");
        assert!(text.contains("ls | wc"), "{text}");
    }

    /// The marks are the formatting, so they must not survive into the text.
    #[test]
    fn the_marks_are_replaced_by_the_formatting() {
        let (text, _) = render("## Notable Features\n- **read** a file\n`inline` code\n");
        assert!(!text.contains("##"), "{text}");
        assert!(!text.contains("**"), "{text}");
        assert!(!text.contains('`'), "{text}");
        assert!(text.contains("Notable Features"), "{text}");
        assert!(text.contains("read"), "{text}");
        assert!(text.contains("inline"), "{text}");
    }

    #[test]
    fn bold_and_code_get_their_own_colors() {
        let skin = skin();
        let (_, runs) = render("- **write** creates `a.txt`");
        let colors: Vec<Option<[u8; 4]>> = runs.iter().map(|r| r.color).collect();
        assert!(colors.contains(&Some(skin.bright)), "bold");
        assert!(colors.contains(&Some(skin.string)), "code");
        assert!(colors.contains(&Some(skin.markup)), "the bullet");
    }

    /// A fenced block is code, colored by the language the fence named.
    #[test]
    fn a_fenced_block_is_syntax_colored_by_its_language() {
        let skin = skin();
        let (text, runs) = render("```python\nx = \"hi\"  # note\n```\n");
        assert!(!text.contains("```"), "{text}");
        assert!(text.contains("python"), "the language is still named: {text}");
        assert!(text.contains("x = \"hi\"  # note"), "{text}");
        let colors: Vec<Option<[u8; 4]>> = runs.iter().map(|r| r.color).collect();
        assert!(colors.contains(&Some(skin.string)), "the string is tinted");
        assert!(colors.contains(&Some(skin.comment)), "the comment is tinted");
    }

    /// Inside a fence, a `#` is a comment and a `-` is not a bullet.
    #[test]
    fn markdown_marks_do_not_apply_inside_code() {
        let (text, _) = render("```sh\n- not a bullet\n# a comment\n```\n");
        assert!(text.contains("- not a bullet"), "{text}");
        assert!(text.contains("# a comment"), "{text}");
        assert!(!text.contains('•'), "{text}");
    }

    /// A window that starts inside a block has to know it is inside one.
    #[test]
    fn the_fence_state_survives_a_scrolling_window() {
        let above = ["prose", "```rust", "let x = 1;"];
        let fence = fence_after(above.into_iter());
        assert!(fence.open());
        assert_eq!(fence.0.as_deref(), Some("rust"));
        let closed = fence_after(["```py", "x=1", "```"].into_iter());
        assert!(!closed.open());
    }

    /// Prose is full of lone asterisks and stray backticks. A parser that
    /// insists on pairing them swallows the rest of the paragraph.
    #[test]
    fn an_unmatched_marker_is_just_text() {
        for line in [
            "2 * 3 = 6",
            "an unclosed `backtick here",
            "**never closed",
            "a * b * c",
            "3 * 4 * 5 = 60",
            "`",
            "spaced ` backtick ` pair",
        ] {
            let (text, _) = render(line);
            assert_eq!(text.trim_end(), line, "{line:?} was mangled");
        }
    }

    /// Whatever it does, reassembling the runs must give the text back, minus
    /// only the marks it deliberately consumed.
    #[test]
    fn plain_prose_passes_through_untouched() {
        for line in [
            "",
            "Just a sentence.",
            "   indented prose",
            "a line with 3 - 1 = 2 in it",
            "héllo wörld 日本語",
        ] {
            let (text, _) = render(line);
            assert_eq!(text.trim_end_matches('\n'), line, "{line:?}");
        }
    }

    /// A line of nothing but rule characters is a thematic break, including
    /// one made of asterisks, which is why it is not in the unmatched-marker
    /// list above.
    #[test]
    fn a_line_of_only_rule_characters_is_a_rule() {
        for rule in ["---", "***", "====", "****"] {
            let (text, _) = render(rule);
            assert!(text.contains('─'), "{rule:?} rendered as {text:?}");
            assert!(!text.contains('*') && !text.contains('='), "{text:?}");
        }
    }

    #[test]
    fn lists_quotes_and_rules_all_render() {
        let (text, _) = render("- one\n* two\n1. three\n> quoted\n---\n");
        assert_eq!(text.matches('•').count(), 2, "{text}");
        assert!(text.contains("1. three"), "{text}");
        assert!(text.contains("│ quoted"), "{text}");
        assert!(text.contains('─'), "the rule is drawn: {text}");
        assert!(!text.contains("---"), "{text}");
    }

    #[test]
    fn indentation_is_kept_so_nested_lists_stay_nested() {
        let (text, _) = render("- top\n  - nested\n");
        assert!(text.contains("\n  • nested"), "{text:?}");
    }

    /// A heading with nothing after the hashes is prose, not an empty heading.
    #[test]
    fn a_bare_hash_is_not_a_heading() {
        let (text, _) = render("# \nplain");
        assert!(text.contains("plain"));
    }
}
