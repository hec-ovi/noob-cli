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
//! The one deliberate exception is the widget list. It is the last row of the
//! widget menu and it opens under itself, so the rows above it never move, and
//! the nine rows it carries are only there while it is open. A menu that always
//! held all nine would be a wall of tab names in front of a Close row.

use crate::dock::{Dock, Space, View};
use crate::icons;

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
    /// Open or shut the list of every widget. Carries whether it is open, so
    /// the row can say which of the two it is about to do.
    Widgets(bool),
    /// One widget on that list, and whether it is out of the window. Picking a
    /// closed one puts it back, picking one that is already in the window shows
    /// it where it is.
    Widget(View, bool),
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
        }
    }

    /// The glyph in front of the label, for the rows that have one.
    ///
    /// Named in [`crate::icons`] rather than written here, because a codepoint
    /// the embedded font lacks draws as nothing at all and the coverage test
    /// over there is what catches that.
    pub fn icon(self) -> Option<char> {
        match self {
            Item::Settings => Some(icons::SETTINGS),
            // The picker's tree marks, for the same gesture: a plus opens what
            // is under the row, a minus takes it away again.
            Item::Widgets(true) => Some(icons::COLLAPSE),
            Item::Widgets(false) => Some(icons::EXPAND),
            // In the window, or closed. The mark is the whole of how the list
            // answers where did that pane go.
            Item::Widget(_, hidden) => Some(match hidden {
                true => icons::CLOSE,
                false => icons::CONFIRM,
            }),
            _ => None,
        }
    }

    /// Columns the label steps in by, so the widget list reads as a list under
    /// the row that opened it rather than as more rows of the menu.
    pub fn indent(self) -> usize {
        match self {
            Item::Widget(..) => 1,
            _ => 0,
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
    /// How many of those rows are the top level. The rest are the list, which
    /// is the part that scrolls, so the two halves have to be told apart.
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
    /// The Widgets row is last, and shut. Everything above it keeps the place
    /// it has always had, so the list can grow and shrink without moving a row
    /// the pointer is already on its way to.
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

    /// How many widgets are on the list: nine while it is open, none while it
    /// is shut. What the scroll is bounded by, and the whole of the difference
    /// between the two states.
    pub fn widgets(&self) -> usize {
        self.rows.len() - self.top
    }

    /// Open the widget list, or shut it again.
    ///
    /// Every view is on it, in [`View::ALL`] order so the list is in the same
    /// order every time it opens, each marked with whether it is in the window.
    /// Nothing on it is greyed: a closed widget comes back and one that is
    /// already in the window is shown where it is, so every row acts.
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

    /// What picking the row at `index` does, or nothing when that row is
    /// disabled or does not exist. The one place a pointer position becomes an
    /// action, so a greyed row cannot act by some other route.
    pub fn pick(&self, index: usize) -> Option<Item> {
        self.rows
            .get(index)
            .filter(|row| row.enabled)
            .map(|row| row.item)
    }

    /// The longest label in it, in characters, counting what a row steps in by.
    /// What the layout sizes the box from, so every row is as wide as the
    /// widest one.
    pub fn width_chars(&self) -> usize {
        self.rows
            .iter()
            .map(|row| row.item.label().chars().count() + row.item.indent())
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
        let mut menu = Menu::for_widget((10.0, 10.0), View::Plan, Space::Left, false);
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

        assert!(menu.toggle_widgets(&dock));
        assert_eq!(menu.widgets(), 0);
        assert_eq!(items(&menu), shut, "it shuts back to what it opened as");
    }

    /// The exception to a menu keeping its shape is allowed to grow downwards
    /// and no other way: nothing above the row that opened the list moves.
    #[test]
    fn opening_the_list_moves_no_row_above_it() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((10.0, 10.0), View::Files, Space::BottomRight, true);
        let before = items(&menu);
        menu.toggle_widgets(&dock);
        assert_eq!(&items(&menu)[..menu.top - 1], &before[..before.len() - 1]);
        assert_eq!(menu.rows[menu.top - 1].item, Item::Widgets(true));
        assert_eq!(menu.pick(0), Some(Item::Settings));
        assert_eq!(menu.pick(2), Some(Item::Close));
        // And the box does not have to grow either: no view's name is longer
        // than the Close row, indent included.
        assert_eq!(menu.width_chars(), Item::Close.label().chars().count());
    }

    /// The list is the answer to where did that pane go, so it says which
    /// widgets are in the window and which are not, and every row acts.
    #[test]
    fn the_list_marks_what_is_closed_and_every_row_of_it_can_be_picked() {
        let dock = Dock::hiding(&[View::Debug, View::Files]);
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::Left, false);
        menu.toggle_widgets(&dock);
        for (step, view) in View::ALL.into_iter().enumerate() {
            let index = menu.top + step;
            let hidden = matches!(view, View::Debug | View::Files);
            assert_eq!(
                menu.pick(index),
                Some(Item::Widget(view, hidden)),
                "{view:?} is pickable and says whether it is closed"
            );
            assert_eq!(
                Item::Widget(view, hidden).icon(),
                Some(match hidden {
                    true => icons::CLOSE,
                    false => icons::CONFIRM,
                })
            );
            assert_eq!(Item::Widget(view, hidden).label(), view.label());
        }
        assert_eq!(menu.pick(menu.top + View::ALL.len()), None);
    }

    /// The row says which way it is about to go, with the picker tree's marks.
    #[test]
    fn the_widget_row_carries_a_plus_shut_and_a_minus_open() {
        assert_eq!(Item::Widgets(false).icon(), Some(icons::EXPAND));
        assert_eq!(Item::Widgets(true).icon(), Some(icons::COLLAPSE));
        assert_eq!(Item::Widgets(false).indent(), 0);
        assert_eq!(Item::Widget(View::Output, false).indent(), 1);
    }

    /// Nine rows do not always fit under a menu opened near the bottom of a
    /// short window, so the list moves, and it cannot be moved off either end.
    #[test]
    fn the_list_scrolls_and_stops_at_both_ends() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::Left, false);
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
                Menu::for_widget((0.0, 0.0), View::Output, Space::Left, true),
                Menu::for_widget((0.0, 0.0), View::Output, Space::Left, false),
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
        assert_eq!(Item::Close.icon(), None);
    }

    #[test]
    fn a_menu_is_as_wide_as_its_longest_label() {
        let menu = Menu::for_widget((0.0, 0.0), View::Output, Space::Left, false);
        assert_eq!(menu.width_chars(), Item::Close.label().chars().count());
        assert_eq!(
            Menu::for_input((0.0, 0.0), false).width_chars(),
            Item::Paste.label().chars().count()
        );
    }
}
