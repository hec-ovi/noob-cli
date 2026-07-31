//! What a right click opens, and what its rows do.
//!
//! A menu is not part of the window: it floats above it. [`crate::view`] draws
//! it last and hit tests it first, which is the whole of what floating means
//! here, and this module is only the model of what is in it.
//!
//! Two menus, because there are two things worth pointing at. Right click the
//! prompt and you get the two things a text field can do. Right click a pane or
//! the tab that names it and you get the things a widget can do.
//!
//! A row that cannot act is greyed rather than absent, so a menu is the same
//! shape and the same height every time it opens for the same target. A menu
//! that grows a row when there is a selection moves every row under it, and the
//! pointer has to read the whole thing again to find the one it came for.
//!
//! ## Groups
//!
//! The pane's menu is not a flat list of acts. It is a few named groups that
//! open, and the rows inside a group only exist while that group is open. Two
//! of them: Settings, holding one row per section of the settings panel, and
//! Widgets, holding one row per widget. A group's rows are inserted into
//! [`Menu::rows`] straight under their header, one step further in
//! ([`Row::depth`]), so the whole menu is still one list and a row is still one
//! number wherever it is drawn.
//!
//! This replaced a flyout: the widget list used to be a second box beside the
//! menu, and it was the only thing in the menu that could open at all. Two
//! boxes at two anchors could hold exactly one submenu between them (one split
//! point, one scroll offset, one anchor hardwired to the last row), so a second
//! group was not expressible. A group that opens in place is one box, one
//! scroll and any number of groups, and it is what a long menu has to be to
//! read as a few names rather than as one column of everything.
//!
//! The cost is real and is paid on purpose: opening a group moves the rows
//! under it. Which is why a group opens from the row's own mark (a chevron that
//! turns down), why the rows it inserts are indented, and why the keyboard
//! walks the menu too, so a group can be opened and shut without the pointer
//! having to re-read the column each time.

use crate::dock::{Dock, Space, View};
use crate::icons;
use crate::settings::SECTIONS;

/// Columns a row reserves at its end for the group marker: the mark and the
/// space in front of it. Reserved in [`Menu::width_chars`] rather than left to
/// the drawing, or a long label and the marker would be written over each other.
pub const MARKER_COLUMNS: usize = 2;

/// Columns a row inside an open group is written in from the rows above it, per
/// step of depth. Two, which is the gutter every row already spends on its own
/// mark, so a child's mark starts where its parent's label does.
pub const INDENT_COLUMNS: usize = 2;

/// One thing a menu can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// Copy what is selected in the prompt.
    Copy,
    /// Put the clipboard into the prompt.
    Paste,
    /// The group holding one row per section of the settings panel. Carries
    /// whether it is open.
    ///
    /// It used to be one row that opened the panel on whatever section it was
    /// last left on. Every section of that panel is a destination, and a menu
    /// that can only reach the first of them is a menu that makes you go and
    /// find the rest.
    Settings(bool),
    /// One section of the settings panel, by its place in
    /// [`crate::settings::SECTIONS`]. Opens the panel with that section chosen.
    Section(usize),
    /// Copy what is selected in the pane the menu opened over.
    CopySelection,
    /// Take the pane out of the window, via `Dock::hide`.
    Close,
    /// The group holding one row per widget. Carries whether it is open.
    Widgets(bool),
    /// One widget on that list, and whether it is out of the window. A switch:
    /// picking a closed one puts it back, picking one that is in the window
    /// takes it out.
    Widget(View, bool),
    /// Carry on the saved session the menu was opened over, which is what
    /// pressing the row does anyway. On the menu because a menu with one row
    /// says the row it does not have is the only thing you can do here.
    OpenSession,
    /// Delete that session's transcript, and the note saying which folder it
    /// belonged to. The only row in this window that destroys anything, which
    /// is why it is the only row that is pressed twice.
    ///
    /// Carries whether the first press has been made. Armed, the row reads
    /// "sure?" instead of "Delete" and the next press on it is the delete;
    /// see [`Menu::press_delete`].
    DeleteSession(bool),
}

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Item::Copy => "Copy",
            Item::Paste => "Paste",
            Item::Settings(_) => "Settings",
            // The section's own name, as the rail beside the panel writes it,
            // so the row and the place it lands on read as the same thing.
            Item::Section(at) => SECTIONS.get(at).copied().unwrap_or(""),
            Item::CopySelection => "Copy selection",
            Item::Close => "Close this widget",
            Item::Widgets(_) => "Widgets",
            // The tab's own name, so the list reads as the tabs it is a list of.
            Item::Widget(view, _) => view.label(),
            Item::OpenSession => "Open",
            Item::DeleteSession(false) => "Delete",
            // The word the settings panel's own delete uses once it has been
            // pressed once. The same act asked the same way, so the two read as
            // one product rather than as two deletes with two manners.
            Item::DeleteSession(true) => "sure?",
        }
    }

    /// The label the box is measured from: for a row that changes what it says
    /// while the menu is open, the longest of its wordings.
    ///
    /// A menu that narrowed when the delete armed would slide its rows sideways
    /// under a pointer that has not moved, and take the row out from under it:
    /// the box is hit tested where it is drawn, so a row that shrank away from
    /// the pointer reads as the pointer leaving the menu and disarms the very
    /// press it just asked for.
    fn sizing_label(self) -> &'static str {
        match self {
            Item::DeleteSession(_) => Item::DeleteSession(false).label(),
            _ => self.label(),
        }
    }

    /// Whether the row is waiting for a second press before it destroys
    /// something. What the drawing puts it in the warning colour for.
    pub fn warns(self) -> bool {
        matches!(self, Item::DeleteSession(true))
    }

    /// Whether this row is a group header, and whether that group is open.
    /// `None` for a row that acts.
    ///
    /// The one place the two kinds of row are told apart. The drawing reads it
    /// for the mark at the end of the row and for the weight the label is
    /// written in, and [`Menu::fold`] reads it to know what a press does.
    pub fn group(self) -> Option<bool> {
        match self {
            Item::Settings(open) | Item::Widgets(open) => Some(open),
            _ => None,
        }
    }

    /// The glyph in front of the label. Every row carries one, so the type is
    /// still an `Option` only because the drawing already handles a row without
    /// a mark and a future row may not have one.
    ///
    /// Named in [`crate::icons`] rather than written here, because a codepoint
    /// the embedded font lacks draws as nothing at all and the coverage test
    /// over there is what catches that.
    ///
    /// Written out variant by variant, with no catch-all: a new row then fails
    /// to compile here rather than shipping with a blank gutter nobody notices.
    pub fn icon(self) -> Option<char> {
        match self {
            // The gear on the group, and the same gear on every section inside
            // it. A child of a group is that group, so a second mark there
            // would say the row is a different kind of thing.
            Item::Settings(_) | Item::Section(_) => Some(icons::SETTINGS),
            // Both copies wear the same mark, the prompt's and the pane's. They
            // are the same act on two different things, and a prompt menu with a
            // mark on Paste and none on Copy above it reads as half drawn.
            Item::Copy | Item::CopySelection => Some(icons::COPY),
            Item::Paste => Some(icons::PASTE),
            Item::Close => Some(icons::CLOSE_WIDGET),
            // The row that lists every widget, marked with what it lists. It
            // keeps its end marker as well: the two say different things, one
            // what the row is and one that the row opens.
            Item::Widgets(_) => Some(icons::WIDGETS),
            // A box with a tick in it for a widget that is in the window, an
            // empty one for a widget that is out. The row is a switch, so it has
            // to say which way it is set before it is pressed: without the mark
            // a click on it is a coin flip.
            Item::Widget(_, hidden) => Some(match hidden {
                true => icons::UNCHECKED,
                false => icons::CHECKED,
            }),
            // The mark the picker's own Open button wears, because it is the
            // same act reached another way, and the bin for the row that is not.
            Item::OpenSession => Some(icons::CONFIRM),
            // The bin either way. The gutter says what the row is, and what the
            // row is does not change when it starts asking.
            Item::DeleteSession(_) => Some(icons::TRASH),
        }
    }

    /// The glyph at the END of the row, for a row that opens a group. The whole
    /// of how a row that opens is told apart from a row that acts, along with
    /// the weight its label is written in.
    ///
    /// A chevron pointing right while the group is shut and down while it is
    /// open, which is what every other tree on the desktop says and what a row
    /// whose rows appear underneath it has to say: the same right-pointing
    /// chevron in both states said the rows were out to the side, which is
    /// where they used to be and no longer are.
    ///
    /// At the end rather than in the gutter in front, which is where the icons
    /// above go: a mark in the gutter says what this row is, a mark at the end
    /// says what pressing it does to the rows under it.
    pub fn marker(self) -> Option<char> {
        match self.group() {
            Some(true) => Some(icons::SUBMENU_OPEN),
            Some(false) => Some(icons::SUBMENU),
            None => None,
        }
    }
}

/// One row: an item, whether it can be picked right now, and how far in it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub item: Item,
    pub enabled: bool,
    /// 0 for a row of the menu itself, 1 for a row inside an open group. What
    /// the drawing indents by, so a group's rows read as belonging to the
    /// header above them rather than as more of the menu.
    pub depth: usize,
}

impl Row {
    fn act(item: Item, enabled: bool) -> Row {
        Row {
            item,
            enabled,
            depth: 0,
        }
    }
}

/// What the right click landed on, which is what the rows act on. Carried on
/// the menu so picking a row does not have to hit test the pointer again: by
/// then the pointer is over the menu, not over what opened it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Input,
    /// A pane or its tab, and the space that pane is in.
    Widget(View, Space),
    /// A row of the picker's session list, by its number in that list.
    ///
    /// The number rather than the id, because that is what every other press on
    /// the picker carries and what the model's own methods take. The row it
    /// points at cannot move while the menu is up: nothing rebuilds the list
    /// until a row of the menu is picked.
    Session(usize),
    /// The document beside the entry list on the settings panel. It carries
    /// nothing: the one row acts on whatever is highlighted there, and the
    /// panel already knows which entry that is.
    SettingsDoc,
}

/// An open menu: where it was opened, what it was opened on, and its rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    /// Where the pointer was. The layout turns this into a box, clamping it so
    /// a menu opened near an edge still fits on screen.
    pub at: (f32, f32),
    pub target: Target,
    /// Every row on it, in the order they are drawn: the menu's own rows, with
    /// each open group's rows inserted straight under their header.
    pub rows: Vec<Row>,
    /// Which row the box starts at, when the window is too short to hold all of
    /// them. Kept here rather than in the layout because it outlives a frame:
    /// the wheel moves it and the next frame draws from it.
    pub first: usize,
    /// Which row the keys are on, or none while the menu has only been pointed
    /// at. Separate from the pointer's own highlight, and each of the two takes
    /// the other one down when it moves: two lit rows in one menu is two
    /// answers to which row Enter presses.
    pub cursor: Option<usize>,
}

impl Menu {
    /// The prompt's menu. Paste is always available: the clipboard is the
    /// display server's, and asking it whether it holds anything means
    /// connecting to it, which is work a right click has no reason to do.
    pub fn for_input(at: (f32, f32), has_selection: bool) -> Menu {
        Menu::of(
            at,
            Target::Input,
            vec![
                Row::act(Item::Copy, has_selection),
                Row::act(Item::Paste, true),
            ],
        )
    }

    /// A pane's menu: two groups and two acts.
    ///
    /// Settings opens shut, and so does Widgets. A menu that opened with every
    /// group already out is the wall of rows the groups exist to stop being.
    pub fn for_widget(at: (f32, f32), view: View, space: Space, has_selection: bool) -> Menu {
        Menu::of(
            at,
            Target::Widget(view, space),
            vec![
                Row::act(Item::Settings(false), true),
                Row::act(Item::CopySelection, has_selection),
                Row::act(Item::Close, true),
                Row::act(Item::Widgets(false), true),
            ],
        )
    }

    /// A saved session's menu: carry it on, or delete it.
    ///
    /// `gone` is whether the folder it was started in has been deleted since. A
    /// session cannot be resumed into a directory that is not there, so Open is
    /// greyed rather than missing, the way every other row that cannot act is:
    /// the menu is the same two rows either way, and Delete still works, which
    /// is the row that session is most likely to be right clicked for.
    ///
    /// Delete opens unarmed, and a menu is built fresh every right click, so
    /// there is no way to open one that is already asking.
    pub fn for_session(at: (f32, f32), index: usize, gone: bool) -> Menu {
        Menu::of(
            at,
            Target::Session(index),
            vec![
                Row::act(Item::OpenSession, !gone),
                Row::act(Item::DeleteSession(false), true),
            ],
        )
    }

    /// The settings document's menu: one row, which copies what is highlighted
    /// in it.
    ///
    /// One row rather than none, because the panel is a takeover: there is no
    /// pane behind it to close, no settings to open from it, and nothing else a
    /// right click on a page of prose could be asking for. Greyed when nothing
    /// is highlighted, the way every other row that cannot act is, so the menu
    /// is the same shape either way.
    pub fn for_settings_doc(at: (f32, f32), has_selection: bool) -> Menu {
        Menu::of(
            at,
            Target::SettingsDoc,
            vec![Row::act(Item::CopySelection, has_selection)],
        )
    }

    fn of(at: (f32, f32), target: Target, rows: Vec<Row>) -> Menu {
        Menu {
            at,
            target,
            rows,
            first: 0,
            cursor: None,
        }
    }

    /// The widget the menu was opened over, if it was opened over one. What
    /// every row but a group's acts on, so a caller can ask whether the list
    /// just took that widget out of the window.
    pub fn target_view(&self) -> Option<View> {
        match self.target {
            Target::Widget(view, _) => Some(view),
            Target::Input | Target::Session(_) | Target::SettingsDoc => None,
        }
    }

    /// Open the group at `index`, or shut it again. Anything that is not a
    /// group header leaves the menu alone.
    ///
    /// Open, its rows go in straight under it, one step further in. Shut, every
    /// row under it that is further in comes back out, which takes a nested
    /// group's rows with it because they are further in still.
    pub fn fold(&mut self, index: usize, dock: &Dock) -> bool {
        let Some(row) = self.rows.get(index) else {
            return false;
        };
        let Some(open) = row.item.group() else {
            return false;
        };
        let depth = row.depth;
        let item = row.item;
        // Whatever this group had out goes first, open or shut: shutting is
        // only this half, and opening a group that somehow already had rows
        // would double them.
        let end = self.end_of(index);
        self.rows.drain(index + 1..end);
        self.rows[index].item = match item {
            Item::Settings(_) => Item::Settings(!open),
            Item::Widgets(_) => Item::Widgets(!open),
            other => other,
        };
        if !open {
            let inside: Vec<Row> = match item {
                Item::Settings(_) => (0..SECTIONS.len())
                    .map(|at| Row {
                        item: Item::Section(at),
                        enabled: true,
                        depth: depth + 1,
                    })
                    .collect(),
                // Nothing on it is greyed: every row is a switch and every
                // switch can be thrown either way.
                _ => View::ALL
                    .into_iter()
                    .map(|view| Row {
                        item: Item::Widget(view, dock.is_hidden(view)),
                        enabled: true,
                        depth: depth + 1,
                    })
                    .collect(),
            };
            self.rows.splice(index + 1..index + 1, inside);
        }
        // The rows moved, so anything counted against them is stale. The
        // cursor follows the header only when the keys had it: a group opened
        // by the pointer must not light a row the keys are not on.
        self.first = 0;
        if self.cursor.is_some() {
            self.cursor = Some(index);
        }
        true
    }

    /// One past the last row belonging to the group at `index`: every row after
    /// it that is further in.
    fn end_of(&self, index: usize) -> usize {
        let depth = self.rows[index].depth;
        let mut end = index + 1;
        while self.rows.get(end).is_some_and(|row| row.depth > depth) {
            end += 1;
        }
        end
    }

    /// Which group row `index` is inside, if any: the nearest header above it
    /// that is one step further out.
    pub fn parent_of(&self, index: usize) -> Option<usize> {
        let depth = self.rows.get(index)?.depth;
        if depth == 0 {
            return None;
        }
        self.rows[..index]
            .iter()
            .rposition(|row| row.depth < depth && row.item.group().is_some())
    }

    /// Read the widget rows' marks off the dock again, keeping the group open
    /// and keeping where it is scrolled to.
    ///
    /// A widget row is a switch, and a menu that stays open after one is thrown
    /// would otherwise go on saying the widget is where it was. The rows
    /// themselves do not move, so the pointer is still over the row it just
    /// pressed and can press it back.
    pub fn relist(&mut self, dock: &Dock) -> bool {
        let mut moved = false;
        for row in &mut self.rows {
            if let Item::Widget(view, was) = row.item {
                let now = dock.is_hidden(view);
                moved |= now != was;
                row.item = Item::Widget(view, now);
            }
        }
        moved
    }

    /// Move the menu, when the window is too short to hold every row. `rows` is
    /// how many are on screen, which only the layout knows.
    pub fn scroll(&mut self, by: usize, down: bool, rows: usize) -> bool {
        let most = self.rows.len().saturating_sub(rows);
        let next = match down {
            true => (self.first + by).min(most),
            false => self.first.saturating_sub(by).min(most),
        };
        let moved = next != self.first;
        self.first = next;
        moved
    }

    /// Move the keyboard's cursor one row, skipping the rows that cannot act.
    ///
    /// From nowhere it lands on the first row that can act, whichever way it is
    /// going, so the first arrow press after a right click puts it somewhere
    /// useful. It stops at both ends rather than wrapping: a menu is short
    /// enough to see the end of.
    pub fn walk(&mut self, down: bool, rows: usize) -> bool {
        let next = match self.cursor {
            None => match down {
                true => self.actionable(0, true),
                false => self.actionable(self.rows.len().saturating_sub(1), false),
            },
            Some(at) => match down {
                true => self.actionable(at + 1, true),
                false => at.checked_sub(1).and_then(|a| self.actionable(a, false)),
            },
        };
        let Some(next) = next else {
            return false;
        };
        let moved = self.cursor != Some(next);
        self.cursor = Some(next);
        self.show(rows);
        moved
    }

    /// The first row from `from` in the given direction that can be picked.
    fn actionable(&self, from: usize, down: bool) -> Option<usize> {
        match down {
            true => (from..self.rows.len()).find(|at| self.rows[*at].enabled),
            false => (0..=from.min(self.rows.len().checked_sub(1)?))
                .rev()
                .find(|at| self.rows[*at].enabled),
        }
    }

    /// Bring the cursor's row on screen, when `rows` of the menu are showing.
    pub fn show(&mut self, rows: usize) {
        let Some(at) = self.cursor else {
            return;
        };
        let rows = rows.max(1);
        if at < self.first {
            self.first = at;
        } else if at >= self.first + rows {
            self.first = at + 1 - rows;
        }
        self.first = self.first.min(self.rows.len().saturating_sub(rows));
    }

    /// What the right arrow does: open the group the cursor is on, and step
    /// into it when it is already open.
    pub fn unfold_here(&mut self, dock: &Dock, rows: usize) -> bool {
        let Some(at) = self.cursor else {
            return false;
        };
        match self.rows.get(at).and_then(|row| row.item.group()) {
            Some(false) => {
                self.fold(at, dock);
                self.show(rows);
                true
            }
            Some(true) => self.walk(true, rows),
            None => false,
        }
    }

    /// What the left arrow does: shut the group the cursor is on, or step out
    /// to the header of the group it is inside.
    pub fn fold_here(&mut self, dock: &Dock, rows: usize) -> bool {
        let Some(at) = self.cursor else {
            return false;
        };
        if self.rows.get(at).and_then(|row| row.item.group()) == Some(true) {
            self.fold(at, dock);
            self.show(rows);
            return true;
        }
        let Some(parent) = self.parent_of(at) else {
            return false;
        };
        self.cursor = Some(parent);
        self.show(rows);
        true
    }

    /// Press the delete row at `index`. `true` means delete it now.
    ///
    /// The first press only arms the row: it starts reading "sure?" and the
    /// menu stays open under the pointer for the second one. Two presses
    /// because a transcript is gone once it is deleted and nothing in this
    /// window can put it back, which is the same reason the settings panel's
    /// delete is pressed twice, and the same wording.
    ///
    /// Anything but a second press on that row leaves it alone: the pointer
    /// moving off it disarms it through [`Menu::point_at`], and every way of
    /// closing the menu takes the arming with it, because the arming is on the
    /// menu and nowhere else.
    pub fn press_delete(&mut self, index: usize) -> bool {
        let Some(row) = self.rows.get_mut(index) else {
            return false;
        };
        match row.item {
            Item::DeleteSession(true) => {
                row.item = Item::DeleteSession(false);
                true
            }
            Item::DeleteSession(false) => {
                row.item = Item::DeleteSession(true);
                false
            }
            _ => false,
        }
    }

    /// The pointer is over row `row`, or over none of them when it is `None`.
    /// Returns whether anything changed, which is whether the menu needs
    /// drawing again.
    ///
    /// Two things happen here. An armed delete disarms as soon as the pointer
    /// is anywhere else: a menu left sitting on "sure?" while the pointer has
    /// wandered to Open and back would take what reads as a first press and
    /// delete on it. And the keyboard's cursor goes out, because the pointer
    /// has taken over saying which row is next.
    pub fn point_at(&mut self, row: Option<usize>) -> bool {
        let mut changed = false;
        if row.is_some() && self.cursor.is_some() {
            self.cursor = None;
            changed = true;
        }
        let Some(armed) = self.arming() else {
            return changed;
        };
        if row == Some(armed) {
            return changed;
        }
        self.rows[armed].item = Item::DeleteSession(false);
        true
    }

    /// Which row is asking for a second press.
    pub fn arming(&self) -> Option<usize> {
        self.rows.iter().position(|row| row.item.warns())
    }

    /// What picking the row at `index` does, or nothing when that row is
    /// disabled or does not exist. The one place a pointer position becomes an
    /// action, so a greyed row cannot act by some other route.
    pub fn pick(&self, index: usize) -> Option<Item> {
        self.rows
            .get(index)
            .filter(|row| row.enabled)
            .map(|row| row.item)
    }

    /// The longest row in the menu, in characters: how far in it is written,
    /// its label, and the room kept at the end of a row that opens a group.
    /// What the layout sizes the box from, so every row is as wide as the
    /// widest one.
    ///
    /// Measured over every row including the ones inside open groups, because
    /// they are in the same box now. A group that opened wider than its menu
    /// would be written off the edge of it.
    pub fn width_chars(&self) -> usize {
        self.rows
            .iter()
            .map(|row| {
                row.depth * INDENT_COLUMNS
                    + row.item.sizing_label().chars().count()
                    + match row.item.marker() {
                        Some(_) => MARKER_COLUMNS,
                        None => 0,
                    }
            })
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(menu: &Menu) -> Vec<Item> {
        menu.rows.iter().map(|row| row.item).collect()
    }

    /// How many rows are inside open groups.
    fn opened(menu: &Menu) -> usize {
        menu.rows.iter().filter(|row| row.depth > 0).count()
    }

    #[test]
    fn the_prompt_gets_the_two_things_a_text_field_can_do() {
        let menu = Menu::for_input((10.0, 10.0), true);
        assert_eq!(items(&menu), vec![Item::Copy, Item::Paste]);
        assert_eq!(menu.target, Target::Input);
        assert_eq!(menu.pick(0), Some(Item::Copy));
        assert_eq!(menu.pick(1), Some(Item::Paste));
        assert_eq!(menu.pick(2), None, "there is no third row");
    }

    /// Two groups and two acts, both groups shut.
    ///
    /// This asserted `Item::Settings` with no state on it, back when that row
    /// opened the panel rather than opening a group.
    #[test]
    fn a_pane_gets_a_settings_group_a_copy_a_close_and_a_widgets_group() {
        let menu = Menu::for_widget((10.0, 10.0), View::Plan, Space::TopRight, true);
        assert_eq!(
            items(&menu),
            vec![
                Item::Settings(false),
                Item::CopySelection,
                Item::Close,
                Item::Widgets(false)
            ]
        );
        assert_eq!(menu.target, Target::Widget(View::Plan, Space::TopRight));
        assert_eq!(menu.pick(1), Some(Item::CopySelection));
        assert_eq!(menu.pick(2), Some(Item::Close));
        assert_eq!(menu.pick(3), Some(Item::Widgets(false)));
        assert_eq!(opened(&menu), 0, "both groups open shut");
        for row in &menu.rows {
            assert_eq!(row.depth, 0, "nothing is inside a group yet");
        }
    }

    /// The correction this round: the widget list is a group that opens under
    /// its own header, in the same box, rather than a second box out to the
    /// side. Shut it is one row, open it is that row and every widget under it,
    /// one step further in.
    #[test]
    fn the_widget_row_opens_into_one_row_per_widget_and_shuts_again() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((10.0, 10.0), View::Plan, Space::TopLeft, false);
        let shut = items(&menu);
        let at = 3;

        assert!(menu.fold(at, &dock));
        assert_eq!(menu.rows[at].item, Item::Widgets(true));
        assert_eq!(opened(&menu), View::ALL.len());
        assert_eq!(menu.rows.len(), shut.len() + View::ALL.len());
        assert_eq!(
            menu.rows[at + 1..]
                .iter()
                .map(|row| {
                    assert_eq!(row.depth, 1, "a widget row is inside the group");
                    match row.item {
                        Item::Widget(view, _) => view,
                        other => panic!("{other:?} is not a widget row"),
                    }
                })
                .collect::<Vec<_>>(),
            View::ALL.to_vec(),
            "the list is in the one order, so it is in the same place every time"
        );
        // Eight rows, and none of them is the pane of failed calls: the list is
        // built from `View::ALL`, so a variant left behind would still be
        // switchable from here with nothing to draw.
        assert_eq!(opened(&menu), 8);
        for row in &menu.rows[at + 1..] {
            let Item::Widget(view, _) = row.item else {
                panic!("{:?} is not a widget row", row.item)
            };
            assert_ne!(view.label(), "DEBUG");
        }

        assert!(menu.fold(at, &dock));
        assert_eq!(opened(&menu), 0);
        assert_eq!(items(&menu), shut, "it shuts back to what it opened as");
    }

    /// The settings group holds one row per section of the panel, in rail
    /// order, and each of them names the section it lands on.
    #[test]
    fn the_settings_row_opens_into_one_row_per_section_of_the_panel() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((10.0, 10.0), View::Plan, Space::TopLeft, false);
        assert!(menu.fold(0, &dock));
        assert_eq!(menu.rows[0].item, Item::Settings(true));
        assert_eq!(opened(&menu), SECTIONS.len());
        for (at, name) in SECTIONS.iter().enumerate() {
            let row = menu.rows[1 + at];
            assert_eq!(row.item, Item::Section(at));
            assert_eq!(row.depth, 1);
            assert_eq!(row.item.label(), *name);
            assert_eq!(menu.pick(1 + at), Some(Item::Section(at)));
            assert_eq!(row.item.marker(), None, "a section is a place, not a group");
        }
        // The rows below it kept their order and their meaning, one group's
        // worth further down.
        assert_eq!(
            menu.pick(1 + SECTIONS.len() + 2),
            Some(Item::Widgets(false))
        );
        assert!(menu.fold(0, &dock));
        assert_eq!(opened(&menu), 0);
    }

    /// Two groups in one menu, which is the thing the flyout could not do: one
    /// box, one scroll, and each group's rows under its own header.
    #[test]
    fn both_groups_open_at_once_and_each_shuts_on_its_own() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((10.0, 10.0), View::Plan, Space::TopLeft, false);
        menu.fold(0, &dock);
        let widgets = menu
            .rows
            .iter()
            .position(|row| matches!(row.item, Item::Widgets(_)))
            .expect("the widgets row is still there");
        menu.fold(widgets, &dock);
        assert_eq!(opened(&menu), SECTIONS.len() + View::ALL.len());
        assert_eq!(menu.rows[0].item, Item::Settings(true));
        assert_eq!(menu.rows[widgets].item, Item::Widgets(true));
        // Shutting the first one takes its own rows and leaves the other's.
        menu.fold(0, &dock);
        assert_eq!(opened(&menu), View::ALL.len());
        assert_eq!(menu.rows[0].item, Item::Settings(false));
        let widgets = menu
            .rows
            .iter()
            .position(|row| row.item == Item::Widgets(true))
            .expect("the widgets group is still open");
        assert_eq!(menu.rows[widgets + 1].depth, 1);
    }

    /// A row that is not a group header does not open, and neither does a row
    /// that is not there.
    #[test]
    fn nothing_but_a_group_header_opens() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, true);
        assert!(!menu.fold(1, &dock), "Copy selection is not a group");
        assert!(!menu.fold(99, &dock), "there is no row 99");
        assert_eq!(opened(&menu), 0);
        let mut prompt = Menu::for_input((0.0, 0.0), false);
        assert!(!prompt.fold(0, &dock));
        assert!(!prompt.fold(1, &dock));
        assert_eq!(opened(&prompt), 0);
    }

    /// The marker is written after the label, so the row has to be wide enough
    /// for both, and a row inside a group has to be wide enough for its indent
    /// as well.
    #[test]
    fn a_row_that_opens_keeps_room_at_its_end_and_an_open_group_keeps_its_indent() {
        let dock = Dock::new();
        let menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        let widgets = "Widgets".chars().count();
        assert!(
            menu.width_chars() >= widgets + MARKER_COLUMNS,
            "the marker and the label would collide"
        );
        // With a label longer than every other row's, the width is that row's.
        let mut wide = menu.clone();
        wide.rows.retain(|row| matches!(row.item, Item::Widgets(_)));
        assert_eq!(wide.width_chars(), widgets + MARKER_COLUMNS);
        // A row with no marker keeps no room, or every menu would be two
        // columns wider than it needs to be.
        let plain = Menu::for_input((0.0, 0.0), false);
        assert_eq!(plain.width_chars(), Item::Paste.label().chars().count());

        // Open the sections and the widest row is a section name written one
        // indent in, if that is longer than any top level row.
        let mut open = menu.clone();
        open.fold(0, &dock);
        let widest = SECTIONS
            .iter()
            .map(|name| name.chars().count())
            .max()
            .expect("there are sections");
        assert_eq!(
            open.width_chars(),
            menu.width_chars().max(widest + INDENT_COLUMNS),
            "a group's rows are measured in the same box as the menu's"
        );
    }

    /// Every row of a group is a switch, and a switch has to say which way it
    /// is set before it is pressed: a tick in a box for a widget in the window,
    /// an empty box for one that is out.
    #[test]
    fn the_list_marks_what_is_closed_and_every_row_of_it_can_be_picked() {
        let dock = Dock::hiding(&[View::Hardware, View::Files]);
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        let at = 3;
        menu.fold(at, &dock);
        for (step, view) in View::ALL.into_iter().enumerate() {
            let index = at + 1 + step;
            let hidden = matches!(view, View::Hardware | View::Files);
            assert_eq!(
                menu.pick(index),
                Some(Item::Widget(view, hidden)),
                "{view:?} is pickable and says whether it is closed"
            );
            assert_eq!(
                Item::Widget(view, hidden).icon(),
                Some(match hidden {
                    true => icons::UNCHECKED,
                    false => icons::CHECKED,
                })
            );
            assert_eq!(Item::Widget(view, hidden).label(), view.label());
            assert_eq!(
                Item::Widget(view, hidden).marker(),
                None,
                "a widget row is a switch, not a group"
            );
        }
        assert_eq!(menu.pick(at + 1 + View::ALL.len()), None);
    }

    /// A group header carries two marks: the one in its gutter saying what the
    /// row is, and the chevron at its end saying it opens. The chevron turns
    /// down while the group is open, because its rows are underneath it now.
    ///
    /// It asserted the same right-pointing chevron in both states, which is
    /// what a row whose rows fly out to the side says and is no longer what
    /// these rows do.
    #[test]
    fn a_group_header_carries_a_mark_in_its_gutter_and_a_chevron_that_turns() {
        assert_eq!(Item::Widgets(false).marker(), Some(icons::SUBMENU));
        assert_eq!(Item::Widgets(true).marker(), Some(icons::SUBMENU_OPEN));
        assert_eq!(Item::Settings(false).marker(), Some(icons::SUBMENU));
        assert_eq!(Item::Settings(true).marker(), Some(icons::SUBMENU_OPEN));
        assert_ne!(
            icons::SUBMENU,
            icons::SUBMENU_OPEN,
            "a group that is open and one that is shut have to be told apart"
        );
        for open in [false, true] {
            assert_eq!(Item::Widgets(open).icon(), Some(icons::WIDGETS));
            assert_eq!(Item::Widgets(open).group(), Some(open));
            assert_eq!(Item::Settings(open).group(), Some(open));
            assert_ne!(
                icons::WIDGETS,
                icons::SUBMENU,
                "the two marks on this row have to be told apart"
            );
        }
        for item in [
            Item::Copy,
            Item::Paste,
            Item::Section(0),
            Item::CopySelection,
            Item::Close,
            Item::Widget(View::Output, false),
            Item::OpenSession,
            Item::DeleteSession(false),
        ] {
            assert_eq!(item.marker(), None, "{item:?} does not open");
            assert_eq!(item.group(), None, "{item:?} is not a group");
        }
    }

    /// The marks follow the dock while the menu stays open, so a row that was
    /// just switched says what it did rather than what it used to say. The rows
    /// keep their places, so the pointer is still over the row it pressed.
    #[test]
    fn the_list_reads_its_marks_off_the_dock_again_without_moving_a_row() {
        let mut dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        menu.fold(3, &dock);
        let places: Vec<View> = menu.rows[4..]
            .iter()
            .map(|row| match row.item {
                Item::Widget(view, _) => view,
                other => panic!("{other:?} is not a widget row"),
            })
            .collect();

        assert!(dock.hide(View::Hardware));
        assert!(menu.relist(&dock));
        assert_eq!(
            menu.rows[4..]
                .iter()
                .map(|row| match row.item {
                    Item::Widget(view, _) => view,
                    other => panic!("{other:?} is not a widget row"),
                })
                .collect::<Vec<_>>(),
            places,
            "no row moved"
        );
        for (step, view) in View::ALL.into_iter().enumerate() {
            assert_eq!(
                menu.pick(4 + step),
                Some(Item::Widget(view, view == View::Hardware))
            );
        }
        // Nothing to read, either because the group is shut or because the menu
        // has no widget row at all.
        menu.fold(3, &dock);
        assert!(!menu.relist(&dock));
        assert!(!Menu::for_input((0.0, 0.0), false).relist(&dock));
    }

    /// Every row but a group's acts on the widget the menu was opened over, so
    /// the menu has to be able to say which one that is.
    #[test]
    fn a_menu_names_the_widget_it_was_opened_over() {
        let menu = Menu::for_widget((0.0, 0.0), View::Agents, Space::TopRight, false);
        assert_eq!(menu.target_view(), Some(View::Agents));
        assert_eq!(Menu::for_input((0.0, 0.0), false).target_view(), None);
    }

    /// Both groups out is seventeen rows, which does not fit under a menu
    /// opened near the bottom of a short window, so the menu scrolls, and it
    /// cannot be scrolled off either end.
    #[test]
    fn the_menu_scrolls_and_stops_at_both_ends() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        assert!(
            !menu.scroll(1, true, 4),
            "a menu that fits has nothing to scroll"
        );
        menu.fold(3, &dock);
        let rows = menu.rows.len();
        assert!(menu.scroll(2, true, 3));
        assert_eq!(menu.first, 2);
        assert!(menu.scroll(99, true, 3));
        assert_eq!(menu.first, rows - 3, "it stops at the last row");
        assert!(!menu.scroll(4, true, 3));
        assert!(menu.scroll(99, false, 3));
        assert_eq!(menu.first, 0);
        assert!(!menu.scroll(1, false, 3));
        // A menu that is entirely on screen does not move at all.
        assert!(!menu.scroll(1, true, rows));
        // And folding it away puts the menu back at the top.
        menu.scroll(4, true, 3);
        menu.fold(3, &dock);
        assert_eq!(menu.first, 0);
    }

    /// The keyboard walks the menu, skips the rows that cannot act, and stops
    /// at both ends. There was no keyboard route at all before: a keystroke
    /// with a menu open only put it away.
    #[test]
    fn the_keys_walk_the_menu_and_skip_what_cannot_act() {
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        assert_eq!(menu.cursor, None, "it opens on no row");
        assert!(menu.walk(true, 8));
        assert_eq!(menu.cursor, Some(0), "the first press lands on the first row");
        assert!(menu.walk(true, 8));
        assert_eq!(menu.cursor, Some(2), "the greyed copy row was stepped over");
        assert!(menu.walk(true, 8));
        assert_eq!(menu.cursor, Some(3));
        assert!(!menu.walk(true, 8), "it stopped at the end");
        assert_eq!(menu.cursor, Some(3));
        assert!(menu.walk(false, 8));
        assert_eq!(menu.cursor, Some(2));
        assert!(menu.walk(false, 8));
        assert_eq!(menu.cursor, Some(0), "and over it going back");
        assert!(!menu.walk(false, 8));

        // Up from nowhere lands on the last row that acts, so both arrows are
        // useful on a menu that has only just opened.
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        assert!(menu.walk(false, 8));
        assert_eq!(menu.cursor, Some(3));
    }

    /// The pointer and the keys never both say which row is next: whichever one
    /// moved last owns it.
    #[test]
    fn pointing_at_a_row_takes_the_keyboards_cursor_off() {
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, true);
        menu.walk(true, 8);
        assert_eq!(menu.cursor, Some(0));
        assert!(menu.point_at(Some(2)));
        assert_eq!(menu.cursor, None);
        assert!(!menu.point_at(Some(2)), "nothing left to take off");
        // The pointer leaving the menu leaves the keys alone: nothing has taken
        // over, so there is nothing to hand back.
        menu.walk(true, 8);
        assert!(!menu.point_at(None));
        assert_eq!(menu.cursor, Some(0));
    }

    /// A group opens and shuts from the keyboard, and the left arrow steps out
    /// to the header when the cursor is already inside one.
    #[test]
    fn the_arrows_open_and_shut_a_group() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        menu.walk(true, 20);
        assert_eq!(menu.cursor, Some(0));

        assert!(menu.unfold_here(&dock, 20), "right did not open the group");
        assert_eq!(menu.rows[0].item, Item::Settings(true));
        assert_eq!(menu.cursor, Some(0), "the cursor stayed on the header");

        // Right again on an open group steps into it, onto its first row.
        assert!(menu.unfold_here(&dock, 20));
        assert_eq!(menu.cursor, Some(1));
        assert_eq!(menu.rows[1].item, Item::Section(0));
        assert!(
            !menu.unfold_here(&dock, 20),
            "a row that is not a group does not open"
        );

        // Left from inside steps out to the header; left again shuts it.
        assert!(menu.fold_here(&dock, 20));
        assert_eq!(menu.cursor, Some(0));
        assert!(menu.fold_here(&dock, 20));
        assert_eq!(menu.rows[0].item, Item::Settings(false));
        assert_eq!(opened(&menu), 0);
        assert!(
            !menu.fold_here(&dock, 20),
            "a shut group at the top level has nothing to close"
        );
    }

    /// A cursor walked past the bottom of a short window brings the menu with
    /// it, and one walked back up brings it back.
    #[test]
    fn walking_past_the_end_of_a_short_menu_scrolls_it() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        menu.fold(3, &dock);
        let rows = 4;
        for _ in 0..menu.rows.len() {
            menu.walk(true, rows);
            let at = menu.cursor.expect("the cursor is somewhere");
            assert!(
                at >= menu.first && at < menu.first + rows,
                "row {at} is off screen: first {}",
                menu.first
            );
        }
        assert_eq!(menu.cursor, Some(menu.rows.len() - 1));
        for _ in 0..menu.rows.len() {
            menu.walk(false, rows);
            let at = menu.cursor.expect("the cursor is somewhere");
            assert!(at >= menu.first && at < menu.first + rows, "row {at}");
        }
        assert_eq!(menu.first, 0);
    }

    /// The prompt's menu has no widgets to list and no settings group: there is
    /// no pane behind it.
    #[test]
    fn the_prompt_menu_has_no_group_at_all() {
        let dock = Dock::new();
        let mut menu = Menu::for_input((0.0, 0.0), false);
        assert!(!menu.fold(0, &dock));
        assert_eq!(items(&menu), vec![Item::Copy, Item::Paste]);
        assert_eq!(opened(&menu), 0);
    }

    /// The row is there either way, so the menu does not change shape under a
    /// pointer that is already on its way to a row further down.
    #[test]
    fn a_copy_with_nothing_to_copy_is_greyed_rather_than_missing() {
        for (with, without) in [
            (
                Menu::for_input((0.0, 0.0), true),
                Menu::for_input((0.0, 0.0), false),
            ),
            (
                Menu::for_widget((0.0, 0.0), View::Output, Space::TopLeft, true),
                Menu::for_widget((0.0, 0.0), View::Output, Space::TopLeft, false),
            ),
        ] {
            assert_eq!(items(&with), items(&without), "the shape changed");
            let copy = items(&with)
                .iter()
                .position(|item| matches!(item, Item::Copy | Item::CopySelection))
                .expect("there is a copy row");
            assert!(with.pick(copy).is_some());
            assert_eq!(without.pick(copy), None, "a greyed row cannot be picked");
            // And nothing else lost its footing with it.
            assert_eq!(
                without.pick(items(&without).len() - 1),
                with.pick(items(&with).len() - 1)
            );
        }
    }

    /// The settings row acts, in every menu that has it, and carries its mark.
    ///
    /// This asserted the row opened the panel on its own. It opens a group of
    /// five sections now, and every one of them opens the panel.
    #[test]
    fn the_settings_row_is_a_group_and_carries_its_mark() {
        for has_selection in [false, true] {
            let menu = Menu::for_widget((0.0, 0.0), View::Files, Space::BottomRight, has_selection);
            assert_eq!(menu.rows[0].item, Item::Settings(false));
            assert!(menu.rows[0].enabled);
            assert_eq!(menu.pick(0), Some(Item::Settings(false)));
        }
        assert_eq!(Item::Settings(false).icon(), Some(icons::SETTINGS));
        assert_eq!(Item::Section(0).icon(), Some(icons::SETTINGS));
        // This asserted `Item::Close.icon() == None`. The close row has its own
        // cross now, and it is a different codepoint from the window button that
        // shuts the application: the same mark on both would say the same thing.
        assert_eq!(Item::Close.icon(), Some(icons::CLOSE_WIDGET));
        assert_ne!(icons::CLOSE_WIDGET, icons::CLOSE);
    }

    /// Every row of every menu is marked, and no two rows that mean different
    /// things wear the same mark. The gutter is spent on every row whether or
    /// not it has a glyph in it (`view::MENU_GUTTER`), so an unmarked row is a
    /// blank column, not a narrower row: the four that shipped blank read as
    /// rows that had lost their icons.
    #[test]
    fn every_row_of_every_menu_is_marked_in_its_gutter() {
        let dock = Dock::hiding(&[View::Hardware]);
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, true);
        menu.fold(3, &dock);
        menu.fold(0, &dock);
        let prompt = Menu::for_input((0.0, 0.0), true);
        let session = Menu::for_session((0.0, 0.0), 0, false);
        for row in menu
            .rows
            .iter()
            .chain(prompt.rows.iter())
            .chain(session.rows.iter())
        {
            assert!(
                row.item.icon().is_some(),
                "{:?} has nothing in its gutter",
                row.item
            );
        }

        assert_eq!(Item::CopySelection.icon(), Some(icons::COPY));
        assert_eq!(Item::Copy.icon(), Some(icons::COPY), "the same act");
        assert_eq!(Item::Paste.icon(), Some(icons::PASTE));
        assert_eq!(Item::Close.icon(), Some(icons::CLOSE_WIDGET));
        assert_eq!(Item::Widgets(false).icon(), Some(icons::WIDGETS));
        assert_eq!(Item::Widgets(true).icon(), Some(icons::WIDGETS));

        // A copy row and a paste row that look alike are two coin flips, and the
        // two switch states have to stay clear of all of them. The gear is not
        // in this list: it is deliberately worn by the settings group and by
        // every section inside it, which are the same thing at two depths.
        let marks = [
            icons::COPY,
            icons::PASTE,
            icons::CLOSE_WIDGET,
            icons::WIDGETS,
            icons::SETTINGS,
            icons::CHECKED,
            icons::UNCHECKED,
            // The session menu's own two. Open is deliberately the picker's
            // confirm glyph and is not in this list twice, because no other
            // menu row wears it.
            icons::CONFIRM,
            icons::TRASH,
        ];
        for (step, one) in marks.iter().enumerate() {
            for other in &marks[step + 1..] {
                assert_ne!(one, other, "two rows wear U+{:04X}", *one as u32);
            }
        }
    }

    /// A saved session's menu: carry it on, or delete it. Both rows are there
    /// whatever the row is, and the one that cannot act is greyed rather than
    /// missing, which is the rule every other menu here follows.
    #[test]
    fn a_saved_session_gets_an_open_row_and_a_delete_row() {
        let menu = Menu::for_session((40.0, 90.0), 3, false);
        assert_eq!(
            items(&menu),
            vec![Item::OpenSession, Item::DeleteSession(false)]
        );
        assert_eq!(menu.target, Target::Session(3));
        assert_eq!(menu.at, (40.0, 90.0));
        assert_eq!(menu.pick(0), Some(Item::OpenSession));
        assert_eq!(menu.pick(1), Some(Item::DeleteSession(false)));
        assert_eq!(menu.pick(2), None, "there is no third row");
        assert_eq!(opened(&menu), 0, "a session menu has no group");
        assert_eq!(menu.target_view(), None, "and it is not a widget");

        // A session whose folder has been deleted cannot be resumed anywhere,
        // so Open is greyed. Delete still acts: that row is the reason the menu
        // was opened over a session like this one.
        let dead = Menu::for_session((40.0, 90.0), 3, true);
        assert_eq!(items(&dead), items(&menu), "the shape changed");
        assert_eq!(dead.pick(0), None);
        assert_eq!(dead.pick(1), Some(Item::DeleteSession(false)));

        // Marked, like every other row, and not with a mark that already means
        // something else. Open wears the picker's own confirm glyph because it
        // is the same act reached another way.
        assert_eq!(Item::OpenSession.icon(), Some(icons::CONFIRM));
        assert_eq!(Item::DeleteSession(false).icon(), Some(icons::TRASH));
        assert_eq!(Item::DeleteSession(true).icon(), Some(icons::TRASH));
        assert_ne!(icons::TRASH, icons::CLOSE_WIDGET, "delete is not close");
        for item in [Item::OpenSession, Item::DeleteSession(false)] {
            assert_eq!(item.marker(), None, "{item:?} does not open");
        }
        let dock = Dock::new();
        let mut menu = menu;
        assert!(!menu.fold(0, &dock), "there is no group to open");
        assert_eq!(
            menu.width_chars(),
            Item::DeleteSession(false).label().chars().count()
        );
    }

    /// The right click's Delete asks before it acts, the way the settings
    /// panel's delete does. The first press only changes what the row says; the
    /// second one on the same row is the delete.
    ///
    /// Before this the picker was the unguarded half of the pair: the panel
    /// asked twice and the menu removed the transcript on the first press, so
    /// somebody who had learned the panel's question lost a conversation to a
    /// right click that never asked one.
    #[test]
    fn the_delete_row_asks_before_it_acts() {
        let mut menu = Menu::for_session((40.0, 90.0), 3, false);
        assert_eq!(menu.arming(), None, "it opened already asking");

        // Once: the row is still there, it reads differently, and nothing was
        // deleted, which the window reads off the `false`.
        assert!(!menu.press_delete(1), "the first press was the delete");
        assert_eq!(menu.arming(), Some(1));
        assert_eq!(menu.pick(1), Some(Item::DeleteSession(true)));
        assert_eq!(Item::DeleteSession(true).label(), "sure?");
        assert!(Item::DeleteSession(true).warns());
        assert!(!Item::DeleteSession(false).warns());

        // Twice on the same row: the delete, and the row goes back to asking so
        // a menu that somehow outlived it cannot fire again.
        assert!(menu.press_delete(1), "the second press did nothing");
        assert_eq!(menu.arming(), None);
    }

    /// What cancels: the pointer moving off the row, and the pointer leaving
    /// the menu. Closing the menu needs nothing here, because the arming lives
    /// on the menu and goes with it.
    #[test]
    fn moving_off_the_delete_row_takes_the_question_back() {
        let mut menu = Menu::for_session((40.0, 90.0), 3, false);
        assert!(!menu.press_delete(1));

        // Staying on it is not moving away.
        assert!(!menu.point_at(Some(1)), "it changed under a still pointer");
        assert_eq!(menu.arming(), Some(1));

        // The other row, and then off the menu altogether. Either one puts the
        // question back, so the next press on Delete is a first press again.
        assert!(menu.point_at(Some(0)));
        assert_eq!(menu.arming(), None);
        assert!(!menu.point_at(Some(0)), "nothing to change twice");
        assert!(!menu.press_delete(1), "it was still armed");
        assert!(menu.point_at(None));
        assert_eq!(menu.arming(), None);
        assert!(!menu.press_delete(1), "it was still armed");
    }

    /// The box does not resize when the row starts asking. It is hit tested
    /// where it is drawn, so a narrower menu would slide out from under the
    /// pointer and disarm the press it just asked for.
    #[test]
    fn arming_the_delete_does_not_move_the_menu_under_the_pointer() {
        let mut menu = Menu::for_session((40.0, 90.0), 3, false);
        let wide = menu.width_chars();
        assert!(!menu.press_delete(1));
        assert_eq!(menu.width_chars(), wide);
        assert!(
            Item::DeleteSession(true).label().chars().count() < wide,
            "the wording is no longer the narrower of the two"
        );
    }

    #[test]
    fn a_menu_is_as_wide_as_its_longest_label() {
        let menu = Menu::for_widget((0.0, 0.0), View::Output, Space::TopLeft, false);
        assert_eq!(menu.width_chars(), Item::Close.label().chars().count());
        assert_eq!(
            Menu::for_input((0.0, 0.0), false).width_chars(),
            Item::Paste.label().chars().count()
        );
    }
}
