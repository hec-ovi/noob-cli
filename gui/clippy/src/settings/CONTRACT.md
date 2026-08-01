# settings

contractVersion: 1.0.0

## Purpose

The settings panel as one box: the panel model and its write-back rows
(mod.rs), where everything sits (places.rs), and what gets drawn
(paint.rs). Placement, paint, and hit regions read the same rectangles.

## Public surface

```rust
pub const SECTIONS: [&str; 5];   // AGENT, SESSIONS, SKILLS, MCP,
                                 // APPEARANCE: the rail, in rail order.
                                 // Contract data; the menu consumes it
pub struct Settings;             // the panel state machine: rows per
                                 // section, cursor and side, field editing,
                                 // sliders, swatches, the sessions table,
                                 // the doc viewer, footer text, and the
                                 // Change/Deed a commit writes
pub enum Row;  pub enum Side;  pub enum Doing;   // what a row is, which
                                 // half is active, what a button does
pub mod places;                  // SettingsPlaces: every rectangle on the
                                 // panel from one Panel + Shape + &Settings
pub mod paint;                   // settings_panel(scene, frame)
```

## Invariants

1. One geometry: the same `places` feed the painter and the layout's hit
   tests, so a click can never land on something drawn elsewhere.
2. The model owns no I/O except its declared write-back: committing a
   Change goes through the config and agent-files boxes; everything else
   is pure state over what those boxes reported.
3. `SECTIONS` order is stability API: the rail, the menu's settings group,
   and section addressing all index by it.
4. The doc viewer is a read-only pane reusing the transcript pane type;
   selecting text in it follows the select box's rules.

## Dependencies

Contracts: the view box (Frame, Shape, chrome vocabulary), the config box
(reads and writes), the agent-files box (env keys, skills, mcp), the
sessions box (the table rows), the design box (scales, icons), the style
box (colors), the state box (the doc pane type).

## Tests

The model's ~57 tests live in mod.rs and drive it through key- and
click-shaped calls with scratch files. Scene-level placement and paint are
asserted by the view box's rendered-scene tests.
