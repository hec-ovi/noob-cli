//! On-the-fly skill install and remove (the /skills command family; REPL
//! only): local paths, git URLs, and the `owner/repo` GitHub shorthand, all
//! staged beside the install root and published with one rename.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use noob_provider::http::INTERRUPTED;

use super::frontmatter::{parse, read_frontmatter_file, validate};
use crate::exec::kill_group;

const GIT_CLONE_TIMEOUT: Duration = Duration::from_secs(120);
const GIT_ERROR_CAP: usize = 64 * 1024;
static STAGING_SERIAL: AtomicU64 = AtomicU64::new(0);

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
    sweep_stale_staging(workspace);
    match git_url_for(source, Path::new(source).exists()) {
        Some(url) => install_git(workspace, &url),
        None => install_path(workspace, Path::new(source)),
    }
}

/// Remove sibling staging leftovers (`.noob/.skill-*`) whose embedded pid is
/// no longer alive: a killed install can never clean up after itself, so the
/// next install does. Entries whose pid is still alive (a concurrent install)
/// or whose name does not parse are kept.
fn sweep_stale_staging(workspace: &Path) {
    let Ok(entries) = std::fs::read_dir(workspace.join(".noob")) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(rest) = name.to_str().and_then(|n| n.strip_prefix(".skill-")) else {
            continue;
        };
        // `.skill-{kind}-{pid}-{stamp:x}-{serial:x}`; the pid segment never
        // contains a hyphen, so a parse failure means "not ours: keep".
        let Some(pid) = rest
            .split('-')
            .nth(1)
            .and_then(|p| p.parse::<libc::pid_t>().ok())
            .filter(|&p| p > 0)
        else {
            continue;
        };
        let dead = unsafe { libc::kill(pid, 0) } != 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
        if dead {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The clone URL for a git-shaped source: explicit URLs pass through, and an
/// `owner/repo` GitHub shorthand (the registry shape `npx skills add` uses)
/// expands, but only when nothing with that name exists on disk, so a real
/// `owner/repo`-shaped local directory always wins. `None` means a local path.
pub(super) fn git_url_for(source: &str, exists_locally: bool) -> Option<String> {
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

/// Remove an installed skill's directory. Confined to the workspace install
/// root: only a directory DIRECTLY under `.noob/skills` is ever deleted. Any
/// other discovered skill (a configured NOOB_SKILL_PATHS entry, a `.claude`/
/// `.agents` ecosystem dir, a global skill, a workspace-root SKILL.md whose
/// dir IS the workspace) can point at real project source, so `/skills
/// remove` refuses it by name instead of deleting code.
pub fn remove(workspace: &Path, skill_dir: &Path) -> Result<(), String> {
    let refusal = |dir: &Path| {
        format!(
            "{} is not under {}; only skills installed there (/skills add) can be \
             removed, delete anything else by hand",
            dir.display(),
            install_root(workspace).display()
        )
    };
    // No install root means nothing was ever installed, so nothing to remove.
    let Ok(root) = install_root(workspace).canonicalize() else {
        return Err(refusal(skill_dir));
    };
    let dir = skill_dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", skill_dir.display()))?;
    if dir.parent() != Some(root.as_path()) {
        return Err(refusal(&dir));
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
