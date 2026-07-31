# text-geometry

How a run of logical lines becomes rows on a monospace screen.

Pure arithmetic over characters. Nothing here shapes text, measures a font, or
touches a GPU: you pass the width of your box in columns and either the length
of each line or the line itself, and you get back which lines to draw, where
they land, and which characters end up on each row. That is what makes the rule
testable without opening a window.

```rust
let rows_of = |line: &str| text_geometry::rows_in(line, cols, Break::Word);
let heights: Vec<usize> = lines.iter().map(|l| rows_of(l).len()).collect();
let window = text_geometry::window(&heights, rows, scrollback);
for line in &lines[window.first..window.first + window.count] { /* draw rows_of(line) */ }
// window.skip is how many rows of the first line sit above the viewport.
```

`rows_in` is the wrap rule, and it is the only one: a row takes as many
characters as fit and ends at the last blank or tab at or before the limit, a
word wider than the box ends on the column, and the character a row broke at is
drawn on neither row while staying in the line, so copying across a break gets
it back exactly once. `Break::Column` turns the break opportunities off, which
is what a caret placed as `row * cols + column` needs. The renderer breaks the
rows it draws with the same call, so what is on a row and what a selection
there copies are the same characters.

## Why it exists

The rule was previously written out at eight call sites in the window and disagreed
with itself at three of them. A pane asked for as many logical lines as rows
fit, the shaper wrapped some of them onto two or more rows, and the overflow
fell out of the clip box with no scroll position that could reach it, so the end
of a long message was permanently invisible. The selection band and the
scrollbar drifted from the text for the same reason.

One owner for the rule, and those three symptoms are one fix.

## The thing to remember

A position is a **visual row**, never a logical line. The two coincide only when
nothing wraps, which is exactly the assumption that broke.

## Tests

```
cargo test -p text-geometry      # behaviour
python3 tests/contract.py        # the boundary: fixtures against schema/
```

The contract test also asserts that the 18 fixtures under `fixtures/invalid/`
are rejected. If one starts passing, the schema has been loosened and the
boundary is no longer failing closed.

[`CONTRACT.md`](CONTRACT.md) is what a caller reads. It is the only file outside
this folder anyone needs.
