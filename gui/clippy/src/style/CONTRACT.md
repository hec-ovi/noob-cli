# style

contractVersion: 1.1.0

## Purpose

Transcript presentation: the palette derived from the window's config
(skin), syntax colors for the file view (syntax), and markdown to styled
runs (markdown). Config in, styled runs out; nothing here lays out or
paints.

## Public surface

```rust
pub mod skin;      // Skin::from(&Config): every color and alpha the
                   // painters use, one palette per theme
pub mod syntax;    // fn spans(line, language) -> colored spans
pub mod markdown;  // fn runs(text, &Skin) -> styled runs for the pane
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
3. Syntax scanning is line-local and total: an unknown language or a
   half-open token still colors something reasonable, never errors.
4. Markdown never re-wraps: it styles runs, and the text-geometry layer
   wraps them; the two never both own line breaks.

## Dependencies

Contracts: the config box (the settings the palette derives from), the
transcript pane types it styles (state, until that boxes), the design box's
type scale.

## Tests

Inline in each file: palette derivation per theme, scanner spans, markdown
run shapes.
