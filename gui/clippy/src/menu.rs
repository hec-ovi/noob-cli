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
//! The one deliberate exception is the widget list, and it is not a row of the
//! menu at all: the Widgets row stays where it is and the list flies out beside
//! it, in a box of its own, the way every other desktop menu opens a submenu.
//! The eight rows it carries are only there while it is open. A menu that
//! always held all eight would be a wall of tab names in front of a Close row,
//! and one that opened them downwards would push its own rows around under the
//! pointer.
//!
//! The rows are still one list here, top level first and the flyout after it, so
//! a row is one number wherever it is drawn. Which of the two boxes a row is in
//! is [`Menu::top`], and where those boxes are is the layout's business.

use crate::dock::{Dock, Space, View};
use crate::icons;

/// Columns a row reserves at its end for the flyout marker: the mark and the
/// space in front of it. Reserved in [`Menu::width_chars`] rather than left to
/// the drawing, or a long label and the marker would be written over each other.
pub const MARKER_COLUMNS: usize = 2;

/// One thing a menu can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// Copy what is selected in the prompt.
    Copy,
    /// Put the clipboard into the prompt.
    Paste,
    /// Open the settings panel, which takes over the whole window.
    Settings,
    /// Copy what is selected in the pane the menu opened over.
    CopySelection,
    /// Take the pane out of the window, via `Dock::hide`.
    Close,
    /// Open or shut the list of every widget. Carries whether it is open, which
    /// the layout reads to know whether to place a flyout. The row itself looks
    /// the same either way: a submenu marker says the list is over there, not
    /// which way it is about to go.
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
            // The word the settings panel's own trash uses once it has been
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

    /// The glyph in front of the label. Every row carries one now, so the type
    /// is still an `Option` only because the drawing already handles a row
    /// without a mark and a future row may not have one.
    ///
    /// Named in [`crate::icons`] rather than written here, because a codepoint
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
            // what the row is and one that there is more of it out to the side.
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

    /// The glyph at the END of the row, for a row that opens a list beside
    /// itself. The standard submenu affordance, and the whole of how a row that
    /// flies out is told apart from a row that acts.
    ///
    /// At the end rather than in the gutter in front, which is where the icons
    /// above go: a mark in the gutter says what this row is, a mark at the end
    /// says there is more of it out to the side.
    pub fn marker(self) -> Option<char> {
        match self {
            Item::Widgets(_) => Some(icons::SUBMENU),
            _ => None,
        }
    }
}

/// One row: an item and whether it can be picked right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub item: Item,
    pub enabled: bool,
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
    /// Every row in it, the top level first and the widget list after it. Shut,
    /// that is the top level alone.
    pub rows: Vec<Row>,
    /// How many of those rows are the top level. The rest are the flyout, which
    /// is a box of its own and the part that scrolls, so the two halves have to
    /// be told apart.
    pub top: usize,
    /// Which widget the list starts at, when the window is too short to hold
    /// all of it. Kept here rather than in the layout because it outlives a
    /// frame: the wheel moves it and the next frame draws from it.
    pub first: usize,
}

impl Menu {
    /// The prompt's menu. Paste is always available: the clipboard is the
    /// display server's, and asking it whether it holds anything means
    /// connecting to it, which is work a right click has no reason to do.
    pub fn for_input(at: (f32, f32), has_selection: bool) -> Menu {
        let rows = vec![
            Row {
                item: Item::Copy,
                enabled: has_selection,
            },
            Row {
                item: Item::Paste,
                enabled: true,
            },
        ];
        Menu {
            at,
            target: Target::Input,
            top: rows.len(),
            rows,
            first: 0,
        }
    }

    /// A pane's menu.
    ///
    /// Settings opens the panel. It shipped greyed for as long as there was no
    /// panel behind it, which read as a broken window rather than an unfinished
    /// one, and is the complaint that built the panel.
    ///
    /// The Widgets row is last, and shut. It never moves and nothing above it
    /// moves either: the list it opens is a box beside it rather than more rows
    /// under it.
    pub fn for_widget(at: (f32, f32), view: View, space: Space, has_selection: bool) -> Menu {
        let rows = vec![
            Row {
                item: Item::Settings,
                enabled: true,
            },
            Row {
                item: Item::CopySelection,
                enabled: has_selection,
            },
            Row {
                item: Item::Close,
                enabled: true,
            },
            Row {
                item: Item::Widgets(false),
                enabled: true,
            },
        ];
        Menu {
            at,
            target: Target::Widget(view, space),
            top: rows.len(),
            rows,
            first: 0,
        }
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
        let rows = vec![
            Row {
                item: Item::OpenSession,
                enabled: !gone,
            },
            Row {
                item: Item::DeleteSession(false),
                enabled: true,
            },
        ];
        Menu {
            at,
            target: Target::Session(index),
            top: rows.len(),
            rows,
            first: 0,
        }
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
        let rows = vec![Row {
            item: Item::CopySelection,
            enabled: has_selection,
        }];
        Menu {
            at,
            target: Target::SettingsDoc,
            top: rows.len(),
            rows,
            first: 0,
        }
    }

    /// How many widgets are on the list: eight while it is open, none while it
    /// is shut. What the scroll is bounded by, and the whole of the difference
    /// between the two states.
    pub fn widgets(&self) -> usize {
        self.rows.len() - self.top
    }

    /// The widget the menu was opened over, if it was opened over one. What
    /// every row but the list acts on, so a caller can ask whether the list just
    /// took that widget out of the window.
    pub fn target_view(&self) -> Option<View> {
        match self.target {
            Target::Widget(view, _) => Some(view),
            Target::Input | Target::Session(_) | Target::SettingsDoc => None,
        }
    }

    /// Open the widget list, or shut it again.
    ///
    /// Every view is on it, in [`View::ALL`] order so the list is in the same
    /// order every time it opens, each marked with whether it is in the window.
    /// Nothing on it is greyed: every row is a switch and every switch can be
    /// thrown either way.
    pub fn toggle_widgets(&mut self, dock: &Dock) -> bool {
        let Some(row) = self.rows.get_mut(self.top.saturating_sub(1)) else {
            return false;
        };
        let Item::Widgets(open) = row.item else {
            return false;
        };
        row.item = Item::Widgets(!open);
        self.rows.truncate(self.top);
        self.first = 0;
        if !open {
            self.rows.extend(View::ALL.into_iter().map(|view| Row {
                item: Item::Widget(view, dock.is_hidden(view)),
                enabled: true,
            }));
        }
        true
    }

    /// Read the list's marks off the dock again, keeping the list open and
    /// keeping where it is scrolled to.
    ///
    /// A row of the list is a switch, and a menu that stays open after one is
    /// thrown would otherwise go on saying the widget is where it was. The rows
    /// themselves do not move, so the pointer is still over the row it just
    /// pressed and can press it back.
    pub fn relist(&mut self, dock: &Dock) -> bool {
        if self.widgets() == 0 {
            return false;
        }
        for row in &mut self.rows[self.top..] {
            if let Item::Widget(view, _) = row.item {
                row.item = Item::Widget(view, dock.is_hidden(view));
            }
        }
        true
    }

    /// Move the widget list, when the window is too short to hold all of it.
    /// `rows` is how many of it are on screen, which only the layout knows.
    pub fn scroll(&mut self, by: usize, down: bool, rows: usize) -> bool {
        let most = self.widgets().saturating_sub(rows);
        let next = match down {
            true => (self.first + by).min(most),
            false => self.first.saturating_sub(by).min(most),
        };
        let moved = next != self.first;
        self.first = next;
        moved
    }

    /// Press the delete row at `index`. `true` means delete it now.
    ///
    /// The first press only arms the row: it starts reading "sure?" and the
    /// menu stays open under the pointer for the second one. Two presses
    /// because a transcript is gone once it is deleted and nothing in this
    /// window can put it back, which is the same reason the settings panel's
    /// trash is pressed twice, and the same wording.
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
    /// An armed delete disarms as soon as the pointer is anywhere else. A menu
    /// left sitting on "sure?" while the pointer has wandered to Open and back
    /// would take what reads as a first press and delete on it.
    pub fn point_at(&mut self, row: Option<usize>) -> bool {
        let Some(armed) = self.arming() else {
            return false;
        };
        if row == Some(armed) {
            return false;
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

    /// The longest label in the menu's own box, in characters, with room kept
    /// at the end of a row that carries a flyout marker. What the layout sizes
    /// the box from, so every row is as wide as the widest one.
    ///
    /// The flyout is not in this: it is a box of its own, sized by
    /// [`Menu::list_width_chars`], so opening the list cannot change the width
    /// of the menu it opened from.
    pub fn width_chars(&self) -> usize {
        Menu::widest(&self.rows[..self.top.min(self.rows.len())])
    }

    /// The same for the flyout's box, which is empty while the list is shut.
    pub fn list_width_chars(&self) -> usize {
        Menu::widest(&self.rows[self.top.min(self.rows.len())..])
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

    #[test]
    fn the_prompt_gets_the_two_things_a_text_field_can_do() {
        let menu = Menu::for_input((10.0, 10.0), true);
        assert_eq!(items(&menu), vec![Item::Copy, Item::Paste]);
        assert_eq!(menu.target, Target::Input);
        assert_eq!(menu.pick(0), Some(Item::Copy));
        assert_eq!(menu.pick(1), Some(Item::Paste));
        assert_eq!(menu.pick(2), None, "there is no third row");
    }

    #[test]
    fn a_pane_gets_settings_a_copy_a_close_and_the_widget_list() {
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
        assert_eq!(menu.pick(1), Some(Item::CopySelection));
        assert_eq!(menu.pick(2), Some(Item::Close));
        assert_eq!(menu.pick(3), Some(Item::Widgets(false)));
        assert_eq!(menu.widgets(), 0, "it opens shut");
    }

    /// Shut it is one row, open it is that row and every widget under it.
    #[test]
    fn the_widget_row_opens_into_one_row_per_widget_and_shuts_again() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((10.0, 10.0), View::Plan, Space::TopLeft, false);
        let shut = items(&menu);

        assert!(menu.toggle_widgets(&dock));
        assert_eq!(menu.widgets(), View::ALL.len());
        assert_eq!(
            items(&menu)[menu.top..]
                .iter()
                .map(|item| match item {
                    Item::Widget(view, _) => *view,
                    other => panic!("{other:?} is not a widget row"),
                })
                .collect::<Vec<_>>(),
            View::ALL.to_vec(),
            "the list is in the one order, so it is in the same place every time"
        );
        // Eight rows, and none of them is the pane of failed calls: the list is
        // built from `View::ALL`, so a variant left behind would still be
        // switchable from here with nothing to draw.
        assert_eq!(menu.widgets(), 8);
        for row in &menu.rows[menu.top..] {
            let Item::Widget(view, _) = row.item else {
                panic!("{:?} is not a widget row", row.item)
            };
            assert_ne!(view.label(), "DEBUG");
        }

        assert!(menu.toggle_widgets(&dock));
        assert_eq!(menu.widgets(), 0);
        assert_eq!(items(&menu), shut, "it shuts back to what it opened as");
    }

    /// The exception to a menu keeping its shape moves no row at all now: the
    /// list is a box beside the row rather than more rows under it, so the menu
    /// it came from is the same menu it was, width included.
    #[test]
    fn opening_the_list_moves_no_row_of_the_menu_and_does_not_widen_it() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((10.0, 10.0), View::Files, Space::BottomRight, true);
        let before = items(&menu);
        let width = menu.width_chars();
        menu.toggle_widgets(&dock);
        assert_eq!(&items(&menu)[..menu.top - 1], &before[..before.len() - 1]);
        assert_eq!(menu.rows[menu.top - 1].item, Item::Widgets(true));
        assert_eq!(menu.pick(0), Some(Item::Settings));
        assert_eq!(menu.pick(2), Some(Item::Close));
        assert_eq!(menu.width_chars(), width, "the menu's own box did not move");
        assert_eq!(menu.width_chars(), Item::Close.label().chars().count());
        // The flyout is sized on its own, by the longest tab name on it.
        let widest = View::ALL
            .into_iter()
            .map(|view| view.label().chars().count())
            .max()
            .expect("there are views");
        assert_eq!(menu.list_width_chars(), widest);
        menu.toggle_widgets(&dock);
        assert_eq!(menu.list_width_chars(), 0, "shut, there is no flyout");
    }

    /// The marker is written after the label, so the row has to be wide enough
    /// for both. A row as wide as its label alone would write the two on top of
    /// each other.
    #[test]
    fn a_row_that_flies_out_keeps_room_at_its_end_for_the_marker() {
        let menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        let widgets = "Widgets".chars().count();
        assert!(
            menu.width_chars() >= widgets + MARKER_COLUMNS,
            "the marker and the label would collide"
        );
        // With a label longer than every other row's, the width is that row's.
        let mut wide = menu.clone();
        wide.rows.retain(|row| matches!(row.item, Item::Widgets(_)));
        wide.top = wide.rows.len();
        assert_eq!(wide.width_chars(), widgets + MARKER_COLUMNS);
        // A row with no marker keeps no room, or every menu would be two
        // columns wider than it needs to be.
        let plain = Menu::for_input((0.0, 0.0), false);
        assert_eq!(plain.width_chars(), Item::Paste.label().chars().count());
    }

    /// Every row of the list is a switch, and a switch has to say which way it
    /// is set before it is pressed: a tick in a box for a widget in the window,
    /// an empty box for one that is out.
    #[test]
    fn the_list_marks_what_is_closed_and_every_row_of_it_can_be_picked() {
        let dock = Dock::hiding(&[View::Hardware, View::Files]);
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        menu.toggle_widgets(&dock);
        for (step, view) in View::ALL.into_iter().enumerate() {
            let index = menu.top + step;
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
                "a widget row is a switch, not a way further in"
            );
        }
        assert_eq!(menu.pick(menu.top + View::ALL.len()), None);
    }

    /// The row that opens the list carries two marks, open or shut: the grid of
    /// frames in the gutter saying what the row is, and the submenu chevron at
    /// its end saying the list is out to the side.
    ///
    /// The gutter half asserted the opposite twice over. First it was a plus and
    /// a minus, which is what a row that folds a list out underneath itself says
    /// and this row does not do that. Then it was nothing at all, which left the
    /// row the only one in the menu with an empty gutter.
    #[test]
    fn the_widget_row_carries_a_grid_in_its_gutter_and_a_submenu_marker_at_its_end() {
        for open in [false, true] {
            assert_eq!(Item::Widgets(open).marker(), Some(icons::SUBMENU));
            assert_eq!(Item::Widgets(open).icon(), Some(icons::WIDGETS));
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
        ] {
            assert_eq!(item.marker(), None, "{item:?} does not fly out");
        }
    }

    /// The marks follow the dock while the menu stays open, so a row that was
    /// just switched says what it did rather than what it used to say. The rows
    /// keep their places and the list keeps its scroll, so the pointer is still
    /// over the row it pressed.
    #[test]
    fn the_list_reads_its_marks_off_the_dock_again_without_moving_a_row() {
        let mut dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        menu.toggle_widgets(&dock);
        menu.scroll(3, true, 4);
        let places: Vec<View> = menu.rows[menu.top..]
            .iter()
            .map(|row| match row.item {
                Item::Widget(view, _) => view,
                other => panic!("{other:?} is not a widget row"),
            })
            .collect();

        assert!(dock.hide(View::Hardware));
        assert!(menu.relist(&dock));
        assert_eq!(menu.first, 3, "the list did not jump back to the top");
        assert_eq!(
            menu.rows[menu.top..]
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
                menu.pick(menu.top + step),
                Some(Item::Widget(view, view == View::Hardware))
            );
        }
        // Shut, there is nothing to read.
        menu.toggle_widgets(&dock);
        assert!(!menu.relist(&dock));
        assert!(!Menu::for_input((0.0, 0.0), false).relist(&dock));
    }

    /// Every row but the list acts on the widget the menu was opened over, so
    /// the menu has to be able to say which one that is.
    #[test]
    fn a_menu_names_the_widget_it_was_opened_over() {
        let menu = Menu::for_widget((0.0, 0.0), View::Agents, Space::TopRight, false);
        assert_eq!(menu.target_view(), Some(View::Agents));
        assert_eq!(Menu::for_input((0.0, 0.0), false).target_view(), None);
    }

    /// Nine rows do not always fit under a menu opened near the bottom of a
    /// short window, so the list moves, and it cannot be moved off either end.
    #[test]
    fn the_list_scrolls_and_stops_at_both_ends() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        assert!(
            !menu.scroll(1, true, 3),
            "a shut list has nothing to scroll"
        );
        menu.toggle_widgets(&dock);
        assert!(menu.scroll(2, true, 3));
        assert_eq!(menu.first, 2);
        assert!(menu.scroll(99, true, 3));
        assert_eq!(menu.first, View::ALL.len() - 3, "it stops at the last row");
        assert!(!menu.scroll(4, true, 3));
        assert!(menu.scroll(99, false, 3));
        assert_eq!(menu.first, 0);
        assert!(!menu.scroll(1, false, 3));
        // A list that is entirely on screen does not move at all.
        assert!(!menu.scroll(1, true, View::ALL.len()));
        // And shutting it puts the list back at the top.
        menu.scroll(4, true, 3);
        menu.toggle_widgets(&dock);
        assert_eq!(menu.first, 0);
    }

    /// The prompt's menu has no widgets to list: there is no pane behind it.
    #[test]
    fn the_prompt_menu_has_no_widget_row() {
        let dock = Dock::new();
        let mut menu = Menu::for_input((0.0, 0.0), false);
        assert!(!menu.toggle_widgets(&dock));
        assert_eq!(items(&menu), vec![Item::Copy, Item::Paste]);
        assert_eq!(menu.widgets(), 0);
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

    /// Settings acts now, in every menu that has it, and carries its mark.
    ///
    /// This asserted the opposite: the row was greyed for as long as there was
    /// nothing behind it. There is a panel behind it now (`crate::settings`), so
    /// the row that reported it as broken is the row that opens it.
    #[test]
    fn settings_opens_the_panel_and_carries_an_icon() {
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

    /// Every row of both menus is marked, and no two rows that mean different
    /// things wear the same mark. The gutter is spent on every row whether or
    /// not it has a glyph in it (`view::MENU_GUTTER`), so an unmarked row is a
    /// blank column, not a narrower row: the four that shipped blank read as
    /// rows that had lost their icons.
    #[test]
    fn every_row_of_both_menus_is_marked_in_its_gutter() {
        let dock = Dock::hiding(&[View::Hardware]);
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, true);
        menu.toggle_widgets(&dock);
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

        // The four the right click menu gained, each on the row it belongs to.
        assert_eq!(Item::CopySelection.icon(), Some(icons::COPY));
        assert_eq!(Item::Copy.icon(), Some(icons::COPY), "the same act");
        assert_eq!(Item::Paste.icon(), Some(icons::PASTE));
        assert_eq!(Item::Close.icon(), Some(icons::CLOSE_WIDGET));
        assert_eq!(Item::Widgets(false).icon(), Some(icons::WIDGETS));
        assert_eq!(Item::Widgets(true).icon(), Some(icons::WIDGETS));

        // A copy row and a paste row that look alike are two coin flips, and the
        // list's two switch states have to stay clear of all of them.
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
        assert_eq!(menu.widgets(), 0, "a session has no widget list");
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
            assert_eq!(item.marker(), None, "{item:?} does not fly out");
        }
        let dock = Dock::new();
        let mut menu = menu;
        assert!(!menu.toggle_widgets(&dock), "there is no list to open");
        assert_eq!(
            menu.width_chars(),
            Item::DeleteSession(false).label().chars().count()
        );
    }

    /// The right click's Delete asks before it acts, the way the settings
    /// panel's trash does. The first press only changes what the row says; the
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
