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
//! ## The widgets flyout
//!
//! The pane's menu is a flat list of acts plus one group: Widgets, holding
//! one switch per widget. Opening it does not move a single row of the menu:
//! its rows go into a second box beside the header ([`Menu::fly_start`]
//! marks where they begin in [`Menu::rows`]), so the column the pointer has
//! already read stays exactly where it was. Settings is an act like any
//! other: it opens the settings panel, and the panel's own rail is where a
//! section is chosen.
//!
//! One list still: the flyout's rows are appended after the menu's own, so a
//! row is one number wherever it is drawn and the keyboard walks straight
//! from the column into the flyout and back.

use crate::dock::{Dock, Space, View};
use crate::design::icons;

/// Columns a row reserves at its end for the group marker: the mark and the
/// space in front of it. Reserved in the width the box is measured from
/// rather than left to the drawing, or a long label and the marker would be
/// written over each other.
pub const MARKER_COLUMNS: usize = 2;

/// One thing a menu can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// Copy what is selected in the prompt.
    Copy,
    /// Put the clipboard into the prompt.
    Paste,
    /// Open the settings panel. One act: the panel's rail is where a section
    /// is chosen, so the menu does not repeat that list.
    Settings,
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
            Item::Settings => "Settings",
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
            Item::Widgets(open) => Some(open),
            _ => None,
        }
    }

    /// The glyph in front of the label. Every row carries one, so the type is
    /// still an `Option` only because the drawing already handles a row without
    /// a mark and a future row may not have one.
    ///
    /// Named in [`crate::design::icons`] rather than written here, because a codepoint
    /// the embedded font lacks draws as nothing at all and the coverage test
    /// over there is what catches that.
    ///
    /// Written out variant by variant, with no catch-all: a new row then fails
    /// to compile here rather than shipping with a blank gutter nobody notices.
    pub fn icon(self) -> Option<char> {
        match self {
            Item::Settings => Some(icons::SETTINGS),
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
    /// A right-pointing chevron either way: the flyout opens out to the side,
    /// and that is where the mark points, open or shut.
    ///
    /// At the end rather than in the gutter in front, which is where the icons
    /// above go: a mark in the gutter says what this row is, a mark at the end
    /// says that pressing it opens something.
    pub fn marker(self) -> Option<char> {
        self.group().map(|_| icons::SUBMENU)
    }
}

/// One row: an item, and whether it can be picked right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub item: Item,
    pub enabled: bool,
}

impl Row {
    fn act(item: Item, enabled: bool) -> Row {
        Row { item, enabled }
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
    /// Every row on it: the menu's own rows first, then, while the widgets
    /// flyout is open, its rows appended after them ([`Menu::fly_start`]).
    /// One list, so a row is one number wherever it is drawn.
    pub rows: Vec<Row>,
    /// Where the flyout's rows begin in [`Menu::rows`], while it is open.
    /// The layout puts everything from here on in a second box beside the
    /// header instead of under it, so opening the flyout never moves a row
    /// the pointer has already read.
    pub fly_start: Option<usize>,
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

    /// A pane's menu: three acts and the widgets flyout.
    ///
    /// The flyout opens shut. A menu that opened with it already out is the
    /// wall of rows the flyout exists to stop being.
    pub fn for_widget(at: (f32, f32), view: View, space: Space, has_selection: bool) -> Menu {
        Menu::of(
            at,
            Target::Widget(view, space),
            vec![
                Row::act(Item::Settings, true),
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
            fly_start: None,
            first: 0,
            cursor: None,
        }
    }

    /// How many rows belong to the menu's own column, flyout excluded.
    pub fn main_len(&self) -> usize {
        self.fly_start.unwrap_or(self.rows.len())
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

    /// Open the flyout from the group header at `index`, or shut it again.
    /// Anything that is not a group header leaves the menu alone.
    ///
    /// Open, its rows are appended after the menu's own and `fly_start` marks
    /// where they begin; the layout puts them in a box beside the header.
    /// Shut, everything from `fly_start` on comes off. Either way, not one
    /// row of the menu's own column moves.
    pub fn fold(&mut self, index: usize, dock: &Dock) -> bool {
        let Some(row) = self.rows.get(index) else {
            return false;
        };
        let Some(open) = row.item.group() else {
            return false;
        };
        if let Some(fly) = self.fly_start.take() {
            self.rows.truncate(fly);
        }
        self.rows[index].item = Item::Widgets(!open);
        if !open {
            self.fly_start = Some(self.rows.len());
            // Nothing on it is greyed: every row is a switch and every
            // switch can be thrown either way. The agent-output view is not
            // on the list: it opens by clicking an agent, and a switch for a
            // pane with no agent chosen would open an empty window.
            self.rows.extend(
                View::ALL
                    .into_iter()
                    .filter(|view| *view != View::Agent)
                    .map(|view| Row {
                        item: Item::Widget(view, dock.is_hidden(view)),
                        enabled: true,
                    }),
            );
        }
        // The cursor follows the header only when the keys had it: a flyout
        // opened by the pointer must not light a row the keys are not on.
        if self.cursor.is_some() {
            self.cursor = Some(index);
        }
        true
    }

    /// The row the flyout hangs off: the one open group header.
    pub fn fly_anchor(&self) -> Option<usize> {
        self.fly_start?;
        self.rows.iter().position(|row| row.item.group() == Some(true))
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
    /// how many are on screen, which only the layout knows. Scrolling is the
    /// menu's own column; the flyout is short enough to always show whole.
    pub fn scroll(&mut self, by: usize, down: bool, rows: usize) -> bool {
        let most = self.main_len().saturating_sub(rows);
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

    /// Bring the cursor's row on screen, when `rows` of the menu's own column
    /// are showing. A cursor in the flyout needs no scrolling: the flyout is
    /// always shown whole.
    pub fn show(&mut self, rows: usize) {
        let Some(at) = self.cursor else {
            return;
        };
        if self.fly_start.is_some_and(|fly| at >= fly) {
            return;
        }
        let rows = rows.max(1);
        if at < self.first {
            self.first = at;
        } else if at >= self.first + rows {
            self.first = at + 1 - rows;
        }
        self.first = self.first.min(self.main_len().saturating_sub(rows));
    }

    /// What the right arrow does: open the flyout from the header the cursor
    /// is on, and step into its first row when it is already open.
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
            Some(true) => {
                let Some(fly) = self.fly_start else {
                    return false;
                };
                self.cursor = Some(fly);
                true
            }
            None => false,
        }
    }

    /// What the left arrow does: shut the flyout from its header, or step out
    /// of it back to that header.
    pub fn fold_here(&mut self, dock: &Dock, rows: usize) -> bool {
        let Some(at) = self.cursor else {
            return false;
        };
        if self.rows.get(at).and_then(|row| row.item.group()) == Some(true) {
            self.fold(at, dock);
            self.show(rows);
            return true;
        }
        if self.fly_start.is_some_and(|fly| at >= fly) {
            self.cursor = self.fly_anchor();
            return true;
        }
        false
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

    /// The pointer resting on a row opens what the row opens: the flyout
    /// comes out on rollover, the way every desktop menu opens its submenus,
    /// and it goes away when the pointer rests on another row of the column.
    /// A pointer inside the flyout, or off the menu entirely, leaves it be.
    pub fn hover(&mut self, row: Option<usize>, dock: &Dock) -> bool {
        let mut changed = self.point_at(row);
        let Some(at) = row else {
            return changed;
        };
        match self.rows.get(at).map(|row| row.item) {
            Some(Item::Widgets(false)) => changed |= self.fold(at, dock),
            Some(item)
                if item.group().is_none() && self.fly_start.is_some_and(|fly| at < fly) =>
            {
                if let Some(anchor) = self.fly_anchor() {
                    changed |= self.fold(anchor, dock);
                }
            }
            _ => {}
        }
        changed
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

    /// The longest row of the menu's own column, in characters: its label and
    /// the room kept at the end of a row that opens the flyout. What the
    /// layout sizes the box from, so every row is as wide as the widest one.
    pub fn width_chars(&self) -> usize {
        Menu::widest(&self.rows[..self.main_len()])
    }

    /// The longest row of the open flyout, sizing its own box the same way.
    pub fn fly_width_chars(&self) -> usize {
        match self.fly_start {
            Some(fly) => Menu::widest(&self.rows[fly..]),
            None => 0,
        }
    }

    fn widest(rows: &[Row]) -> usize {
        rows.iter()
            .map(|row| {
                row.item.sizing_label().chars().count()
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

    /// The views the flyout lists: every one but the agent-output view,
    /// which opens by clicking an agent rather than from a switch.
    fn switchable() -> Vec<View> {
        View::ALL
            .into_iter()
            .filter(|view| *view != View::Agent)
            .collect()
    }

    /// The rows the open flyout holds, as views, in order.
    fn fly_views(menu: &Menu) -> Vec<View> {
        let fly = menu.fly_start.expect("the flyout is open");
        menu.rows[fly..]
            .iter()
            .map(|row| match row.item {
                Item::Widget(view, _) => view,
                other => panic!("{other:?} is not a widget row"),
            })
            .collect()
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

    /// Three acts and the flyout's header, the flyout shut.
    #[test]
    fn a_pane_gets_settings_copy_close_and_the_widgets_header() {
        let menu = Menu::for_widget((10.0, 10.0), View::Plan, Space::TopRight, true);
        assert_eq!(
            items(&menu),
            vec![
                Item::Settings,
                Item::CopySelection,
                Item::Close,
                Item::Widgets(false)
            ]
        );
        assert_eq!(menu.target, Target::Widget(View::Plan, Space::TopRight));
        assert_eq!(menu.pick(0), Some(Item::Settings));
        assert_eq!(menu.pick(1), Some(Item::CopySelection));
        assert_eq!(menu.pick(2), Some(Item::Close));
        assert_eq!(menu.pick(3), Some(Item::Widgets(false)));
        assert_eq!(menu.fly_start, None, "the flyout opens shut");
        assert_eq!(menu.main_len(), 4);
    }

    /// The correction this round: opening the widget list moves nothing. Its
    /// rows are appended after the menu's own and drawn in a box beside the
    /// header, so the column the pointer already read stays where it was.
    #[test]
    fn the_widget_header_opens_a_flyout_and_shuts_again() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((10.0, 10.0), View::Plan, Space::TopLeft, false);
        let shut = items(&menu);
        let at = 3;

        assert!(menu.fold(at, &dock));
        assert_eq!(menu.rows[at].item, Item::Widgets(true));
        assert_eq!(menu.fly_start, Some(shut.len()));
        assert_eq!(menu.main_len(), shut.len(), "no row of the column moved");
        assert_eq!(&items(&menu)[..3], &shut[..3]);
        assert_eq!(
            fly_views(&menu),
            switchable(),
            "the list is in the one order, so it is in the same place every time"
        );
        assert_eq!(menu.fly_anchor(), Some(at));
        // Eight rows, and none of them is the pane of failed calls: the list is
        // built from `View::ALL`, so a variant left behind would still be
        // switchable from here with nothing to draw.
        assert_eq!(fly_views(&menu).len(), 8);
        for view in fly_views(&menu) {
            assert_ne!(view.label(), "DEBUG");
        }

        assert!(menu.fold(at, &dock));
        assert_eq!(menu.fly_start, None);
        assert_eq!(items(&menu), shut, "it shuts back to what it opened as");
    }

    /// Settings acts; it is not a group, and neither is anything on the
    /// prompt's menu. Only the widgets header opens.
    #[test]
    fn nothing_but_the_widgets_header_opens() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, true);
        assert!(!menu.fold(0, &dock), "Settings acts, it does not open");
        assert!(!menu.fold(1, &dock), "Copy selection is not a group");
        assert!(!menu.fold(99, &dock), "there is no row 99");
        assert_eq!(menu.fly_start, None);
        assert_eq!(Item::Settings.group(), None);
        assert_eq!(Item::Settings.marker(), None, "an act keeps no room at its end");
        let mut prompt = Menu::for_input((0.0, 0.0), false);
        assert!(!prompt.fold(0, &dock));
        assert!(!prompt.fold(1, &dock));
        assert_eq!(prompt.fly_start, None);
    }

    /// The marker is written after the label, so the header row has to be wide
    /// enough for both, and the flyout is measured as its own box.
    #[test]
    fn each_box_is_as_wide_as_its_own_longest_row() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        let widgets = "Widgets".chars().count();
        assert!(
            menu.width_chars() >= widgets + MARKER_COLUMNS,
            "the marker and the label would collide"
        );
        assert_eq!(menu.fly_width_chars(), 0, "a shut flyout is no box at all");
        let column = menu.width_chars();
        menu.fold(3, &dock);
        assert_eq!(
            menu.width_chars(),
            column,
            "opening the flyout does not widen the column"
        );
        let widest = View::ALL
            .into_iter()
            .map(|view| view.label().chars().count())
            .max()
            .expect("there are widgets");
        assert_eq!(menu.fly_width_chars(), widest);
        // A row with no marker keeps no room, or every menu would be two
        // columns wider than it needs to be.
        let plain = Menu::for_input((0.0, 0.0), false);
        assert_eq!(plain.width_chars(), Item::Paste.label().chars().count());
    }

    /// Every row of the flyout is a switch, and a switch has to say which way
    /// it is set before it is pressed: a tick in a box for a widget in the
    /// window, an empty box for one that is out.
    #[test]
    fn the_flyout_marks_what_is_closed_and_every_row_of_it_can_be_picked() {
        let dock = Dock::hiding(&[View::Hardware, View::Files]);
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        let at = 3;
        menu.fold(at, &dock);
        for (step, view) in switchable().into_iter().enumerate() {
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
        assert_eq!(menu.pick(at + 1 + switchable().len()), None);
    }

    /// The header carries two marks: the one in its gutter saying what the row
    /// is, and the chevron at its end saying it opens. The chevron points out
    /// to the side in both states, because that is where the rows go.
    #[test]
    fn the_header_carries_a_mark_in_its_gutter_and_a_side_chevron() {
        assert_eq!(Item::Widgets(false).marker(), Some(icons::SUBMENU));
        assert_eq!(Item::Widgets(true).marker(), Some(icons::SUBMENU));
        for open in [false, true] {
            assert_eq!(Item::Widgets(open).icon(), Some(icons::WIDGETS));
            assert_eq!(Item::Widgets(open).group(), Some(open));
            assert_ne!(
                icons::WIDGETS,
                icons::SUBMENU,
                "the two marks on this row have to be told apart"
            );
        }
        for item in [
            Item::Copy,
            Item::Paste,
            Item::Settings,
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
    fn the_flyout_reads_its_marks_off_the_dock_again_without_moving_a_row() {
        let mut dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        menu.fold(3, &dock);
        let places = fly_views(&menu);

        assert!(dock.hide(View::Hardware));
        assert!(menu.relist(&dock));
        assert_eq!(fly_views(&menu), places, "no row moved");
        for (step, view) in switchable().into_iter().enumerate() {
            assert_eq!(
                menu.pick(4 + step),
                Some(Item::Widget(view, view == View::Hardware))
            );
        }
        // Nothing to read, either because the flyout is shut or because the
        // menu has no widget row at all.
        menu.fold(3, &dock);
        assert!(!menu.relist(&dock));
        assert!(!Menu::for_input((0.0, 0.0), false).relist(&dock));
    }

    /// Every row but the header acts on the widget the menu was opened over,
    /// so the menu has to be able to say which one that is.
    #[test]
    fn a_menu_names_the_widget_it_was_opened_over() {
        let menu = Menu::for_widget((0.0, 0.0), View::Agents, Space::TopRight, false);
        assert_eq!(menu.target_view(), Some(View::Agents));
        assert_eq!(Menu::for_input((0.0, 0.0), false).target_view(), None);
    }

    /// Scrolling moves the menu's own column and stops at both ends; the
    /// flyout is always shown whole, so opening it adds nothing to scroll.
    #[test]
    fn scrolling_is_the_menus_own_column() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        assert!(
            !menu.scroll(1, true, 4),
            "a menu that fits has nothing to scroll"
        );
        assert!(menu.scroll(99, true, 3));
        assert_eq!(menu.first, 1, "it stops at the last row of the column");
        assert!(menu.scroll(99, false, 3));
        assert_eq!(menu.first, 0);
        assert!(!menu.scroll(1, false, 3));
        menu.fold(3, &dock);
        assert!(
            !menu.scroll(1, true, 4),
            "the flyout added nothing to scroll through"
        );
    }

    /// The keyboard walks the menu, skips the rows that cannot act, and stops
    /// at both ends.
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

    /// The flyout opens on rollover, the way every desktop menu opens its
    /// submenus: resting on the header brings it out, resting on another row
    /// of the column puts it away, and a pointer inside the flyout or off the
    /// menu leaves it be.
    #[test]
    fn resting_on_the_header_opens_the_flyout_and_resting_elsewhere_shuts_it() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, true);
        assert!(menu.hover(Some(3), &dock), "rollover did not open it");
        assert_eq!(menu.rows[3].item, Item::Widgets(true));
        let fly = menu.fly_start.expect("the flyout is out");

        // Inside the flyout, and off the menu entirely: it stays out.
        assert!(!menu.hover(Some(fly + 2), &dock));
        assert_eq!(menu.rows[3].item, Item::Widgets(true));
        assert!(!menu.hover(None, &dock));
        assert_eq!(menu.rows[3].item, Item::Widgets(true));

        // Back on the header: still out, not toggled shut under the pointer.
        assert!(!menu.hover(Some(3), &dock));
        assert_eq!(menu.rows[3].item, Item::Widgets(true));

        // Resting on another row of the column puts it away.
        assert!(menu.hover(Some(0), &dock));
        assert_eq!(menu.rows[3].item, Item::Widgets(false));
        assert_eq!(menu.fly_start, None);
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

    /// The flyout opens and shuts from the keyboard: right opens it from the
    /// header and steps in, left steps back out to the header and shuts it.
    #[test]
    fn the_arrows_open_the_flyout_step_in_and_out_and_shut_it() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        menu.cursor = Some(3);

        assert!(menu.unfold_here(&dock, 20), "right did not open the flyout");
        assert_eq!(menu.rows[3].item, Item::Widgets(true));
        assert_eq!(menu.cursor, Some(3), "the cursor stayed on the header");

        // Right again steps into the flyout, onto its first row.
        assert!(menu.unfold_here(&dock, 20));
        assert_eq!(menu.cursor, Some(4));
        assert!(matches!(menu.rows[4].item, Item::Widget(..)));
        assert!(
            !menu.unfold_here(&dock, 20),
            "a row that is not a header does not open"
        );

        // Left from inside steps out to the header; left again shuts it.
        assert!(menu.fold_here(&dock, 20));
        assert_eq!(menu.cursor, Some(3));
        assert!(menu.fold_here(&dock, 20));
        assert_eq!(menu.rows[3].item, Item::Widgets(false));
        assert_eq!(menu.fly_start, None);
        assert!(
            !menu.fold_here(&dock, 20),
            "a shut flyout has nothing to close"
        );
    }

    /// A cursor walked past the bottom of a short window brings the column
    /// with it, and a cursor in the flyout never scrolls the column: the
    /// flyout is beside it, always whole.
    #[test]
    fn walking_keeps_the_cursor_on_screen_and_the_flyout_leaves_first_alone() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, true);
        let rows = 3;
        for expect in [0usize, 1, 2, 3] {
            menu.walk(true, rows);
            assert_eq!(menu.cursor, Some(expect));
            let at = expect;
            assert!(
                at >= menu.first && at < menu.first + rows,
                "row {at} is off screen: first {}",
                menu.first
            );
        }
        assert_eq!(menu.first, 1, "the column followed the cursor down");
        menu.fold(3, &dock);
        menu.cursor = Some(3);
        let first = menu.first;
        assert!(menu.walk(true, rows), "down crosses into the flyout");
        assert_eq!(menu.cursor, Some(4));
        assert_eq!(menu.first, first, "the flyout does not scroll the column");
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

    /// The settings row acts, in every menu that has it, and carries the gear.
    #[test]
    fn the_settings_row_acts_and_carries_the_gear() {
        for has_selection in [false, true] {
            let menu = Menu::for_widget((0.0, 0.0), View::Files, Space::BottomRight, has_selection);
            assert_eq!(menu.rows[0].item, Item::Settings);
            assert!(menu.rows[0].enabled);
            assert_eq!(menu.pick(0), Some(Item::Settings));
        }
        assert_eq!(Item::Settings.icon(), Some(icons::SETTINGS));
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
        // two switch states have to stay clear of all of them.
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
        assert_eq!(menu.pick(0), Some(Item::OpenSession));
        assert_eq!(menu.pick(1), Some(Item::DeleteSession(false)));

        let gone = Menu::for_session((40.0, 90.0), 3, true);
        assert_eq!(items(&gone), items(&menu), "the shape changed");
        assert_eq!(gone.pick(0), None, "a session with no folder cannot open");
        assert_eq!(gone.pick(1), Some(Item::DeleteSession(false)));
    }

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
