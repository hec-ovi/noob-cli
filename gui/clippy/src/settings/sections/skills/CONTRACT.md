# settings/sections/skills

contractVersion: 1.0.0

## Purpose

The SKILLS section of the settings panel: the install form with its
validate-then-install cycle, and under it the installed skills as entry
rows behind a line saying how many there are.

## Public surface

```rust
pub const SKILL_SOURCE: &str;    // the install field's key; never a key of
                                 // any file. Re-exported at the settings root
pub const WEBSEARCH_SUGGESTION: &str;
                                 // the owner/name the fresh field suggests
pub struct SkillsSection;        // the section's own state: source, verdict,
                                 // install cycle
impl SkillsSection {
    pub fn new(skills: &[Skill]) -> SkillsSection;  // suggestion decided here
    pub fn take_source(&mut self, typed: Option<String>) -> String;
    pub fn note_check(&mut self, source: String, verdict: Result<String, String>);
    pub fn checked_ok(&self) -> bool;   // validate approved what the field holds
    pub fn begin_install(&mut self, source: String);
    pub fn end_install(&mut self, source: String, answer: Result<String, String>);
    pub fn rows(&self, agent: &Agent) -> Vec<Row>;
}
```

## Invariants

1. No I/O: the list is the `Agent` snapshot, the cycle state is what the
   frame reported; the clone itself runs in `main`.
2. The install button exists only while `checked_ok`: the verdict is voided
   the moment the field says another source.
3. A failed install keeps the typed source on screen; a landed one clears
   the field, and the list still comes off the disk reading, never off what
   the install said.
4. The suggestion appears only while no `web-search` skill is installed.
5. The form is the first row of the section and the list is under it: a card
   at the foot of a list of skills reads as a note about the last one rather
   than as the way to add another.

## Dependencies

The settings box's shared vocabulary (Row, Card, CardField, Entry, Paper,
Doing, Which); the agent-files box for the skills reading.

## Tests

Inline: the list off the disk, the install card and its cycle, the toggle
and the two-press uninstall (4 tests), driven through the frame's
`Settings`.
