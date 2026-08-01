# scroll

contractVersion: 1.0.0

## Purpose

Where each self-scrolling pane is scrolled to: one top-anchored offset per
view, converted to the bottom-anchored window text-geometry speaks in one
place.

## Public surface

```rust
pub struct Scrolls;                        // one offset per View::ALL entry
impl Scrolls {
    pub fn first(&self, view: View) -> usize;
    pub fn scroll(&mut self, view: View, by: usize, down: bool,
                  heights: &[usize], rows: usize) -> bool;
    pub fn scroll_to(&mut self, view: View, fraction: f32,
                     heights: &[usize], rows: usize) -> bool;
                                           // a dragged bar: 0.0 the top,
                                           // 1.0 the last screenful
    pub fn settle(&mut self, view: View, heights: &[usize], rows: usize)
        -> bool;                           // clamp after content shrank
    pub fn window(&self, view: View, heights: &[usize], rows: usize)
        -> text_geometry::Window;
    pub fn thumb(&self, view: View, heights: &[usize], rows: usize)
        -> Option<(f32, f32)>;             // scrollbar extent, or none
}
```

## Invariants

1. Top-anchored: a row arriving at the end never moves the rows above it.
2. The bottom-anchored conversion happens in exactly one place
   (`window`), so two conventions never coexist.
3. Offsets clamp to content: `settle` after shrink, `scroll` returns
   whether anything moved.

## Dependencies

Contracts: the dock box (`View` order),
[`text-geometry`](../../../layers/text-geometry/CONTRACT.md) (`Window`).

The file explorer's flat list has the same operations as free functions
(`file_thumb`, `scroll_files`, `file_scroll_to`, `reveal_file`).

## Tests

Inline: clamping, windows, thumbs, dragged fractions.
