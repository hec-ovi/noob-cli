# settings/sections/sessions

contractVersion: 1.1.0

## Purpose

The SESSIONS section of the settings panel: the saved conversations as one
table inside one card, plus the folder they live in and any transcript that
could not be read.

## Public surface

```rust
pub fn rows(agent: &Agent) -> Vec<Row>
                                 // the folder card, the table, one bad note
                                 // per skipped transcript
pub enum Align;                  // Left | Right: which edge a cell is
                                 // written against
pub const SESSION_COLUMNS: [(&str, usize, Align); 6];
pub const SESSION_CELLS: usize;  // columns that carry text (all but the mark)
pub const TABLE_ROWS: usize;     // conversations on screen inside the body
pub fn table_body_lines() -> f32 // the body's height in lines
pub const SESSION_TITLE: &str;   // what the panel's heading calls the section
```

All of it is re-exported at the settings root, so callers keep one
`settings::` path.

## Invariants

1. Pure: rows come from the `Agent` snapshot's session listing; no I/O here.
2. The cells say what the picker's session rows say, formatted by the
   picker's own helpers, so the two lists read as the same sessions.
3. Each `Kept` carries the transcript id off the reader, so a delete names
   what the reader read, never a path parsed off the screen. It carries no
   document: the column beside a list is for the tables whose rows have one.
4. A table with no rows is never built; an empty listing is the folder card
   saying so.

## Dependencies

The settings box's shared vocabulary (Row, Card, CardField, Table, Kept);
the agent-files box for the listing; the sessions box for `dir()` and
`ago()`; the picker box for the size and context labels.

## Tests

Inline: the picker match, arrow keys inside the table, marks and the
armed delete (6 tests), driven through the frame's `Settings`.
