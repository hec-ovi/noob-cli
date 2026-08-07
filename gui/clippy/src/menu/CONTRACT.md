# menu

contractVersion: 2.0.0

## Purpose

The right-click menus as a model: the prompt's, a widget's, a session's, the
window's own, rows with icons and warning colors, and the widgets flyout. The
view draws it last and hit-tests it first; this box only decides what is in it.

## Public surface

```rust
pub enum Item;               // every row kind; label/icon/marker/warns,
                             // group(): whether it opens the flyout.
                             // Item::Settings is an act: open the panel
pub struct Row;              // one visible row: item, enabled
pub enum Target;             // what was right-clicked
pub struct Menu;
impl Menu {
    pub fn for_input(at: (f32, f32), has_selection: bool) -> Menu;
    pub fn for_widget(at: (f32, f32), view: View, space: Space,
                      has_selection: bool) -> Menu;
    pub fn for_window(at: (f32, f32)) -> Menu;
                             // the title strip and the room around the
                             // panes: Settings, New session, Widgets
    pub fn for_session(at: (f32, f32), index: usize, gone: bool) -> Menu;
    // rows, pick(index), fold (the flyout), fly_start/fly_anchor/main_len,
    // width_chars/fly_width_chars, walk/scroll/point_at/hover
}
```

## Invariants

1. Opening the widgets flyout moves no row of the menu: its rows are
   appended after the menu's own (`fly_start` marks where they begin) and
   the view places them in a second box beside the header, top-aligned
   with it. It opens on rollover of the header (`hover`), stays while the
   pointer is inside it or off the menu, and goes away when the pointer
   rests on another row of the column.
2. A row that would do nothing (copy with no selection, a gone session) is
   greyed, not hidden, so the menu's shape is stable.
3. Settings is one act that opens the settings panel; the panel's rail is
   the only list of sections.
4. The widgets flyout is on the window's menu as well as on a pane's, so a
   window with every widget closed still has the list that puts one back.

## Dependencies

Contracts: the dock box (`View`, `Space`) and the design box (icons).

## Tests

Inline: flyout open and shut, pick indices, grey rules, keyboard walk,
widths (43 tests). Its drawing is asserted in `paint.rs`, on a window rendered through `view::testkit`.
