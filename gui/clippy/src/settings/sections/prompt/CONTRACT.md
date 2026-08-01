# settings/sections/prompt

contractVersion: 1.1.0

## Purpose

The SYSTEM PROMPT section of the settings panel: the agent's global
AGENTS.md, the first layer of every prompt, as one document with the file's
path as a reading over it, an offer to write a starter file when there is
none, and an editor that changes the file in place.

## Public surface

```rust
pub struct PromptSection;        // the section's own state: the document's
                                 // editor (lines, caret, follow-the-caret
                                 // scroll) while it is open. The frame embeds
                                 // it and routes keys to it through its own
                                 // edit_instructions/type_instructions/...
                                 // delegation; the save is a Deed done in
                                 // main through the agent-files box
pub fn PromptSection::rows(&self, agent: &Agent) -> Vec<Row>
                                 // THE FILE card naming the path, then the
                                 // document as a Paper: the file's text, or
                                 // the buffer while the editor is open
pub fn PromptSection::editing(&self) -> bool
```

## Invariants

1. No I/O here: rows come from the `Agent` snapshot's `Instructions` and the
   editor's own buffer; the save is the whole file at once, written in
   `main` through `agent::write_instructions`.
2. Missing and whitespace-only files are one thing, because they are one
   thing to the agent: both show the offer, and the offer carries the path
   the agent would read.
3. While the editor is open the block shows the buffer with the caret kept
   on screen, and the file has not changed; abandoning the edit (Escape)
   restores the block to the file's own text.
4. A file longer than the CLI's 16 KiB cap shows exactly what the model
   gets, with a line saying the file goes further, and refuses the editor:
   saving the capped text would lose the tail.
5. No config directory is said as trouble, never offered and never edited.

## Dependencies

The settings box's shared vocabulary (Row, Card, CardField, Paper,
PAPER_LINES); the agent-files box (`crate::agent`) for the `Instructions`
snapshot, the starter write and the whole-file write.

## Tests

Inline: the rail order, the document with its path, the offer round trip,
the edit-and-save round trip, the caret-following scroll, the abandoned
edit, the refused write (6 tests), driven through the frame's `Settings`.
Scene-level drawing is asserted by the view box's rendered-scene tests.
