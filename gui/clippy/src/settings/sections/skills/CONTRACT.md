# settings/sections/skills

contractVersion: 2.0.0

## Purpose

The SKILLS section of the settings panel: the install form with its
validate-then-install cycle, and under it what the agent can reach as one
table: the web search the CLI ships with, then every skill installed under
`skills/`.

## Public surface

```rust
pub const SKILL_SOURCE: &str;    // the install field's key; never a key of
                                 // any file. Re-exported at the settings root
pub const SKILL_COLUMNS: [(&str, usize, Align); 4];
                                 // skill, on, where it is, what it is for
pub struct SkillsSection;        // the section's own state: source, verdict,
                                 // install cycle
impl SkillsSection {
    pub fn new() -> SkillsSection;
    pub fn take_source(&mut self, typed: Option<String>) -> String;
    pub fn note_check(&mut self, source: String, verdict: Result<String, String>);
    pub fn checked_ok(&self) -> bool;   // validate approved what the field holds
    pub fn begin_install(&mut self, source: String);
    pub fn end_install(&mut self, source: String, answer: Result<String, String>);
    pub fn rows(&self, agent: &Agent) -> Vec<Row>;
}
```

## Invariants

1. No I/O beyond the PATH lookup behind the shipped web search: the list is
   the `Agent` snapshot, the cycle state is what the frame reported; the
   clone itself runs in `main`.
2. The install button exists only while `checked_ok`: the verdict is voided
   the moment the field says another source.
3. A failed install keeps the typed source on screen; a landed one clears
   the field, and the list still comes off the disk reading, never off what
   the install said.
4. The first row of the table is the web search the CLI ships with. Its `on`
   is `None`, so neither table button acts on it, and its cell reads yes
   exactly when the `websearch` program resolves (`NOOB_WEBSEARCH` overrides,
   including turning it off).
5. Every other row carries `on: Some(_)` and its directory as the id, which
   is what the turn and the uninstall name.
6. The form is the first row of the section and the table is under it: a card
   at the foot of a list of skills reads as a note about the last one rather
   than as the way to add another.
7. The source field is a link (`CardField::link`): typed into like any other
   field, drawn with the chain and a dotted rule instead of a box.

## Dependencies

The settings box's shared vocabulary (Row, Card, CardField, Table, Kept,
TableOf, Paper, Doing, Align); the agent-files box for the skills reading and
for `websearch_on`.

## Tests

Inline: the table off the disk, the install card and its cycle, the turn and
the two-press uninstall (6 tests), driven through the frame's `Settings`.
