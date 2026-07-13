//! grep: regex search over file contents, gitignore-aware, `path: line`
//! output with no line numbers (the no-line-numbers rule is global: number
//! prefixes contaminate small-model edit strings).

use std::path::Path;
use std::sync::atomic::Ordering;

use noob_provider::http::INTERRUPTED;
use serde_json::Value;

use super::truncate::{GREP_BYTE_CAP, GREP_MATCH_CAP, clip_line, grep_trailer};
use super::{ToolCtx, ToolOutcome, display_path, need_str, opt_bool, opt_str};

const CANCELED: &str = "grep canceled by user";

pub fn run(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    run_with(ctx, args, || INTERRUPTED.load(Ordering::SeqCst))
}

fn run_with(ctx: &ToolCtx, args: &Value, interrupted: impl Fn() -> bool) -> ToolOutcome {
    match run_inner(ctx, args, &interrupted) {
        Ok(out) => out,
        Err(msg) if msg == CANCELED => ToolOutcome::canceled_with(msg),
        Err(msg) => ToolOutcome::err(msg),
    }
}

fn run_inner(
    ctx: &ToolCtx,
    args: &Value,
    interrupted: &impl Fn() -> bool,
) -> Result<ToolOutcome, String> {
    let pattern = need_str(args, "pattern")?;
    let ignore_case = opt_bool(args, "ignore_case")?.unwrap_or(false);
    let re = regex::RegexBuilder::new(pattern)
        .case_insensitive(ignore_case)
        .build()
        .map_err(|e| {
            format!("bad regex {pattern:?}: {e}; escape literal characters like ( with a backslash")
        })?;

    let root_raw = opt_str(args, "path")?.unwrap_or(".");
    let root = super::guard::resolve_path(&ctx.workspace, root_raw);
    if !root.exists() {
        return Err(format!(
            "cannot search {}: no such path; check it with ls",
            display_path(ctx, &root)
        ));
    }

    let mut walk = ignore::WalkBuilder::new(&root);
    walk.sort_by_file_path(|a, b| a.cmp(b))
        .hidden(false)
        // Honor .gitignore even when the workspace is not a git repo yet.
        .require_git(false)
        .max_filesize(Some(8 * 1024 * 1024));
    if let Some(pat) = opt_str(args, "glob")? {
        let mut over = ignore::overrides::OverrideBuilder::new(&root);
        over.add(pat)
            .map_err(|e| format!("bad glob {pat:?}: {e}; use gitignore syntax, e.g. *.rs"))?;
        walk.overrides(over.build().map_err(|e| format!("bad glob {pat:?}: {e}"))?);
    }

    let mut total = 0usize;
    let mut shown = 0usize;
    let mut out = String::new();
    for entry in walk.build().flatten() {
        if interrupted() {
            return Err(CANCELED.to_string());
        }
        // An explicitly named file is searched even when gitignored.
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        if !scan_file(
            ctx,
            entry.path(),
            &re,
            &mut total,
            &mut shown,
            &mut out,
            interrupted,
        ) {
            return Err(CANCELED.to_string());
        }
    }

    let trailer = grep_trailer(total, shown);
    let content = if total == 0 {
        format!("no matches for {pattern:?}; loosen the pattern or check the path")
    } else {
        format!("{out}{trailer}")
    };
    Ok(ToolOutcome::ok(
        content,
        format!("grep {pattern} ({trailer})"),
    ))
}

fn scan_file(
    ctx: &ToolCtx,
    path: &Path,
    re: &regex::Regex,
    total: &mut usize,
    shown: &mut usize,
    out: &mut String,
    interrupted: &impl Fn() -> bool,
) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return true;
    };
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return true; // binary
    }
    let text = String::from_utf8_lossy(&bytes);
    let rel = display_path(ctx, path);
    for line in text.lines() {
        if interrupted() {
            return false;
        }
        if !re.is_match(line) {
            continue;
        }
        *total += 1;
        if *shown < GREP_MATCH_CAP && out.len() < GREP_BYTE_CAP {
            out.push_str(&rel);
            out.push_str(": ");
            out.push_str(clip_line(line.trim_end()).as_ref());
            out.push('\n');
            *shown += 1;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::test_ctx;
    use super::*;
    use serde_json::json;

    fn seed(ctx: &ToolCtx) {
        std::fs::create_dir_all(ctx.workspace.join("src")).unwrap();
        std::fs::write(
            ctx.workspace.join("src/a.rs"),
            "fn alpha() {}\nfn beta() {}\n",
        )
        .unwrap();
        std::fs::write(ctx.workspace.join("notes.md"), "alpha note\n").unwrap();
        std::fs::write(ctx.workspace.join(".gitignore"), "skipme/\n").unwrap();
        std::fs::create_dir_all(ctx.workspace.join("skipme")).unwrap();
        std::fs::write(ctx.workspace.join("skipme/x.rs"), "fn alpha() {}\n").unwrap();
    }

    #[test]
    fn path_colon_line_no_line_numbers_and_gitignore_respected() {
        let (_t, ctx) = test_ctx();
        seed(&ctx);
        let out = run(&ctx, &json!({"pattern": "alpha"}));
        assert!(!out.is_error);
        assert_eq!(
            out.content,
            "notes.md: alpha note\nsrc/a.rs: fn alpha() {}\n2 matches"
        );
    }

    #[test]
    fn glob_filter_narrows_files() {
        let (_t, ctx) = test_ctx();
        seed(&ctx);
        let out = run(&ctx, &json!({"pattern": "alpha", "glob": "*.rs"}));
        assert_eq!(out.content, "src/a.rs: fn alpha() {}\n1 match");
    }

    #[test]
    fn ignore_case_flag_works() {
        let (_t, ctx) = test_ctx();
        std::fs::write(ctx.workspace.join("f.txt"), "ALPHA\n").unwrap();
        assert!(
            run(&ctx, &json!({"pattern": "alpha"}))
                .content
                .contains("no matches")
        );
        let out = run(&ctx, &json!({"pattern": "alpha", "ignore_case": true}));
        assert!(out.content.contains("f.txt: ALPHA"));
    }

    #[test]
    fn golden_cap_trailer_and_total_count() {
        let (_t, ctx) = test_ctx();
        let body: String = (0..312).map(|i| format!("needle {i}\n")).collect();
        std::fs::write(ctx.workspace.join("big.txt"), body).unwrap();
        let out = run(&ctx, &json!({"pattern": "needle"}));
        assert!(
            out.content
                .ends_with("312 matches, showing 100; narrow the pattern or add a glob")
        );
        assert_eq!(out.content.lines().count(), 101);
    }

    #[test]
    fn single_file_path_searches_just_that_file() {
        let (_t, ctx) = test_ctx();
        seed(&ctx);
        let out = run(&ctx, &json!({"pattern": "beta", "path": "src/a.rs"}));
        assert_eq!(out.content, "src/a.rs: fn beta() {}\n1 match");
    }

    #[test]
    fn bad_regex_names_the_remedy() {
        let (_t, ctx) = test_ctx();
        let out = run(&ctx, &json!({"pattern": "fn ("}));
        assert!(out.is_error);
        assert!(out.content.contains("bad regex"));
        assert!(out.content.contains("backslash"));
    }

    #[test]
    fn zero_matches_is_not_an_error() {
        let (_t, ctx) = test_ctx();
        seed(&ctx);
        let out = run(&ctx, &json!({"pattern": "does_not_exist_anywhere"}));
        assert!(!out.is_error);
        assert!(out.content.contains("no matches"));
    }

    #[test]
    fn search_cancellation_is_structural() {
        let (_t, ctx) = test_ctx();
        std::fs::write(ctx.workspace.join("a.txt"), "needle\n").unwrap();
        let out = run_with(&ctx, &json!({"pattern": "needle"}), || true);
        assert!(out.canceled);
        assert!(out.is_error);
    }
}
