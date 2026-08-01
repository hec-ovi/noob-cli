# sessions

contractVersion: 1.0.0

## Purpose

The saved sessions the first screen offers: rows read from JSONL heads,
each resolved to the folder it belongs to, plus the remembered-folder
index that survives restarts.

## Public surface

```rust
pub trait Folders {          // the port this box asks its folder questions
    fn list(&self, at: &Path) -> Result<Vec<String>, String>;
    fn is_folder(&self, at: &Path) -> bool;
}
pub struct Saved;            // one row: id, when, context, workspace
pub struct Context;          // tokens held / window, percent()
pub struct Listing;          // the rows plus any read trouble
pub fn dir() -> Option<PathBuf>;    // the agent's sessions directory
pub fn read(at: &Path, index: &Index, folders: &dyn Folders) -> Listing;
pub fn ago(when: SystemTime, now: SystemTime) -> String;
pub struct Index;            // id -> workspace (+context) memory, capped
pub const REMEMBERED: usize = 400;
```

## Invariants

1. This box owns `Folders`: consumers implement it (the real filesystem,
   or a test tree), so reading never reaches for `std::fs` directly and
   the picker depends on this box, never the reverse.
2. Only JSONL heads are read, never bodies: listing stays fast whatever a
   transcript grew to.
3. The index is capped at 400 and forgets oldest-first; a session still on
   screen cannot have lost its folder.

## Dependencies

Contracts: the CLI session box's file conventions (`<config>/sessions/`,
the meta line) as published in
[`crates/noob/src/session/CONTRACT.md`](../../../../crates/noob/src/session/CONTRACT.md).

## Tests

Inline: head parsing, index round trips and caps, ago wording (11 tests).
