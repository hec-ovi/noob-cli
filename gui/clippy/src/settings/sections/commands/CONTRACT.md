# settings/sections/commands

contractVersion: 1.0.0

## Purpose

The COMMANDS section of the settings panel: the slash commands as a
read-only list, one row per command, with the doc column showing the manual
of the one under the cursor.

## Public surface

```rust
pub fn rows() -> Vec<Row>        // a note saying where these are typed,
                                 // then one fixed entry per command
```

## Invariants

1. The command registry (`crate::commands::ALL`) is the single source: the
   rows are it, in its order, each carrying the registry's own line, usage
   and manual. Nothing here is written by hand.
2. Read-only: every entry is `Which::Fixed`, so no toggle, no uninstall and
   no deed ever comes off this section.

## Dependencies

The settings box's shared vocabulary (Row, Entry, Which); the commands box
for the registry and its texts.

## Tests

Inline: the list is the registry one to one, and the rows answer no press
(2 tests), driven through the frame's `Settings`.
