# widgets/popup

contractVersion: 2.0.0

## Purpose

The call popup: one activity row opened out as a full-panel overlay. A
metadata header (when the call happened, which turn, how long, the tool, the
thing it reached for, the file when the arguments name one) with the
settings-style close mark, over two halves splitting the remaining room: CALL
(what the model generated, as real lines - a multiline command reads as its
lines, never as `\n` spelled out) and RESULT (what came back: summary,
output, and the whole failure story). Each half scrolls on its own and holds
its own selection.

## Surface

`Half` names the two: `Half::Call | Half::Result`, `Half::BOTH`,
`Half::index()` for per-half state (`Frame::popup_scroll` is `[usize; 2]`).
`popup(scene, frame)` paints it from `frame.state.popped()`,
`frame.layout.call_popup` and the two scroll offsets, selection bands
included. `scroll_geometry(frame, half)` is what the wheel, the dragged
track and the shell's clamp measure with. `half_at(frame, y)` says which
half a point is over. `half_document(call, half)` is a half flattened to the
lines it is drawn as - what a selection resolves and copies against - and
`spot_at(frame, half, x, y)` is the character under a point, for the shell's
press and drag; the half is the caller's, so a drag stays in the half it
began in.

## Invariants

1. Pure paint: no state outside the call, no clock past `State::popup_header`,
   no filesystem.
2. Measured and drawn in one geometry: each half clips through the same
   `scrolled` window the panes use, so nothing is drawn over the header or
   past its box.
3. The content is the state box's `Call::call_lines`, `Call::result_lines`
   and `State::popup_header`; this box never composes content.

## Dependencies

Contracts: the view box (Frame, Hit, the panel/track vocabulary, the one
dispatch), [`noob-draw`](../../../../noob-draw/CONTRACT.md) (Scene, Panel,
Run, Text), [`text-geometry`](../../../../layers/text-geometry/CONTRACT.md)
(rows, windows, the thumb), the state box (the call and the header), the
style box (colors), the design box (the close icon).

## Tests

Inline: the halves as documents and what a selection in one copies, the
rendered header-plus-halves scene (2 tests).
