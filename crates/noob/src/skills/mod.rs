//! SKILL.md discovery and the L1 index (agentskills.io standard). Four
//! discovery roots at session start plus any explicitly configured resolver
//! paths (NOOB_SKILL_PATHS), first hit per name wins; a hand-rolled
//! frontmatter scanner (plain scalars, quoted strings, `|`/`>` blocks);
//! malformed skills are skipped with a stderr warning, never a crash.
//! Level 2 (the `skill` tool) lives in tools/skill.rs; level 3 is plain read.

use std::path::{Path, PathBuf};

mod frontmatter;
mod install;
#[cfg(test)]
mod tests;

pub use frontmatter::body_of;
pub use install::{install, install_into, remove};

/// Index section budget in chars (~1,000 tokens at chars/4).
pub const INDEX_CHAR_BUDGET: usize = 4_000;
/// Per-skill description clip in the index, in chars.
pub const INDEX_DESC_CLIP: usize = 200;

#[derive(Clone, Debug)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// The skill's directory (bundled files live here).
    pub dir: PathBuf,
    /// `dir/SKILL.md`.
    pub file: PathBuf,
}

/// Discovery at session start. First hit per NAME wins across the four
/// default roots in priority order (alphabetical within a root so the result
/// is deterministic), then across the explicitly configured resolver paths in
/// the order they were listed. `extra_paths` are additional and lowest
/// priority: a default-root skill wins a name clash against a configured one.
/// A directory without SKILL.md is silently not a skill; a SKILL.md that fails
/// to parse is skipped with a stderr warning.
///
/// Each `extra_paths` entry is treated as ONE skill directory, not a root:
/// discovery registers a single Skill whose `dir` is that path and never
/// recurses into it, so a dispatcher like `cli/SKILL.md` (whose body routes to
/// `cli/skills/*` sub-skills via `read`) is indexed once and its sub-skills
/// are not separately surfaced. The same guards as the default roots apply: a
/// symlinked directory is skipped, and a symlinked/FIFO/special SKILL.md is
/// rejected rather than opened.
pub fn discover(workspace: &Path, config_dir: &Path, extra_paths: &[PathBuf]) -> Vec<Skill> {
    let roots = [
        workspace.join(".noob/skills"),
        workspace.join(".claude/skills"),
        workspace.join(".agents/skills"),
        config_dir.join("skills"),
    ];
    let mut out: Vec<Skill> = Vec::new();
    for root in &roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                entry.file_type().ok()?.is_dir().then(|| entry.path())
            })
            .collect();
        dirs.sort();
        for dir in dirs {
            register_skill(dir, &mut out);
        }
    }
    // Explicitly configured resolver paths, in listed order, after the default
    // roots. Each entry is a single skill directory; a symlinked directory is
    // skipped, mirroring the symlink-aware `is_dir()` guard the roots get from
    // read_dir's non-following file type.
    for dir in extra_paths {
        match std::fs::symlink_metadata(dir) {
            Ok(meta) if meta.file_type().is_dir() => register_skill(dir.clone(), &mut out),
            _ => continue,
        }
    }
    out
}

/// Register the skill at `dir` (from `dir/SKILL.md`) into `out`, or skip it:
/// silently if there is no SKILL.md, with a stderr warning if the file is
/// present but unreadable/unparseable, and not at all if a skill of the same
/// name was already registered by a higher-priority location (first wins).
fn register_skill(dir: PathBuf, out: &mut Vec<Skill>) {
    let file = dir.join("SKILL.md");
    let text = match frontmatter::read_frontmatter_file(&file) {
        Ok(t) => t,
        // No SKILL.md: not a skill dir, and not worth a warning.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        // Present but unreadable (permissions, invalid UTF-8, a symlink or
        // FIFO in place of a regular file): the mandated stderr warning, not a
        // silent disappearance.
        Err(e) => {
            eprintln!("noob: skipping skill {}: cannot read: {e}", file.display());
            return;
        }
    };
    match frontmatter::parse(&text).and_then(|p| frontmatter::validate(&p.fields)) {
        Ok((name, description)) => {
            if out.iter().any(|s| s.name == name) {
                return; // shadowed by a higher-priority location
            }
            out.push(Skill {
                name,
                description,
                dir,
                file,
            });
        }
        Err(reason) => {
            eprintln!("noob: skipping skill {}: {reason}", file.display());
        }
    }
}

/// The L1 index: one `- name: description` line per skill (description
/// clipped), the whole section capped; skills past the budget get name-only
/// lines, then a count note. None when no skills exist (and then the skill
/// tool is not registered either).
pub fn index(skills: &[Skill]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut name_only = false;
    let mut hidden = 0usize;
    for s in skills {
        let mut candidate = if name_only {
            format!("- {}", s.name)
        } else {
            format!("- {}: {}", s.name, clip_one_line(&s.description))
        };
        if !name_only && used + candidate.len() + 1 > INDEX_CHAR_BUDGET {
            name_only = true;
            candidate = format!("- {}", s.name);
        }
        if used + candidate.len() + 1 > INDEX_CHAR_BUDGET {
            hidden += 1;
            continue;
        }
        used += candidate.len() + 1;
        lines.push(candidate);
    }
    if hidden > 0 {
        lines.push(format!("[{hidden} more skills not listed]"));
    }
    Some(lines.join("\n"))
}

/// Collapse a description to one clipped line for the index.
fn clip_one_line(desc: &str) -> String {
    let one = desc.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= INDEX_DESC_CLIP {
        return one;
    }
    let cut: String = one.chars().take(INDEX_DESC_CLIP).collect();
    format!("{cut}…")
}

/// One clipped description line for the in-band `[skills updated]` note, the
/// same shape as the L1 index so the model reads it as a resolver entry.
pub fn clip_description(desc: &str) -> String {
    clip_one_line(desc)
}
