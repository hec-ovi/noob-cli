# widgets/popup

contractVersion: 1.0.0

## Purpose

The call popup: one activity row opened out as a full-panel overlay. A
header (the tool, its target, how it stands) with the settings-style close
mark, over a scrolled stack of blocks, one per `Call::cells()` entry, each
with a bar down its left in the cell's tone.

## Surface

Two functions: `popup(scene, frame)` paints it from `frame.state.popped()`,
`frame.layout.call_popup` and `frame.popup_scroll`;
`scroll_geometry(frame) -> Option<(total_rows, fit_rows)>` is what the
wheel, the dragged track and the shell's clamp measure with, from the same
columns the glyphs wrap in.

## Invariants

1. Pure paint: no state outside the call, no clock, no filesystem.
2. Measured and drawn in one geometry: a block crossing the viewport edge
   clips through the same `scrolled` window the panes use, so nothing is
   ever drawn over the header or past the box.
3. What the blocks say is the state box's `Call::cells()`; this box never
   composes content.

## Dependencies

Contracts: the view box (Frame, Hit, the panel/track vocabulary, the one
dispatch), [`noob-draw`](../../../../noob-draw/CONTRACT.md) (Scene, Panel,
Run, Text), [`text-geometry`](../../../../layers/text-geometry/CONTRACT.md)
(rows, windows, the thumb), the state box (the call), the style box
(colors), the design box (the close icon).

## Tests

Scene-level: the view box's rendered-scene tests assert the popup's visible
behavior through `build`; the shell's hit tests cover the close mark and
the track.
