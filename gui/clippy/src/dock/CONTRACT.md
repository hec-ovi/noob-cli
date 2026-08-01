# dock

contractVersion: 1.0.0

## Purpose

Which view lives in which cell of the capped 2x2 grid, and moving views
between cells. Deliberately not a splitter tree: four cells, two dividers,
every view a tab in exactly one cell.

## Public surface

```rust
pub enum View;               // the nine views; View::ALL is the one
                             // canonical order every per-view array indexes by.
                             // View::Agent (one sub-agent's output) starts
                             // hidden and is opened by clicking an agent,
                             // never from the widget switches
impl View { pub fn label(self) -> &'static str }
pub enum Space;              // the four cells; row/column/index/at,
                             // in_column/in_row partners, neighbours
pub struct Slot;             // one cell's tab strip: tabs, active view
pub struct Dock;             // the whole arrangement: which view where,
                             // divider ratios, move/focus operations
```

## Invariants

1. `View::ALL`'s order is stability API: scroll offsets, per-view arrays,
   and the wire all index by it. New views append.
2. Every view is in exactly one cell or on the hidden list at all times; a
   move is a remove plus an insert that can never drop or duplicate a tab.
3. The grid never nests: two dividers, four spaces, no deeper structure.

## Dependencies

None. This is a pure model; placement and paint live in the view layer,
routing in the shell.

## Tests

Inline: moves, spans, divider clamps, neighbour arithmetic (29 tests).
