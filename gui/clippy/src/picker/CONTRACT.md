# picker

contractVersion: 1.0.0

## Purpose

The startup chooser as one box: the folder tree and saved-session model
(mod.rs), where everything sits (places.rs), and what gets drawn
(paint.rs). Folders come through the sessions box's port; the shell routes
input.

## Public surface

```rust
pub struct Picker;           // the model: mode (folders or sessions), the
                             // walked tree, cursor, filter field, what
                             // choosing returns
pub enum Row;                // one visible row: a folder at a depth, or a
                             // saved session
pub mod places;              // PickerPlaces from one Panel + Shape + &Picker
pub mod paint;               // folder_picker(scene, frame)
```

## Invariants

1. One geometry: places feed both the painter and the layout's hit tests.
2. The model walks folders only through `sessions::Folders`, so a test
   drives it over a tree that exists only in the test.
3. Remembered paths that stopped existing render marked, never crash, and
   choosing one is refused with the reason.

## Dependencies

Contracts: the sessions box (`Folders`, `Saved`, `Listing`), the view box
(Frame, Shape, chrome), the design and style boxes, the config box (the
remembered folders it starts from).

## Tests

The model's tests live in mod. Placement and paint are asserted here, on a window rendered through `view::testkit`.
