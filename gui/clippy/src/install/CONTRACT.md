# install

contractVersion: 1.0.0

## Purpose

Installing a skill from the settings window: the same sources the CLI
accepts (a local path, a git URL, an owner/repo shorthand), staged and
published atomically, reporting progress the panel can render.

## Public surface

The install job the settings controller starts on its thread: source in,
progress lines and a terminal result out.

## Invariants

1. The CLI's skills box owns the rules: what a valid source is, where a
   skill lands, and the staged-rename publish are the conventions of the
   skills contract; this box mirrors them and says so.
2. A failed or canceled install leaves no partial skill.

## Dependencies

Contracts: [`crates/noob/src/skills/CONTRACT.md`](../../../../crates/noob/src/skills/CONTRACT.md)
(the conventions), the agent-files box (where the config lives).

## Tests

Inline: source parsing, staging, failure cleanup against scratch dirs.
