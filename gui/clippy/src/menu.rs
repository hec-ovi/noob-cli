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
//! The nine rows it carries are only there while it is open. A menu that always
//! held all nine would be a wall of tab names in front of a Close row, and one
//! that opened them downwards would push its own rows around under the pointer.
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
            // A box with a tick in it for a widget that is in the window, an
            // empty one for a widget that is out. The row is a switch, so it has
            // to say which way it is set before it is pressed: without the mark
            // a click on it is a coin flip.
            Item::Widget(_, hidden) => Some(match hidden {
                true => icons::UNCHECKED,
                false => icons::CHECKED,
            }),
            _ => None,
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

    /// How many widgets are on the list: nine while it is open, none while it
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
            Target::Input => None,
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
                row.item.label().chars().count()
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
        let dock = Dock::hiding(&[View::Debug, View::Files]);
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
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

    /// The row that opens the list carries a submenu marker at its end, open or
    /// shut, and carries nothing in the gutter in front of it. This asserted
    /// the opposite: a plus and a minus in the gutter, which is what a row that
    /// folds a list out underneath itself says, and this row no longer does
    /// that.
    #[test]
    fn the_widget_row_carries_a_submenu_marker_at_its_end_and_no_mark_in_front() {
        for open in [false, true] {
            assert_eq!(Item::Widgets(open).marker(), Some(icons::SUBMENU));
            assert_eq!(Item::Widgets(open).icon(), None);
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

        assert!(dock.hide(View::Debug));
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
                Some(Item::Widget(view, view == View::Debug))
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
        assert_eq!(Item::Close.icon(), None);
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
