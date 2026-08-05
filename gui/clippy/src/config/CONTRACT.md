# config

contractVersion: 1.1.0

## Purpose

The window's own settings: the file format, every default, validated parse
and atomic write, plus the published layout defaults and clamps the view
starts from.

## Public surface

```rust
pub struct Config;           // every setting the window remembers: theme,
                             // opacity, text size, grid ratios, input rows,
                             // per-view state, the saved pane arrangement
                             // (`dock`, written by the window on every tab
                             // change, read back at launch); parse()
                             // tolerant, write() atomic via replace. `tuned`
                             // says whether any colour key was set
                             // explicitly (the palette is then the user's
                             // own); colour_of(key) reads one
pub fn own_palette(path);    // write every colour in hand into the file as a
                             // live line under its own key, values unchanged:
                             // what the panel's custom option does
pub const LEFT_WIDTH/TOP_HEIGHT/SETTINGS_RAIL: f32;   // fresh-window
                             // divider defaults; dragged values persist
pub const SPLIT_LOW/SPLIT_HIGH, RAIL_LOW/RAIL_HIGH: f32;  // drag clamps
pub const THEMES: [&str; 3]; // noob-matrix, noob-cool, noob-red
pub const RETIRED_THEMES;    // old names mapped to current ones on read
pub const TOOL_KEYS/GAUGE_KEYS;  // the per-view key vocabularies
```

## Invariants

1. Parse is tolerant: an unknown key is kept for round-tripping, a
   malformed value falls to its default, and a retired theme name maps to
   its current one; a settings file never takes the window down.
2. Writes are atomic (temp then replace): a concurrent reader sees old or
   new, never a mix.
3. Defaults live here and nowhere else: the view carries dragged values on
   its Shape, and a fresh window starts from these constants.

## Dependencies

None inside the window: this box owns its file format. The agent's
configuration is the agent-files box, deliberately separate.

## Tests

Inline: parse/write round trips, tolerance, clamps, theme mapping (48
tests).
