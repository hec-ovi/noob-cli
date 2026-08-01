//! Where each pane that scrolls itself is scrolled to.
//!
//! One offset per view, in one array, indexed the way [`View::ALL`] is indexed.
//! PLAN, AGENTS and the three monitors are built from lists rather than from a
//! [`crate::state::Pane`], so they have no scrollback of their own to keep; before this they had nothing at all, and content past the bottom of the
//! box was simply not drawn.
//!
//! Top-anchored, like the file explorer: a list is read from its first entry
//! down, and a row arriving at the end must not move the rows above it. The
//! bottom-anchored `scrollback` that every operation in `text_geometry` speaks
//! is derived through `scrollback_for`, in one place, so this module never
//! carries two conventions at once.
//!
//! **The arithmetic is not here.** Which rows are on screen, how far past the
//! end an offset is allowed to go and where the thumb sits all come from
//! `text_geometry`, which owns that rule for the whole window. This module is
//! the memory: where each view was left, clamped to what its pane can currently
//! hold. A pane that shrank while scrolled to its end is the failure that
//! reaches a reader, and [`Scrolls::settle`] is what prevents it.

use crate::dock::View;

/// The first visual row on screen, per view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scrolls {
    firsts: [usize; View::ALL.len()],
}

impl Scrolls {
    /// Which slot of the array a view keeps. By position in [`View::ALL`], the
    /// same order the palette and the config keys are indexed by, so a view
    /// added there gets an offset without anything here changing.
    fn slot(view: View) -> usize {
        View::ALL.iter().position(|v| *v == view).unwrap_or(0)
    }

    /// The first visual row this view is showing, counted from the top of its
    /// content.
    pub fn first(&self, view: View) -> usize {
        self.firsts[Scrolls::slot(view)]
    }

    /// Put this view on that row. Clamped, so a caller cannot ask for a row past
    /// the last screenful.
    fn put(&mut self, view: View, first: usize, heights: &[usize], rows: usize) -> bool {
        let most = text_geometry::max_scrollback(heights, rows);
        let next = first.min(most);
        let at = &mut self.firsts[Scrolls::slot(view)];
        let moved = next != *at;
        *at = next;
        moved
    }

    /// Move by `by` visual rows, down the content when `down`. Returns whether
    /// anything moved, so a caller only redraws when it did.
    pub fn scroll(&mut self, view: View, by: usize, down: bool, heights: &[usize], rows: usize) -> bool {
        let from = self.first(view);
        let next = match down {
            true => from + by,
            false => from.saturating_sub(by),
        };
        self.put(view, next, heights, rows)
    }

    /// Pull an offset back inside content that shrank, or a pane that did.
    ///
    /// Called every frame. Nothing else can catch it: the content of these panes
    /// changes without the pointer being anywhere near them, and a window
    /// dragged shorter changes how many rows fit without changing the content at
    /// all. Left alone, the offset stays past the end and the pane reads as
    /// empty.
    pub fn settle(&mut self, view: View, heights: &[usize], rows: usize) -> bool {
        self.put(view, self.first(view), heights, rows)
    }

    /// Which rows of the content are on screen, and how far the first of them is
    /// scrolled off the top.
    pub fn window(&self, view: View, heights: &[usize], rows: usize) -> text_geometry::Window {
        text_geometry::window(heights, rows, self.back(view, heights, rows))
    }

    /// Where the thumb sits and how tall it is, or nothing when the content
    /// fits and a bar would say nothing.
    pub fn thumb(&self, view: View, heights: &[usize], rows: usize) -> Option<(f32, f32)> {
        text_geometry::thumb(heights, rows, self.back(view, heights, rows))
    }

    /// The one conversion from this module's top-anchored row to the
    /// bottom-anchored scrollback the layer speaks.
    fn back(&self, view: View, heights: &[usize], rows: usize) -> usize {
        text_geometry::scrollback_for(heights, rows, self.first(view))
    }
}


// ---------------------------------------------------------------------------
// The file explorer's list, as plain arithmetic over its first visible row.
// Every file row is exactly one row (the explorer clips a long name rather
// than wrapping it, so a click can never resolve to a different file than
// the one under the pointer), which is what makes these functions total.
// ---------------------------------------------------------------------------

fn file_heights(count: usize) -> Vec<usize> {
    text_geometry::heights((0..count).map(|_| 0), 1)
}

/// Where the explorer's scrollbar sits, or nothing when every file fits.
pub fn file_thumb(first: usize, count: usize, rows: usize) -> Option<(f32, f32)> {
    let heights = file_heights(count);
    let back = text_geometry::scrollback_for(&heights, rows, first);
    text_geometry::thumb(&heights, rows, back)
}

/// The list moved by `by` rows, clamped so the last file stays on screen: a
/// list scrolled into empty space says nothing about what is in it.
pub fn scroll_files(first: usize, by: usize, down: bool, count: usize, rows: usize) -> usize {
    let most = text_geometry::max_scrollback(&file_heights(count), rows);
    match down {
        true => (first + by).min(most),
        false => first.saturating_sub(by),
    }
}

/// The least scroll that brings the open file's row on screen. The agent
/// moves this selection by touching a file, not the pointer, so a list
/// scrolled elsewhere has to come back to it.
pub fn reveal_file(first: usize, open: usize, count: usize, rows: usize) -> usize {
    if rows == 0 || count == 0 {
        return first;
    }
    let most = text_geometry::max_scrollback(&file_heights(count), rows);
    let mut next = first.min(open);
    if open + 1 > next + rows {
        next = open + 1 - rows;
    }
    next.min(most)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One row each, which is what a list of clipped rows has.
    fn flat(count: usize) -> Vec<usize> {
        crate::view::flat_heights(count)
    }

    /// Every view has its own offset, and nothing else moves when one does.
    #[test]
    fn each_view_keeps_its_own_place() {
        let mut scrolls = Scrolls::default();
        let heights = flat(40);
        assert!(scrolls.scroll(View::Plan, 5, true, &heights, 10));
        assert_eq!(scrolls.first(View::Plan), 5);
        for view in View::ALL {
            if view == View::Plan {
                continue;
            }
            assert_eq!(scrolls.first(view), 0, "{view:?} moved with PLAN");
        }
    }

    /// Scrolling stops where the last row is on screen, and coming back stops at
    /// the top. Neither end is allowed to run off into blank space.
    #[test]
    fn scrolling_clamps_at_both_ends() {
        let mut scrolls = Scrolls::default();
        let heights = flat(30);
        assert!(scrolls.scroll(View::Agents, 999, true, &heights, 10));
        assert_eq!(scrolls.first(View::Agents), 20, "30 rows in a box of 10");
        assert!(
            !scrolls.scroll(View::Agents, 1, true, &heights, 10),
            "the last row is already on screen"
        );
        assert!(scrolls.scroll(View::Agents, 999, false, &heights, 10));
        assert_eq!(scrolls.first(View::Agents), 0);
        assert!(!scrolls.scroll(View::Agents, 1, false, &heights, 10));
    }

    /// A pane scrolled to its end and then made shorter, or emptied, is the case
    /// that reaches a reader as a blank pane. The offset comes back inside.
    #[test]
    fn a_pane_that_shrank_leaves_a_valid_offset() {
        let mut scrolls = Scrolls::default();
        let heights = flat(30);
        scrolls.scroll(View::Agents, 999, true, &heights, 10);
        assert_eq!(scrolls.first(View::Agents), 20);

        // The window was dragged taller: the same content, more rows on screen.
        assert!(scrolls.settle(View::Agents, &heights, 25));
        assert_eq!(scrolls.first(View::Agents), 5);
        // And the content itself went away.
        assert!(scrolls.settle(View::Agents, &flat(3), 25));
        assert_eq!(scrolls.first(View::Agents), 0);
        assert!(!scrolls.settle(View::Agents, &flat(3), 25), "already settled");
    }

    /// The window and the thumb are the layer's, taken from the same offset, so
    /// the bar cannot disagree with what is drawn.
    #[test]
    fn the_window_and_the_thumb_come_from_one_offset() {
        let mut scrolls = Scrolls::default();
        let heights = flat(20);
        assert_eq!(scrolls.thumb(View::Plan, &heights, 20), None, "it all fits");

        let window = scrolls.window(View::Plan, &heights, 5);
        assert_eq!((window.first, window.count, window.skip), (0, 5, 0));
        let (top, size) = scrolls.thumb(View::Plan, &heights, 5).expect("20 rows in 5");
        assert!(top.abs() < 0.001, "the thumb is at the top: {top}");
        assert!((size - 0.25).abs() < 0.001, "5 of 20 rows: {size}");

        scrolls.scroll(View::Plan, 999, true, &heights, 5);
        let window = scrolls.window(View::Plan, &heights, 5);
        assert_eq!(window.first, 15, "the last screenful");
        assert_eq!(window.count, 5);
        let (top, size) = scrolls.thumb(View::Plan, &heights, 5).expect("a thumb");
        assert!((top + size - 1.0).abs() < 0.001, "at the foot: {top} + {size}");
    }
}

#[cfg(test)]
mod file_tests {
    use super::*;

    #[test]
    fn the_file_list_scrolls_and_stops_at_both_ends() {
        let mut first = 0usize;
        first = scroll_files(first, 3, false, 20, 8);
        assert_eq!(first, 0, "already at the top");
        first = scroll_files(first, 5, true, 20, 8);
        assert_eq!(first, 5);
        // Twenty files in an eight row list leaves twelve rows to scroll.
        first = scroll_files(first, 99, true, 20, 8);
        assert_eq!(first, 12);
        assert_eq!(scroll_files(first, 1, true, 20, 8), 12, "already at the bottom");
        assert_eq!(scroll_files(first, 99, false, 20, 8), 0);
        // A list that fits has nowhere to go and no thumb to say otherwise.
        assert!(file_thumb(0, 4, 8).is_none());
        assert!(file_thumb(12, 20, 8).is_some(), "twenty files in eight rows");
    }

    #[test]
    fn the_list_comes_back_to_the_file_the_agent_touched() {
        assert_eq!(reveal_file(0, 15, 20, 5), 11, "scrolled by the least it takes");
        // Already showing, so it is left where the reader put it.
        assert_eq!(reveal_file(11, 13, 20, 5), 11);
        // And upwards, when the touched file is above the window.
        assert_eq!(reveal_file(11, 2, 20, 5), 2);
    }
}
