# commands

contractVersion: 1.0.0

## Purpose

The slash commands: every capability the settings panel exposes as a typed
`/command`, one registry the COMMANDS section lists and the prompt's
dispatcher parses, with `/help` over all of it.

## Public surface

```rust
pub const ALL: [Command; 30];    // the registry, in the order the section
                                 // lists it: one entry per command
pub struct Command;              // name, about (one line), help (the long
                                 // story), args, and what running it means
pub struct Arg;                  // one word of the usage line
pub fn spelled(&Command) -> String;      // "/set_theme <theme>, one of ..."
pub fn usage(&Command) -> String;        // the same with "usage: " in front
pub fn manual(&Command) -> Vec<String>;  // what /help <name> prints, and
                                 // what the section's doc column shows
pub fn is_command(text: &str) -> bool;   // it starts with /
pub fn dispatch(text: &str, agent: &Agent) -> Answer;
pub enum Answer {                // Do(Act): something for main to route;
                                 // Say(lines): /help, or nothing to do;
                                 // Refuse(why): the usage line on a bad
                                 // call, the nearest name on an unknown one
    Do(Act), Say(Vec<String>), Refuse(String),
}
pub enum Act {                   // what a command asks main for, with the
                                 // line to say once it lands
    Change { change, said },     // one setting, by the panel's own writers
    Deed { deed, said },         // one of the panel's deeds
    Install { source, said },    // a source the checker already approved
    Open { section, said },      // the panel, on a rail index or as it was
}
```

## Invariants

1. No I/O here: dispatch reads the `Agent` snapshot it is handed and answers
   with data; `main` routes every act through the same functions the panel's
   own presses use.
2. Complete against the panel's own definitions: every key on the settings
   tables, every theme, every colour key, every section and every deed and
   card button has a command, proven by tests that walk those definitions,
   so a capability added to the panel without a command fails the build.
3. A command's bounds are the panel's: values are validated against the same
   tables the sliders move in and spelled the way the file spells them, so
   nothing a command writes could not have been written from the panel.
4. A bad call answers with the command's own usage line and applies nothing;
   an unknown name answers with the nearest registered one and `/help`.
5. `/skill_install` keeps the panel's validate rule: only a source
   `install::check` approves becomes an install.
6. What cannot run headless (editing the system prompt, bulk session
   deletes) opens the right section instead, and its help says so.

## Dependencies

Contracts: the settings box (Change, Deed, SECTIONS, the two settings
tables), the agent-files box (the snapshot naming skills and servers), the
config box (themes, colour keys, the parser as colour authority), the
install box (the validate check).

## Tests

Inline: the registry's shape, the completeness sweeps (settings keys, agent
keys, themes, colour keys, sections, deeds, card buttons), parsing good, bad
and unknown, snapshot naming, the validate rule, `/help` whole and single,
the usage notes, and a theme and a transparency landing in a real file
through the panel's own commit (10 tests).
