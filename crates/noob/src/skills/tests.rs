use super::*;
use super::frontmatter::{FRONTMATTER_BYTE_CAP, parse, read_frontmatter_file, validate};
use super::install::git_url_for;

#[test]
fn git_url_maps_owner_repo_shorthand_and_passes_urls_through() {
    assert_eq!(
        git_url_for("hec-ovi/research-skill", false).as_deref(),
        Some("https://github.com/hec-ovi/research-skill.git")
    );
    assert_eq!(
        git_url_for("a_b/c.d", false).as_deref(),
        Some("https://github.com/a_b/c.d.git")
    );
    // Explicit git sources pass through untouched, even when a local path
    // of the same name exists.
    assert_eq!(
        git_url_for("https://example.com/x.git", true).as_deref(),
        Some("https://example.com/x.git")
    );
    assert_eq!(
        git_url_for("git@github.com:o/r.git", false).as_deref(),
        Some("git@github.com:o/r.git")
    );
}

#[test]
fn git_url_never_shadows_a_local_path_or_misreads_one() {
    // A directory literally named like `owner/repo` wins over the
    // GitHub-registry reading.
    assert_eq!(git_url_for("acme/tools", true), None);
    // Not a shorthand: paths, hidden segments, extra slashes, bare names.
    assert_eq!(git_url_for("./local/dir", false), None);
    assert_eq!(git_url_for(".hidden/repo", false), None);
    assert_eq!(git_url_for("a/b/c", false), None);
    assert_eq!(git_url_for("just-a-dir", false), None);
    assert_eq!(git_url_for("owner/", false), None);
    assert_eq!(git_url_for("/repo", false), None);
    assert_eq!(git_url_for("owner/re po", false), None);
}

fn skill_md(name: &str, desc: &str) -> String {
    format!("---\nname: {name}\ndescription: {desc}\n---\nBody of {name}.\n")
}

fn write_skill(root: &Path, dir_name: &str, content: &str) {
    let dir = root.join(dir_name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), content).unwrap();
}

// --- the skills this repo ships ---

/// Every skill under `config/skills/` is copied into a real user's config by
/// `install.sh`, so a broken one ships broken: it is dropped from discovery
/// with no build failure to notice. Each must parse, validate, and fit the
/// index line, because a clipped description is the text the model matches
/// the task against and the clip lands mid-sentence.
#[test]
fn the_shipped_skills_parse_and_fit_the_resolver_index() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("config/skills");
    let mut seen = 0;
    for entry in std::fs::read_dir(&root).unwrap() {
        let dir = entry.unwrap().path();
        if !dir.is_dir() {
            continue;
        }
        let file = dir.join("SKILL.md");
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let parsed =
            parse(&text).unwrap_or_else(|e| panic!("{} does not parse: {e}", file.display()));
        let (name, description) = validate(&parsed.fields)
            .unwrap_or_else(|e| panic!("{} is invalid: {e}", file.display()));
        assert_eq!(
            name,
            dir.file_name().unwrap().to_str().unwrap(),
            "{} declares a name that is not its directory",
            file.display()
        );
        let line = clip_one_line(&description);
        assert!(
            !line.ends_with('…'),
            "{} description is {} chars and clips at {INDEX_DESC_CLIP} in the index: {line}",
            file.display(),
            description
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .count()
        );
        assert!(
            text.len() > parsed.body_start,
            "{} has no body",
            file.display()
        );
        seen += 1;
    }
    assert!(seen >= 2, "expected the shipped skills, found {seen}");
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
    for header in [
        "|2",
        ">-2",
        "|+",
        ">2-",
        "| # keep newlines",
        ">-  # folded",
    ] {
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
    let skills = discover(ws.path(), cfg.path(), &[]);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "good");
}

#[test]
fn missing_or_unterminated_frontmatter_errors() {
    assert!(
        parse("# just markdown\n")
            .unwrap_err()
            .contains("no frontmatter")
    );
    assert!(
        parse("---\nname: x\n")
            .unwrap_err()
            .contains("unterminated")
    );
    assert!(
        parse("---\njust a stray line\n---\n")
            .unwrap_err()
            .contains("key: value")
    );
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
        (
            "---\ndescription: d\n---\n",
            "missing required field `name`",
        ),
        ("---\nname: Bad_Name\ndescription: d\n---\n", "lowercase"),
        (long_name.as_str(), "max 64"),
        (
            "---\nname: n\n---\n",
            "missing required field `description`",
        ),
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
    write_skill(
        src.parent().unwrap(),
        "some-folder",
        &skill_md("installed", "d"),
    );
    std::fs::write(src.join("helper.sh"), "echo hi\n").unwrap();
    std::fs::create_dir_all(src.join(".git")).unwrap();
    std::fs::write(src.join(".git/config"), "private clone metadata").unwrap();
    let name = install(ws.path(), src.to_str().unwrap()).unwrap();
    assert_eq!(name, "installed");
    let dest = ws.path().join(".noob/skills/installed");
    assert!(dest.join("SKILL.md").is_file(), "SKILL.md must be copied");
    assert!(
        dest.join("helper.sh").is_file(),
        "bundled files must be copied"
    );
    assert!(
        !dest.join(".git").exists(),
        "VCS metadata must not be installed"
    );
    assert!(
        std::fs::read_dir(ws.path().join(".noob"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".skill-")),
        "a completed install must not leave staging data"
    );
    // It is now discoverable.
    let cfg = tempfile::tempdir().unwrap();
    assert!(
        discover(ws.path(), cfg.path(), &[])
            .iter()
            .any(|s| s.name == "installed")
    );
}

#[test]
fn installing_from_the_workspace_root_does_not_copy_staging_into_itself() {
    let ws = tempfile::tempdir().unwrap();
    std::fs::write(ws.path().join("SKILL.md"), skill_md("root-skill", "d")).unwrap();
    std::fs::create_dir_all(ws.path().join("bundle")).unwrap();
    std::fs::write(ws.path().join("bundle/note.txt"), "note").unwrap();

    let name = install(ws.path(), ws.path().to_str().unwrap()).unwrap();
    assert_eq!(name, "root-skill");
    let dest = ws.path().join(".noob/skills/root-skill");
    assert_eq!(
        std::fs::read_to_string(dest.join("bundle/note.txt")).unwrap(),
        "note"
    );
    assert_eq!(
        std::fs::read_dir(dest.join(".noob")).unwrap().count(),
        0,
        "the destination must not contain its own staging directory"
    );
}

#[test]
fn install_rejects_malformed_and_duplicate_without_writing() {
    let ws = tempfile::tempdir().unwrap();
    // Malformed frontmatter: rejected, nothing written.
    let bad = ws.path().join("bad");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("SKILL.md"), "no frontmatter here\n").unwrap();
    assert!(install(ws.path(), bad.to_str().unwrap()).is_err());
    assert!(
        !ws.path().join(".noob/skills").exists(),
        "a rejected install must write nothing"
    );
    // A valid install, then a duplicate is refused.
    let src = ws.path().join("src");
    write_skill(ws.path(), "src", &skill_md("dup", "d"));
    assert_eq!(install(ws.path(), src.to_str().unwrap()).unwrap(), "dup");
    let err = install(ws.path(), src.to_str().unwrap()).unwrap_err();
    assert!(err.contains("already installed"), "{err}");
}

#[test]
fn special_or_symlinked_skill_files_are_rejected_without_opening_them() {
    use std::os::unix::fs::symlink;

    let ws = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    let fifo_dir = ws.path().join(".noob/skills/fifo");
    std::fs::create_dir_all(&fifo_dir).unwrap();
    let fifo = fifo_dir.join("SKILL.md");
    let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert!(discover(ws.path(), cfg.path(), &[]).is_empty());
    let err = install(ws.path(), fifo_dir.to_str().unwrap()).unwrap_err();
    assert!(err.contains("regular non-symlink"), "{err}");

    let real = ws.path().join("real-SKILL.md");
    std::fs::write(&real, skill_md("linked", "d")).unwrap();
    let linked = ws.path().join("linked-SKILL.md");
    symlink(&real, &linked).unwrap();
    let err = install(ws.path(), linked.to_str().unwrap()).unwrap_err();
    assert!(err.contains("must not be a symlink"), "{err}");
}

#[test]
fn validation_reads_only_bounded_frontmatter_not_the_skill_body() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("SKILL.md");
    let mut text = skill_md("large-body", "d");
    text.push_str(&"x".repeat(FRONTMATTER_BYTE_CAP * 4));
    std::fs::write(&path, text).unwrap();
    let frontmatter = read_frontmatter_file(&path).unwrap();
    assert!(frontmatter.len() < 1024);
    assert_eq!(
        validate(&parse(&frontmatter).unwrap().fields).unwrap().0,
        "large-body"
    );
}

#[test]
fn remove_deletes_only_dirs_directly_under_the_install_root() {
    let ws = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let global = outside.path().join("skills/demo");
    std::fs::create_dir_all(&global).unwrap();
    let err = remove(ws.path(), &global).unwrap_err();
    assert!(err.contains("is not under"), "{err}");
    assert!(global.exists(), "an outside skill must not be deleted");
    // An installed skill is removed; its bundled files go with it.
    let inside = ws.path().join(".noob/skills/demo");
    std::fs::create_dir_all(&inside).unwrap();
    assert!(remove(ws.path(), &inside).is_ok());
    assert!(!inside.exists());
}

#[test]
fn remove_refuses_a_configured_resolver_skill_and_deletes_nothing() {
    // A NOOB_SKILL_PATHS entry resolves workspace-relative and may point
    // at real project source (here `cli/` holding code next to SKILL.md);
    // /skills remove must refuse it even though it is inside the
    // workspace and even though installed skills exist.
    let ws = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(ws.path().join(".noob/skills/installed")).unwrap();
    write_skill(ws.path(), "cli", &skill_md("censurado", "d"));
    std::fs::write(ws.path().join("cli/main.rs"), "fn main() {}\n").unwrap();
    let err = remove(ws.path(), &ws.path().join("cli")).unwrap_err();
    assert!(err.contains("is not under"), "{err}");
    assert!(err.contains("cli"), "the refusal must name the dir: {err}");
    assert!(
        ws.path().join("cli/main.rs").is_file(),
        "project source must survive the refusal"
    );
    // Nesting deeper than one level under the root is refused too.
    let nested = ws.path().join(".noob/skills/a/b");
    std::fs::create_dir_all(&nested).unwrap();
    assert!(remove(ws.path(), &nested).is_err());
    assert!(nested.exists());
}

#[test]
fn remove_refuses_a_workspace_root_skill() {
    // A SKILL.md at the workspace root makes the skill's dir == the
    // workspace; removal would delete the entire project.
    let ws = tempfile::tempdir().unwrap();
    std::fs::write(ws.path().join("SKILL.md"), skill_md("root-skill", "d")).unwrap();
    std::fs::write(ws.path().join("code.rs"), "// project code\n").unwrap();
    std::fs::create_dir_all(ws.path().join(".noob/skills/other")).unwrap();
    let err = remove(ws.path(), ws.path()).unwrap_err();
    assert!(err.contains("is not under"), "{err}");
    assert!(
        ws.path().join("code.rs").is_file(),
        "the workspace must survive removing a root skill"
    );
}

#[test]
fn stale_staging_from_dead_pids_is_swept_by_the_next_install() {
    let ws = tempfile::tempdir().unwrap();
    let noob = ws.path().join(".noob");
    // A pid far above any Linux pid_max: kill(pid, 0) fails with ESRCH.
    let stale = noob.join(".skill-install-999999999-abc-0");
    let live = noob.join(format!(".skill-git-{}-abc-0", std::process::id()));
    let unparsed = noob.join(".skill-strange");
    for dir in [&stale, &live, &unparsed] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let src = ws.path().join("src");
    write_skill(ws.path(), "src", &skill_md("swept", "d"));
    install(ws.path(), src.to_str().unwrap()).unwrap();
    assert!(!stale.exists(), "dead-pid staging must be swept");
    assert!(live.exists(), "live-pid staging must be kept");
    assert!(unparsed.exists(), "unattributable entries must be kept");
}

#[test]
fn frontmatter_open_rejects_fifo_and_symlink_at_the_fd_level() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let fifo = tmp.path().join("SKILL.md");
    let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    // Must not block waiting for a writer, and must carry the message.
    let err = read_frontmatter_file(&fifo).unwrap_err();
    assert!(err.to_string().contains("regular non-symlink"), "{err}");

    let real = tmp.path().join("real.md");
    std::fs::write(&real, skill_md("s", "d")).unwrap();
    let linked = tmp.path().join("linked-SKILL.md");
    symlink(&real, &linked).unwrap();
    let err = read_frontmatter_file(&linked).unwrap_err();
    assert!(err.to_string().contains("regular non-symlink"), "{err}");
}

#[test]
fn discovery_covers_all_four_roots_in_priority_order() {
    let ws = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    write_skill(
        &ws.path().join(".noob/skills"),
        "a",
        &skill_md("alpha", "from noob"),
    );
    write_skill(
        &ws.path().join(".claude/skills"),
        "b",
        &skill_md("beta", "from claude"),
    );
    write_skill(
        &ws.path().join(".agents/skills"),
        "c",
        &skill_md("gamma", "from agents"),
    );
    write_skill(
        &cfg.path().join("skills"),
        "d",
        &skill_md("delta", "from config"),
    );
    let skills = discover(ws.path(), cfg.path(), &[]);
    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["alpha", "beta", "gamma", "delta"]);
}

#[test]
fn first_hit_per_name_wins_across_roots() {
    let ws = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    write_skill(
        &ws.path().join(".noob/skills"),
        "s",
        &skill_md("dup", "project wins"),
    );
    write_skill(
        &cfg.path().join("skills"),
        "s",
        &skill_md("dup", "global loses"),
    );
    let skills = discover(ws.path(), cfg.path(), &[]);
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
    let skills = discover(ws.path(), cfg.path(), &[]);
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
    let names: Vec<String> = discover(ws.path(), cfg.path(), &[])
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(names, ["alpha", "zeta"]);
}

// --- configured resolver paths (NOOB_SKILL_PATHS) ---

#[test]
fn configured_path_registers_one_resolver_skill_not_its_sub_skills() {
    let ws = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    // A censurado-style dispatcher at a non-root path: `cli/SKILL.md`
    // routes to sub-skills under `cli/skills/*/SKILL.md`.
    write_skill(
        ws.path(),
        "cli",
        &skill_md("censurado", "dispatcher that routes verbs"),
    );
    write_skill(
        &ws.path().join("cli/skills"),
        "walk",
        &skill_md("walk", "sub-skill a"),
    );
    write_skill(
        &ws.path().join("cli/skills"),
        "build",
        &skill_md("build", "sub-skill b"),
    );

    let extra = vec![ws.path().join("cli")];
    let skills = discover(ws.path(), cfg.path(), &extra);

    // Exactly one skill: the dispatcher, named from its frontmatter, with
    // `dir` pointing at the configured path. Sub-skills are NOT indexed
    // (the dispatcher loads them by `read`).
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "censurado");
    assert_eq!(skills[0].dir, ws.path().join("cli"));
    assert_eq!(skills[0].file, ws.path().join("cli/SKILL.md"));
    assert!(!skills.iter().any(|s| s.name == "walk" || s.name == "build"));
}

#[test]
fn configured_paths_coexist_with_default_roots_after_them() {
    let ws = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    write_skill(
        &ws.path().join(".noob/skills"),
        "foo-dir",
        &skill_md("foo", "a default root"),
    );
    write_skill(
        ws.path(),
        "cli",
        &skill_md("censurado", "a configured resolver"),
    );

    let extra = vec![ws.path().join("cli")];
    let skills = discover(ws.path(), cfg.path(), &extra);
    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    // Both present; configured paths come after the four default roots.
    assert_eq!(names, ["foo", "censurado"]);
}

#[test]
fn default_roots_win_a_name_clash_against_configured_paths() {
    let ws = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    write_skill(
        &ws.path().join(".noob/skills"),
        "dup-dir",
        &skill_md("dup", "default wins"),
    );
    write_skill(ws.path(), "cli", &skill_md("dup", "configured loses"));

    let extra = vec![ws.path().join("cli")];
    let skills = discover(ws.path(), cfg.path(), &extra);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].description, "default wins");
    assert!(skills[0].dir.starts_with(ws.path().join(".noob/skills")));
}

#[test]
fn configured_path_rejects_symlinked_or_special_skill_md() {
    use std::os::unix::fs::symlink;

    let ws = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();

    // A symlinked SKILL.md at the configured dir is rejected, same as the
    // default roots reject one (the file must be a regular non-symlink).
    let cli = ws.path().join("cli");
    std::fs::create_dir_all(&cli).unwrap();
    let real = ws.path().join("real-SKILL.md");
    std::fs::write(&real, skill_md("censurado", "d")).unwrap();
    symlink(&real, cli.join("SKILL.md")).unwrap();
    assert!(discover(ws.path(), cfg.path(), std::slice::from_ref(&cli)).is_empty());

    // A FIFO in place of SKILL.md is likewise refused without opening it.
    let fifo_dir = ws.path().join("fifo-cli");
    std::fs::create_dir_all(&fifo_dir).unwrap();
    let fifo = fifo_dir.join("SKILL.md");
    let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert!(discover(ws.path(), cfg.path(), &[fifo_dir]).is_empty());
}

#[test]
fn configured_path_that_is_a_symlinked_directory_is_skipped() {
    use std::os::unix::fs::symlink;

    let ws = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    // A real skill dir, and a symlink pointing at it. Configuring the
    // symlink (not the real dir) is refused: a mounted workspace cannot
    // smuggle a skill in via a directory symlink.
    write_skill(ws.path(), "real-cli", &skill_md("censurado", "d"));
    let linked = ws.path().join("linked-cli");
    symlink(ws.path().join("real-cli"), &linked).unwrap();
    assert!(discover(ws.path(), cfg.path(), &[linked]).is_empty());
    // The real directory still resolves, proving only the symlink is the issue.
    let real = vec![ws.path().join("real-cli")];
    assert_eq!(discover(ws.path(), cfg.path(), &real).len(), 1);
}

#[test]
fn configured_path_without_a_skill_md_is_silently_not_a_skill() {
    let ws = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    // A plain directory (no SKILL.md) and a nonexistent path: neither is a
    // skill and neither aborts discovery.
    std::fs::create_dir_all(ws.path().join("cli")).unwrap();
    let extra = vec![ws.path().join("cli"), ws.path().join("does-not-exist")];
    assert!(discover(ws.path(), cfg.path(), &extra).is_empty());
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
    assert!(
        idx.len() <= INDEX_CHAR_BUDGET + 40,
        "index is {} chars",
        idx.len()
    );
    assert!(
        idx.contains("- skill-000: "),
        "early skills keep descriptions"
    );
    assert!(
        idx.lines().any(|l| l == "- skill-020"),
        "overflow skills get name-only lines: {idx}"
    );
    let note = idx.lines().last().unwrap();
    assert!(
        note.contains("more skills not listed"),
        "count note missing: {note}"
    );
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
