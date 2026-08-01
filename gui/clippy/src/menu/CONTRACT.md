# menu

contractVersion: 1.0.0

## Purpose

The right-click menus as a model: two menus (the prompt's, a widget's),
rows with icons and warning colors, groups that expand in place. The view
draws it last and hit-tests it first; this box only decides what is in it.

## Public surface

```rust
pub enum Item;               // every row kind; label/icon/marker/warns,
                             // group(): whether it expands in place
pub struct Row;              // one visible row: item, depth, enabled
pub enum Target;             // what was right-clicked
pub struct Menu;
impl Menu {
    pub fn for_input(at: (f32, f32), has_selection: bool) -> Menu;
    pub fn for_widget(at: (f32, f32), view: View, space: Space,
                      has_selection: bool) -> Menu;
    pub fn for_session(at: (f32, f32), index: usize, gone: bool) -> Menu;
    // rows(), pick(index), and the open/close of groups
}
```

## Invariants

1. Groups expand in place: picking a group toggles its children into the
   row list; nothing floats beyond the one menu.
2. A row that would do nothing (copy with no selection, a gone session) is
   greyed, not hidden, so the menu's shape is stable.
3. The settings group's entries are the settings box's published section
   list, in its order; this box never invents section names.

## Dependencies

Contracts: the dock box (`View`, `Space`), the design box (icons), and the
settings box's published section list (`SECTIONS`).

## Tests

Inline: row expansion, pick indices, grey rules, widths (25 tests).
