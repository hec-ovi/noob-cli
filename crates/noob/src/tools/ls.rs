//! ls: one directory's entries, sorted by name, directories marked with a
//! trailing slash. Explicit listing, so gitignore does NOT apply here.

use serde_json::Value;

use super::truncate::{LIST_ENTRY_CAP, list_trailer};
use super::{ToolCtx, ToolOutcome, display_path, opt_str};

pub fn run(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    match run_inner(ctx, args) {
        Ok(out) => out,
        Err(msg) => ToolOutcome::err(msg),
    }
}

fn run_inner(ctx: &ToolCtx, args: &Value) -> Result<ToolOutcome, String> {
    let raw = opt_str(args, "path")?.unwrap_or(".");
    let path = super::guard::resolve_path(&ctx.workspace, raw);
    let shown = display_path(ctx, &path);

    let entries = std::fs::read_dir(&path)
        .map_err(|e| format!("cannot list {shown}: {e}; check the path"))?;
    let mut names: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let mut name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            name.push('/');
        }
        names.push(name);
    }
    names.sort();
    let total = names.len();
    if total == 0 {
        return Ok(ToolOutcome::ok(
            format!("{shown} is empty"),
            format!("ls {shown} (empty)"),
        ));
    }
    let cut = total.min(LIST_ENTRY_CAP);
    let mut content = names[..cut].join("\n");
    if let Some(trailer) = list_trailer("entries", total, cut) {
        content.push('\n');
        content.push_str(&trailer);
    }
    Ok(ToolOutcome::ok(
        content,
        format!("ls {shown} ({total} entries)"),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::test_ctx;
    use super::*;
    use serde_json::json;

    #[test]
    fn sorted_with_dir_slash() {
        let (_t, ctx) = test_ctx();
        std::fs::create_dir(ctx.workspace.join("sub")).unwrap();
        std::fs::write(ctx.workspace.join("b.txt"), "").unwrap();
        std::fs::write(ctx.workspace.join("a.txt"), "").unwrap();
        let out = run(&ctx, &json!({}));
        assert!(!out.is_error);
        assert_eq!(out.content, "a.txt\nb.txt\nsub/");
        assert_eq!(out.summary, "ls . (3 entries)");
    }

    #[test]
    fn entry_cap_appends_the_count_trailer() {
        let (_t, ctx) = test_ctx();
        for i in 0..250 {
            std::fs::write(ctx.workspace.join(format!("f{i:03}")), "").unwrap();
        }
        let out = run(&ctx, &json!({}));
        assert!(out.content.ends_with("250 entries, showing 200; narrow the pattern"));
        assert_eq!(out.content.lines().count(), 201);
    }

    #[test]
    fn missing_dir_is_a_remedy_error() {
        let (_t, ctx) = test_ctx();
        let out = run(&ctx, &json!({"path": "nope"}));
        assert!(out.is_error);
        assert!(out.content.contains("cannot list nope"));
    }

    #[test]
    fn empty_dir_says_so() {
        let (_t, ctx) = test_ctx();
        std::fs::create_dir(ctx.workspace.join("void")).unwrap();
        let out = run(&ctx, &json!({"path": "void"}));
        assert_eq!(out.content, "void is empty");
    }
}
