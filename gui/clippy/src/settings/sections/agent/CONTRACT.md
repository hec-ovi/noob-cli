# settings/sections/agent

contractVersion: 1.1.0

## Purpose

The AGENT section of the settings panel: what the CLI is pointed at, as
cards over the agent's own `.env`, with the whole assembled prompt as one
block under them. The global AGENTS.md is the prompt section's own document
([`sections/prompt`](../prompt/CONTRACT.md)).

## Public surface

```rust
pub fn rows(agent: &Agent, prompt: &Assembled) -> Vec<Row>
                                 // the section's rows, built fresh from the
                                 // snapshot and the prompt state handed in
pub(crate) const AGENT_SETTINGS: [(&str, Kind); 2]
                                 // NOOB_CTX and NOOB_TASK_CONCURRENCY as
                                 // tracks, bounds read off the CLI; the frame's
                                 // slider test walks them
```

## Invariants

1. Pure: rows are built from the `Agent` snapshot and the `Assembled` prompt
   and nothing else; no I/O here.
2. A credential is reported as set and never shown; a missing key reads as
   the frame's UNSET.
3. Every key the file carries is on a card above the prompt block: known
   keys by their plain-words field, the rest on THE REST OF THE FILE.
4. The two numbers with CLI bounds are tracks; unset, they show the CLI's
   default and the card says so.

## Dependencies

The settings box's shared vocabulary (Row, Card, CardField, Paper, Kind,
File, Assembled, SECRET, UNSET); the agent-files box (`crate::agent`) for
the snapshot, the key names and the bounds.

## Tests

Inline: the section's cards, the walkable keyboard, the prompt block, the
defaults, and the failed-prompt reason (7 tests), driven through the
frame's `Settings`.
