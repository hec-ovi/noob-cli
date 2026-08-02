# agent-files

contractVersion: 1.3.0

## Purpose

The agent's files, read and written the way the CLI does: the `.env` keys
the settings panel edits, the two prompt files (AGENTS.md and TOOLS.md)
with the CLI's shipped defaults behind them, installed skills and their
frontmatter, mcp.json entries, and the on/off conventions.

## Public surface

```rust
pub fn config_dir() -> Option<PathBuf>;   // the AGENT's rule: NOOB_CONFIG_DIR,
                                          // /config when present, ~/.config/noob
pub const ENDPOINT/API_KEY/MODEL/API_STYLE/REASONING/CTX/MAX_ROUNDS/
          TASK_CONCURRENCY/TASK_MAX_TURNS/TASK_TOOLS/TASK_WALL_CLOCK: &str;
pub const OWNED: [&str; 8];               // the keys the window edits freely
pub const AGENTS_MD/TOOLS_MD: &str;  pub const AGENTS_CAP: u64;  // 16 KiB cap
pub const AGENTS_DEFAULT/TOOLS_DEFAULT: &str;  // the CLI's shipped texts,
                                          // included from its own sources
pub const CTX_STOPS/TASK_CONCURRENCY_STOPS/ROUNDS_STOPS/WALL_CLOCK_STOPS:
          [f32; _];                       // slider detents; every budget's
                                          // low end is the CLI's 0, no limit
pub const TASK_TOOLS_CHOICES: [&str; 3];  // read-only | web | all
pub const OFF: &str;  pub const DISABLED: &str;          // on-disk toggles
pub fn is_secret(key: &str) -> bool;      // by name, wrong in the safe way
pub fn read_tools(dir) -> Instructions;   // TOOLS.md, read like AGENTS.md
pub fn write_instructions(path: &Path, text: &str) -> Result<(), String>;
                                          // one prompt file whole, by the
                                          // same atomic rename as every
                                          // write here
pub fn restore_prompt(path: &Path, default: &str) -> Result<(), String>;
                                          // park the file in <name>.bak
                                          // beside it (skipped when there is
                                          // no file), then write the default
pub fn bak_path(path: &Path) -> PathBuf;  // that .bak's name
pub fn load_md(path: &Path) -> Result<Vec<String>, String>;
                                          // a named .md as editor lines,
                                          // refused past the cap; no writes
pub const WEBSEARCH_PROGRAM/WEBSEARCH_OVERRIDE: &str;
pub fn websearch_on() -> bool;            // whether the CLI would register
                                          // its websearch tool: the program
                                          // on PATH, or what the override
                                          // names (which can turn it off)
// plus the read/toggle/remove operations the settings panel calls
```

## Invariants

1. Secrets never render: `is_secret` errs toward calling something a
   secret, and the panel then shows set/unset, never the value.
2. This box mirrors the CLI's conventions and says so; it never invents
   its own file shapes. The window's OWN settings are the config box,
   deliberately separate (the two rules must be free to differ).
3. Toggling a skill or server is an on-disk rename to the `.off`/
   `disabled` convention, reversible, never a deletion.
4. `websearch_on` reads the same two things the CLI reads and nothing else,
   so the panel's row cannot claim a tool the agent does not have.

## Dependencies

Contracts: [`crates/noob/src/config/CONTRACT.md`](../../../../crates/noob/src/config/CONTRACT.md)
and [`crates/noob/src/skills/CONTRACT.md`](../../../../crates/noob/src/skills/CONTRACT.md)
(the conventions this box mirrors), the sessions box (list embedding).

## Tests

Inline: config-dir rule, secret naming, frontmatter reads, toggles, the
whole-file prompt writes, the restore with its bak, the load (19 tests).
