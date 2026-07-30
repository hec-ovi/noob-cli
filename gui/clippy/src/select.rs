//! Selecting text with the pointer, and what gets copied.
//!
//! Every pane is monospace, which is what makes this arithmetic rather than a
//! layout query: a pixel is a column and a row, and the renderer never has to
//! be asked where a glyph landed.
//!
//! A selection holds absolute line numbers, not screen rows. The panes scroll
//! themselves whenever anything arrives, so a selection anchored to a row on
//! screen would quietly slide onto different text while you were still
//! dragging it, and one anchored to a position in the ring would slide every
//! time an old line was evicted. Absolute numbers survive both, and a line
//! that scrolls out of the ring simply stops resolving.
//!
//! The range is half-open at the end column, the way a text cursor works: a
//! drag that ends on column 4 has not selected the character at column 4.

use crate::dock::View;
use crate::state::Pane;

/// One character position: which line, and how many characters into it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Spot {
    pub line: usize,
    pub column: usize,
}

impl Spot {
    pub fn new(line: usize, column: usize) -> Spot {
        Spot { line, column }
    }
}

/// A drag in progress, or a finished one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Which pane it is in. A selection belongs to one pane: dragging out of
    /// one and into another selects more of the first, not some span across
    /// two unrelated lists.
    pub view: View,
    anchor: Spot,
    focus: Spot,
}

impl Selection {
    pub fn new(view: View, at: Spot) -> Selection {
        Selection {
            view,
            anchor: at,
            focus: at,
        }
    }

    /// Move the loose end. The anchor stays where the drag began, so dragging
    /// back past the start selects backwards rather than collapsing.
    pub fn extend(&mut self, to: Spot) {
        self.focus = to;
    }

    /// Start and end in reading order, whichever way the drag went.
    pub fn range(&self) -> (Spot, Spot) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    /// A click that never moved selects nothing, and must not read as an empty
    /// selection that swallows the next Ctrl-C.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.focus
    }

    /// The columns selected on one line, for drawing the band behind it.
    ///
    /// `len` is that line's own length. A line inside the selection is covered
    /// to its end plus one column, so a run of selected lines reads as a block
    /// rather than as a ragged right edge that stops at each line's last word.
    pub fn columns_on(&self, line: usize, len: usize) -> Option<(usize, usize)> {
        if self.is_empty() {
            return None;
        }
        let (start, end) = self.range();
        if line < start.line || line > end.line {
            return None;
        }
        let from = if line == start.line { start.column } else { 0 };
        // The extra column on a line that is not the last is the newline, which
        // is a real character in what gets copied.
        let to = if line == end.line { end.column } else { len + 1 };
        (to > from).then_some((from, to.min(len + 1)))
    }

    /// What a copy would put on the clipboard.
    ///
    /// Lines that have scrolled out of the pane are skipped rather than
    /// guessed at, so a selection made before a flood of output copies what is
    /// still there instead of inventing the rest.
    pub fn text(&self, pane: &Pane) -> String {
        if self.is_empty() {
            return String::new();
        }
        let (start, end) = self.range();
        let mut out = String::new();
        for number in start.line..=end.line {
            let Some(line) = pane.line(number) else {
                continue;
            };
            let chars: Vec<char> = line.text.chars().collect();
            let from = if number == start.line { start.column } else { 0 };
            let to = if number == end.line {
                end.column.min(chars.len())
            } else {
                chars.len()
            };
            if from < to {
                out.extend(&chars[from..to]);
            }
            if number != end.line {
                out.push('\n');
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Line, Tone};

    fn pane(lines: &[&str]) -> Pane {
        let mut pane = Pane::new(100);
        for text in lines {
            pane.push(Line::new(*text, Tone::Body));
        }
        pane
    }

    fn drag(from: (usize, usize), to: (usize, usize)) -> Selection {
        let mut selection = Selection::new(View::Output, Spot::new(from.0, from.1));
        selection.extend(Spot::new(to.0, to.1));
        selection
    }

    #[test]
    fn a_selection_inside_one_line_copies_that_run() {
        let pane = pane(&["hello world", "second line"]);
        assert_eq!(drag((0, 6), (0, 11)).text(&pane), "world");
        assert_eq!(drag((0, 0), (0, 5)).text(&pane), "hello");
    }

    /// Dragging back past where it started selects backwards. Collapsing to
    /// nothing is the bug every naive version has.
    #[test]
    fn dragging_backwards_selects_the_same_run() {
        let pane = pane(&["hello world"]);
        assert_eq!(drag((0, 11), (0, 6)).text(&pane), "world");
        assert_eq!(drag((0, 6), (0, 11)).range(), drag((0, 11), (0, 6)).range());
    }

    #[test]
    fn a_selection_across_lines_keeps_the_line_breaks() {
        let pane = pane(&["first", "second", "third"]);
        assert_eq!(drag((0, 3), (2, 2)).text(&pane), "st\nsecond\nth");
        // Whole lines, from the start of one to the end of another.
        assert_eq!(drag((0, 0), (1, 6)).text(&pane), "first\nsecond");
    }

    /// A click is not a selection. If it were, the next Ctrl-C would copy
    /// nothing instead of doing what Ctrl-C does.
    #[test]
    fn a_click_that_never_moved_selects_nothing() {
        let pane = pane(&["hello"]);
        let click = Selection::new(View::Output, Spot::new(0, 2));
        assert!(click.is_empty());
        assert_eq!(click.text(&pane), "");
        assert_eq!(click.columns_on(0, 5), None);
    }

    /// Dragging past the end of a short line selects to its end and no
    /// further, rather than copying spaces that were never there.
    #[test]
    fn a_drag_past_the_end_of_a_line_stops_at_the_text() {
        let pane = pane(&["ab", "longer line"]);
        assert_eq!(drag((0, 0), (0, 40)).text(&pane), "ab");
        assert_eq!(drag((0, 0), (1, 40)).text(&pane), "ab\nlonger line");
    }

    /// The band drawn behind a run of lines covers each one past its last
    /// character, so the block reads as a block instead of a ragged edge.
    #[test]
    fn the_band_covers_the_line_break_on_every_line_but_the_last() {
        let selection = drag((0, 2), (2, 3));
        assert_eq!(selection.columns_on(0, 5), Some((2, 6)));
        assert_eq!(selection.columns_on(1, 4), Some((0, 5)));
        assert_eq!(selection.columns_on(2, 9), Some((0, 3)));
        // Nothing outside the range.
        assert_eq!(selection.columns_on(3, 9), None);
    }

    /// A pane is a ring. Lines that fell out of it are gone, and a copy says
    /// what is left rather than inventing the rest.
    #[test]
    fn lines_that_scrolled_out_of_the_pane_are_skipped() {
        let mut pane = Pane::new(3);
        for n in 0..6 {
            pane.push(Line::new(format!("line {n}"), Tone::Body));
        }
        // Lines 0, 1 and 2 are gone; 3, 4 and 5 remain.
        assert!(pane.line(0).is_none());
        assert_eq!(pane.line(3).map(|l| l.text.as_str()), Some("line 3"));
        assert_eq!(drag((1, 0), (4, 6)).text(&pane), "line 3\nline 4");
    }

    /// A pane keeps counting past what it holds, so numbering does not restart
    /// and a selection cannot come to mean an older line.
    #[test]
    fn absolute_numbering_survives_eviction() {
        let mut pane = Pane::new(2);
        for n in 0..5 {
            pane.push(Line::new(format!("line {n}"), Tone::Body));
        }
        assert_eq!(pane.last(), 5);
        assert_eq!(pane.line(4).map(|l| l.text.as_str()), Some("line 4"));
        // The row on screen resolves to the line it is really showing.
        assert_eq!(pane.showing_from(2, 200), 3);
    }
}
