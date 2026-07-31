# skills

contractVersion: 1.0.0

## Purpose

SKILL.md discovery and the L1 index (agentskills.io standard), plus install
and remove for the `/skills` command family. Level 2, the `skill` tool, lives
in the tools box; level 3 is a plain read of the skill's files.

## Public surface

```rust
pub struct Skill { pub name, pub description, pub dir, pub file }

pub fn discover(workspace: &Path, config_dir: &Path, extra_paths: &[PathBuf])
    -> Vec<Skill>;
    // Four roots in priority order (workspace .noob/skills, .claude/skills,
    // .agents/skills, then <config>/skills), alphabetical within a root,
    // first hit per name wins; then each extra path as ONE skill directory,
    // lowest priority, never recursed into

pub fn index(skills: &[Skill]) -> Option<String>;
    // the L1 prompt section: `- name: description` lines, descriptions
    // clipped to 200 chars, the section capped at 4000 chars; past the
    // budget skills get name-only lines, then a count note; None when empty
pub const INDEX_CHAR_BUDGET: usize = 4_000;
pub const INDEX_DESC_CLIP: usize = 200;
pub fn clip_description(desc: &str) -> String;   // one index-shaped line

pub fn install(workspace: &Path, source: &str) -> Result<String, String>;
    // a local skill dir or bare SKILL.md, a git URL, or an owner/repo
    // GitHub shorthand; validated before anything is written; returns the
    // installed name
pub fn remove(workspace: &Path, skill_dir: &Path) -> Result<(), String>;

pub fn body_of(text: &str) -> (Cow<'_, str>, usize);
    // the body with frontmatter stripped, byte-exact, lenient: a file whose
    // frontmatter no longer parses returns whole
```

Internal files: `frontmatter.rs` (the bounded fenced-metadata reader and
hand-rolled scanner: plain and quoted scalars, `|`/`>` blocks, field
validation), `install.rs` (staging, git clone with timeout, publish by
rename), `tests.rs`.

## Errors

`install` and `remove` fail with a `String` naming the reason: a malformed
skill's exact frontmatter problem, a name collision (with the remove
command to run), a git failure with the captured stderr, a timeout after
120 s, cancellation. Discovery never fails: a missing SKILL.md is silently
not a skill, an unreadable or unparseable one is skipped with a stderr
warning.

## Invariants

1. Install is atomic to discovery: the skill is staged as a hidden sibling
   (`.noob/.skill-*`, never a discovery candidate) and published with one
   rename, so discovery can never observe half a skill. Stale staging from
   dead installs is swept by the next install.
2. Nothing is written before the source validates; a failed install leaves
   no partial directory.
3. `remove` deletes only a directory DIRECTLY under `.noob/skills`; any
   other discovered skill is refused by name, because it can point at real
   project source.
4. Skill files cannot smuggle specials: symlinked directories are skipped,
   a symlinked/FIFO/device SKILL.md is rejected at the opened fd (no TOCTOU
   window), copies skip symlinks, and frontmatter reading stops at the
   closing fence or a 64 KiB cap, so the body is never loaded at discovery.
5. The `owner/repo` shorthand expands to GitHub only when nothing with that
   name exists locally; a real local path always wins.
6. The index is deterministic for a given skill list, and its budget holds
   whatever the descriptions do.

## Dependencies

Contracts: [`crates/noob-provider/CONTRACT.md`](../../../noob-provider/CONTRACT.md)
(the interrupt flag; cancellation aborts a copy or clone mid-flight).
Consumers pass extra paths from the config box's `skill_paths`.

## Tests

`tests.rs` in this folder: discovery priority and guards, index budgets, the
scanner's scalar and block forms, install/remove atomicity and refusals,
fd-level special-file rejection. Boundary: `crates/noob/tests/e2e_p3.rs`
(skills through the real binary), `crates/noob/tests/ui_commands.rs`
(`/skills`).
