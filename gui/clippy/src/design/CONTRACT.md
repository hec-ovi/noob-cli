# design

contractVersion: 1.0.0

## Purpose

The window's space scale and type scale as arithmetic, all derived from one
number (the pane text size), plus the named icon codepoints. `gui/DESIGN.md`
is the prose twin.

## Public surface

```rust
pub const TIGHT/STEP/ROOM/APART: f32;      // the four gaps, in line units
pub fn tight/step/room/apart(line: f32) -> f32;
pub const PANEL_TITLE/CARD_TITLE/LABEL/VALUE/HINT: f32;   // the type scale
pub fn panel_title_size(...) and friends;  // scale times the base size
pub mod icons;                             // named codepoints in the
                                           // embedded symbol font
```

## Invariants

1. One number scales everything: raising the pane text size scales gaps
   and titles together, nothing overruns its box.
2. Rounding happens once, at paint time, never in the scale arithmetic.
3. An icon is a named constant here or it does not exist: no bare
   codepoints elsewhere.

## Dependencies

None. Constants and arithmetic only.

## Tests

Inline: scale monotonicity and the two rounding rules, icon table shape (9 tests across the two files).
