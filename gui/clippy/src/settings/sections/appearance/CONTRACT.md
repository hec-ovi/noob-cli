# settings/sections/appearance

contractVersion: 1.2.0

## Purpose

The APPEARANCE section of the settings panel: the window-file settings as
cards (TRANSPARENCY with the base application before the widget windows,
BACKGROUNDS with the colour of each of those two surfaces, INPUT PROMPT),
the DEFAULT THEMES card with the three presets beside a custom option, the
rest of the palette as labelled swatch grids under it (each swatch editable
in place by a press and a typed hex value), and the restore back to the
defaults.

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
3. The theme row names the palette actually in hand: a file carrying any
   colour line of its own reads as custom, never as a preset's name, even
   when the values match one (ownership is what custom means).
4. Custom is a pickable option, the right column of the DEFAULT THEMES
   card: two presses (it writes many lines), and the second writes every
   colour of the palette in hand into the file as the user's own live
   lines, values unchanged. A preset pick is the way back; it comments
   those lines out.
5. Every swatch is labelled with what it colours in plain words, never with
   its key; the key is said on the footer when the swatch is pressed.
6. Every swatch is settable from the panel: a press opens the frame's
   editing line on the value in hand, a typed `#rrggbb` lands in the window
   file under the swatch's key and overrides its theme, and a value the
   parser refuses is said on the footer with nothing written. The swatch
   always shows the value the file really carries.
7. `restoring()` never contains an `OFF_PANEL` key.
8. `background` and `panel` are swatches on the BACKGROUNDS card beside the
   two transparencies that move the same two surfaces, and are not in the
   grid as well: one line of the file is one control on the panel.

## Dependencies

The settings box's shared vocabulary (Row, Card, CardField, Palette, Swatch,
Kind, File, Doing); the config box for keys, themes, colour tables and
bounds.

## Tests

Inline: the key coverage sweep, the palette's labels and groups, the theme
naming and picking, the custom pick's two-press write, the swatch hex write
path, the two transparencies, the round trip through the real file, and the
restore (21 tests), driven through the frame's `Settings`.
