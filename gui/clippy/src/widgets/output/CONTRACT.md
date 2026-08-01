# widgets/output

contractVersion: 1.0.0

## Purpose

The transcript widget: the conversation pane, wrapped text with selection
bands. Messages waiting behind the running turn (`state.queued`) are pinned
to the panel's bottom rows as dim one-line `› message [queued]` rows that
stand outside the scrollback; the transcript is measured, scrolled and
banded in the rows that remain (`state.output_reserved`).

## Surface

One painter: props in, draw calls out. It reads frame.state.output, the scrolls, the selection and the layout's
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

Scene-level: the view box's rendered-scene tests assert every widget's
visible behavior through `build`, the real entry point.
