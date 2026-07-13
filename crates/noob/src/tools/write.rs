//! write: create or replace a whole file, atomically, behind the
//! check-and-set guard. Overwriting a file the model has never read is
//! refused: it would destroy content the model has not seen.

use serde_json::Value;

use super::guard::{FileStamp, atomic_write, check_write_allowed, resolve_path};
use super::{ToolCtx, ToolOutcome, display_path, need_str};

pub fn run(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    match run_inner(ctx, args) {
        Ok(out) => out,
        Err(msg) => ToolOutcome::err(msg),
    }
}

fn run_inner(ctx: &ToolCtx, args: &Value) -> Result<ToolOutcome, String> {
    let raw = need_str(args, "path")?;
    let content = need_str(args, "content")?;
    if let Some(refusal) = ctx.skills_write_refusal(raw) {
        return Err(refusal);
    }
    let path = resolve_path(&ctx.workspace, raw);
    check_write_allowed(ctx.sandbox, &ctx.workspace, &path)?;
    let shown = display_path(ctx, &path);

    if path.is_dir() {
        return Err(format!("{shown} is a directory; write needs a file path"));
    }
    if let Ok(current) = std::fs::read(&path) {
        // The file exists: the staleness rules apply.
        match ctx.seen.get(&path) {
            None => {
                return Err(format!(
                    "{shown} already exists and you have not read it; read it first \
                     so no unseen content is lost"
                ));
            }
            Some(stamp) if stamp != FileStamp::of(&current) => {
                return Err(format!(
                    "{shown} changed on disk since your last read; re-read it"
                ));
            }
            Some(_) => {}
        }
    }
    atomic_write(&path, content.as_bytes())?;
    ctx.seen.record(&path, FileStamp::of(content.as_bytes()));
    Ok(ToolOutcome::ok(
        format!("wrote {shown} ({} bytes)", content.len()),
        format!("write {shown} ({} bytes)", content.len()),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::test_ctx;
    use super::*;
    use serde_json::json;

    #[test]
    fn creates_new_files_and_parents() {
        let (_t, ctx) = test_ctx();
        let out = run(&ctx, &json!({"path": "a/b/f.txt", "content": "hello"}));
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(out.content, "wrote a/b/f.txt (5 bytes)");
        assert_eq!(
            std::fs::read_to_string(ctx.workspace.join("a/b/f.txt")).unwrap(),
            "hello"
        );
    }

    #[test]
    fn refuses_overwrite_of_a_never_read_file() {
        let (_t, ctx) = test_ctx();
        std::fs::write(ctx.workspace.join("f.txt"), "precious").unwrap();
        let out = run(&ctx, &json!({"path": "f.txt", "content": "clobber"}));
        assert!(out.is_error);
        assert!(out.content.contains("you have not read it; read it first"));
        assert_eq!(
            std::fs::read_to_string(ctx.workspace.join("f.txt")).unwrap(),
            "precious"
        );
    }

    #[test]
    fn refuses_stale_overwrite_after_disk_change() {
        let (_t, ctx) = test_ctx();
        let p = ctx.workspace.join("f.txt");
        std::fs::write(&p, "v1").unwrap();
        super::super::read::run(&ctx, &json!({"path": "f.txt"}));
        std::fs::write(&p, "v2-from-elsewhere").unwrap();
        let out = run(&ctx, &json!({"path": "f.txt", "content": "v3"}));
        assert!(out.is_error);
        assert!(
            out.content
                .contains("changed on disk since your last read; re-read it")
        );
    }

    #[test]
    fn overwrite_after_read_succeeds_and_updates_the_stamp() {
        let (_t, ctx) = test_ctx();
        let p = ctx.workspace.join("f.txt");
        std::fs::write(&p, "v1").unwrap();
        super::super::read::run(&ctx, &json!({"path": "f.txt"}));
        let out = run(&ctx, &json!({"path": "f.txt", "content": "v2"}));
        assert!(!out.is_error, "{}", out.content);
        // A second write without re-reading is fine: we know what we wrote.
        let out = run(&ctx, &json!({"path": "f.txt", "content": "v3"}));
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "v3");
    }

    #[test]
    fn workspace_mode_refuses_outside_writes() {
        let (_t, mut ctx) = test_ctx();
        ctx.sandbox = super::super::guard::Sandbox::Workspace;
        let out = run(&ctx, &json!({"path": "/tmp/outside.txt", "content": "x"}));
        assert!(out.is_error);
        assert!(out.content.contains("outside the workspace"));
    }

    #[test]
    fn skills_dir_write_is_refused_unless_the_target_was_approved() {
        let (_t, ctx) = test_ctx();
        std::fs::create_dir_all(ctx.workspace.join(".claude/skills/x")).unwrap();
        let args = json!({"path": ".claude/skills/x/SKILL.md", "content": "y"});
        // Unapproved: refused at execution time, nothing written.
        let out = run(&ctx, &args);
        assert!(out.is_error);
        assert!(out.content.contains("refused"), "{}", out.content);
        assert!(!ctx.workspace.join(".claude/skills/x/SKILL.md").exists());
        // Approve exactly this real target (what the agent gate records on
        // grant) and the write proceeds.
        let target =
            super::super::guard::skill_write_target(&ctx.workspace, ".claude/skills/x/SKILL.md")
                .unwrap();
        ctx.approved_skill_writes.lock().unwrap().insert(target, 1);
        let out = run(&ctx, &args);
        assert!(!out.is_error, "{}", out.content);
        assert!(ctx.workspace.join(".claude/skills/x/SKILL.md").exists());
        // The confirmation is scoped to that one operation, not the rest of
        // the session. A second write needs a fresh explicit grant.
        let out = run(&ctx, &args);
        assert!(out.is_error);
        assert!(out.content.contains("refused"), "{}", out.content);
    }
}
