# text-geometry

How a run of logical lines becomes rows on a monospace screen.

Pure arithmetic over character counts. Nothing here shapes text, measures a
font, or touches a GPU: you pass the width of your box in columns and the length
of each line in characters, and you get back which lines to draw and where they
land. That is what makes the rule testable without opening a window.

```rust
let heights = text_geometry::heights(lines.iter().map(|l| l.chars().count()), cols);
let window = text_geometry::window(&heights, rows, scrollback);
for line in &lines[window.first..window.first + window.count] { /* draw */ }
// window.skip is how many rows of the first line sit above the viewport.
```

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

The contract test also asserts that the 8 fixtures under `fixtures/invalid/`
are rejected. If one starts passing, the schema has been loosened and the
boundary is no longer failing closed.

[`CONTRACT.md`](CONTRACT.md) is what a caller reads. It is the only file outside
this folder anyone needs.
