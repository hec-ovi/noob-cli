//! What a right click opens, and what its rows do.
//!
//! A menu is not part of the window: it floats above it. [`crate::view`] draws
//! it last and hit tests it first, which is the whole of what floating means
//! here, and this module is only the model of what is in it.
//!
//! Two menus, because there are two things worth pointing at. Right click the
//! prompt and you get the two things a text field can do. Right click a pane or
//! the tab that names it and you get the three things a widget can do.
//!
//! A row that cannot act is greyed rather than absent, so a menu is the same
//! shape and the same height every time it opens for the same target. A menu
//! that grows a row when there is a selection moves every row under it, and the
//! pointer has to read the whole thing again to find the one it came for.

use crate::dock::{Space, View};
use crate::icons;

/// One thing a menu can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// Copy what is selected in the prompt.
    Copy,
    /// Put the clipboard into the prompt.
    Paste,
    /// Where the settings panel will go. There is no panel yet, so every menu
    /// carrying this row carries it disabled; see [`Menu::for_widget`].
    Settings,
    /// Copy what is selected in the pane the menu opened over.
    CopySelection,
    /// Take the pane out of the window, via `Dock::hide`.
    Close,
}

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Item::Copy => "Copy",
            Item::Paste => "Paste",
            Item::Settings => "Settings",
            Item::CopySelection => "Copy selection",
            Item::Close => "Close this widget",
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
}

/// An open menu: where it was opened, what it was opened on, and its rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    /// Where the pointer was. The layout turns this into a box, clamping it so
    /// a menu opened near an edge still fits on screen.
    pub at: (f32, f32),
    pub target: Target,
    pub rows: Vec<Row>,
}

impl Menu {
    /// The prompt's menu. Paste is always available: the clipboard is the
    /// display server's, and asking it whether it holds anything means
    /// connecting to it, which is work a right click has no reason to do.
    pub fn for_input(at: (f32, f32), has_selection: bool) -> Menu {
        Menu {
            at,
            target: Target::Input,
            rows: vec![
                Row {
                    item: Item::Copy,
                    enabled: has_selection,
                },
                Row {
                    item: Item::Paste,
                    enabled: true,
                },
            ],
        }
    }

    /// A pane's menu.
    ///
    /// Settings is always disabled. There is no settings panel to open yet, and
    /// a row that opens nothing is worse than one that says out loud it cannot:
    /// the first reads as a broken window, the second as an unfinished one.
    pub fn for_widget(at: (f32, f32), view: View, space: Space, has_selection: bool) -> Menu {
        Menu {
            at,
            target: Target::Widget(view, space),
            rows: vec![
                Row {
                    item: Item::Settings,
                    enabled: false,
                },
                Row {
                    item: Item::CopySelection,
                    enabled: has_selection,
                },
                Row {
                    item: Item::Close,
                    enabled: true,
                },
            ],
        }
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

    /// The longest label in it, in characters. What the layout sizes the box
    /// from, so every row is as wide as the widest one.
    pub fn width_chars(&self) -> usize {
        self.rows
            .iter()
            .map(|row| row.item.label().chars().count())
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
    fn a_pane_gets_settings_a_copy_and_a_close() {
        let menu = Menu::for_widget((10.0, 10.0), View::Plan, Space::TopRight, true);
        assert_eq!(
            items(&menu),
            vec![Item::Settings, Item::CopySelection, Item::Close]
        );
        assert_eq!(menu.target, Target::Widget(View::Plan, Space::TopRight));
        assert_eq!(menu.pick(1), Some(Item::CopySelection));
        assert_eq!(menu.pick(2), Some(Item::Close));
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
                Menu::for_widget((0.0, 0.0), View::Talk, Space::Left, true),
                Menu::for_widget((0.0, 0.0), View::Talk, Space::Left, false),
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

    /// Settings is drawn and refuses to act, in every menu that has it, because
    /// there is nothing behind it yet.
    #[test]
    fn settings_is_always_disabled_and_carries_an_icon() {
        for has_selection in [false, true] {
            let menu = Menu::for_widget((0.0, 0.0), View::Files, Space::BottomRight, has_selection);
            assert_eq!(menu.rows[0].item, Item::Settings);
            assert!(!menu.rows[0].enabled);
            assert_eq!(menu.pick(0), None);
        }
        assert_eq!(Item::Settings.icon(), Some(icons::SETTINGS));
        assert_eq!(Item::Close.icon(), None);
    }

    #[test]
    fn a_menu_is_as_wide_as_its_longest_label() {
        let menu = Menu::for_widget((0.0, 0.0), View::Talk, Space::Left, false);
        assert_eq!(menu.width_chars(), Item::Close.label().chars().count());
        assert_eq!(
            Menu::for_input((0.0, 0.0), false).width_chars(),
            Item::Paste.label().chars().count()
        );
    }
}
