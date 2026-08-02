# settings/sections/agent

contractVersion: 3.0.0

## Purpose

The AGENT section of the settings panel: what the CLI is pointed at, as
cards over the agent's own `.env`. The prompt's own files are the prompt
section's documents ([`sections/prompt`](../prompt/CONTRACT.md)).

## Public surface

```rust
pub fn rows(agent: &Agent, health: Option<&str>, show_key: bool) -> Vec<Row>
                                 // the section's rows, built fresh from the
                                 // snapshot handed in. `health` is what the
                                 // last connection check answered and
                                 // `show_key` whether the credential is
                                 // uncovered; both are the frame's state
pub(crate) const AGENT_SETTINGS: [(&str, Kind); 8]
                                 // every control of the agent's file:
                                 // NOOB_API_STYLE and NOOB_REASONING
                                 // (choices), NOOB_CTX, NOOB_MAX_ROUNDS,
                                 // NOOB_TASK_CONCURRENCY,
                                 // NOOB_TASK_MAX_TURNS,
                                 // NOOB_TASK_WALL_CLOCK_S (tracks),
                                 // NOOB_TASK_TOOLS (choice); bounds and
                                 // detents read off the CLI box
```

## The cards

CONNECTION (endpoint, api style, what the last check answered; its button
writes what is typed and asks), BACK TO THE DEFAULT ENDPOINT (one button,
writing `crate::agent::ENDPOINT_DEFAULT`), CREDENTIAL (the key as dots, its
button shows it), MODEL (model, reasoning), LIMITS, MULTI-AGENT,
MULTI-AGENT BUDGETS, THE SETTINGS FILE, and THE REST OF THE FILE when the
file carries keys this window has no control for.

## Invariants

1. Pure: rows are built from the `Agent` snapshot, the health line and the
   reveal flag, and nothing else; no I/O here.
2. A field's label is the plain words with the key after them, `rounds per
   input (NOOB_MAX_ROUNDS)`. The line under it carries only what neither
   says: that nobody has set it, and what its values mean.
3. A credential is dots until the card's own button is pressed, and the
   value again as soon as it is pressed a second time. Nothing about that
   is remembered: a panel opens covered.
4. Every key the file carries is on a card: known keys by their own field,
   the rest on THE REST OF THE FILE.
5. Everything the CLI accepts a value for is set from here. A number with
   CLI bounds is a track showing the CLI's default until it is written; a
   choice with no default reads UNSET until it is.
6. The connection card reports what it was told and never a verdict of its
   own: before any check it says so.
7. The endpoint's way back writes the address the CLI's autodetect would
   have found first, llama.cpp's own port, and the card says which address
   that is rather than leaving it to be discovered.

## Dependencies

The settings box's shared vocabulary (Row, Card, CardField, Doing, Kind,
File, SECRET, UNSET); the agent-files box (`crate::agent`) for the
snapshot, the key names, the bounds and the detents. The health line comes
from the link box's `noob doctor` reader, through the frame.

## Tests

Inline: the section's cards, the credential's two states, the health line,
the walkable keyboard, the tracks and their defaults, the magnetic
checkpoints, the way back to the default endpoint, and every key kept on a
card (7 tests), driven through the frame's `Settings`.
