# settings/sections/prompt

contractVersion: 2.0.0

## Purpose

The SYSTEM PROMPT section of the settings panel: the prompt as the three
layers the CLI assembles it from, stacked in order. AGENTS.md and TOOLS.md
are documents edited in place behind an enable-edition checkbox, each with
its shipped default shown honestly when the file is absent; the environment
block is what `noob debug env` printed, read and never edited; a line under
them names the assembly order.

## Public surface

```rust
pub struct PromptSection;        // the section's own state: the document
                                 // editor (lines, caret, follow-the-caret
                                 // scroll) for whichever file has edition
                                 // enabled, one at a time. The frame embeds
                                 // it and routes keys through its own
                                 // instructions_* delegation; every save and
                                 // restore is a Deed done in main through
                                 // the agent-files box
pub enum PromptFile;             // Agents | Tools: which file a row or the
                                 // open editor is about
pub fn PromptSection::rows(&self, agent: &Agent, env: &EnvBlock) -> Vec<Row>
                                 // the three blocks in assembly order, then
                                 // the note naming that order. Each file
                                 // block carries PaperActs (the footer);
                                 // only AGENTS.md offers the load
pub fn PromptSection::editing(&self) -> bool
```

The frame turns a block into a pane for selection and copying
(`Settings::paper_pane`, `paper_pane_at`), since a drag is the window's
business and not the section's.

## Invariants

1. No I/O here: rows come from the `Agent` snapshot, the shipped default
   constants and the editor's own buffer; every write is a Deed done in
   `main` through the agent-files box.
2. Missing and whitespace-only files are one thing, because they are one
   thing to the agent: both show the shipped default under a note naming
   the path and saying the text is not written yet.
3. Edition gates the editor: typing does nothing until the checkbox is
   ticked, ticking opens the buffer on the file's text (the default when
   there is none), and Escape or unticking drops the buffer with the file
   untouched. One editor at a time; ticking one file drops the other's.
   Every button the block has stands in its footer either way, drawn dim
   and doing nothing while edition is off.
4. Each block saves its own file whole; a file past the CLI's 16 KiB cap
   refuses edition, since saving the capped text would lose the tail.
5. TOOLS.md says on the block itself what editing it costs: it is what the
   model knows its tools from.
6. The environment block never offers edition: it is computed by the CLI
   for each request, and its `under` line says so.
7. No config directory is said as trouble, never offered and never edited.
8. A block's text is selected with the pointer and copied like any other
   text in the window: the drag resolves against the block's own lines at
   the scroll it is read at.

## Dependencies

The settings box's shared vocabulary (Row, Paper, PaperActs, EnvBlock,
PAPER_LINES); the agent-files box (`crate::agent`) for the two `Instructions`
snapshots, the shipped defaults, and the whole-file writes.

## Tests

Inline: the three blocks in order with the env answer and its failure, the
default shown and owned by saving, the edition gate, the two independent
saves, the armed restore with its bak, the load into the editor, the
read-only environment, the caret-following scroll, and the refused write
with the cap (9 tests), driven through the frame's `Settings`.
