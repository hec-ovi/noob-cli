# text-geometry

contractVersion: 1.3.0

## Purpose

Turn a run of logical lines into the rows they occupy on a monospace screen,
and answer every question a caller has about that mapping.

## Inputs

| Name | Schema | Preconditions |
|---|---|---|
| Line height request | [`schema/line-height-request.json`](schema/line-height-request.json) | `chars` is a character count, not bytes. `cols` may be 0 (a window mid-resize). |
| Wrap request | [`schema/wrap-request.json`](schema/wrap-request.json) | `lengths` are character counts, not bytes. `cols` may be 0 (a window mid-resize). |
| Rows request | [`schema/rows-request.json`](schema/rows-request.json) | `text` is one logical line; a newline in it is an ordinary character. `cols` may be 0. `break` is how the box is drawn, and the drawing and the counting must pass the same one. |
| Visual row request | [`schema/visual-row-request.json`](schema/visual-row-request.json) | Same pairing rule as the row request: `window` came from this layer and was built from the same `heights` and `rows`. |
| Scrollback bound request | [`schema/max-scrollback-request.json`](schema/max-scrollback-request.json) | `heights` came from this layer. `rows` may be 0. |
| Window request | [`schema/window-request.json`](schema/window-request.json) | `heights` came from this layer. `rows` may be 0. `scrollback` past the top is clamped, not refused. |
| Row request | [`schema/row-request.json`](schema/row-request.json) | `window` came from this layer and was built from the same `heights` and `rows`. |
| Band request | [`schema/band-request.json`](schema/band-request.json) | Same pairing rule as the row request. `line` is absolute, not relative to the window. |
| Thumb request | [`schema/thumb-request.json`](schema/thumb-request.json) | `heights` came from this layer. |
| Scrollback request | [`schema/scrollback-request.json`](schema/scrollback-request.json) | `heights` came from this layer. `firstRow` counts visual rows from the top; past the last screenful is clamped, not refused. |

## Outputs

| Name | Schema | Postconditions |
|---|---|---|
| Line height | [`schema/line-height.json`](schema/line-height.json) | At least 1. An empty line still occupies a row. |
| Heights | [`schema/heights.json`](schema/heights.json) | One entry per input length, same order, every entry at least 1. |
| Scrollback bound | [`schema/max-scrollback.json`](schema/max-scrollback.json) | 0 when the pane fits its viewport. Otherwise the scrollback that puts the first row at the top, and the value every other scrollback here is clamped against. |
| Window | [`schema/window.json`](schema/window.json) | The lines it names cover the viewport and overshoot it by less than the height of one line. `first + count <= heights.len()`. |
| Rows | [`schema/rows.json`](schema/rows.json) | At least one row, in order, none wider than `cols`. Together they cover the line except for the one character each break was spent on. A caller measuring a whole pane can have the same answer written into a buffer it owns, which is the same operation and the same shape. |
| Line hit | [`schema/line-hit.json`](schema/line-hit.json) | Null exactly when the row is past the last line. Otherwise the line is inside the window. The offset is `row * cols`, which is where the row starts only under a `column` break. |
| Visual row | [`schema/visual-row.json`](schema/visual-row.json) | Null exactly when the row is past the last line. Otherwise the line is inside the window and the row number is below that line's height. |
| Band | [`schema/band.json`](schema/band.json) | Null exactly when the line is not visible. Otherwise `top + height <= rows`. |
| Thumb | [`schema/thumb.json`](schema/thumb.json) | Null exactly when the content fits. Otherwise `top + size <= 1`. |
| Scrollback | [`schema/scrollback.json`](schema/scrollback.json) | Never above `max_scrollback`. Feeding it to `window` puts `firstRow` at the top, or the last screenful when `firstRow` is past the end. |

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
5. Nothing here allocates per frame beyond the heights vector and the rows of
   the lines a caller asks about, and nothing shapes text or measures a font.
6. There is one wrap rule and it lives here. A row takes as many characters as
   fit; under a `word` break it ends at the last break opportunity at or before
   the column limit, and a word wider than the whole box ends on the column
   rather than running off the edge. A break opportunity is a blank or a tab,
   and nothing else: breaking after a hyphen or a slash would split a path or a
   flag, and a transcript is full of both. The character a row broke at is
   spent on the break, drawn on neither row, so nothing is indented by a blank
   the reader cannot see. It stays in the logical line, so copying a run that
   crosses a break gets it back exactly once. A `column` break has no break
   opportunities at all, so its rows are `cols` characters each and its row
   count is the one `rows_of` gives.
7. The drawing and the counting pass the same `text`, `cols` and `break`, so
   the characters on a drawn row are the characters a selection on that row
   copies. Two callers wrapping the same line two ways is the defect this
   operation exists to remove.
6. A position is bottom-anchored everywhere except in `scrollback_for`, which is
   the only place a top-anchored row is understood. A caller with a list to
   scroll converts once through it rather than keeping two conventions.

## How to modify this blackbox safely

The one rule: `row_at` (and `line_at`, which is the same walk) and `band` are
inverses of each other over the visible rows, and `window` defines what visible
means. Change any one of the three and the test
`every_row_the_band_covers_maps_back_to_that_line` is what catches you.

The second rule: `rows_in` is where wrapping is decided for the whole window,
for the pixels as well as for the arithmetic. Change how it breaks and both
move together, which is the point. `the_rows_of_a_line_cover_it_in_order_and_lose_only_the_breaks`
is what catches you, and on the caller's side the window's own test
`a_pane_is_drawn_in_the_columns_its_selection_is_counted_in`.

Adding an operation is additive: new function, new pair of schema files, new
row in the tables above, minor `contractVersion` bump. Changing the meaning of
`Window.skip` or of `heights` is breaking, so add the new shape alongside and
migrate callers rather than editing in place.

Callers must not re-derive wrapping themselves. If a caller needs a number this
contract does not expose, the fix is a new operation here, not arithmetic at
the call site: eight call sites each doing their own version is exactly how the
original bug happened.
