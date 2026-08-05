//! How a run of logical lines becomes rows on a monospace screen.
//!
//! This layer exists because the rule was previously written out at eight call
//! sites and disagreed with itself at three of them: a pane asked for as many
//! logical lines as rows fit, the shaper wrapped some of them onto two or more
//! rows, and the overflow fell out of the clip box with no scroll position that
//! could reach it. The selection band and the scrollbar drifted for the same
//! reason.
//!
//! Everything here is pure arithmetic over characters. Nothing shapes text,
//! measures a font, or touches a GPU: a caller supplies the width of its box in
//! columns and either the length of each line or the line itself, and gets back
//! which lines to draw, where they land, and which characters end up on each
//! row. That is what makes the rule testable without a window.
//!
//! The wrap rule lives in [`rows_in`] and lives there only. The renderer breaks
//! the rows it draws with that call and the window counts them with the same
//! one, which is what keeps the characters on a row and the characters a
//! selection on that row copies the same characters. The two used to be written
//! out separately and disagreed the moment a line had a blank in it.
//!
//! Positions are always **visual rows**, never logical lines. The two are the
//! same only when nothing wraps, which is exactly the assumption that broke.

/// How many columns one character occupies on screen.
///
/// Not always one. An emoji and a CJK ideograph take two, a combining mark
/// takes none, and a grid that counts every character as one column puts the
/// selection band, the caret and the clipboard on different characters from the
/// ones a reader is looking at. The table is Unicode Annex 11's.
///
/// A control character has no width of its own and is counted as one, because
/// that is the cell the renderer leaves for it.
///
/// This is the character alone. A one-column symbol followed by the emoji
/// variation selector is drawn as a two-column emoji, which only a walk over
/// the string can see: [`widths`] is that walk, and every count in this
/// contract uses it.
pub fn width_of(ch: char) -> usize {
    unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1)
}

/// The emoji variation selector: invisible itself, and it turns the
/// one-column symbol before it (a spade, a heart, a skull) into a two-column
/// emoji. Counting the pair as one column is how a glyph ended up drawn past
/// the edge of its row.
pub const VS16: char = '\u{fe0f}';

/// Whether a character the selector follows is promoted by it: a one-column
/// character becomes the two-column emoji form.
pub fn promoted(ch: char, next: Option<char>) -> bool {
    next == Some(VS16) && width_of(ch) == 1
}

/// Each character of `text` with the columns it occupies, selector pairs
/// resolved: the promoted character counts two, the selector itself none.
fn widths(text: &str) -> impl Iterator<Item = (char, usize)> + '_ {
    let mut chars = text.chars().peekable();
    std::iter::from_fn(move || {
        let ch = chars.next()?;
        let width = match promoted(ch, chars.peek().copied()) {
            true => 2,
            false => width_of(ch),
        };
        Some((ch, width))
    })
}

/// How many columns a string occupies, which is what every `cols` here means.
pub fn columns_in(text: &str) -> usize {
    widths(text).map(|(_, w)| w).sum()
}

/// How many columns the characters `from..to` of `text` occupy.
///
/// What turns a character range into a place on screen, which is the step every
/// caller used to skip: a band drawn `to - from` columns wide covers the right
/// characters only while every character is one column.
pub fn columns_between(text: &str, from: usize, to: usize) -> usize {
    widths(text)
        .take(to)
        .skip(from)
        .map(|(_, w)| w)
        .sum()
}

/// The column a character sits at, counting from the start of `text`.
///
/// Past the end gives the width of the whole string, which is the column just
/// after the last character: a caret at the end of a line sits there.
pub fn column_of(text: &str, chars: usize) -> usize {
    columns_between(text, 0, chars)
}

/// The character `column` lands on, counting from the start of `text`.
///
/// The inverse of [`column_of`] and the half a pointer needs. A column inside a
/// two-column character takes that character, not the one after it, so clicking
/// either half of an emoji selects the emoji.
pub fn char_at(text: &str, column: usize) -> usize {
    let mut used = 0;
    for (index, (_, width)) in widths(text).enumerate() {
        let next = used + width;
        if next > column {
            return index;
        }
        used = next;
    }
    text.chars().count()
}

/// How many rows one logical line occupies in a box `cols` wide.
///
/// `columns` is the width of the line, not its character count: see
/// [`columns_in`]. An empty line still occupies one row, because a blank line
/// in a transcript is a paragraph break and collapsing it would reflow the pane.
pub fn rows_of(columns: usize, cols: usize) -> usize {
    if cols == 0 {
        return 1;
    }
    columns.div_ceil(cols).max(1)
}

/// The wrapped height of every line, which is the input to everything else.
pub fn heights(lengths: impl IntoIterator<Item = usize>, cols: usize) -> Vec<usize> {
    lengths.into_iter().map(|n| rows_of(n, cols)).collect()
}

/// Where a visual row is allowed to end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Break {
    /// Exactly `cols` characters to the row, wherever that falls, mid-word or
    /// not. What a box whose caret is placed as `row * cols + column` needs,
    /// which is the prompt.
    Column,
    /// At the last break opportunity that fits, so words stay whole. What
    /// prose wants, and the default.
    #[default]
    Word,
}

/// One visual row of one logical line: the half-open character range of that
/// line which is drawn on it.
///
/// `end` is not always the start of the next row. A row that broke at a break
/// opportunity leaves that one character between them: it is drawn on neither
/// row, because a blank pushed to the front of the row below reads as an
/// indent that nobody typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub start: usize,
    pub end: usize,
}

impl Row {
    /// How many characters this row shows.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Whether the row shows nothing, which only the single row of an empty
    /// line does.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A character a row is allowed to end in front of.
///
/// A blank and a tab, and nothing else. Punctuation is deliberately not a
/// break opportunity: breaking after a hyphen or a slash would split a path or
/// a flag across two rows, and a transcript is full of both. A tab counts
/// because it is a word separator wherever it appears, and it is one column
/// wide here like every other character, since this whole layer is arithmetic
/// over character counts and nothing expands tab stops.
fn is_break(ch: char) -> bool {
    ch == ' ' || ch == '\t'
}

/// The visual rows one logical line is drawn as, in a box `cols` wide.
///
/// **The one wrap rule.** Greedy, and used by the drawing and by the counting
/// alike: a row takes as many characters as fit, ending at the last break
/// opportunity at or before the column limit, and a word wider than the whole
/// box breaks at the limit rather than running off the edge. The character a
/// row broke at is consumed by the break: it is on neither row, so no row
/// starts with a blank that the reader cannot see the reason for. It is still
/// in the logical line, so copying across a break gets it back exactly once.
///
/// `text` is one logical line. A newline inside it ends the row it is on and
/// starts the next one, and is spent on that break the way a blank is: it is
/// drawn on neither row and stays in the line, so copying across it gets it
/// back exactly once. That is what lets one line hold a shape of its own, a
/// laid-out table row being the case it exists for, and be counted in the rows
/// it is really drawn as.
///
/// Always at least one row: an empty line still occupies a row. Never a
/// trailing empty row either, so a line that ends at a break opportunity does
/// not gain a blank row under it.
pub fn rows_in(text: &str, cols: usize, at: Break) -> Vec<Row> {
    let mut rows = Vec::new();
    rows_into(text, cols, at, &mut rows);
    rows
}

/// [`rows_in`] written into a buffer the caller already has.
///
/// The same operation and the same answer. It exists for a pane measuring
/// every line it holds, which happens whenever a line arrives or the window is
/// resized: one buffer reused down the pane rather than one allocation per
/// line. The buffer is cleared first, so what comes back is this line's rows
/// and nothing else.
pub fn rows_into(text: &str, cols: usize, at: Break, rows: &mut Vec<Row>) {
    rows.clear();
    let mut base = 0;
    for (n, segment) in text.split('\n').enumerate() {
        // The newline that ended the segment before this one: one character of
        // the line, drawn on neither row.
        base += usize::from(n > 0);
        base += wrap(segment, cols, at, base, rows);
    }
}

/// One segment of a line, wrapped into `rows` with every position offset by
/// `base`. Returns the characters the segment holds.
fn wrap(text: &str, cols: usize, at: Break, base: usize, rows: &mut Vec<Row>) -> usize {
    if cols == 0 {
        let count = text.chars().count();
        rows.push(Row {
            start: base,
            end: base + count,
        });
        return count;
    }
    let mut start = 0;
    // A break opportunity, and the columns the row holds up to and including it,
    // so cutting there leaves the next row's fill without a re-walk.
    let mut last_break: Option<(usize, usize)> = None;
    let mut count = 0;
    let taken = rows.len();
    // Columns the row being filled holds so far. A character is measured, not
    // counted: an emoji fills two of them and a combining mark none.
    let mut used = 0;
    let mut walk = text.chars().peekable();
    let mut i = 0usize;
    while let Some(ch) = walk.next() {
        let width = match promoted(ch, walk.peek().copied()) {
            true => 2,
            false => width_of(ch),
        };
        count = i + 1;
        // Whether this character was spent on a break, and so belongs to
        // neither the row that just ended nor the one that just started.
        let mut spent = false;
        // A row may not be empty, so a character wider than the whole box goes
        // on a row of its own rather than never fitting anywhere.
        if used + width > cols && i > start {
            // This character does not fit on the row being filled. Either the
            // row ends at a break opportunity, or the word is wider than the
            // box and the row ends on the column.
            let cut = match at {
                // A break opportunity sitting exactly on the boundary is the
                // one the row ends at: the row before it is full, and the
                // character is spent on the break.
                Break::Word if is_break(ch) => Some((i, used + width)),
                Break::Word => last_break,
                Break::Column => None,
            };
            match cut {
                Some((p, through)) => {
                    rows.push(Row {
                        start: base + start,
                        end: base + p,
                    });
                    start = p + 1;
                    // What the new row already holds: everything after the
                    // break that the old row had counted.
                    used = used.saturating_sub(through);
                    spent = p == i;
                }
                None => {
                    rows.push(Row {
                        start: base + start,
                        end: base + i,
                    });
                    start = i;
                    used = 0;
                }
            }
            // Nothing between the cut and here can be a break opportunity:
            // it would have been the cut.
            last_break = None;
        }
        if !spent {
            used += width;
        }
        // A row may not be empty, so the character it starts on is never a
        // break opportunity for that row.
        if i > start && is_break(ch) {
            last_break = Some((i, used));
        }
        i += 1;
    }
    // An empty segment is still a row: a blank line inside a laid-out shape is
    // a gap the reader can see, the same way a blank logical line is.
    if start < count || rows.len() == taken {
        rows.push(Row {
            start: base + start,
            end: base + count,
        });
    }
    count
}

/// Which line a visual row inside the window belongs to, and which of that
/// line's own rows it is.
///
/// `row` counts from the top of the viewport. The line index is absolute, not
/// relative to the window. A row past the last line returns `None`, so
/// clicking empty space below a short transcript selects nothing rather than
/// the last character.
///
/// The second half of a hit test: [`rows_in`] turns the row number this
/// returns into the characters on it. Kept apart because this half needs only
/// the heights, which a pane caches, and the other half needs the one line's
/// text.
pub fn row_at(heights: &[usize], window: Window, row: usize) -> Option<(usize, usize)> {
    let target = row + window.skip;
    let mut at = 0;
    for step in 0..window.count {
        let line = window.first + step;
        let height = *heights.get(line)?;
        if target < at + height {
            return Some((line, target - at));
        }
        at += height;
    }
    None
}

/// Every row the lines occupy together.
///
/// Private: it is not on the contract, because no caller outside this layer
/// wants a total on its own. What they want is the bound it feeds,
/// [`max_scrollback`], and a total published beside it is one more number for a
/// call site to do its own arithmetic on.
fn total_rows(heights: &[usize]) -> usize {
    heights.iter().sum()
}

/// The furthest back a pane can scroll before its first line is at the top.
///
/// Measured in visual rows, so a pane full of wrapped lines can scroll further
/// than it has lines, which is the whole point.
pub fn max_scrollback(heights: &[usize], rows: usize) -> usize {
    total_rows(heights).saturating_sub(rows)
}

/// The scrollback that puts visual row `first_row` at the top of the viewport.
///
/// Everything else here counts back from the live end, which is what a
/// transcript wants: new content arrives at the bottom and zero follows it. A
/// list wants the opposite anchor, its first row at the top and new entries
/// appearing below. This is the one conversion between the two, so a caller
/// holding a top-anchored position never has to do the subtraction itself and
/// get the clamp wrong.
///
/// A `first_row` past the last screenful clamps to the end rather than
/// scrolling into empty space.
pub fn scrollback_for(heights: &[usize], rows: usize, first_row: usize) -> usize {
    max_scrollback(heights, rows).saturating_sub(first_row)
}

/// Which logical lines to draw, and how much of the first one to hide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Window {
    /// Index of the first line to draw.
    pub first: usize,
    /// How many lines to draw, starting at `first`.
    pub count: usize,
    /// Rows of the first line that sit above the viewport.
    ///
    /// A line taller than the space left at the top is drawn in full and
    /// scrolled by this much rather than being dropped, so a long paragraph
    /// scrolls through the pane a row at a time instead of jumping.
    /// This is what the renderer's `Text::scrolled` hook was built for.
    pub skip: usize,
}

/// The window ending `scrollback` rows back from the last row.
///
/// `scrollback` of zero follows the live end, which is where a pane returns
/// whenever new content arrives.
pub fn window(heights: &[usize], rows: usize, scrollback: usize) -> Window {
    if rows == 0 || heights.is_empty() {
        return Window::default();
    }
    let total = total_rows(heights);
    let scrollback = scrollback.min(max_scrollback(heights, rows));
    // The row this window ends on, exclusive, counting from the very top.
    let end_row = total - scrollback;
    let start_row = end_row.saturating_sub(rows);

    // Walk from the top accumulating heights until the row the window starts
    // on falls inside a line. Panes hold at most a few thousand lines and this
    // runs once per pane per frame, so a walk is cheaper than the bookkeeping
    // an index would need to survive eviction.
    let mut at = 0;
    let mut first = 0;
    while first < heights.len() && at + heights[first] <= start_row {
        at += heights[first];
        first += 1;
    }
    let skip = start_row - at;

    let mut count = 0;
    let mut drawn = 0;
    while first + count < heights.len() && drawn < skip + rows {
        drawn += heights[first + count];
        count += 1;
    }
    Window { first, count, skip }
}

/// Which line a visual row inside the window belongs to, and how far into it.
///
/// `row` counts from the top of the viewport. Returns the index of the line
/// (absolute, not relative to the window) and the character offset of the
/// start of that visual row within the line, so a caller can add its column.
///
/// A row past the last line returns `None`, so clicking empty space below a
/// short transcript selects nothing rather than the last character.
///
/// The offset is `wrapped_row * cols`, which is where the row starts only when
/// the box is drawn with [`Break::Column`]. A box that wraps at words asks
/// [`row_at`] and then [`rows_in`] instead, because there the row a reader is
/// pointing at starts wherever the words let it.
pub fn line_at(heights: &[usize], window: Window, cols: usize, row: usize) -> Option<(usize, usize)> {
    let (line, wrapped_row) = row_at(heights, window, row)?;
    Some((line, wrapped_row * cols))
}

/// The rows a line occupies inside the window, as `(top, height)`.
///
/// `top` may be negative in principle when the line is partly scrolled off, so
/// it is returned clamped to the viewport along with the height still visible.
/// Returns `None` when the line is not on screen at all.
pub fn band(heights: &[usize], window: Window, rows: usize, line: usize) -> Option<(usize, usize)> {
    if line < window.first || line >= window.first + window.count {
        return None;
    }
    let mut at = 0;
    for step in 0..window.count {
        let current = window.first + step;
        let height = *heights.get(current)?;
        if current == line {
            // Where this line starts relative to the top of the viewport.
            let top = at as isize - window.skip as isize;
            let bottom = top + height as isize;
            if bottom <= 0 || top >= rows as isize {
                return None;
            }
            let visible_top = top.max(0) as usize;
            let visible_bottom = (bottom.min(rows as isize)) as usize;
            return Some((visible_top, visible_bottom - visible_top));
        }
        at += height;
    }
    None
}

/// Where the scrollbar thumb sits and how tall it is, as fractions of the
/// track, or `None` when everything fits and there is nothing to indicate.
///
/// Counted in visual rows, so a pane of wrapped lines gets a thumb that
/// matches how far it can actually scroll.
pub fn thumb(heights: &[usize], rows: usize, scrollback: usize) -> Option<(f32, f32)> {
    let total = total_rows(heights);
    if rows == 0 || total <= rows {
        return None;
    }
    let total = total as f32;
    let size = (rows as f32 / total).clamp(0.06, 1.0);
    let top = (total - rows as f32 - scrollback as f32).max(0.0) / total;
    Some((top.min(1.0 - size), size))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole layer used to count a character as a column. It is not: the
    /// selection band covered six and a half of eight emoji, because the eight
    /// were counted as eight columns and drawn as sixteen.
    #[test]
    fn a_character_is_worth_the_columns_it_is_drawn_in() {
        assert_eq!(width_of('a'), 1);
        assert_eq!(width_of('\u{2705}'), 2, "an emoji fills two cells");
        assert_eq!(width_of('\u{4e2d}'), 2, "and so does an ideograph");
        assert_eq!(width_of('\u{0301}'), 0, "a combining mark rides the letter");
        assert_eq!(columns_in("ok \u{2705}"), 5);
    }

    /// A one-column symbol with the emoji selector after it is drawn as the
    /// two-column emoji, so it is counted as one: a spade suit was counted as
    /// one column, drawn as two, and the last glyph of the row stood past the
    /// edge of the panel.
    #[test]
    fn the_emoji_selector_promotes_its_symbol_to_two_columns() {
        assert_eq!(columns_in("\u{2660}"), 1, "the bare spade is a text symbol");
        assert_eq!(columns_in("\u{2660}\u{fe0f}"), 2, "the pair is an emoji");
        assert_eq!(columns_in("a\u{2764}\u{fe0f}b"), 4);
        // The pointer and the caret agree: both columns of the pair are the
        // symbol, the selector has no cell of its own.
        assert_eq!(char_at("\u{2764}\u{fe0f}x", 0), 0);
        assert_eq!(char_at("\u{2764}\u{fe0f}x", 1), 0);
        assert_eq!(char_at("\u{2764}\u{fe0f}x", 2), 2, "column two is the x, past the pair");
        assert_eq!(column_of("\u{2764}\u{fe0f}x", 2), 2, "heart and selector, two columns");
        assert_eq!(column_of("\u{2764}\u{fe0f}x", 3), 3);
        // And the wrap: four suit-pairs in a four-column box are two rows.
        let suits = "\u{2660}\u{fe0f}\u{2665}\u{fe0f}\u{2666}\u{fe0f}\u{2663}\u{fe0f}";
        let rows = rows_in(suits, 4, Break::Column);
        assert_eq!(rows.len(), 2, "{rows:?}");
        for row in &rows {
            let text: String = suits.chars().take(row.end).skip(row.start).collect();
            assert!(columns_in(&text) <= 4, "{text:?} overruns the box");
        }
    }

    /// A pointer lands on a column, and every column of a wide character
    /// belongs to that character: clicking either half of an emoji takes the
    /// emoji, never the character after it.
    #[test]
    fn a_column_maps_back_to_the_character_drawn_under_it() {
        let line = "a\u{2705}b";
        assert_eq!(column_of(line, 0), 0);
        assert_eq!(column_of(line, 1), 1);
        assert_eq!(column_of(line, 2), 3, "b starts past both halves");
        assert_eq!(column_of(line, 99), 4, "past the end is the caret column");
        assert_eq!(char_at(line, 0), 0);
        assert_eq!(char_at(line, 1), 1);
        assert_eq!(char_at(line, 2), 1, "the emoji's second column is still it");
        assert_eq!(char_at(line, 3), 2);
        assert_eq!(char_at(line, 99), 3);
    }

    /// And the wrap rule counts the same way, or a row of emoji overruns the
    /// box it was measured for.
    #[test]
    fn a_row_holds_the_columns_it_fits_not_the_characters() {
        let emoji = "\u{2705}".repeat(5);
        let rows = rows_in(&emoji, 4, Break::Column);
        assert_eq!(rows.len(), 3, "four columns hold two emoji: {rows:?}");
        assert_eq!(rows[0], Row { start: 0, end: 2 });
        assert_eq!(rows[1], Row { start: 2, end: 4 });
        assert_eq!(rows[2], Row { start: 4, end: 5 });
        for row in &rows {
            let text: String = emoji.chars().take(row.end).skip(row.start).collect();
            assert!(columns_in(&text) <= 4, "{text:?} overruns the box");
        }
    }

    /// A box one column wide cannot hold a two-column character, and must put
    /// it somewhere anyway rather than never advancing.
    #[test]
    fn a_character_wider_than_the_box_still_gets_a_row() {
        let rows = rows_in("\u{2705}\u{2705}", 1, Break::Word);
        assert_eq!(rows.len(), 2, "one per row, not an empty loop: {rows:?}");
    }

    /// Wrapping at blanks measures in columns too, and the blank is still spent
    /// on the break.
    #[test]
    fn a_wide_word_wraps_at_the_blank_before_it() {
        let rows = rows_in("ab \u{2705}\u{2705}", 4, Break::Word);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0], Row { start: 0, end: 2 }, "the blank is spent");
        assert_eq!(rows[1], Row { start: 3, end: 5 });
    }

    #[test]
    fn a_line_that_fits_is_one_row_and_an_empty_line_still_is() {
        assert_eq!(rows_of(0, 80), 1, "a blank line is a paragraph break");
        assert_eq!(rows_of(1, 80), 1);
        assert_eq!(rows_of(80, 80), 1, "exactly full still fits on one row");
    }

    #[test]
    fn a_line_wider_than_the_box_takes_as_many_rows_as_it_needs() {
        assert_eq!(rows_of(81, 80), 2);
        assert_eq!(rows_of(160, 80), 2);
        assert_eq!(rows_of(161, 80), 3);
    }

    /// A zero-width box is a window mid-resize, not a caller bug. It must not
    /// divide by zero and must not report a line as taking no room.
    #[test]
    fn a_box_with_no_columns_does_not_divide_by_zero() {
        assert_eq!(rows_of(100, 0), 1);
    }

    /// The defect this layer exists for: the tail of a wrapped line used to be
    /// unreachable because the window was chosen in logical lines.
    #[test]
    fn the_last_row_of_a_wrapped_line_is_reachable_at_the_live_end() {
        // Three lines, the last of which wraps to three rows. A four row pane
        // following the live end must end on that line's last row.
        let h = heights([10, 10, 240], 80);
        assert_eq!(h, vec![1, 1, 3]);
        let w = window(&h, 4, 0);
        // Rows total 5, the pane shows 4, so the top row is hidden.
        assert_eq!(total_rows(&h), 5);
        assert_eq!(w.first, 1, "the first line scrolled off");
        assert_eq!(w.count, 2);
        assert_eq!(w.skip, 0);
        // The bottom of the last line is on screen: rows 1..4 of the viewport.
        let (top, height) = band(&h, w, 4, 2).unwrap();
        assert_eq!((top, height), (1, 3), "all three of its rows are visible");
    }

    /// The old model let a pane show `rows` logical lines regardless of their
    /// wrapped height, so more rows were handed to the shaper than fit.
    #[test]
    fn a_window_never_asks_for_more_rows_than_the_pane_has() {
        let h = heights([200, 200, 200, 200], 80); // 3 rows each, 12 total
        let w = window(&h, 5, 0);
        let drawn: usize = h[w.first..w.first + w.count].iter().sum();
        assert!(
            drawn - w.skip >= 5,
            "the window must cover the viewport, covered {}",
            drawn - w.skip
        );
        assert!(
            drawn - w.skip < 5 + 3,
            "and must not overshoot by more than one line, covered {}",
            drawn - w.skip
        );
    }

    #[test]
    fn scrolling_back_moves_by_visual_rows_not_by_lines() {
        let h = heights([240, 240], 80); // two lines, three rows each
        assert_eq!(total_rows(&h), 6);
        let at_end = window(&h, 2, 0);
        // Following the tail: the last two rows, both from the second line.
        assert_eq!(at_end.first, 1);
        assert_eq!(at_end.skip, 1, "one row of the second line is above");
        // One row back is still inside the second line, not jumped to the first.
        let back_one = window(&h, 2, 1);
        assert_eq!(back_one.first, 1);
        assert_eq!(back_one.skip, 0);
        // Three rows back crosses into the first line.
        let back_three = window(&h, 2, 3);
        assert_eq!(back_three.first, 0);
        assert_eq!(back_three.skip, 1);
    }

    #[test]
    fn scrollback_stops_at_the_oldest_row() {
        let h = heights([240, 240], 80); // 6 rows
        assert_eq!(max_scrollback(&h, 2), 4);
        let clamped = window(&h, 2, 999);
        assert_eq!(clamped, window(&h, 2, 4), "asking for more stops at the top");
        assert_eq!(clamped.first, 0);
        assert_eq!(clamped.skip, 0);
    }

    /// A list is anchored at its top, so the conversion has to put the row it
    /// is given at the top of the window, and clamp rather than scroll past the
    /// end when the list has shrunk under it.
    #[test]
    fn a_top_anchored_position_becomes_the_scrollback_that_shows_it() {
        let h = heights([0, 0, 0, 0, 0, 0], 1); // six rows, one each
        assert_eq!(h, vec![1; 6]);
        // Four rows on screen, so the top row can be 0, 1 or 2.
        assert_eq!(scrollback_for(&h, 4, 0), 2, "the top of the list");
        assert_eq!(window(&h, 4, scrollback_for(&h, 4, 0)).first, 0);
        assert_eq!(window(&h, 4, scrollback_for(&h, 4, 1)).first, 1);
        assert_eq!(window(&h, 4, scrollback_for(&h, 4, 2)).first, 2);
        // Past the end, both agree on the last screenful rather than on nothing.
        assert_eq!(scrollback_for(&h, 4, 99), 0);
        assert_eq!(window(&h, 4, scrollback_for(&h, 4, 99)).first, 2);
        // And a list that fits has one window whatever it is asked for.
        assert_eq!(scrollback_for(&h, 6, 0), 0);
        assert_eq!(scrollback_for(&h, 9, 3), 0);
    }

    #[test]
    fn a_row_maps_to_the_line_it_is_actually_showing() {
        let h = heights([10, 240, 10], 80); // 1, 3, 1
        let w = window(&h, 5, 0);
        assert_eq!(w, Window { first: 0, count: 3, skip: 0 });
        assert_eq!(line_at(&h, w, 80, 0), Some((0, 0)));
        assert_eq!(line_at(&h, w, 80, 1), Some((1, 0)), "first row of the long line");
        assert_eq!(line_at(&h, w, 80, 2), Some((1, 80)), "second row, 80 chars in");
        assert_eq!(line_at(&h, w, 80, 3), Some((1, 160)));
        assert_eq!(line_at(&h, w, 80, 4), Some((2, 0)));
    }

    /// Clicking below the last line must select nothing rather than snapping to
    /// the final character, which is what a row-equals-line model did.
    #[test]
    fn a_row_past_the_end_maps_to_nothing() {
        let h = heights([10, 10], 80);
        let w = window(&h, 10, 0);
        assert_eq!(line_at(&h, w, 80, 1), Some((1, 0)));
        assert_eq!(line_at(&h, w, 80, 2), None);
        assert_eq!(line_at(&h, w, 80, 99), None);
    }

    /// The band and the hit test have to agree, or the highlight covers text
    /// the clipboard does not contain.
    #[test]
    fn every_row_the_band_covers_maps_back_to_that_line() {
        let h = heights([10, 240, 95, 0], 80); // 1, 3, 2, 1
        let rows = 7;
        let w = window(&h, rows, 0);
        for line in w.first..w.first + w.count {
            let (top, height) = band(&h, w, rows, line).expect("on screen");
            for row in top..top + height {
                let (hit, _) = line_at(&h, w, 80, row).expect("inside the pane");
                assert_eq!(hit, line, "row {row} is banded to {line} but hits {hit}");
            }
        }
    }

    #[test]
    fn a_line_scrolled_off_the_top_is_banded_only_by_its_visible_part() {
        let h = heights([240, 10], 80); // 3, 1
        let w = window(&h, 2, 0);
        assert_eq!(w, Window { first: 0, count: 2, skip: 2 });
        // Only the last row of the long line is on screen, at the very top.
        assert_eq!(band(&h, w, 2, 0), Some((0, 1)));
        assert_eq!(band(&h, w, 2, 1), Some((1, 1)));
    }

    #[test]
    fn a_line_outside_the_window_has_no_band() {
        let h = heights([10, 10, 10], 80);
        let w = window(&h, 1, 0);
        assert_eq!(w.first, 2);
        assert_eq!(band(&h, w, 1, 0), None);
        assert_eq!(band(&h, w, 1, 2), Some((0, 1)));
    }

    #[test]
    fn the_thumb_reflects_wrapped_height_not_line_count() {
        // Four lines that fit would show no thumb under the old model even
        // though they wrap to twelve rows in a five row pane.
        let h = heights([200, 200, 200, 200], 80);
        assert_eq!(total_rows(&h), 12);
        let (top, size) = thumb(&h, 5, 0).expect("it overflows, so there is a thumb");
        assert!((size - 5.0 / 12.0).abs() < 1e-6, "size was {size}");
        assert!((top - 7.0 / 12.0).abs() < 1e-6, "top was {top}");
    }

    #[test]
    fn there_is_no_thumb_when_everything_fits() {
        let h = heights([10, 10], 80);
        assert_eq!(thumb(&h, 5, 0), None);
        assert_eq!(thumb(&h, 2, 0), None, "exactly full is still no overflow");
    }

    #[test]
    fn an_empty_pane_is_an_empty_window() {
        assert_eq!(window(&[], 10, 0), Window::default());
        assert_eq!(total_rows(&[]), 0);
        assert_eq!(thumb(&[], 10, 0), None);
        assert_eq!(line_at(&[], Window::default(), 80, 0), None);
    }

    /// A pane mid-resize can be given zero rows. It must produce nothing rather
    /// than panicking or reporting a window it cannot draw.
    #[test]
    fn a_pane_with_no_rows_draws_nothing() {
        let h = heights([10, 10], 80);
        assert_eq!(window(&h, 0, 0), Window::default());
        assert_eq!(thumb(&h, 0, 0), None);
    }

    /// The characters each row shows, which is what a caller draws and what a
    /// selection on that row copies.
    fn shown(text: &str, cols: usize, at: Break) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        rows_in(text, cols, at)
            .into_iter()
            .map(|row| chars[row.start..row.end].iter().collect())
            .collect()
    }

    #[test]
    fn a_row_ends_at_the_last_break_that_fits() {
        // The blank at index 20 is the one on the boundary: the row before it
        // is exactly full, and it is spent on the break rather than starting
        // the row below.
        let prose = "hello worldly people everywhere now";
        assert_eq!(prose.chars().nth(20), Some(' '));
        assert_eq!(
            shown(prose, 20, Break::Word),
            vec!["hello worldly people", "everywhere now"]
        );
        assert_eq!(
            rows_in(prose, 20, Break::Word),
            vec![Row { start: 0, end: 20 }, Row { start: 21, end: 35 }],
            "the row below starts past the blank, not on it"
        );
    }

    #[test]
    fn a_word_wider_than_the_box_breaks_on_the_column() {
        assert_eq!(
            shown("aaaa bbbbbbbbbbbbbbbbbbbbbbbbbb cc", 10, Break::Word),
            vec!["aaaa", "bbbbbbbbbb", "bbbbbbbbbb", "bbbbbb cc"],
            "a word with nowhere to break takes whole rows and does not overflow"
        );
        assert_eq!(shown("abcdefgh", 3, Break::Word), vec!["abc", "def", "gh"]);
    }

    #[test]
    fn breaking_on_a_column_ignores_the_blanks_entirely() {
        assert_eq!(
            shown("hello worldly people everywhere now", 20, Break::Column),
            vec!["hello worldly people", " everywhere now"]
        );
    }

    /// Every row of a column-broken line holds exactly `cols` characters but
    /// the last, which is the arithmetic `rows_of` and the prompt's caret both
    /// do. If the two ever disagreed the caret would sit off its glyph.
    #[test]
    fn a_column_break_agrees_with_the_row_count_it_is_measured_by() {
        for text in ["", "a", "abcde", "abcdef", "a b c d e f g h", "     ", "ab cd ef gh ij"] {
            for cols in 1..8 {
                let rows = rows_in(text, cols, Break::Column);
                assert_eq!(
                    rows.len(),
                    rows_of(text.chars().count(), cols),
                    "{text:?} at {cols} columns"
                );
                for (r, row) in rows.iter().enumerate() {
                    assert_eq!(row.start, r * cols, "{text:?} at {cols} columns, row {r}");
                }
            }
        }
    }

    /// A line may carry a shape of its own, and a laid-out table row is one:
    /// its cells wrap inside their columns, so the row reaches the pane as
    /// several rows of text already broken where they belong. The break is the
    /// newline, and it is counted here so a caller measuring the line and a
    /// renderer drawing it land on the same rows.
    #[test]
    fn a_newline_ends_the_row_it_is_on() {
        let rows = rows_in("head\nbody\ntail", 40, Break::Word);
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert_eq!(rows[0], Row { start: 0, end: 4 });
        assert_eq!(rows[1], Row { start: 5, end: 9 });
        assert_eq!(rows[2], Row { start: 10, end: 14 });

        // A segment still wraps by the one rule, so a wide cell adds rows of
        // its own between the breaks it was handed.
        let rows = rows_in("one two three\nfour", 7, Break::Word);
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert_eq!(rows[2], Row { start: 14, end: 18 });

        // A blank segment is a row, the way a blank line is.
        assert_eq!(rows_in("a\n\nb", 40, Break::Word).len(), 3);
        // And a box mid-resize keeps the shape rather than losing it.
        assert_eq!(rows_in("a\nb", 0, Break::Word).len(), 2);
    }

    /// The property the whole layer is for: what is on a row and what a
    /// selection there copies are the same characters, and the line can be put
    /// back together from its rows plus the one character each break ate.
    #[test]
    fn the_rows_of_a_line_cover_it_in_order_and_lose_only_the_breaks() {
        let cases = [
            "hello worldly people everywhere now and then and again for luck",
            "a supercalifragilisticexpialidociously long word in the middle here",
            "runs   of    blanks     between      words",
            "",
            "   ",
            "wordwiderthanthepane",
            " leading blank",
            "trailing blank ",
            "a shape\nof its own\nover three rows",
            "with a gap\n\nin the middle of it",
            "ending on a break\n",
            "\nstarting on one",
        ];
        for text in cases {
            for cols in 1..24 {
                let chars: Vec<char> = text.chars().collect();
                let rows = rows_in(text, cols, Break::Word);
                assert!(!rows.is_empty(), "{text:?} at {cols} has no rows");
                let mut previous: Option<Row> = None;
                for row in &rows {
                    assert!(row.start <= row.end, "{text:?} at {cols}: {row:?} runs backwards");
                    assert!(row.end <= chars.len(), "{text:?} at {cols}: {row:?} past the end");
                    assert!(row.len() <= cols, "{text:?} at {cols}: {row:?} overflows the box");
                    if let Some(before) = previous {
                        match row.start - before.end {
                            0 => {}
                            1 => assert!(
                                is_break(chars[before.end]) || chars[before.end] == '\n',
                                "{text:?} at {cols}: dropped {:?}, which is not a break",
                                chars[before.end]
                            ),
                            gap => panic!("{text:?} at {cols}: {gap} characters fell out of the line"),
                        }
                    } else {
                        assert_eq!(row.start, 0, "{text:?} at {cols} does not start at the line");
                    }
                    previous = Some(*row);
                }
                // The end of the line is covered too, but for the one break
                // character a line ending in a blank spends on its last break:
                // that blank sits past the right edge of the box, so there is
                // no column left to draw it in.
                let end = rows.last().expect("at least one row").end;
                match chars.len() - end {
                    0 => {}
                    1 => assert!(
                        is_break(chars[end]) || chars[end] == '\n',
                        "{text:?} at {cols}: {:?} is off the end and is not a break",
                        chars[end]
                    ),
                    left => panic!("{text:?} at {cols}: the last {left} characters are on no row"),
                }
            }
        }
    }

    /// A blank at the very end of a line is eaten by the break like any other,
    /// and must not leave a row with nothing on it behind.
    #[test]
    fn a_line_that_ends_at_a_break_gains_no_empty_row() {
        assert_eq!(shown("aaaa ", 4, Break::Word), vec!["aaaa"]);
        assert_eq!(shown("aaaa bbbb ", 4, Break::Word), vec!["aaaa", "bbbb"]);
        assert_eq!(shown("", 4, Break::Word), vec![""], "an empty line is still a row");
    }

    /// A box mid-resize reports no columns. Everything lands on one row rather
    /// than dividing by zero or reporting a line that occupies nothing.
    #[test]
    fn a_box_with_no_columns_is_one_row_of_everything() {
        assert_eq!(rows_in("hello there", 0, Break::Word), vec![Row { start: 0, end: 11 }]);
        assert_eq!(rows_in("", 0, Break::Column), vec![Row { start: 0, end: 0 }]);
    }

    /// The hit test's two halves: which of a line's rows was pointed at, and
    /// then which characters are on it.
    #[test]
    fn a_visual_row_names_the_line_and_the_row_within_it() {
        let h = heights([10, 240, 10], 80); // 1, 3, 1
        let w = window(&h, 5, 0);
        assert_eq!(row_at(&h, w, 0), Some((0, 0)));
        assert_eq!(row_at(&h, w, 1), Some((1, 0)));
        assert_eq!(row_at(&h, w, 2), Some((1, 1)), "the long line's second row");
        assert_eq!(row_at(&h, w, 3), Some((1, 2)));
        assert_eq!(row_at(&h, w, 4), Some((2, 0)));
        assert_eq!(row_at(&h, w, 5), None, "past the last line");
        // And it is the same walk `line_at` does, so a column-broken caller
        // reading either gets the same line.
        for row in 0..6 {
            assert_eq!(
                row_at(&h, w, row).map(|(line, _)| line),
                line_at(&h, w, 80, row).map(|(line, _)| line)
            );
        }
    }

    /// Narrowing the window makes lines taller, so a scrollback that was legal
    /// before must still land somewhere drawable rather than past the top.
    #[test]
    fn a_scrollback_from_a_wider_window_is_clamped_not_trusted() {
        let wide = heights([240, 240], 240); // 1 row each
        assert_eq!(max_scrollback(&wide, 2), 0);
        let narrow = heights([240, 240], 80); // 3 rows each
        assert_eq!(max_scrollback(&narrow, 2), 4);
        // A scrollback taken at the wide width is still valid when narrowed.
        let w = window(&narrow, 2, 0);
        assert!(w.first < narrow.len());
    }
}
