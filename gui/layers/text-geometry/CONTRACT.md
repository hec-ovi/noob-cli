# text-geometry

contractVersion: 1.0.0

## Purpose

Turn a run of logical lines into the rows they occupy on a monospace screen,
and answer every question a caller has about that mapping.

## Inputs

| Name | Schema | Preconditions |
|---|---|---|
| Wrap request | [`schema/wrap-request.json`](schema/wrap-request.json) | `lengths` are character counts, not bytes. `cols` may be 0 (a window mid-resize). |
| Window request | [`schema/window-request.json`](schema/window-request.json) | `heights` came from this layer. `rows` may be 0. `scrollback` past the top is clamped, not refused. |
| Row request | [`schema/row-request.json`](schema/row-request.json) | `window` came from this layer and was built from the same `heights` and `rows`. |
| Band request | [`schema/band-request.json`](schema/band-request.json) | Same pairing rule as the row request. `line` is absolute, not relative to the window. |
| Thumb request | [`schema/thumb-request.json`](schema/thumb-request.json) | `heights` came from this layer. |

## Outputs

| Name | Schema | Postconditions |
|---|---|---|
| Heights | [`schema/heights.json`](schema/heights.json) | One entry per input length, same order, every entry at least 1. |
| Window | [`schema/window.json`](schema/window.json) | The lines it names cover the viewport and overshoot it by less than the height of one line. `first + count <= heights.len()`. |
| Line hit | [`schema/line-hit.json`](schema/line-hit.json) | Null exactly when the row is past the last line. Otherwise the line is inside the window. |
| Band | [`schema/band.json`](schema/band.json) | Null exactly when the line is not visible. Otherwise `top + height <= rows`. |
| Thumb | [`schema/thumb.json`](schema/thumb.json) | Null exactly when the content fits. Otherwise `top + size <= 1`. |

## Events

None. Every operation is a pure function of its input.

## Errors

None. The closed set is empty by construction: every input this contract admits
has a defined answer, including the degenerate ones (no lines, no rows, no
columns), which return an empty window rather than failing. That is deliberate,
because a window mid-resize legitimately produces all three.

## Dependencies

None. This layer has no dependencies on other contracts and no crate
dependencies, so it builds and tests without a GPU, a font, or a window.

## Invariants

1. A row is a **visual** row. Logical lines and visual rows coincide only when
   nothing wraps, and assuming they always do is the defect this layer exists
   to remove.
2. An empty line occupies one row. Collapsing it would reflow the pane, because
   a blank line in a transcript is a paragraph break.
3. `band` and `line_at` agree. Every row a band covers maps back to that same
   line. A contract test asserts this over a mixed-height pane, because a
   highlight that covers text the clipboard does not contain is the failure
   this pairing prevents.
4. A line partly scrolled off the top is drawn in full and offset by `skip`,
   never dropped. This is what makes a long paragraph scroll a row at a time.
5. Nothing here allocates per frame beyond the heights vector, and nothing
   shapes text or measures a font.

## How to modify this blackbox safely

The one rule: `line_at` and `band` are inverses of each other over the visible
rows, and `window` defines what visible means. Change any one of the three and
the test `every_row_the_band_covers_maps_back_to_that_line` is what catches you.

Adding an operation is additive: new function, new pair of schema files, new
row in the tables above, minor `contractVersion` bump. Changing the meaning of
`Window.skip` or of `heights` is breaking, so add the new shape alongside and
migrate callers rather than editing in place.

Callers must not re-derive wrapping themselves. If a caller needs a number this
contract does not expose, the fix is a new operation here, not arithmetic at
the call site: eight call sites each doing their own version is exactly how the
original bug happened.
