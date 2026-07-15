//! SKILL.md discovery and the L1 index (agentskills.io standard). Four
//! discovery roots at session start plus any explicitly configured resolver
//! paths (NOOB_SKILL_PATHS), first hit per name wins; a hand-rolled
//! frontmatter scanner (plain scalars, quoted strings, `|`/`>` blocks);
//! malformed skills are skipped with a stderr warning, never a crash.
//! Level 2 (the `skill` tool) lives in tools/skill.rs; level 3 is plain read.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use noob_provider::http::INTERRUPTED;

/// Index section budget in chars (~1,000 tokens at chars/4).
pub const INDEX_CHAR_BUDGET: usize = 4_000;
/// Per-skill description clip in the index, in chars.
pub const INDEX_DESC_CLIP: usize = 200;
const GIT_CLONE_TIMEOUT: Duration = Duration::from_secs(120);
const GIT_ERROR_CAP: usize = 64 * 1024;
const FRONTMATTER_BYTE_CAP: usize = 64 * 1024;
static STAGING_SERIAL: AtomicU64 = AtomicU64::new(0);

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
    let text = match read_frontmatter_file(&file) {
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
    match parse(&text).and_then(|p| validate(&p.fields)) {
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

// ---------------------------------------------------------------------------
// On-the-fly install / remove (the /skills command family; REPL only)
// ---------------------------------------------------------------------------

/// The workspace root a user-installed skill lands under, so it is picked up
/// by the highest-priority discovery path on the next reload.
fn install_root(workspace: &Path) -> PathBuf {
    workspace.join(".noob/skills")
}

/// Install a skill from a local path (a skill directory or a bare SKILL.md),
/// a git URL, or an `owner/repo` GitHub shorthand (the same registry shape
/// `npx skills add` uses) into `<workspace>/.noob/skills/<name>`. The source
/// is parsed and validated before anything is committed, so a malformed skill
/// is rejected with a reason and nothing is copied. Returns the installed name.
pub fn install(workspace: &Path, source: &str) -> Result<String, String> {
    match git_url_for(source, Path::new(source).exists()) {
        Some(url) => install_git(workspace, &url),
        None => install_path(workspace, Path::new(source)),
    }
}

/// The clone URL for a git-shaped source: explicit URLs pass through, and an
/// `owner/repo` GitHub shorthand (the registry shape `npx skills add` uses)
/// expands, but only when nothing with that name exists on disk, so a real
/// `owner/repo`-shaped local directory always wins. `None` means a local path.
fn git_url_for(source: &str, exists_locally: bool) -> Option<String> {
    let looks_git = source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("git@")
        || source.ends_with(".git");
    if looks_git {
        return Some(source.to_string());
    }
    if exists_locally {
        return None;
    }
    // Exactly one slash, both segments limited to GitHub's name alphabet, and
    // neither starting with a dot (so `./dir` and hidden paths never read as
    // a repo).
    let (owner, repo) = source.split_once('/')?;
    let valid = |s: &str| {
        !s.is_empty()
            && !s.starts_with('.')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (valid(owner) && valid(repo)).then(|| format!("https://github.com/{owner}/{repo}.git"))
}

fn install_path(workspace: &Path, source: &Path) -> Result<String, String> {
    let source_type = std::fs::symlink_metadata(source)
        .map_err(|e| format!("cannot inspect {}: {e}", source.display()))?
        .file_type();
    if source_type.is_symlink() {
        return Err(format!(
            "{}: the skill source must not be a symlink",
            source.display()
        ));
    }
    let source = source
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", source.display()))?;
    let is_bare_md = source.file_name() == Some(OsStr::new("SKILL.md")) && source.is_file();
    let skill_md = if source.is_dir() {
        source.join("SKILL.md")
    } else if is_bare_md {
        source.clone()
    } else {
        return Err(format!(
            "{}: expected a skill directory containing SKILL.md, or a SKILL.md file",
            source.display()
        ));
    };
    let text = read_frontmatter_file(&skill_md)
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
    // Build beside the skills root and publish with one rename. Discovery can
    // never observe half a skill, and a failed copy only leaves hidden staging
    // data that this invocation removes before returning.
    let staging = staging_path(workspace, "install");
    let copied = if source.is_dir() {
        copy_dir(&source, &staging)
    } else {
        std::fs::create_dir_all(&staging)
            .and_then(|_| copy_file(&skill_md, &staging.join("SKILL.md")))
    };
    if let Err(error) = copied {
        let _ = std::fs::remove_dir_all(&staging);
        if INTERRUPTED.load(Ordering::SeqCst) {
            return Err("skill installation canceled by user".to_string());
        }
        return Err(format!("copying the skill failed: {error}"));
    }
    if INTERRUPTED.load(Ordering::SeqCst) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err("skill installation canceled by user".to_string());
    }
    if let Err(error) = std::fs::create_dir_all(install_root(workspace)) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("cannot create the skills dir: {error}"));
    }
    if dest.exists() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!(
            "a skill named {name:?} was installed concurrently; remove it first with /skills remove {name}"
        ));
    }
    if let Err(error) = std::fs::rename(&staging, &dest) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("publishing the skill failed: {error}"));
    }
    Ok(name)
}

fn install_git(workspace: &Path, url: &str) -> Result<String, String> {
    // Clone into a hidden sibling of the install root. It is never a discovery
    // candidate, even if the process dies before cleanup.
    let staging = staging_path(workspace, "git");
    if let Some(parent) = staging.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create the skills dir: {e}"))?;
    }
    let mut child = Command::new("git")
        .args(["clone", "--quiet", "--depth", "1", "--", url])
        .arg(&staging)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("could not run git (is it installed?): {e}"))?;
    let stderr = child.stderr.take().expect("piped stderr");
    let error_reader = std::thread::spawn(move || read_capped(stderr, GIT_ERROR_CAP));
    let deadline = Instant::now() + GIT_CLONE_TIMEOUT;
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                break find_skill_dir(&staging).and_then(|dir| install_path(workspace, &dir));
            }
            Ok(Some(_)) => {
                let detail = error_reader.join().unwrap_or_default();
                let _ = std::fs::remove_dir_all(&staging);
                return Err(format!("git clone failed: {}", detail.trim()));
            }
            Ok(None) => {}
            Err(error) => {
                kill_group(&mut child);
                break Err(format!("cannot wait for git clone: {error}"));
            }
        }
        if INTERRUPTED.load(Ordering::SeqCst) {
            kill_group(&mut child);
            break Err("skill installation canceled by user".to_string());
        }
        if Instant::now() >= deadline {
            kill_group(&mut child);
            break Err(format!(
                "git clone timed out after {}s; check the repository URL and network",
                GIT_CLONE_TIMEOUT.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let _ = error_reader.join();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn staging_path(workspace: &Path, kind: &str) -> PathBuf {
    let serial = STAGING_SERIAL.fetch_add(1, Ordering::Relaxed);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    workspace.join(".noob").join(format!(
        ".skill-{kind}-{}-{stamp:x}-{serial:x}",
        std::process::id(),
    ))
}

fn kill_group(child: &mut std::process::Child) {
    let pid = child.id() as libc::pid_t;
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn read_capped(mut stream: impl Read, cap: usize) -> String {
    let mut kept = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let take = n.min(cap.saturating_sub(kept.len()));
                kept.extend_from_slice(&chunk[..take]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&kept).into_owned()
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
    copy_dir_avoiding(src, dst, dst)
}

fn copy_dir_avoiding(src: &Path, dst: &Path, excluded: &Path) -> std::io::Result<()> {
    if INTERRUPTED.load(Ordering::SeqCst) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "skill installation canceled by user",
        ));
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        if INTERRUPTED.load(Ordering::SeqCst) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "skill installation canceled by user",
            ));
        }
        let entry = entry?;
        // A user may install a skill whose source is the workspace root. The
        // hidden staging directory then sits inside the source tree and must
        // not recursively copy itself.
        if entry.path().starts_with(excluded) || entry.file_name() == OsStr::new(".git") {
            continue;
        }
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_avoiding(&entry.path(), &to, excluded)?;
        } else if ty.is_file() {
            copy_file(&entry.path(), &to)?;
        }
    }
    Ok(())
}

fn copy_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    let mut input = std::fs::File::open(src)?;
    let mut output = std::fs::File::create(dst)?;
    let mut chunk = [0u8; 64 * 1024];
    loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "skill installation canceled by user",
            ));
        }
        let n = input.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        output.write_all(&chunk[..n])?;
    }
    output.flush()?;
    std::fs::set_permissions(dst, input.metadata()?.permissions())
}

/// Read only the fenced metadata needed for discovery and validation. The
/// file must be a real regular file: following a FIFO, device, or symlink here
/// could block startup or consume unbounded memory before cancellation exists.
fn read_frontmatter_file(path: &Path) -> std::io::Result<String> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "SKILL.md must be a regular non-symlink file",
        ));
    }

    let mut file = std::fs::File::open(path)?;
    let mut kept = Vec::with_capacity(8 * 1024);
    let mut chunk = [0u8; 8 * 1024];
    let mut line_start = 0usize;
    let mut line_count = 0usize;
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        for &byte in &chunk[..n] {
            if kept.len() >= FRONTMATTER_BYTE_CAP {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "frontmatter exceeds 64 KiB or has no closing `---` fence",
                ));
            }
            kept.push(byte);
            if byte != b'\n' {
                continue;
            }
            line_count += 1;
            if line_count > 1 && fence_line(&kept[line_start..kept.len() - 1])? {
                return String::from_utf8(kept)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e));
            }
            line_start = kept.len();
        }
    }
    if line_start < kept.len() && fence_line(&kept[line_start..])? {
        return String::from_utf8(kept)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e));
    }
    String::from_utf8(kept).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

fn fence_line(bytes: &[u8]) -> std::io::Result<bool> {
    let line = std::str::from_utf8(bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(line.trim_end() == "---")
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
    let mut lines = text.split('\n').enumerate().peekable();
    if lines.next().map(|(_, line)| line.trim_end()) != Some("---") {
        return Err("no frontmatter: the file must start with a `---` line".to_string());
    }
    let mut fields = HashMap::new();
    while let Some((index, raw_line)) = lines.next() {
        let line = raw_line.trim_end();
        if line == "---" {
            return Ok(Parsed {
                fields,
                body_start: index + 1,
            });
        }
        let line_number = index + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            continue; // nested content under an ignored key
        }
        let Some(colon) = line.find(':') else {
            return Err(format!(
                "line {line_number}: expected `key: value`, got {line:?}"
            ));
        };
        let key = line[..colon].trim_end();
        if key.is_empty()
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(format!("line {line_number}: invalid key {key:?}"));
        }
        let value = line[colon + 1..].trim();
        // A block header may carry a trailing YAML comment (`description: | #
        // keep newlines`), so test only the first token for the indicator.
        let indicator = value.split_whitespace().next().unwrap_or("");
        let parsed = if is_block_indicator(indicator) {
            scan_block(&mut lines, indicator.starts_with('>'))
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
fn scan_block<'a>(
    lines: &mut std::iter::Peekable<impl Iterator<Item = (usize, &'a str)>>,
    folded: bool,
) -> String {
    let mut out = String::new();
    let mut indent = None;
    let mut first = true;
    while let Some((_, raw_line)) = lines.peek() {
        let line = raw_line.trim_end();
        if line == "---" {
            break;
        }
        if !line.is_empty() && !line.starts_with(' ') && !line.starts_with('\t') {
            break;
        }
        let (_, raw_line) = lines.next().expect("peeked line exists");
        let line = raw_line.trim_end();
        let line_indent = line
            .bytes()
            .take_while(|&byte| byte == b' ' || byte == b'\t')
            .count();
        let indent = match indent {
            Some(indent) => indent,
            None if !line.trim().is_empty() => {
                indent = Some(line_indent);
                line_indent
            }
            None => 0,
        };
        let bytes = line.as_bytes();
        let mut cut = 0;
        while cut < indent && cut < bytes.len() && matches!(bytes[cut], b' ' | b'\t') {
            cut += 1;
        }
        let stripped = &line[cut..];
        if folded {
            if stripped.trim().is_empty() {
                out.push('\n');
            } else {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str(stripped.trim_end());
            }
        } else {
            if !first {
                out.push('\n');
            }
            out.push_str(stripped);
        }
        first = false;
    }
    out
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
}
