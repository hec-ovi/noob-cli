# settings

contractVersion: 2.5.0

## Purpose

The settings panel: a frame (the rail, the cursor and scroll machinery, the
shared row vocabulary, the write-back) over seven nested section boxes, one
folder each under `sections/`, plus where everything sits (places.rs) and
what gets drawn (paint.rs). Placement, paint, and hit regions read the same
rectangles.

## Nested boxes

One folder, one contract each; a design change to one section happens inside
its folder alone:

- [`sections/agent`](sections/agent/CONTRACT.md) - the CLI's file as cards.
- [`sections/prompt`](sections/prompt/CONTRACT.md) - the prompt's three
  layers: AGENTS.md and TOOLS.md edited behind an edition checkbox, the
  environment block read out.
- [`sections/sessions`](sections/sessions/CONTRACT.md) - the saved
  conversations table, its columns and cells.
- [`sections/skills`](sections/skills/CONTRACT.md) - the installed list and
  the validate-then-install cycle.
- [`sections/mcp`](sections/mcp/CONTRACT.md) - the configured servers and
  the add-a-server card.
- [`sections/commands`](sections/commands/CONTRACT.md) - the slash commands
  as a read-only list off the command registry.
- [`sections/appearance`](sections/appearance/CONTRACT.md) - the window's
  looks, the palette, the restore.

## Public surface

```rust
pub const SECTIONS: [&str; 7];   // AGENT, SYSTEM PROMPT, SESSIONS, SKILLS,
                                 // MCP, COMMANDS, APPEARANCE: the rail, in
                                 // rail order. Contract data; the menu
                                 // consumes it
pub struct Settings;             // the panel state machine: rows per
                                 // section, cursor and side, field editing,
                                 // sliders, swatches, the sessions table,
                                 // the doc viewer, footer text, and the
                                 // Change/Deed a commit writes. Embeds the
                                 // section boxes' state and delegates
pub enum Row;  pub enum Side;  pub enum Doing;   // what a row is, which
                                 // half is active, what a button does
pub mod places;                  // SettingsPlaces: every rectangle on the
                                 // panel from one Panel + Shape + &Settings
pub mod paint;                   // settings_panel(scene, frame)
```

Section vocabulary the rest of the window reads keeps its `settings::` path
by re-export: `SESSION_COLUMNS` and the table constants, `SKILL_SOURCE`,
`SERVER_NAME`, `SERVER_HOW`, `restoring`. The two settings tables and the
preset key (`LOOKS`, `AGENT_SETTINGS`, `THEME`) are re-exported crate-wide
for the command registry, whose bounds are the panel's own. An `Entry` can
be `Which::Fixed`: only read, no toggle, no uninstall, no deed.

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
5. A section box produces `Vec<Row>` from the frame's shared vocabulary and
   holds only its own state; the cursor, the scroll, the editing buffer and
   every write stay in the frame. The one multiline buffer, the system
   prompt's document editor, is that section's own state: the frame routes
   keys to it and its save is still a Deed done in `main`.
6. The list scrolls by rows of text, not by rows of the list: a wheel notch
   slides a few of them, a card can stand partly past either edge and is
   drawn cut, and a control is drawn and pressable only while it is wholly
   on screen. Pointer presses never scroll the list; only keyboard movement
   reveals the cursor.
7. A pressed swatch is edited through the frame's own line: the press opens
   the editing buffer on the colour's current hex value, Enter asks the
   config parser and commits the value under the swatch's key into the
   window file, and a value the parser refuses is said on the footer with
   nothing written. Escape or any cursor movement lets the press go.
8. A prompt document is edited behind its enable-edition checkbox: ticking
   it opens the editor on the file's text (the shipped default when there is
   none), the block shows the buffer with a caret while it is open, nothing
   lands until Ctrl-S or the save button writes the whole file through the
   agent-files box, and Escape or the checkbox drops the buffer. The restore
   parks the file in the `.bak` beside it and writes the shipped default,
   armed on the first press; the load reads a named `.md` into the editor
   and writes nothing. A failed write keeps the buffer, with the reason on
   the footer.

## Dependencies

Contracts: the view box (Frame, Shape, chrome vocabulary), the config box
(reads and writes), the agent-files box (env keys, skills, mcp), the
sessions box (the table rows), the design box (scales, icons), the style
box (colors), the state box (the doc pane type).

A slider's `Kind::Number` can carry detents (`stops`): while a drag passes
within a small window of one the value snaps to it, each detent is a tick
drawn on the track, and the keyboard nudge keeps stepping by the plain step.

## Tests

70 model tests drive key- and click-shaped calls with scratch files: the
frame's 28 (cursor, rail, scroll, sliders, doc viewer, footer) in mod.rs,
each section's in its own box. Scene-level placement and paint are asserted
by the view box's rendered-scene tests.
