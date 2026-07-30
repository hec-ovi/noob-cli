//! How a run of logical lines becomes rows on a monospace screen.
//!
//! This layer exists because the rule was previously written out at eight call
//! sites and disagreed with itself at three of them: a pane asked for as many
//! logical lines as rows fit, the shaper wrapped some of them onto two or more
//! rows, and the overflow fell out of the clip box with no scroll position that
//! could reach it. The selection band and the scrollbar drifted for the same
//! reason.
//!
//! Everything here is pure arithmetic over character counts. Nothing shapes
//! text, measures a font, or touches a GPU: a caller supplies the width of its
//! box in columns and the length of each line in characters, and gets back
//! which lines to draw and where they land. That is what makes the rule
//! testable without a window.
//!
//! Positions are always **visual rows**, never logical lines. The two are the
//! same only when nothing wraps, which is exactly the assumption that broke.

/// How many rows one logical line occupies in a box `cols` wide.
///
/// An empty line still occupies one row: a blank line in a transcript is a
/// paragraph break, and collapsing it to nothing would reflow the whole pane.
pub fn rows_of(chars: usize, cols: usize) -> usize {
    if cols == 0 {
        return 1;
    }
    chars.div_ceil(cols).max(1)
}

/// The wrapped height of every line, which is the input to everything else.
pub fn heights(lengths: impl IntoIterator<Item = usize>, cols: usize) -> Vec<usize> {
    lengths.into_iter().map(|n| rows_of(n, cols)).collect()
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
pub fn line_at(heights: &[usize], window: Window, cols: usize, row: usize) -> Option<(usize, usize)> {
    let target = row + window.skip;
    let mut at = 0;
    for step in 0..window.count {
        let line = window.first + step;
        let height = *heights.get(line)?;
        if target < at + height {
            let wrapped_row = target - at;
            return Some((line, wrapped_row * cols));
        }
        at += height;
    }
    None
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
