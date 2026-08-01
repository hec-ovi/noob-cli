//! write: create or replace a whole file, atomically, behind the
//! check-and-set guard. Overwriting a file the model has never read is
//! refused: it would destroy content the model has not seen.

use serde_json::Value;

use super::guard::{FileStamp, atomic_write, check_write_allowed, resolve_path};
use super::{Core, FsState, WriteGrants, ToolOutcome, display_path, need_str};

pub fn run(core: &Core, fs: &FsState, grants: &WriteGrants, args: &Value) -> ToolOutcome {
    match run_inner(core, fs, grants, args) {
        Ok(out) => out,
        Err(msg) => ToolOutcome::err(msg),
    }
}

fn run_inner(core: &Core, fs: &FsState, grants: &WriteGrants, args: &Value) -> Result<ToolOutcome, String> {
    let raw = need_str(args, "path")?;
    let content = need_str(args, "content")?;
    if let Some(refusal) = grants.refusal(&core.workspace, raw) {
        return Err(refusal);
    }
    let path = resolve_path(&core.workspace, raw);
    check_write_allowed(core.sandbox, &core.workspace, &path)?;
    let shown = display_path(&core.workspace, &path);

    if path.is_dir() {
        return Err(format!("{shown} is a directory; write needs a file path"));
    }
    // Only a confirmed-absent file skips the read-before-write guard; any
    // other read failure (permissions, EIO) must not silently authorize an
    // overwrite of content the model has never seen.
    let current = match std::fs::read(&path) {
        Ok(current) => Some(current),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {shown} before overwriting it: {e}")),
    };
    if let Some(current) = current.as_deref() {
        // The file exists: the staleness rules apply.
        match fs.seen.get(&path) {
            None => {
                return Err(format!(
                    "{shown} already exists and you have not read it; read it first \
                     so no unseen content is lost"
                ));
            }
            Some(stamp) if stamp != FileStamp::of(current) => {
                return Err(format!(
                    "{shown} changed on disk since your last read; re-read it"
                ));
            }
            Some(_) => {}
        }
    }
    // Replacing a file the model already had costs one full regeneration of
    // every byte, whether or not most of them changed. Say so, with the real
    // numbers. In a local bake-off one task rewrote the same file four times
    // (21,480 then 15,355 then 12,276 then 8,936 bytes) where an edit would
    // have sent the changed span; that single behaviour was the largest share
    // of the run's generated tokens.
    let rewrite = current
        .as_deref()
        .map(|before| rewrite_note(before, content.as_bytes()));

    atomic_write(&path, content.as_bytes())?;
    fs.seen
        .record_written(&path, FileStamp::of(content.as_bytes()));
    grants.consume(&core.workspace, raw);
    // After the write, so nothing is announced that did not land. A file that
    // did not exist has no before side; one that is not text is reported as
    // what it looked like, which is the honest answer for a diff view.
    if core.emitter.is_on() {
        let before = current
            .as_deref()
            .map(String::from_utf8_lossy)
            .unwrap_or_default();
        core.emitter.send(crate::emit::file_edit(
            shown.clone(),
            &before,
            content,
            crate::emit::current_call(),
        ));
    }
    Ok(ToolOutcome::ok(
        format!(
            "wrote {shown} ({} bytes){}",
            content.len(),
            rewrite.unwrap_or_default()
        ),
        format!("write {shown} ({} bytes)", content.len()),
    ))
}

/// How much of a replaced file actually changed, by line. Cheap and
/// order-insensitive: the point is the ratio, not a diff. Silent when the
/// rewrite genuinely replaced most of the file, so it reads as information
/// about waste rather than a scolding on every legitimate rewrite.
fn rewrite_note(before: &[u8], after: &[u8]) -> String {
    let (Ok(before), Ok(after)) = (std::str::from_utf8(before), std::str::from_utf8(after)) else {
        return String::new();
    };
    let old_lines: std::collections::HashSet<&str> = before.lines().collect();
    let new_lines: Vec<&str> = after.lines().collect();
    if new_lines.is_empty() {
        return String::new();
    }
    let kept = new_lines.iter().filter(|l| old_lines.contains(*l)).count();
    // Only speak up when the rewrite was mostly a copy of what was there.
    if kept * 4 < new_lines.len() * 3 {
        return String::new();
    }
    let changed = new_lines.len() - kept;
    format!(
        ", replacing a file that already existed; {changed} of {} lines differ, \
         so edit would have sent less",
        new_lines.len()
    )
}

#[cfg(test)]
mod tests {
    use super::super::test_ctx;
    use super::*;
    use serde_json::json;

    #[test]
    fn creates_new_files_and_parents() {
        let (_t, ctx) = test_ctx();
        let out = run(&ctx.core, &ctx.fs, &ctx.grants, &json!({"path": "a/b/f.txt", "content": "hello"}));
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(out.content, "wrote a/b/f.txt (5 bytes)");
        assert_eq!(
            std::fs::read_to_string(ctx.core.workspace.join("a/b/f.txt")).unwrap(),
            "hello"
        );
    }

    /// Rewriting a file to change a little of it says what that cost. This is
    /// the largest single waste seen in the local bake-off: one task wrote the
    /// same file four times rather than editing it.
    #[test]
    fn a_mostly_copied_rewrite_says_edit_would_have_sent_less() {
        let (_t, ctx) = test_ctx();
        let before: String = (1..=40).map(|i| format!("line {i}\n")).collect();
        let out = run(&ctx.core, &ctx.fs, &ctx.grants, &json!({"path": "f.txt", "content": before}));
        assert!(!out.is_error, "{}", out.content);
        // Change two lines out of forty, but send the whole file.
        let after: String = (1..=40)
            .map(|i| {
                if i == 7 || i == 9 {
                    format!("CHANGED {i}\n")
                } else {
                    format!("line {i}\n")
                }
            })
            .collect();
        let out = run(&ctx.core, &ctx.fs, &ctx.grants, &json!({"path": "f.txt", "content": after}));
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("2 of 40 lines differ"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("edit would have sent less"),
            "{}",
            out.content
        );
        // The one-line summary stays clean for the surfaces.
        assert!(!out.summary.contains("differ"), "{}", out.summary);
    }

    /// A genuine full replacement is not nagged at.
    #[test]
    fn a_real_rewrite_gets_no_note() {
        let (_t, ctx) = test_ctx();
        run(
            &ctx.core,
            &ctx.fs,
            &ctx.grants,
            &json!({"path": "f.txt", "content": "alpha\nbeta\ngamma\n"}),
        );
        let out = run(
            &ctx.core,
            &ctx.fs,
            &ctx.grants,
            &json!({"path": "f.txt", "content": "totally\ndifferent\nthing\n"}),
        );
        assert!(!out.is_error, "{}", out.content);
        assert!(!out.content.contains("differ"), "{}", out.content);
    }

    #[test]
    fn a_brand_new_file_gets_no_note() {
        let (_t, ctx) = test_ctx();
        let out = run(&ctx.core, &ctx.fs, &ctx.grants, &json!({"path": "new.txt", "content": "one\ntwo\n"}));
        assert_eq!(out.content, "wrote new.txt (8 bytes)");
    }

    #[test]
    fn refuses_overwrite_of_a_never_read_file() {
        let (_t, ctx) = test_ctx();
        std::fs::write(ctx.core.workspace.join("f.txt"), "precious").unwrap();
        let out = run(&ctx.core, &ctx.fs, &ctx.grants, &json!({"path": "f.txt", "content": "clobber"}));
        assert!(out.is_error);
        assert!(out.content.contains("you have not read it; read it first"));
        assert_eq!(
            std::fs::read_to_string(ctx.core.workspace.join("f.txt")).unwrap(),
            "precious"
        );
    }

    #[test]
    fn unreadable_existing_file_is_an_error_not_a_silent_overwrite() {
        // Root reads through any mode; the guard is only observable unprivileged.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let (_t, ctx) = test_ctx();
        let p = ctx.core.workspace.join("f.txt");
        std::fs::write(&p, "precious").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o200)).unwrap();
        let out = run(&ctx.core, &ctx.fs, &ctx.grants, &json!({"path": "f.txt", "content": "clobber"}));
        assert!(out.is_error, "{}", out.content);
        assert!(
            out.content.contains("cannot read f.txt before overwriting"),
            "{}",
            out.content
        );
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "precious");
    }

    #[test]
    fn refuses_stale_overwrite_after_disk_change() {
        let (_t, ctx) = test_ctx();
        let p = ctx.core.workspace.join("f.txt");
        std::fs::write(&p, "v1").unwrap();
        super::super::read::run(&ctx.core, &ctx.fs, &json!({"path": "f.txt"}));
        std::fs::write(&p, "v2-from-elsewhere").unwrap();
        let out = run(&ctx.core, &ctx.fs, &ctx.grants, &json!({"path": "f.txt", "content": "v3"}));
        assert!(out.is_error);
        assert!(
            out.content
                .contains("changed on disk since your last read; re-read it")
        );
    }

    #[test]
    fn overwrite_after_read_succeeds_and_updates_the_stamp() {
        let (_t, ctx) = test_ctx();
        let p = ctx.core.workspace.join("f.txt");
        std::fs::write(&p, "v1").unwrap();
        super::super::read::run(&ctx.core, &ctx.fs, &json!({"path": "f.txt"}));
        let out = run(&ctx.core, &ctx.fs, &ctx.grants, &json!({"path": "f.txt", "content": "v2"}));
        assert!(!out.is_error, "{}", out.content);
        // A second write without re-reading is fine: we know what we wrote.
        let out = run(&ctx.core, &ctx.fs, &ctx.grants, &json!({"path": "f.txt", "content": "v3"}));
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "v3");
    }

    #[test]
    fn workspace_mode_refuses_outside_writes() {
        let (_t, mut ctx) = test_ctx();
        ctx.core.sandbox = super::super::guard::Sandbox::Workspace;
        let out = run(&ctx.core, &ctx.fs, &ctx.grants, &json!({"path": "/tmp/outside.txt", "content": "x"}));
        assert!(out.is_error);
        assert!(out.content.contains("outside the workspace"));
    }

    #[test]
    fn skills_dir_write_is_refused_unless_the_target_was_approved() {
        let (_t, ctx) = test_ctx();
        std::fs::create_dir_all(ctx.core.workspace.join(".claude/skills/x")).unwrap();
        let args = json!({"path": ".claude/skills/x/SKILL.md", "content": "y"});
        // Unapproved: refused at execution time, nothing written.
        let out = run(&ctx.core, &ctx.fs, &ctx.grants, &args);
        assert!(out.is_error);
        assert!(out.content.contains("refused"), "{}", out.content);
        assert!(!ctx.core.workspace.join(".claude/skills/x/SKILL.md").exists());
        // Approve exactly this real target (what the agent gate records on
        // grant) and the write proceeds.
        let target =
            super::super::guard::skill_write_target(&ctx.core.workspace, ".claude/skills/x/SKILL.md")
                .unwrap();
        ctx.grants.grant(target);
        let out = run(&ctx.core, &ctx.fs, &ctx.grants, &args);
        assert!(!out.is_error, "{}", out.content);
        assert!(ctx.core.workspace.join(".claude/skills/x/SKILL.md").exists());
        // The confirmation is scoped to that one operation, not the rest of
        // the session. A second write needs a fresh explicit grant.
        let out = run(&ctx.core, &ctx.fs, &ctx.grants, &args);
        assert!(out.is_error);
        assert!(out.content.contains("refused"), "{}", out.content);
    }
}
