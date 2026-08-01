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
pub(crate) const AGENT_SETTINGS: [(&str, Kind); 6]
                                 // the six controls of the agent's file:
                                 // NOOB_CTX, NOOB_MAX_ROUNDS,
                                 // NOOB_TASK_CONCURRENCY,
                                 // NOOB_TASK_MAX_TURNS (tracks),
                                 // NOOB_TASK_TOOLS (choice),
                                 // NOOB_TASK_WALL_CLOCK_S (track); bounds
                                 // and detents read off the CLI box; the
                                 // frame's slider test walks them
```

## Invariants

1. Pure: rows are built from the `Agent` snapshot and nothing else; no I/O
   here.
2. A credential is reported as set and never shown; a missing key reads as
   the frame's UNSET.
3. Every key the file carries is on a card: known keys by their plain-words
   field, the rest on THE REST OF THE FILE. The fleet's four live on the
   MULTI-AGENT and MULTI-AGENT BUDGETS cards; a round or clock budget of 0
   reads as the CLI's "no limit" and is the default.
4. Every number with CLI bounds is a track; unset, it shows the CLI's
   default and the card says so.

## Dependencies

The settings box's shared vocabulary (Row, Card, CardField, Kind, File,
SECRET, UNSET); the agent-files box (`crate::agent`) for the snapshot, the
key names, the bounds and the detents.

## Tests

Inline: the section's cards, the walkable keyboard, the tracks and their
defaults, the magnetic checkpoints, and every key kept on a card (6 tests),
driven through the frame's `Settings`.
