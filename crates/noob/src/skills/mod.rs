//! SKILL.md discovery and the L1 index (agentskills.io standard). Four
//! discovery paths at session start, first hit per name wins; a hand-rolled
//! frontmatter scanner (plain scalars, quoted strings, `|`/`>` blocks);
//! malformed skills are skipped with a stderr warning, never a crash.
//! Level 2 (the `skill` tool) lives in tools/skill.rs; level 3 is plain read.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Discovery at session start, first hit per NAME wins across the four
/// roots in priority order; alphabetical within a root so the result is
/// deterministic. A directory without SKILL.md is silently not a skill;
/// a SKILL.md that fails to parse is skipped with a stderr warning.
pub fn discover(workspace: &Path, config_dir: &Path) -> Vec<Skill> {
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
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            let file = dir.join("SKILL.md");
            let text = match std::fs::read_to_string(&file) {
                Ok(t) => t,
                // No SKILL.md: not a skill dir, and not worth a warning.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                // Present but unreadable (permissions, invalid UTF-8): the
                // mandated stderr warning, not a silent disappearance.
                Err(e) => {
                    eprintln!("noob: skipping skill {}: cannot read: {e}", file.display());
                    continue;
                }
            };
            match parse(&text).and_then(|p| validate(&p.fields)) {
                Ok((name, description)) => {
                    if out.iter().any(|s| s.name == name) {
                        continue; // shadowed by a higher-priority root
                    }
                    out.push(Skill { name, description, dir, file });
                }
                Err(reason) => {
                    eprintln!("noob: skipping skill {}: {reason}", file.display());
                }
            }
        }
    }
    out
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

// ---------------------------------------------------------------------------
// On-the-fly install / remove (the /skills command family; REPL only)
// ---------------------------------------------------------------------------

/// The workspace root a user-installed skill lands under, so it is picked up
/// by the highest-priority discovery path on the next reload.
fn install_root(workspace: &Path) -> PathBuf {
    workspace.join(".noob/skills")
}

/// Install a skill from a local path (a skill directory or a bare SKILL.md)
/// or a git URL into `<workspace>/.noob/skills/<name>`. The source is parsed
/// and validated before anything is committed, so a malformed skill is
/// rejected with a reason and nothing is copied. Returns the installed name.
pub fn install(workspace: &Path, source: &str) -> Result<String, String> {
    let looks_git = source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("git@")
        || source.ends_with(".git");
    if looks_git {
        install_git(workspace, source)
    } else {
        install_path(workspace, Path::new(source))
    }
}

fn install_path(workspace: &Path, source: &Path) -> Result<String, String> {
    let is_bare_md = source.file_name() == Some(OsStr::new("SKILL.md")) && source.is_file();
    let skill_md = if source.is_dir() {
        source.join("SKILL.md")
    } else if is_bare_md {
        source.to_path_buf()
    } else {
        return Err(format!(
            "{}: expected a skill directory containing SKILL.md, or a SKILL.md file",
            source.display()
        ));
    };
    let text = std::fs::read_to_string(&skill_md)
        .map_err(|e| format!("cannot read {}: {e}", skill_md.display()))?;
    // Validate the frontmatter up front: a bad skill is rejected before any
    // file is written, so a failed install never leaves a partial dir.
    let parsed = parse(&text)?;
    let (name, _desc) = validate(&parsed.fields)?;

    let dest = install_root(workspace).join(&name);
    if dest.exists() {
        return Err(format!(
            "a skill named {name:?} is already installed; remove it first with /skills remove {name}"
        ));
    }
    if source.is_dir() {
        copy_dir(source, &dest).map_err(|e| format!("copying the skill failed: {e}"))?;
    } else {
        std::fs::create_dir_all(&dest)
            .and_then(|_| std::fs::copy(&skill_md, dest.join("SKILL.md")).map(|_| ()))
            .map_err(|e| format!("copying the skill failed: {e}"))?;
    }
    Ok(name)
}

fn install_git(workspace: &Path, url: &str) -> Result<String, String> {
    // Shallow-clone into a staging dir under the install root; the `git`
    // binary does the fetching, so noob itself opens no new sockets and the
    // egress invariant holds. Staging is always cleaned up.
    let staging = install_root(workspace).join(format!(".staging-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    if let Some(parent) = staging.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create the skills dir: {e}"))?;
    }
    let out = Command::new("git")
        .args(["clone", "--depth", "1", url])
        .arg(&staging)
        .output();
    let result = match out {
        Ok(o) if o.status.success() => find_skill_dir(&staging)
            .and_then(|dir| install_path(workspace, &dir)),
        Ok(o) => Err(format!(
            "git clone failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => Err(format!("could not run git (is it installed?): {e}")),
    };
    let _ = std::fs::remove_dir_all(&staging);
    result
}

/// The skill directory inside a freshly cloned repo: the root when it holds a
/// SKILL.md, else a single immediate subdirectory that does.
fn find_skill_dir(root: &Path) -> Result<PathBuf, String> {
    if root.join("SKILL.md").is_file() {
        return Ok(root.to_path_buf());
    }
    let mut hits: Vec<PathBuf> = std::fs::read_dir(root)
        .map_err(|e| format!("cannot read the cloned repo: {e}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.join("SKILL.md").is_file())
        .collect();
    hits.sort();
    match hits.len() {
        1 => Ok(hits.into_iter().next().unwrap()),
        0 => Err("the repo has no SKILL.md at its root or in a subdirectory".to_string()),
        _ => Err("the repo has several skills; clone it and add one subdirectory".to_string()),
    }
}

/// Remove an installed skill's directory. Confined to the workspace tree: a
/// global or ecosystem skill (outside the workspace) is refused rather than
/// deleted, so `/skills remove` can never nuke a shared install.
pub fn remove(workspace: &Path, skill_dir: &Path) -> Result<(), String> {
    let ws = workspace
        .canonicalize()
        .map_err(|e| format!("cannot resolve the workspace: {e}"))?;
    let dir = skill_dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", skill_dir.display()))?;
    if !dir.starts_with(&ws) {
        return Err(format!(
            "{} is outside the workspace (a global or ecosystem skill); remove it by hand",
            skill_dir.display()
        ));
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("removing the skill failed: {e}"))
}

/// Recursively copy a directory's regular files and subdirectories (symlinks
/// and special files are skipped, so a skill cannot smuggle one in).
fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}

/// One clipped description line for the in-band `[skills updated]` note, the
/// same shape as the L1 index so the model reads it as a resolver entry.
pub fn clip_description(desc: &str) -> String {
    clip_one_line(desc)
}

// ---------------------------------------------------------------------------
// Frontmatter scanner
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct Parsed {
    pub fields: HashMap<String, String>,
    /// Number of frontmatter lines (== 0-based line index where the body
    /// starts; file line numbers are this + 1).
    pub body_start: usize,
}

/// Scan the `---`-fenced frontmatter: `key: value` plain scalars, quoted
/// strings, and `|`/`>` block scalars. Indented lines under an unknown key
/// (nested metadata) are ignored; a top-level line that is not `key: value`
/// is an error. All values land in the map trimmed.
pub fn parse(text: &str) -> Result<Parsed, String> {
    // YAML permits a BOM at stream start and Windows editors add one; it
    // must not hide the opening fence. Line indexing is unchanged (the BOM
    // is not a newline), so body_of's byte math over the original text
    // stays exact.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return Err("no frontmatter: the file must start with a `---` line".to_string());
    }
    let mut fields = HashMap::new();
    let mut i = 1;
    while i < lines.len() {
        let line = lines[i].trim_end();
        if line == "---" {
            return Ok(Parsed { fields, body_start: i + 1 });
        }
        i += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            continue; // nested content under an ignored key
        }
        let Some(colon) = line.find(':') else {
            return Err(format!("line {i}: expected `key: value`, got {line:?}"));
        };
        let key = line[..colon].trim_end();
        if key.is_empty()
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(format!("line {i}: invalid key {key:?}"));
        }
        let value = line[colon + 1..].trim();
        // A block header may carry a trailing YAML comment (`description: | #
        // keep newlines`), so test only the first token for the indicator.
        let indicator = value.split_whitespace().next().unwrap_or("");
        let parsed = if is_block_indicator(indicator) {
            let (block, next) = scan_block(&lines, i, indicator.starts_with('>'));
            i = next;
            block
        } else {
            scalar(value)?
        };
        fields.insert(key.to_string(), parsed.trim().to_string());
    }
    Err("unterminated frontmatter: no closing `---` line".to_string())
}

/// `|` or `>` with optional chomping (`+`/`-`) and an optional explicit
/// indentation digit, in either order (YAML block headers like `|2` or
/// `>-2`). The digit is accepted but indentation stays auto-detected:
/// values are trimmed for validation, so the distinction cannot matter.
fn is_block_indicator(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('|') | Some('>'))
        && value.len() <= 3
        && chars.all(|c| c == '+' || c == '-' || c.is_ascii_digit())
}

/// Collect a block scalar's indented lines starting at `i`; returns the
/// joined text and the index of the first line after the block. Literal
/// (`|`) keeps line breaks; folded (`>`) joins lines with spaces and keeps
/// blank lines as breaks. Chomping is irrelevant here: values are trimmed.
fn scan_block(lines: &[&str], mut i: usize, folded: bool) -> (String, usize) {
    let mut block: Vec<&str> = Vec::new();
    while i < lines.len() {
        let line = lines[i].trim_end();
        if line == "---" {
            break;
        }
        if !line.is_empty() && !line.starts_with(' ') && !line.starts_with('\t') {
            break;
        }
        block.push(line);
        i += 1;
    }
    // Indentation is ASCII space/tab BYTES only, and the strip below
    // consumes only such bytes: a continuation line less indented than the
    // first (or indented with exotic whitespace) can never split a
    // multi-byte character, so this cannot panic on any input.
    let indent = block
        .iter()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.bytes().take_while(|&b| b == b' ' || b == b'\t').count())
        .unwrap_or(0);
    let stripped: Vec<&str> = block
        .iter()
        .map(|l| {
            let bytes = l.as_bytes();
            let mut cut = 0;
            while cut < indent && cut < bytes.len() && (bytes[cut] == b' ' || bytes[cut] == b'\t')
            {
                cut += 1;
            }
            &l[cut..]
        })
        .collect();
    let text = if folded {
        let mut out = String::new();
        for l in &stripped {
            if l.trim().is_empty() {
                out.push('\n');
            } else {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str(l.trim_end());
            }
        }
        out
    } else {
        stripped.join("\n")
    };
    (text, i)
}

/// One-line scalar: double-quoted (with `\"` `\\` `\n` `\t` escapes),
/// single-quoted (`''` is a literal quote), or plain.
fn scalar(value: &str) -> Result<String, String> {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let inner = &value[1..value.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(other) => out.push(other), // \" \\ and anything else
                None => return Err("dangling backslash in a quoted value".to_string()),
            }
        }
        return Ok(out);
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return Ok(value[1..value.len() - 1].replace("''", "'"));
    }
    Ok(value.to_string())
}

/// Validate the agentskills.io required fields; the error becomes the
/// stderr skip warning.
fn validate(fields: &HashMap<String, String>) -> Result<(String, String), String> {
    let name = fields
        .get("name")
        .filter(|n| !n.is_empty())
        .ok_or("missing required field `name`")?;
    if name.len() > 64 {
        return Err(format!("name is {} chars (max 64)", name.len()));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(format!(
            "name {name:?} must contain only lowercase letters, digits, and hyphens"
        ));
    }
    let description = fields
        .get("description")
        .filter(|d| !d.is_empty())
        .ok_or("missing required field `description`")?;
    let chars = description.chars().count();
    if chars > 1024 {
        return Err(format!("description is {chars} chars (max 1024)"));
    }
    Ok((name.clone(), description.clone()))
}

/// The body with the frontmatter stripped (byte-exact suffix of the file)
/// plus the number of leading lines removed. Lenient: a file whose
/// frontmatter no longer parses is returned whole, because at `skill`-tool
/// call time a stale file should degrade, not error.
pub fn body_of(text: &str) -> (Cow<'_, str>, usize) {
    match parse(text) {
        Ok(p) => {
            let mut offset = 0usize;
            for (n, line) in text.split('\n').enumerate() {
                if n == p.body_start {
                    break;
                }
                offset += line.len() + 1;
            }
            (Cow::Borrowed(&text[offset.min(text.len())..]), p.body_start)
        }
        Err(_) => (Cow::Borrowed(text), 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill_md(name: &str, desc: &str) -> String {
        format!("---\nname: {name}\ndescription: {desc}\n---\nBody of {name}.\n")
    }

    fn write_skill(root: &Path, dir_name: &str, content: &str) {
        let dir = root.join(dir_name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), content).unwrap();
    }

    // --- parser ---

    #[test]
    fn plain_scalars_and_body_offset() {
        let p = parse("---\nname: my-skill\ndescription: does things\n---\nbody\n").unwrap();
        assert_eq!(p.fields["name"], "my-skill");
        assert_eq!(p.fields["description"], "does things");
        assert_eq!(p.body_start, 4);
    }

    #[test]
    fn quoted_scalars_unescape() {
        let p = parse(
            "---\nname: \"quoted-name\"\ndescription: 'it''s quoted: fine'\nextra: \"a\\nb \\\"c\\\"\"\n---\n",
        )
        .unwrap();
        assert_eq!(p.fields["name"], "quoted-name");
        assert_eq!(p.fields["description"], "it's quoted: fine");
        assert_eq!(p.fields["extra"], "a\nb \"c\"");
    }

    #[test]
    fn literal_block_keeps_line_breaks() {
        let text = "---\nname: b\ndescription: |\n  line one\n  line two\n\n  line three\n---\n";
        let p = parse(text).unwrap();
        assert_eq!(p.fields["description"], "line one\nline two\n\nline three");
    }

    #[test]
    fn folded_block_joins_lines_and_keeps_blank_breaks() {
        let text = "---\nname: b\ndescription: >-\n  one\n  two\n\n  three\n---\n";
        let p = parse(text).unwrap();
        assert_eq!(p.fields["description"], "one two\nthree");
    }

    #[test]
    fn block_ends_at_the_next_top_level_key() {
        let text = "---\nname: b\ndescription: |\n  the block\nlicense: MIT\n---\nbody";
        let p = parse(text).unwrap();
        assert_eq!(p.fields["description"], "the block");
        assert_eq!(p.fields["license"], "MIT");
    }

    #[test]
    fn nested_metadata_and_comments_are_ignored() {
        let text = "---\nname: n\n# a comment\nmetadata:\n  author: someone\n  version: 2\ndescription: d\n---\n";
        let p = parse(text).unwrap();
        assert_eq!(p.fields["name"], "n");
        assert_eq!(p.fields["description"], "d");
        assert_eq!(p.fields["metadata"], "");
        assert!(!p.fields.contains_key("author"));
    }

    #[test]
    fn crlf_files_parse() {
        let text = "---\r\nname: crlf-skill\r\ndescription: windows line ends\r\n---\r\nbody\r\n";
        let p = parse(text).unwrap();
        assert_eq!(p.fields["name"], "crlf-skill");
        let (body, skipped) = body_of(text);
        assert_eq!(skipped, 4);
        assert_eq!(body.as_ref(), "body\r\n");
    }

    #[test]
    fn multibyte_char_at_the_indent_offset_never_panics() {
        // A continuation line less indented than the first, with a
        // multi-byte char straddling the indent byte offset: the old
        // byte-slice strip panicked here and killed discovery.
        let text = "---\nname: b\ndescription: |\n    deeply\n  中文 content\n---\nbody\n";
        let p = parse(text).unwrap();
        assert_eq!(p.fields["description"], "deeply\n中文 content");
        // Same shape with NBSP (unicode whitespace) indentation: NBSP is
        // not YAML indentation, so the line reads as a malformed key line.
        // The contract is a clean Err (skip with warning), never a panic.
        let text = "---\nname: b\ndescription: |\n\u{a0}\u{a0}first\n  中 x\n---\n";
        assert!(parse(text).unwrap_err().contains("key: value"));
    }

    #[test]
    fn bom_and_trailing_fence_whitespace_are_tolerated() {
        let text = "\u{feff}---\nname: bom-skill\ndescription: windows authored\n--- \nbody\n";
        let p = parse(text).unwrap();
        assert_eq!(p.fields["name"], "bom-skill");
        assert_eq!(p.body_start, 4);
        let (body, skipped) = body_of(text);
        assert_eq!(body.as_ref(), "body\n");
        assert_eq!(skipped, 4);
    }

    #[test]
    fn block_headers_with_explicit_indent_digits_parse_as_blocks() {
        for header in ["|2", ">-2", "|+", ">2-", "| # keep newlines", ">-  # folded"] {
            let text = format!("---\nname: n\ndescription: {header}\n  real text\n---\n");
            let p = parse(&text).unwrap();
            assert_eq!(p.fields["description"], "real text", "header {header:?}");
        }
        // A pipe inside a plain scalar is NOT a block header.
        let p = parse("---\nname: n\ndescription: a | b\n---\n").unwrap();
        assert_eq!(p.fields["description"], "a | b");
    }

    #[test]
    fn unreadable_skill_md_warns_and_skips_without_crashing() {
        use std::io::Write;
        let ws = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        let root = ws.path().join(".claude/skills");
        write_skill(&root, "good", &skill_md("good", "fine"));
        // Invalid UTF-8: read_to_string fails with a non-NotFound error.
        std::fs::create_dir_all(root.join("binary")).unwrap();
        let mut f = std::fs::File::create(root.join("binary/SKILL.md")).unwrap();
        f.write_all(&[0xff, 0xfe, 0x00, 0x80]).unwrap();
        let skills = discover(ws.path(), cfg.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "good");
    }

    #[test]
    fn missing_or_unterminated_frontmatter_errors() {
        assert!(parse("# just markdown\n").unwrap_err().contains("no frontmatter"));
        assert!(parse("---\nname: x\n").unwrap_err().contains("unterminated"));
        assert!(parse("---\njust a stray line\n---\n").unwrap_err().contains("key: value"));
    }

    #[test]
    fn body_of_is_byte_exact_and_lenient() {
        let text = "---\nname: n\ndescription: d\n---\n\n# Title\n\ncontent";
        let (body, skipped) = body_of(text);
        assert_eq!(body.as_ref(), "\n# Title\n\ncontent");
        assert_eq!(skipped, 4);
        // No parseable frontmatter: the whole file is the body.
        let (body, skipped) = body_of("plain file");
        assert_eq!(body.as_ref(), "plain file");
        assert_eq!(skipped, 0);
    }

    // --- validation ---

    #[test]
    fn validation_enforces_the_standard_limits() {
        let ok = parse(&skill_md("a-1", "fine")).unwrap();
        assert!(validate(&ok.fields).is_ok());
        let long_name = format!("---\nname: {}\ndescription: d\n---\n", "x".repeat(65));
        let long_desc = format!("---\nname: n\ndescription: {}\n---\n", "d".repeat(1025));
        for (fm, needle) in [
            ("---\ndescription: d\n---\n", "missing required field `name`"),
            ("---\nname: Bad_Name\ndescription: d\n---\n", "lowercase"),
            (long_name.as_str(), "max 64"),
            ("---\nname: n\n---\n", "missing required field `description`"),
            (long_desc.as_str(), "max 1024"),
        ] {
            let p = parse(fm).unwrap();
            let err = validate(&p.fields).unwrap_err();
            assert!(err.contains(needle), "{fm:?}: {err}");
        }
    }

    // --- discovery ---

    // --- on-the-fly install / remove ---

    #[test]
    fn install_local_copies_the_skill_and_names_it_from_frontmatter() {
        let ws = tempfile::tempdir().unwrap();
        // A source dir whose folder name differs from the skill name: the
        // install must key off the frontmatter, and carry bundled files.
        let src = ws.path().join("some-folder");
        write_skill(&src.parent().unwrap().to_path_buf(), "some-folder", &skill_md("installed", "d"));
        std::fs::write(src.join("helper.sh"), "echo hi\n").unwrap();
        let name = install(ws.path(), src.to_str().unwrap()).unwrap();
        assert_eq!(name, "installed");
        let dest = ws.path().join(".noob/skills/installed");
        assert!(dest.join("SKILL.md").is_file(), "SKILL.md must be copied");
        assert!(dest.join("helper.sh").is_file(), "bundled files must be copied");
        // It is now discoverable.
        let cfg = tempfile::tempdir().unwrap();
        assert!(discover(ws.path(), cfg.path()).iter().any(|s| s.name == "installed"));
    }

    #[test]
    fn install_rejects_malformed_and_duplicate_without_writing() {
        let ws = tempfile::tempdir().unwrap();
        // Malformed frontmatter: rejected, nothing written.
        let bad = ws.path().join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("SKILL.md"), "no frontmatter here\n").unwrap();
        assert!(install(ws.path(), bad.to_str().unwrap()).is_err());
        assert!(!ws.path().join(".noob/skills").exists(), "a rejected install must write nothing");
        // A valid install, then a duplicate is refused.
        let src = ws.path().join("src");
        write_skill(&ws.path().to_path_buf(), "src", &skill_md("dup", "d"));
        assert_eq!(install(ws.path(), src.to_str().unwrap()).unwrap(), "dup");
        let err = install(ws.path(), src.to_str().unwrap()).unwrap_err();
        assert!(err.contains("already installed"), "{err}");
    }

    #[test]
    fn remove_refuses_paths_outside_the_workspace() {
        let ws = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let global = outside.path().join("skills/demo");
        std::fs::create_dir_all(&global).unwrap();
        let err = remove(ws.path(), &global).unwrap_err();
        assert!(err.contains("outside the workspace"), "{err}");
        assert!(global.exists(), "an outside skill must not be deleted");
        // A workspace skill is removed.
        let inside = ws.path().join(".noob/skills/demo");
        std::fs::create_dir_all(&inside).unwrap();
        assert!(remove(ws.path(), &inside).is_ok());
        assert!(!inside.exists());
    }

    #[test]
    fn discovery_covers_all_four_roots_in_priority_order() {
        let ws = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        write_skill(&ws.path().join(".noob/skills"), "a", &skill_md("alpha", "from noob"));
        write_skill(&ws.path().join(".claude/skills"), "b", &skill_md("beta", "from claude"));
        write_skill(&ws.path().join(".agents/skills"), "c", &skill_md("gamma", "from agents"));
        write_skill(&cfg.path().join("skills"), "d", &skill_md("delta", "from config"));
        let skills = discover(ws.path(), cfg.path());
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["alpha", "beta", "gamma", "delta"]);
    }

    #[test]
    fn first_hit_per_name_wins_across_roots() {
        let ws = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        write_skill(&ws.path().join(".noob/skills"), "s", &skill_md("dup", "project wins"));
        write_skill(&cfg.path().join("skills"), "s", &skill_md("dup", "global loses"));
        let skills = discover(ws.path(), cfg.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].description, "project wins");
        assert!(skills[0].dir.starts_with(ws.path()));
    }

    #[test]
    fn malformed_skills_are_skipped_and_good_ones_survive() {
        let ws = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        let root = ws.path().join(".claude/skills");
        write_skill(&root, "bad", "no frontmatter here\n");
        write_skill(&root, "good", &skill_md("good", "works"));
        // A directory without SKILL.md is not a skill and not a warning.
        std::fs::create_dir_all(root.join("not-a-skill")).unwrap();
        let skills = discover(ws.path(), cfg.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "good");
    }

    #[test]
    fn alphabetical_within_a_root() {
        let ws = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        let root = ws.path().join(".noob/skills");
        write_skill(&root, "zeta-dir", &skill_md("zeta", "z"));
        write_skill(&root, "alpha-dir", &skill_md("alpha", "a"));
        let names: Vec<String> = discover(ws.path(), cfg.path())
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["alpha", "zeta"]);
    }

    // --- index ---

    #[test]
    fn index_lines_and_empty_case() {
        assert!(index(&[]).is_none());
        let s = Skill {
            name: "fmt".into(),
            description: "multi\nline   description".into(),
            dir: PathBuf::from("/x"),
            file: PathBuf::from("/x/SKILL.md"),
        };
        assert_eq!(index(&[s]).unwrap(), "- fmt: multi line description");
    }

    #[test]
    fn index_clips_long_descriptions_at_200_chars() {
        let s = Skill {
            name: "long".into(),
            description: "d".repeat(500),
            dir: PathBuf::from("/x"),
            file: PathBuf::from("/x/SKILL.md"),
        };
        let line = index(&[s]).unwrap();
        assert!(line.starts_with("- long: "));
        assert!(line.ends_with('…'));
        assert_eq!(line.chars().count(), "- long: ".chars().count() + 200 + 1);
    }

    #[test]
    fn index_overflows_to_name_only_then_a_count_note() {
        // 400 skills x ~214-char full lines against a 4,000-char budget:
        // ~18 keep descriptions, a few dozen degrade to name-only, the rest
        // land in the count note. Every skill must be accounted for.
        let skills: Vec<Skill> = (0..400)
            .map(|i| Skill {
                name: format!("skill-{i:03}"),
                description: "d".repeat(200),
                dir: PathBuf::from("/x"),
                file: PathBuf::from("/x/SKILL.md"),
            })
            .collect();
        let idx = index(&skills).unwrap();
        assert!(idx.len() <= INDEX_CHAR_BUDGET + 40, "index is {} chars", idx.len());
        assert!(idx.contains("- skill-000: "), "early skills keep descriptions");
        assert!(
            idx.lines().any(|l| l == "- skill-020"),
            "overflow skills get name-only lines: {idx}"
        );
        let note = idx.lines().last().unwrap();
        assert!(note.contains("more skills not listed"), "count note missing: {note}");
        let listed = idx.lines().filter(|l| l.starts_with("- ")).count();
        let counted: usize = note
            .trim_start_matches('[')
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(listed + counted, 400, "every skill listed or counted");
    }
}
