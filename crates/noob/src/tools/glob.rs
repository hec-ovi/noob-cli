//! glob: files matching a gitignore-style pattern, newest first. Matching
//! inside node_modules/target poisons a 131k window faster than anything
//! else, so the walk respects .gitignore.

use std::time::SystemTime;

use serde_json::Value;

use super::truncate::{LIST_ENTRY_CAP, list_trailer};
use super::{ToolCtx, ToolOutcome, need_str};

pub fn run(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    match run_inner(ctx, args) {
        Ok(out) => out,
        Err(msg) => ToolOutcome::err(msg),
    }
}

fn run_inner(ctx: &ToolCtx, args: &Value) -> Result<ToolOutcome, String> {
    let pattern = need_str(args, "pattern")?;
    let mut over = ignore::overrides::OverrideBuilder::new(&ctx.workspace);
    over.add(pattern).map_err(|e| {
        format!("bad glob pattern {pattern:?}: {e}; use gitignore syntax, e.g. src/**/*.rs")
    })?;
    let over = over
        .build()
        .map_err(|e| format!("bad glob pattern {pattern:?}: {e}"))?;

    let mut hits: Vec<(SystemTime, String)> = Vec::new();
    let walk = ignore::WalkBuilder::new(&ctx.workspace)
        .overrides(over)
        .sort_by_file_path(|a, b| a.cmp(b))
        .hidden(false)
        // Honor .gitignore even when the workspace is not a git repo yet.
        .require_git(false)
        .build();
    for entry in walk.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let mtime = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let rel = entry
            .path()
            .strip_prefix(&ctx.workspace)
            .unwrap_or(entry.path());
        hits.push((mtime, rel.display().to_string()));
    }
    // Newest first; equal mtimes fall back to the path order from the walk,
    // kept stable by the sort.
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    let total = hits.len();
    if total == 0 {
        return Ok(ToolOutcome::ok(
            format!("no files match {pattern:?}; try a broader pattern or ls"),
            format!("glob {pattern} (0 files)"),
        ));
    }
    let cut = total.min(LIST_ENTRY_CAP);
    let mut content = hits[..cut]
        .iter()
        .map(|(_, p)| p.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(trailer) = list_trailer("files", total, cut) {
        content.push('\n');
        content.push_str(&trailer);
    }
    Ok(ToolOutcome::ok(
        content,
        format!("glob {pattern} ({total} files)"),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::test_ctx;
    use super::*;
    use serde_json::json;

    #[test]
    fn matches_recursively_and_respects_gitignore() {
        let (_t, ctx) = test_ctx();
        std::fs::create_dir_all(ctx.workspace.join("src/deep")).unwrap();
        std::fs::create_dir_all(ctx.workspace.join("target")).unwrap();
        std::fs::write(ctx.workspace.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(ctx.workspace.join("src/a.rs"), "").unwrap();
        std::fs::write(ctx.workspace.join("src/deep/b.rs"), "").unwrap();
        std::fs::write(ctx.workspace.join("target/junk.rs"), "").unwrap();
        std::fs::write(ctx.workspace.join("readme.md"), "").unwrap();
        let out = run(&ctx, &json!({"pattern": "*.rs"}));
        assert!(!out.is_error, "{}", out.content);
        let mut lines: Vec<&str> = out.content.lines().collect();
        lines.sort();
        assert_eq!(lines, vec!["src/a.rs", "src/deep/b.rs"]);
    }

    #[test]
    fn newest_first_by_mtime() {
        let (_t, ctx) = test_ctx();
        std::fs::write(ctx.workspace.join("old.rs"), "").unwrap();
        std::fs::write(ctx.workspace.join("new.rs"), "").unwrap();
        let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        let f = std::fs::File::open(ctx.workspace.join("old.rs")).unwrap();
        f.set_modified(old_time).unwrap();
        let out = run(&ctx, &json!({"pattern": "*.rs"}));
        assert_eq!(out.content, "new.rs\nold.rs");
    }

    #[test]
    fn zero_matches_names_the_next_action() {
        let (_t, ctx) = test_ctx();
        let out = run(&ctx, &json!({"pattern": "*.zig"}));
        assert!(!out.is_error);
        assert!(out.content.contains("try a broader pattern or ls"));
    }

    #[test]
    fn bad_pattern_is_a_remedy_error() {
        let (_t, ctx) = test_ctx();
        let out = run(&ctx, &json!({"pattern": "{unclosed"}));
        assert!(out.is_error);
        assert!(out.content.contains("bad glob pattern"));
    }

    #[test]
    fn entry_cap_with_trailer() {
        let (_t, ctx) = test_ctx();
        for i in 0..230 {
            std::fs::write(ctx.workspace.join(format!("f{i:03}.txt")), "").unwrap();
        }
        let out = run(&ctx, &json!({"pattern": "*.txt"}));
        assert!(out.content.ends_with("230 files, showing 200; narrow the pattern"));
    }
}
