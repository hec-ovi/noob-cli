# orb

contractVersion: 1.0.0

## Purpose

The thinking orb, the one animated thing in the window: dots projected
orthographically, far to near, drawn as rounded rects through the pipeline
the window already has. No shader, no second pipeline.

## Public surface

```rust
pub fn discs(block: Panel, seconds: f32, morph: f32, skin: &Skin) -> Vec<Rect>;
    // the frame at `seconds`; morph 0..=1 blends the square rest plate
    // into the turning formation and back
```

## Invariants

1. Depth is size and colour weight, never blur; painter's order is push
   order, far to near.
2. `morph` is the ONE transition: at 0 the dots form the idle square, at 1
   the orbit; the caller animates the scalar, this box owns the geometry.
3. The maths follows `docs/ORB-SPEC.md`, ported from thinking-orbs (MIT),
   not guessed.

## Dependencies

Contracts: [`noob-draw`](../../../noob-draw/CONTRACT.md) (`Panel`, `Rect`),
the style box (`Skin` colors).

## Tests

Inline: formation counts, sort order, morph endpoints (17 tests).
