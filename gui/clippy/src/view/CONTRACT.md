# view

contractVersion: 1.0.0

## Purpose

The window's geometry and chrome: Layout computes every rectangle from the
window size and the Shape, hit testing maps a point to what is under it,
and the chrome painters (title bar, tabs, panes, menus, overlays, the
cut-corner vocabulary) compose the widgets and surface boxes into one
scene.

## Public surface

```rust
pub struct Shape<'a>;        // everything placement needs: the dock, the
                             // dragged ratios, per-surface size inputs
pub struct Layout;           // compute(w, h, &Shape) -> every rectangle
impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit>;   // one point, one
                             // answer, from the same rectangles paint used
    // capacity and geometry queries per surface (rows, cells, carets)
}
pub enum Hit;                // every clickable thing, one variant each
pub enum Act;                // what a button does
pub struct Frame<'a>;        // everything a frame is drawn from
pub fn build(frame: &Frame) -> Scene;   // the one dispatch
// the shared vocabulary: cut corners, panel edges, tab blocks, list rows,
// scrollbars, text metrics; pub(crate) for the widget and surface boxes
```

## Invariants

1. Placement, paint, and hit testing take the same numbers from the same
   place: a surface's rectangles are computed once and consumed by all
   three, so nothing can be drawn where it is not hit-tested.
2. `build` is the only composition point: it dispatches to the widget
   boxes and the settings/picker surfaces; adding a surface is one arm.
3. The cut-corner vocabulary is the one drawing language: every box draws
   through it, and nothing crosses a cut corner.

## Dependencies

Contracts: the dock box (the grid model), the design box (scales, icons),
[`noob-draw`](../../../noob-draw/CONTRACT.md) (Scene, Panel, Rect, Run),
[`text-geometry`](../../../layers/text-geometry/CONTRACT.md) (wrapping and
windows), the widget boxes and the settings/picker surfaces it composes,
the state box (what the Frame borrows).

## Tests

Scene-level: rendered-scene tests assert placement, hit testing, and every
composed surface's visible behavior through `build` and `Layout::hit`, the
real entry points (the largest test suite in the workspace).
