# widgets/context

contractVersion: 1.0.0

## Purpose

The context widget: what the run holds now, drawn through the gauge vocabulary.

## Surface

One painter: props in, draw calls out. It reads frame.state.context and the monitor context gauges and the layout's
panel, pushes rects and text into the scene, and owns nothing between
frames. Extent questions (how many rows exist) go through
`view::scroll_extent`, which asks this box's row builders where they exist.

## Invariants

1. Pure paint: no state outside the call, no clock, no filesystem.
2. Placement, paint, and hit regions use the same rectangles: the panel
   handed in is the one the layout hit-tests.
3. Adding a widget is adding a folder here plus one dispatch arm in the
   view's build; nothing else changes.

## Dependencies

Contracts: the view box (Frame, the shared chrome and list vocabulary, the
one dispatch), [`noob-draw`](../../../../noob-draw/CONTRACT.md) (Scene,
Panel, Run), the state box (what it reads), the style box (colors).

## Tests

Scene-level: this box renders its own window through `view::testkit` and reads what was drawn (3 tests in all here).
