//! SKILL.md discovery and the L1 index (agentskills.io standard). Four
//! discovery paths at session start, first hit per name wins; a hand-rolled
//! frontmatter scanner (plain scalars, quoted strings, `|`/`>` blocks);
//! malformed skills are skipped with a stderr warning, never a crash.
//! Level 2 (the `skill` tool) lives in tools/skill.rs; level 3 is plain read.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
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
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.first().map(|l| l.trim_end_matches('\r')) != Some("---") {
        return Err("no frontmatter: the file must start with a `---` line".to_string());
    }
    let mut fields = HashMap::new();
    let mut i = 1;
    while i < lines.len() {
        let line = lines[i].trim_end_matches('\r');
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
        let parsed = if matches!(value, "|" | "|-" | "|+" | ">" | ">-" | ">+") {
            let (block, next) = scan_block(&lines, i, value.starts_with('>'));
            i = next;
            block
        } else {
            scalar(value)?
        };
        fields.insert(key.to_string(), parsed.trim().to_string());
    }
    Err("unterminated frontmatter: no closing `---` line".to_string())
}

/// Collect a block scalar's indented lines starting at `i`; returns the
/// joined text and the index of the first line after the block. Literal
/// (`|`) keeps line breaks; folded (`>`) joins lines with spaces and keeps
/// blank lines as breaks. Chomping is irrelevant here: values are trimmed.
fn scan_block(lines: &[&str], mut i: usize, folded: bool) -> (String, usize) {
    let mut block: Vec<&str> = Vec::new();
    while i < lines.len() {
        let line = lines[i].trim_end_matches('\r');
        if line == "---" {
            break;
        }
        if !line.is_empty() && !line.starts_with(' ') && !line.starts_with('\t') {
            break;
        }
        block.push(line);
        i += 1;
    }
    let indent = block
        .iter()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .unwrap_or(0);
    let stripped: Vec<&str> = block
        .iter()
        .map(|l| if l.len() >= indent { &l[indent..] } else { "" })
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
