# agent-files

contractVersion: 1.0.0

## Purpose

The agent's files, read and written the way the CLI does: the `.env` keys
the settings panel edits, AGENTS.md, installed skills and their
frontmatter, mcp.json entries, and the on/off conventions.

## Public surface

```rust
pub fn config_dir() -> Option<PathBuf>;   // the AGENT's rule: NOOB_CONFIG_DIR,
                                          // /config when present, ~/.config/noob
pub const ENDPOINT/API_KEY/MODEL/API_STYLE/REASONING/CTX/TASK_CONCURRENCY: &str;
pub const OWNED: [&str; 2];               // the keys the window edits freely
pub const AGENTS_MD: &str;  pub const AGENTS_CAP: u64;   // 16 KiB read cap
pub const OFF: &str;  pub const DISABLED: &str;          // on-disk toggles
pub fn is_secret(key: &str) -> bool;      // by name, wrong in the safe way
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

## Dependencies

Contracts: [`crates/noob/src/config/CONTRACT.md`](../../../../crates/noob/src/config/CONTRACT.md)
and [`crates/noob/src/skills/CONTRACT.md`](../../../../crates/noob/src/skills/CONTRACT.md)
(the conventions this box mirrors), the sessions box (list embedding).

## Tests

Inline: config-dir rule, secret naming, frontmatter reads, toggles (16
tests).
