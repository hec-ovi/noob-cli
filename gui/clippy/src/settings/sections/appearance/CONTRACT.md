# settings/sections/appearance

contractVersion: 1.0.0

## Purpose

The APPEARANCE section of the settings panel: the window-file settings as
cards, the palette as labelled swatch grids under their theme card, and the
restore back to the defaults.

## Public surface

```rust
pub fn rows(config: &Config, file: Option<&Path>) -> Vec<Row>
                                 // the size and opacity cards, the palette,
                                 // where the file is, the restore card
pub fn restoring() -> Vec<&'static str>
                                 // every key a restore comments out; the
                                 // frame's restore() honours it. Re-exported
                                 // at the settings root
pub const OFF_PANEL: [&str; 7];  // keys deliberately not rows: panes and
                                 // dividers, set by using the window
pub fn colours(config: &Config) -> Vec<(&'static str, [u8; 3])>
                                 // every colour in file order; also what
                                 // names the theme
pub(crate) const THEME: &str;    // the preset key; the frame's commit()
                                 // routes it through pick_theme
pub(crate) const LOOKS: [(&str, Kind); 6]
                                 // the six settings with their bounds; the
                                 // frame's slider test walks them
```

## Invariants

1. Pure: rows come from the `Config` in hand and the file path; no I/O here.
2. Every key the file understands is a field, a swatch, or on `OFF_PANEL`;
   nothing falls between the lists.
3. The theme row names the palette actually in hand: a file overriding a
   preset reads as custom, never as the preset's name.
4. Every swatch is labelled with what it colours in plain words, never with
   its key.
5. `restoring()` never contains an `OFF_PANEL` key.

## Dependencies

The settings box's shared vocabulary (Row, Card, CardField, Palette, Swatch,
Kind, File, Doing); the config box for keys, themes, colour tables and
bounds.

## Tests

Inline: the key coverage sweep, the palette's labels and groups, the theme
naming and picking, the two transparencies, the round trip through the real
file, and the restore (12 tests), driven through the frame's `Settings`.
