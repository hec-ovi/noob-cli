//! glob: files matching a gitignore-style pattern, newest first. Matching
//! inside node_modules/target poisons a 131k window faster than anything
//! else, so the walk respects .gitignore.

use std::cmp::Ordering as CmpOrdering;
use std::sync::atomic::Ordering;
use std::time::SystemTime;

use noob_provider::http::INTERRUPTED;
use serde_json::Value;

use super::truncate::{LIST_ENTRY_CAP, list_trailer};
use super::{ToolCtx, ToolOutcome, need_str};

const CANCELED: &str = "glob canceled by user";

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
    let pattern = need_str(args, "pattern")?;
    let mut over = ignore::overrides::OverrideBuilder::new(&ctx.workspace);
    over.add(pattern).map_err(|e| {
        format!("bad glob pattern {pattern:?}: {e}; use gitignore syntax, e.g. src/**/*.rs")
    })?;
    let over = over
        .build()
        .map_err(|e| format!("bad glob pattern {pattern:?}: {e}"))?;

    // Retain only the entries that can be shown while still counting every
    // match. This keeps a huge workspace from becoming a huge allocation.
    let mut hits: Vec<(SystemTime, String)> = Vec::with_capacity(LIST_ENTRY_CAP);
    let mut total = 0usize;
    let walk = ignore::WalkBuilder::new(&ctx.workspace)
        .overrides(over)
        .sort_by_file_path(|a, b| a.cmp(b))
        .hidden(false)
        // Honor .gitignore even when the workspace is not a git repo yet.
        .require_git(false)
        .build();
    for entry in walk.flatten() {
        if interrupted() {
            return Err(CANCELED.to_string());
        }
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
        total = total.saturating_add(1);
        let candidate = (mtime, rel.display().to_string());
        if hits.len() < LIST_ENTRY_CAP {
            hits.push(candidate);
        } else if let Some((worst, _)) = hits
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| hit_order(a, b))
            && hit_order(&candidate, &hits[worst]) == CmpOrdering::Less
        {
            hits[worst] = candidate;
        }
    }
    // Newest first; equal mtimes fall back to the path order from the walk,
    // kept stable by the sort.
    hits.sort_by(hit_order);

    if total == 0 {
        return Ok(ToolOutcome::ok(
            format!("no files match {pattern:?}; try a broader pattern or ls"),
            format!("glob {pattern} (0 files)"),
        ));
    }
    let shown = hits.len();
    let mut content = hits
        .iter()
        .map(|(_, p)| p.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(trailer) = list_trailer("files", total, shown) {
        content.push('\n');
        content.push_str(&trailer);
    }
    Ok(ToolOutcome::ok(
        content,
        format!("glob {pattern} ({total} files)"),
    ))
}

fn hit_order(a: &(SystemTime, String), b: &(SystemTime, String)) -> CmpOrdering {
    b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1))
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
    fn walk_cancellation_is_structural() {
        let (_t, ctx) = test_ctx();
        std::fs::write(ctx.workspace.join("a.txt"), "").unwrap();
        let out = run_with(&ctx, &json!({"pattern": "*.txt"}), || true);
        assert!(out.canceled);
        assert!(out.is_error);
    }

    #[test]
    fn entry_cap_with_trailer() {
        let (_t, ctx) = test_ctx();
        let same_time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        for i in 0..230 {
            let path = ctx.workspace.join(format!("f{i:03}.txt"));
            std::fs::write(&path, "").unwrap();
            std::fs::File::open(path)
                .unwrap()
                .set_modified(same_time)
                .unwrap();
        }
        let out = run(&ctx, &json!({"pattern": "*.txt"}));
        assert!(out.content.ends_with("230 files, showing 200; narrow the pattern"));
        assert!(out.content.starts_with("f000.txt\n"));
        assert!(out.content.contains("f199.txt\n230 files"));
        assert!(!out.content.contains("f200.txt"));
    }
}
