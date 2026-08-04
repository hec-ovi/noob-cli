# style

contractVersion: 1.2.0

## Purpose

Transcript presentation: the palette derived from the window's config
(skin), syntax colors for the file view (syntax), markdown to styled runs
(markdown), and a markdown table laid out for the box it is drawn in
(table). Config in, styled runs out; nothing here paints.

## Public surface

```rust
pub mod skin;      // Skin::from(&Config): every color and alpha the
                   // painters use, one palette per theme
pub mod syntax;    // fn spans(line, language) -> colored spans
pub mod markdown;  // fn runs(text, &Skin) -> styled runs for the pane,
                   // and fn inline_shown(text) -> the marks consumed
pub mod table;     // fn opens(head, rule) / fn is_row(line): where a
                   // block starts and how far it runs
                   // fn layout(&[&str], cols) -> one drawn string per
                   // source row, newline separated where a cell wrapped
                   // enum Part { Head, Rule, Body }, MAX_COLUMNS, MAX_ROWS
```

## Invariants

1. The palette is a pure function of the config: same settings, same
   colors, no ambient state.
2. A theme is a whole palette: the text tints, the bar the title strip is
   filled with, the pane, the syntax colors and the gauges all come off it.
   Each theme leads with its own hue (matrix green, cool blue, red red) and
   the other two share no tone with matrix; `good` stays green and `bad` a
   hot red in every theme, because they mean yes and no rather than the
   theme.
3. Selection and structure wear the accent: the showing tab's border, the
   picked band and its picker mark, the lit-row hover, headings and card
   titles all derive from the theme's accent, so they restyle with it.
   `good` and `bad` are outcome inks only.
4. Syntax scanning is line-local and total: an unknown language or a
   half-open token still colors something reasonable, never errors.
5. Markdown never re-wraps: it styles runs, and the text-geometry layer
   wraps them; the two never both own line breaks.
6. A table is the one thing laid out here, because its columns cannot be
   known from one line: `layout` takes the whole block and the width of the
   box and answers one string per source row, so a caller's line count is
   the same before and after. No row it returns is wider than the box, and a
   cell that does not fit wraps inside its column by the text-geometry rule,
   the break carried in the string as a newline.
7. Nothing is cut. A box too narrow for columns anything fits in gets the
   same rows as a list of `Header: value` lines instead of a grid.

## Dependencies

Contracts: the config box (the settings the palette derives from), the
transcript pane types it styles (state, until that boxes), the design box's
type scale, [`text-geometry`](../../../layers/text-geometry/CONTRACT.md)
(the one wrap rule, which a table cell breaks by).

## Tests

Inline in each file: palette derivation per theme, scanner spans, markdown
run shapes, table blocks at three widths (~40 tests).
