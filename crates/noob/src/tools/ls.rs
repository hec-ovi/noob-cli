//! ls: one directory's entries, sorted by name, directories marked with a
//! trailing slash. Explicit listing, so gitignore does NOT apply here.
//!
//! The content opens with a `<dir>:` header line (the `ls -R` convention) so the
//! model always sees the base path next to the bare names: a live session showed
//! a small model reading `contract.md` out of an `ls /config` listing and then
//! asking for `/contract.md`, because nothing in the result carried the prefix.

use std::sync::atomic::Ordering;

use noob_provider::http::INTERRUPTED;
use serde_json::Value;

use super::truncate::{LIST_ENTRY_CAP, list_trailer};
use super::{ToolCtx, ToolOutcome, display_path, opt_str};

const CANCELED: &str = "ls canceled by user";

pub fn run(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    run_with(ctx, args, || INTERRUPTED.load(Ordering::SeqCst))
}

fn run_with(ctx: &ToolCtx, args: &Value, interrupted: impl Fn() -> bool) -> ToolOutcome {
    match run_inner(ctx, args, interrupted) {
        Ok(out) => out,
        Err(msg) if msg == CANCELED => ToolOutcome::canceled_with(msg),
        Err(msg) => ToolOutcome::err(msg),
    }
}

fn run_inner(
    ctx: &ToolCtx,
    args: &Value,
    interrupted: impl Fn() -> bool,
) -> Result<ToolOutcome, String> {
    let raw = opt_str(args, "path")?.unwrap_or(".");
    let path = super::guard::resolve_path(&ctx.workspace, raw);
    let shown = display_path(ctx, &path);

    let entries = std::fs::read_dir(&path)
        .map_err(|e| format!("cannot list {shown}: {e}; check the path"))?;
    let mut names: Vec<String> = Vec::with_capacity(LIST_ENTRY_CAP);
    let mut total = 0usize;
    for entry in entries.flatten() {
        if interrupted() {
            return Err(CANCELED.to_string());
        }
        let mut name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            name.push('/');
        }
        total = total.saturating_add(1);
        if names.len() < LIST_ENTRY_CAP {
            names.push(name);
        } else if let Some((largest, _)) = names.iter().enumerate().max_by(|a, b| a.1.cmp(b.1))
            && name < names[largest]
        {
            names[largest] = name;
        }
    }
    names.sort();
    if total == 0 {
        return Ok(ToolOutcome::ok(
            format!("{shown} is empty"),
            format!("ls {shown} (empty)"),
        ));
    }
    let shown_count = names.len();
    let mut content = format!("{shown}:\n");
    content.push_str(&names.join("\n"));
    if let Some(trailer) = list_trailer("entries", total, shown_count) {
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
        assert_eq!(out.content, ".:\na.txt\nb.txt\nsub/");
        assert_eq!(out.summary, "ls . (3 entries)");
    }

    #[test]
    fn listing_anchors_the_names_to_the_asked_dir() {
        // A model must never have to guess the base path of a bare name: the
        // header line carries the directory it asked for.
        let (_t, ctx) = test_ctx();
        std::fs::create_dir(ctx.workspace.join("cfg")).unwrap();
        std::fs::write(ctx.workspace.join("cfg/contract.md"), "").unwrap();
        let out = run(&ctx, &json!({"path": "cfg"}));
        assert_eq!(out.content, "cfg:\ncontract.md");
    }

    #[test]
    fn entry_cap_appends_the_count_trailer() {
        let (_t, ctx) = test_ctx();
        for i in 0..250 {
            std::fs::write(ctx.workspace.join(format!("f{i:03}")), "").unwrap();
        }
        let out = run(&ctx, &json!({}));
        assert!(
            out.content
                .ends_with("250 entries, showing 200; narrow the pattern")
        );
        assert_eq!(out.content.lines().count(), 202);
        assert!(out.content.starts_with(".:\nf000\n"));
        assert!(out.content.contains("f199\n250 entries"));
        assert!(!out.content.contains("f200\n"));
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

    #[test]
    fn directory_walk_cancellation_is_structural() {
        let (_t, ctx) = test_ctx();
        std::fs::write(ctx.workspace.join("a.txt"), "").unwrap();
        let out = run_with(&ctx, &json!({}), || true);
        assert!(out.canceled);
        assert!(out.is_error);
    }
}
