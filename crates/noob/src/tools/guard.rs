//! File-safety guards shared by the mutating tools: check-and-set staleness
//! (fnv1a64 of the last-seen content), workspace sandbox path policy, and
//! atomic writes (temp + fsync + rename; no partial write is ever visible).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// fnv1a64: tiny, dependency-free, good enough to detect any edit.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    fnv1a64_extend(0xcbf29ce484222325, bytes)
}

/// Continue an FNV-1a hash over another chunk. File tools use this to stamp
/// large files without first allocating their entire contents.
pub fn fnv1a64_extend(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub len: u64,
    pub hash: u64,
}

impl FileStamp {
    pub fn of(bytes: &[u8]) -> FileStamp {
        FileStamp {
            len: bytes.len() as u64,
            hash: fnv1a64(bytes),
        }
    }
}

/// Two states total; no permission-rule DSL. The container is the wall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sandbox {
    /// Inside a container (or `--yolo`): tools run unrestricted.
    Container,
    /// Outside a container: write/edit refuse paths outside the workspace.
    Workspace,
}

/// Held across one write/edit call. Locking the workspace directory inode
/// works across the parent and child processes, and across containers sharing
/// the same bind mount, without adding lock files to the user's project.
pub struct WorkspaceWriteLease {
    _directory: std::fs::File,
}

#[derive(Debug)]
pub enum WorkspaceLeaseError {
    Busy,
    Canceled,
    Io(String),
}

pub fn workspace_write_lease(
    workspace: &Path,
    wait: Duration,
    interrupted: impl Fn() -> bool,
) -> Result<WorkspaceWriteLease, WorkspaceLeaseError> {
    // Cancellation is a mutation barrier, not only a way out of lock
    // contention. A tool canceled before dispatch must not acquire a lease
    // and then continue into write/edit merely because the lock was free.
    if interrupted() {
        return Err(WorkspaceLeaseError::Canceled);
    }
    let directory = std::fs::File::open(workspace)
        .map_err(|error| WorkspaceLeaseError::Io(error.to_string()))?;
    let deadline = Instant::now() + wait;
    loop {
        let rc = unsafe { libc::flock(directory.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc == 0 {
            // Close the check-to-acquire window. Returning drops `directory`,
            // which releases the just-acquired flock before any tool runs.
            if interrupted() {
                return Err(WorkspaceLeaseError::Canceled);
            }
            return Ok(WorkspaceWriteLease {
                _directory: directory,
            });
        }
        let error = std::io::Error::last_os_error();
        if !error
            .raw_os_error()
            .is_some_and(|code| code == libc::EWOULDBLOCK || code == libc::EAGAIN)
        {
            return Err(WorkspaceLeaseError::Io(error.to_string()));
        }
        if interrupted() {
            return Err(WorkspaceLeaseError::Canceled);
        }
        if Instant::now() >= deadline {
            return Err(WorkspaceLeaseError::Busy);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Session-scoped registry of last-known file content, keyed by resolved
/// path. `read` records, successful `write`/`edit` update; a mismatch means
/// the file changed on disk behind the model's back.
pub struct SeenFiles {
    map: Mutex<HashMap<PathBuf, FileStamp>>,
}

impl SeenFiles {
    pub fn new() -> SeenFiles {
        SeenFiles {
            map: Mutex::new(HashMap::new()),
        }
    }

    pub fn record(&self, path: &Path, stamp: FileStamp) {
        self.map.lock().unwrap().insert(path.to_path_buf(), stamp);
    }

    pub fn get(&self, path: &Path) -> Option<FileStamp> {
        self.map.lock().unwrap().get(path).copied()
    }

    /// Every path seen this session, sorted. Compaction pins this list
    /// deterministically (the harness, not the summarizer, owns file facts).
    pub fn paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self.map.lock().unwrap().keys().cloned().collect();
        paths.sort();
        paths
    }
}

/// Resolve a tool `path` argument: absolute or workspace-relative, lexically
/// normalized (`.` and `..` folded without touching the filesystem).
pub fn resolve_path(workspace: &Path, raw: &str) -> PathBuf {
    let p = Path::new(raw);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace.join(p)
    };
    let mut out = PathBuf::new();
    for comp in joined.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Workspace-mode write gate. `path` must already be resolved. Checks the
/// lexical path AND the canonicalized deepest existing ancestor, so a
/// symlink inside the tree cannot smuggle a write outside it.
pub fn check_write_allowed(sandbox: Sandbox, workspace: &Path, path: &Path) -> Result<(), String> {
    if sandbox == Sandbox::Container {
        return Ok(());
    }
    let refusal = |p: &Path| {
        format!(
            "refused: {} is outside the workspace {}; without a container sandbox, \
             write and edit are limited to the workspace tree (run with --yolo to lift this)",
            p.display(),
            workspace.display()
        )
    };
    if !path.starts_with(workspace) {
        return Err(refusal(path));
    }
    // Symlink escape: canonicalize the deepest existing ancestor and re-check.
    let real = resolve_real(path)?;
    if !real.starts_with(workspace) {
        return Err(refusal(&real));
    }
    Ok(())
}

/// `**/skills/**`: true when any ancestor directory component of `path` is
/// named exactly `skills`. Agent-created skills are persistent injection
/// vectors, so write/edit into such a path needs explicit confirmation in
/// EVERY mode, container included (headless surfaces degrade to deny).
pub fn in_skills_dir(path: &Path) -> bool {
    let mut dirs: Vec<&std::ffi::OsStr> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    dirs.pop(); // the final component is the file itself, not an ancestor
    // Case-insensitive: a case-preserving filesystem must not let SKILLS/
    // dodge the gate.
    dirs.iter().any(|name| {
        name.to_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("skills"))
    })
}

/// If a write to `raw` would land inside a skills directory, return the
/// filesystem-real target path (the key the write gate approves and the
/// write/edit tools re-check at execution time). Uses the real path so a
/// symlinked directory into a skills tree is caught, and falls back to the
/// lexical form when the real path cannot be resolved.
pub fn skill_write_target(workspace: &Path, raw: &str) -> Option<PathBuf> {
    let resolved = resolve_path(workspace, raw);
    let real = resolve_real(&resolved).unwrap_or_else(|_| resolved.clone());
    (in_skills_dir(&real) || in_skills_dir(&resolved)).then_some(real)
}

/// The filesystem-real form of a (possibly not-yet-existing) path: the
/// deepest existing ancestor canonicalized, the remainder re-appended.
/// This is what a write through a symlinked directory actually lands on.
pub fn resolve_real(path: &Path) -> Result<PathBuf, String> {
    let mut probe = path.to_path_buf();
    let mut rest = Vec::new();
    while !probe.exists() {
        match probe.file_name() {
            Some(name) => {
                rest.push(name.to_os_string());
                probe.pop();
            }
            None => break,
        }
    }
    let canon = probe
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", probe.display()))?;
    let mut real = canon;
    for name in rest.iter().rev() {
        real.push(name);
    }
    Ok(real)
}

/// Atomic write: temp file in the same directory, fsync, rename over the
/// target. Preserves the target's mode when it already exists. A symlink
/// destination is written THROUGH, never replaced: the staleness stamp and
/// mode checks all followed the target, and renaming onto the link itself
/// would silently swap the link for a regular file. A dangling link is an
/// error, not a link replacement.
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<(), String> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        let real = path.canonicalize().map_err(|e| {
            format!(
                "cannot write {}: it is a symlink that cannot be resolved: {e}",
                path.display()
            )
        })?;
        return atomic_write(&real, content);
    }
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let dir = dir.ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(dir)
        .map_err(|e| format!("cannot create directory {}: {e}", dir.display()))?;
    let mode = fs::metadata(path).ok().map(|m| {
        use std::os::unix::fs::MetadataExt;
        m.mode()
    });
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("noob-tmp");
    // Uniquified per process+attempt so parallel sessions cannot collide.
    let tmp = dir.join(format!(
        ".{file_name}.noob-{}-{:x}",
        std::process::id(),
        fnv1a64(content) ^ content.len() as u64
    ));
    let write = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(content)?;
        f.sync_all()?;
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        drop(f);
        fs::rename(&tmp, path)
    })();
    if let Err(e) = write {
        let _ = fs::remove_file(&tmp);
        return Err(format!("cannot write {}: {e}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canceled_before_lock_never_receives_a_workspace_lease() {
        let tmp = tempfile::tempdir().unwrap();
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let result = workspace_write_lease(tmp.path(), Duration::ZERO, || {
            checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            true
        });
        assert!(matches!(result, Err(WorkspaceLeaseError::Canceled)));
        assert_eq!(checks.load(std::sync::atomic::Ordering::SeqCst), 1);

        // No lease leaked from the canceled call.
        assert!(workspace_write_lease(tmp.path(), Duration::ZERO, || false).is_ok());
    }

    #[test]
    fn cancellation_immediately_after_acquisition_releases_the_lease() {
        let tmp = tempfile::tempdir().unwrap();
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let result = workspace_write_lease(tmp.path(), Duration::ZERO, || {
            checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0
        });
        assert!(matches!(result, Err(WorkspaceLeaseError::Canceled)));
        assert_eq!(checks.load(std::sync::atomic::Ordering::SeqCst), 2);

        // The canceled path dropped the directory fd and its flock.
        assert!(workspace_write_lease(tmp.path(), Duration::ZERO, || false).is_ok());
    }

    #[test]
    fn fnv_matches_reference_vectors() {
        // Published FNV-1a 64-bit vectors.
        assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a64(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn resolve_folds_dots_and_joins_relative() {
        let ws = Path::new("/work");
        assert_eq!(
            resolve_path(ws, "src/main.rs"),
            PathBuf::from("/work/src/main.rs")
        );
        assert_eq!(resolve_path(ws, "./a/../b"), PathBuf::from("/work/b"));
        assert_eq!(
            resolve_path(ws, "/etc/passwd"),
            PathBuf::from("/etc/passwd")
        );
        assert_eq!(resolve_path(ws, "../../etc/x"), PathBuf::from("/etc/x"));
    }

    #[test]
    fn workspace_mode_refuses_outside_and_container_allows() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();
        let outside = PathBuf::from("/etc/nope");
        assert!(check_write_allowed(Sandbox::Workspace, &ws, &outside).is_err());
        assert!(check_write_allowed(Sandbox::Container, &ws, &outside).is_ok());
        let inside = ws.join("new/file.txt");
        assert!(check_write_allowed(Sandbox::Workspace, &ws, &inside).is_ok());
    }

    #[test]
    fn symlink_escape_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let ws = ws.canonicalize().unwrap();
        std::os::unix::fs::symlink(&elsewhere, ws.join("link")).unwrap();
        let target = ws.join("link/escape.txt");
        let err = check_write_allowed(Sandbox::Workspace, &ws, &target).unwrap_err();
        assert!(err.contains("outside the workspace"), "{err}");
    }

    #[test]
    fn skills_dir_predicate_matches_ancestor_dirs_only() {
        for hit in [
            "/work/.claude/skills/pdf/SKILL.md",
            "/work/.noob/skills/x/nested/helper.py",
            "/config/skills/a/SKILL.md",
            "/work/skills/thing.md",
        ] {
            assert!(in_skills_dir(Path::new(hit)), "{hit} must be gated");
        }
        for miss in [
            "/work/src/main.rs",
            "/work/skills",             // a file named skills, not a dir of it
            "/work/my-skills/notes.md", // exact component match only
            "/work/docs/skillset/a.md",
        ] {
            assert!(!in_skills_dir(Path::new(miss)), "{miss} must not be gated");
        }
        // Case-preserving filesystems must not dodge the gate by casing.
        assert!(in_skills_dir(Path::new("/work/.claude/SKILLS/x/SKILL.md")));
        assert!(in_skills_dir(Path::new("/work/Skills/thing.md")));
    }

    #[test]
    fn resolve_real_follows_symlinked_dirs_for_nonexistent_leaves() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();
        let target = ws.join(".claude/skills/pdf");
        std::fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, ws.join("innocent")).unwrap();
        // A write to innocent/new.md really lands inside the skills dir.
        let real = resolve_real(&ws.join("innocent/new.md")).unwrap();
        assert_eq!(real, target.join("new.md"));
        assert!(in_skills_dir(&real));
        assert!(
            !in_skills_dir(&ws.join("innocent/new.md")),
            "lexical form alone is blind"
        );
    }

    #[test]
    fn atomic_write_preserves_mode_and_replaces_content() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("f.sh");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        atomic_write(&path, b"new content").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new content");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        // No temp litter left behind.
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    #[test]
    fn atomic_write_through_a_symlink_updates_the_target_and_keeps_the_link() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("real.txt");
        std::fs::write(&target, "old").unwrap();
        let link = tmp.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        atomic_write(&link, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the link must survive as a link, not become a regular file"
        );
    }

    #[test]
    fn atomic_write_refuses_a_dangling_symlink_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let dangling = tmp.path().join("dangling.txt");
        std::os::unix::fs::symlink(tmp.path().join("missing.txt"), &dangling).unwrap();
        let err = atomic_write(&dangling, b"x").unwrap_err();
        assert!(err.contains("cannot be resolved"), "{err}");
        assert!(
            std::fs::symlink_metadata(&dangling)
                .unwrap()
                .file_type()
                .is_symlink(),
            "a dangling link must not be silently replaced"
        );
        assert!(!tmp.path().join("missing.txt").exists());
    }

    #[test]
    fn atomic_write_creates_parent_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("deep/nested/f.txt");
        atomic_write(&path, b"x").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "x");
    }
}
