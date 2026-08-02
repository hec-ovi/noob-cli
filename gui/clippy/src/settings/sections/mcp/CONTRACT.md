# settings/sections/mcp

contractVersion: 1.0.0

## Purpose

The MCP section of the settings panel: the configured servers out of the two
files the CLI merges, and the ADD A SERVER card whose deed writes the global
one.

## Public surface

```rust
pub const SERVER_NAME: &str;     // the add card's two field keys; keys of
pub const SERVER_HOW: &str;      // the model, never of any file.
                                 // Re-exported at the settings root
pub struct McpSection;           // the section's own state: the two fields
impl McpSection {
    pub fn keep_edit(&mut self, key: &str, typed: String);  // store trimmed
    pub fn add_deed(&self) -> Result<Deed, String>;  // the AddServer deed,
                                                     // or the refusal to say
    pub fn clear(&mut self);     // after the deed landed
    pub fn rows(&self, agent: &Agent) -> Vec<Row>;
}
pub fn file(agent: &Agent, project: bool) -> Option<&Path>
                                 // which mcp.json a server belongs to
```

## Invariants

1. No I/O: the list is the `Agent` snapshot; the deed built here is done by
   `main` through the agent-files box.
2. An empty name or command is a refusal with its reason, never a deed; on a
   refusal the fields keep what was typed.
3. Every server row is removable and carries its file entry as the doc
   column, fenced as JSON.
4. A broken file is a bad note on the section; the file that parsed still
   lists.

## Dependencies

The settings box's shared vocabulary (Row, Card, CardField, Entry, Deed,
Doing, Which); the agent-files box for the merged reading and the Server
shape.

## Tests

Inline: the broken-file note, the toggle in its own file, the two-press
uninstall on and off, and the refused removal (6 tests), driven through the
frame's `Settings`.
