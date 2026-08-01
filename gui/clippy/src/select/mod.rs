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

/// Which body of text a selection is in.
///
/// A selection belongs to one of them: dragging out of one and into another
/// selects more of the first, not some span across two unrelated lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Where {
    /// A pane in the dock, named by the view showing in it.
    Pane(View),
    /// The call popup's document: its blocks flattened to the lines they
    /// are drawn as. Not a view either; the popup floats over every space.
    CallPopup,
    /// The document beside the entry list on the settings panel.
    ///
    /// Not a view and not in a space: the panel is a takeover and covers every
    /// space there is, so the only way to say where this selection lives is to
    /// say the panel. What resolves it is the pane the panel builds its showing
    /// document into, which is why everything below still takes a
    /// [`crate::state::Pane`].
    SettingsDoc,
}

/// A drag in progress, or a finished one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Which text it is in.
    pub at: Where,
    anchor: Spot,
    focus: Spot,
}

impl Selection {
    pub fn new(at: Where, spot: Spot) -> Selection {
        Selection {
            at,
            anchor: spot,
            focus: spot,
        }
    }

    /// The pane this is in, when it is in one. Nothing for a selection over the
    /// settings document, which no view and no space can name.
    pub fn view(&self) -> Option<View> {
        match self.at {
            Where::Pane(view) => Some(view),
            Where::CallPopup => None,
            Where::SettingsDoc => None,
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
    /// The text as it is drawn, which in the transcript is not the text the
    /// model wrote: a bullet copies as `• read a file`, the marks around the
    /// bold gone the same way they are gone from the screen. Copying the source
    /// instead would hand back characters that have no glyph to point at, and
    /// the columns of the selection are counted in glyphs.
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
            let chars: Vec<char> = line.shown().chars().collect();
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

    /// The transcript: a pane that renders what the model wrote.
    fn rendered(lines: &[&str]) -> Pane {
        let mut pane = Pane::new(100).rendered();
        for text in lines {
            pane.push(Line::new(*text, Tone::Body));
        }
        pane
    }

    fn drag(from: (usize, usize), to: (usize, usize)) -> Selection {
        let mut selection = Selection::new(Where::Pane(View::Output), Spot::new(from.0, from.1));
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
        let click = Selection::new(Where::Pane(View::Output), Spot::new(0, 2));
        assert!(click.is_empty());
        assert_eq!(click.text(&pane), "");
        assert_eq!(click.columns_on(0, 5), None);
    }

    /// A selection says which text it is in, and the two kinds are told apart
    /// rather than both answering with a view: the settings document is not in
    /// a space, so a caller that asked it for one would be asking the dock
    /// about text the dock has never seen.
    #[test]
    fn a_selection_names_the_text_it_is_in() {
        let pane = Selection::new(Where::Pane(View::Output), Spot::new(0, 0));
        assert_eq!(pane.view(), Some(View::Output));
        let doc = Selection::new(Where::SettingsDoc, Spot::new(0, 0));
        assert_eq!(doc.view(), None);
        assert_ne!(pane.at, doc.at);
    }

    /// Dragging past the end of a short line selects to its end and no
    /// further, rather than copying spaces that were never there.
    #[test]
    fn a_drag_past_the_end_of_a_line_stops_at_the_text() {
        let pane = pane(&["ab", "longer line"]);
        assert_eq!(drag((0, 0), (0, 40)).text(&pane), "ab");
        assert_eq!(drag((0, 0), (1, 40)).text(&pane), "ab\nlonger line");
    }

    /// A run taken off one row of a wrapped line is exactly the row that is on
    /// screen: no space is inserted at the break, and none is picked up from
    /// one either.
    ///
    /// The wrap is invisible to the copy by construction, because this slices
    /// the whole logical line and only '\n' is ever pushed. What was not
    /// guaranteed is that the row the reader points at holds the characters the
    /// columns say it does. It does now, because the pane and the renderer
    /// break the rows with the same `text-geometry` call, asserted against
    /// here. Left to the shaper, the blank at each break was swallowed, so a
    /// drag starting at the left edge of a wrapped row began one character
    /// early and copied a space that was nowhere on the screen.
    ///
    /// This used to assert rows of exactly `cols` characters, which was that
    /// same guarantee bought by breaking prose in the middle of words. The
    /// guarantee is the same now and the words are whole, so the rows are read
    /// off the pane rather than multiplied out.
    #[test]
    fn a_run_taken_off_a_wrapped_row_is_the_row_that_is_on_screen() {
        let prose = "hello worldly people everywhere now and then";
        let pane = pane(&[prose]);
        let cols = 20;

        // The rows the renderer lays this line out as.
        let laid = noob_draw::Run::wrapped(
            &[noob_draw::Run::plain(prose)],
            cols,
            crate::state::PANE_WRAP,
        )
        .swap_remove(0)
        .text;
        let drawn: Vec<&str> = laid.split('\n').collect();
        let counted = pane.rows_of_line(0, cols);
        assert_eq!(
            drawn.len(),
            counted.len(),
            "the pane budgets a different number of rows than are drawn"
        );
        assert!(drawn.len() > 2, "the line has to wrap for this to prove anything");
        assert!(
            drawn.iter().all(|row| !row.starts_with(' ') && !row.ends_with(' ')),
            "a break leaves its blank on neither row: {drawn:?}"
        );

        // And a drag across the whole of each row copies that row.
        for (row, shown) in drawn.iter().enumerate() {
            let span = counted[row];
            assert_eq!(
                drag((0, span.start), (0, span.end)).text(&pane),
                *shown,
                "row {row}"
            );
        }
        // A run that crosses a break gets the blank back, exactly once: it is
        // still a character of the line, it is only on no row.
        let across = drag((0, counted[0].end - 5), (0, counted[1].start + 5)).text(&pane);
        assert_eq!(across, "eople every");
        assert_eq!(across.matches(' ').count(), 1);
    }

    /// A transcript line is Markdown, and the marks are not on the screen. The
    /// columns are counted in glyphs, so the last glyph is selectable and what
    /// comes back is what was highlighted.
    ///
    /// The source is four characters longer than the rendering, and while the
    /// pane was measured in the source those four had no glyph to point at: the
    /// pointer ran out of line before the columns did, and a drag to the right
    /// edge copied `- **read** a file with ` with the tail of it missing.
    #[test]
    fn a_markdown_line_is_selected_and_copied_as_it_is_drawn() {
        let source = "- **read** a file with `cat`";
        let pane = rendered(&[source]);
        let drawn = pane.line(0).expect("the line is held").shown().to_string();
        assert_eq!(drawn, "• read a file with cat");
        assert!(drawn.chars().count() < source.chars().count());

        // Every column of the drawn line is reachable, the last one included.
        let last = drawn.chars().count();
        let all = drag((0, 0), (0, last));
        assert_eq!(all.text(&pane), drawn);
        assert_eq!(drag((0, last - 1), (0, last)).text(&pane), "t");
        assert!(
            !all.text(&pane).contains('*') && !all.text(&pane).contains('`'),
            "a marker that is nowhere on screen came back on the clipboard: {:?}",
            all.text(&pane)
        );
        // And a run picked out of the middle is the glyphs under it.
        assert_eq!(drag((0, 2), (0, 6)).text(&pane), "read");
    }

    /// A wrapped Markdown line breaks the text that is on screen: the rows the
    /// pane counts are the rows the renderer lays out, and a drag across one
    /// copies that row.
    #[test]
    fn a_wrapped_markdown_line_breaks_what_is_on_screen() {
        let source = "- **read** a file, then `write` it back out again with \
                      __every__ mark it came with";
        let pane = rendered(&[source]);
        let cols = 20;
        let drawn = pane.line(0).expect("the line is held").shown().to_string();
        // Ten marks left the line: two pairs of stars, one pair of underscores
        // and a pair of backticks.
        assert_eq!(drawn.chars().count() + 10, source.chars().count(), "{drawn:?}");

        let counted = pane.rows_of_line(0, cols);
        let laid = noob_draw::Run::wrapped(
            &[noob_draw::Run::plain(&drawn)],
            cols,
            crate::state::PANE_WRAP,
        )
        .swap_remove(0)
        .text;
        let rows: Vec<&str> = laid.split('\n').collect();
        assert!(rows.len() > 2, "the line has to wrap for this to prove anything");
        assert_eq!(
            rows.len(),
            counted.len(),
            "the pane budgets a different number of rows than are drawn"
        );
        for (row, shown) in rows.iter().enumerate() {
            let span = counted[row];
            assert_eq!(drag((0, span.start), (0, span.end)).text(&pane), *shown, "row {row}");
        }
        // The second row starts where the hit test says it starts, which is
        // where the drawn row starts and not eight characters of consumed
        // marks earlier.
        assert_eq!(
            pane.spot_in(rows.len(), cols, 1, 0),
            Some((0, counted[1].start)),
            "the second row of the line begins somewhere else"
        );
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
