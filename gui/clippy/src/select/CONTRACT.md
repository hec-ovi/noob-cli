# select

contractVersion: 1.0.0

## Purpose

Pointer selection over the monospace panes, held as absolute line numbers
so scrolling and ring eviction never slide a selection onto different text.

## Public surface

```rust
pub struct Spot { line, column }           // absolute line, char column
pub enum Where;                            // which pane the selection is in
pub struct Selection;
impl Selection {
    pub fn new(at: Where, spot: Spot) -> Selection;
    pub fn view(&self) -> Option<View>;
    pub fn extend(&mut self, to: Spot);
    pub fn range(&self) -> (Spot, Spot);   // ordered
    pub fn is_empty(&self) -> bool;
    pub fn columns_on(&self, line: usize, len: usize)
        -> Option<(usize, usize)>;         // the band on one line
    pub fn text(&self, pane: &Pane) -> String;   // what copy copies
}
```

## Invariants

1. Absolute lines: a selection survives scrolling and eviction; a line that
   left the ring stops resolving instead of resolving wrongly.
2. Monospace arithmetic only: no layout queries; a pixel maps to row and
   column outside this box.
3. `range` is always ordered regardless of drag direction.

## Dependencies

Contracts: the dock box (`View`) and the transcript pane type it reads
(`state::Pane`, until the transcript model becomes its own box).

## Tests

Inline: band arithmetic, direction independence, eviction behavior (12
tests).
