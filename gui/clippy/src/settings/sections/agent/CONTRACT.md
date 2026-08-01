# settings/sections/agent

contractVersion: 2.0.0

## Purpose

The AGENT section of the settings panel: what the CLI is pointed at, as
cards over the agent's own `.env`. The prompt's own files are the prompt
section's documents ([`sections/prompt`](../prompt/CONTRACT.md)).

## Public surface

```rust
pub fn rows(agent: &Agent) -> Vec<Row>
                                 // the section's rows, built fresh from the
                                 // snapshot handed in
pub(crate) const AGENT_SETTINGS: [(&str, Kind); 2]
                                 // NOOB_CTX and NOOB_TASK_CONCURRENCY as
                                 // tracks, bounds and detents read off the
                                 // CLI box (64k/128k/256k and 3/5); the
                                 // frame's slider test walks them
```

## Invariants

1. Pure: rows are built from the `Agent` snapshot and nothing else; no I/O
   here.
2. A credential is reported as set and never shown; a missing key reads as
   the frame's UNSET.
3. Every key the file carries is on a card: known keys by their plain-words
   field, the rest on THE REST OF THE FILE.
4. The two numbers with CLI bounds are tracks; unset, they show the CLI's
   default and the card says so.

## Dependencies

The settings box's shared vocabulary (Row, Card, CardField, Kind, File,
SECRET, UNSET); the agent-files box (`crate::agent`) for the snapshot, the
key names, the bounds and the detents.

## Tests

Inline: the section's cards, the walkable keyboard, the tracks and their
defaults, the magnetic checkpoints, and every key kept on a card (6 tests),
driven through the frame's `Settings`.
