# settings/sections/prompt

contractVersion: 1.0.0

## Purpose

The SYSTEM PROMPT section of the settings panel: the agent's global
AGENTS.md, the first layer of every prompt, as one document with the file's
path as a reading over it and an offer to write a starter file when there is
none.

## Public surface

```rust
pub fn rows(agent: &Agent) -> Vec<Row>
                                 // THE FILE card naming the path, then the
                                 // document as a Paper the page keys read
```

## Invariants

1. Pure: rows come from the `Agent` snapshot's `Instructions`; no I/O here.
2. Missing and whitespace-only files are one thing, because they are one
   thing to the agent: both show the offer, and the offer carries the path
   the agent would read.
3. A file longer than the CLI's 16 KiB cap shows exactly what the model
   gets, with a line saying the file goes further.
4. No config directory is said as trouble, never offered.

## Dependencies

The settings box's shared vocabulary (Row, Card, CardField, Paper); the
agent-files box (`crate::agent`) for the `Instructions` snapshot and the
starter write the offer's press lands in.

## Tests

Inline: the rail order, the document with its path, the offer round trip
(2 tests), driven through the frame's `Settings`. Scene-level drawing is
asserted by the view box's rendered-scene tests.
