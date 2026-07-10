//! read: plain text, NO line numbers (number prefixes are the most common
//! contaminant of small-model edit `old` strings). One-line header states
//! the page and the total so the model can page with offset/limit.

use serde_json::Value;

use super::guard::FileStamp;
use super::truncate::{READ_BYTE_CAP, READ_LINE_CAP, clip_line, read_byte_cap_marker};
use super::{ToolCtx, ToolOutcome, display_path, need_str, opt_u64};

pub fn run(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    match run_inner(ctx, args) {
        Ok(out) => out,
        Err(msg) => ToolOutcome::err(msg),
    }
}

fn run_inner(ctx: &ToolCtx, args: &Value) -> Result<ToolOutcome, String> {
    let raw = need_str(args, "path")?;
    let path = super::guard::resolve_path(&ctx.workspace, raw);
    let offset = opt_u64(args, "offset")?.unwrap_or(1).max(1) as usize;
    let limit = opt_u64(args, "limit")?.unwrap_or(READ_LINE_CAP as u64) as usize;
    let limit = limit.clamp(1, READ_LINE_CAP);

    let bytes = std::fs::read(&path).map_err(|e| {
        format!(
            "cannot read {}: {e}; check the path with ls or glob",
            display_path(ctx, &path)
        )
    })?;
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Err(format!(
            "{} looks binary ({} bytes); read only works on text files",
            display_path(ctx, &path),
            bytes.len()
        ));
    }
    // Record the stamp of the FULL file, not the page: staleness is about
    // the file on disk, not about what fit in one read.
    ctx.seen.record(&path, FileStamp::of(&bytes));

    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let shown_path = display_path(ctx, &path);

    if total == 0 {
        return Ok(ToolOutcome::ok(
            format!("{shown_path} is empty (0 lines)"),
            format!("read {shown_path} (empty)"),
        ));
    }
    if offset > total {
        return Err(format!(
            "{shown_path} has {total} lines; offset {offset} is past the end"
        ));
    }

    let end = (offset - 1 + limit).min(total);
    let mut out = String::new();
    let mut last_emitted = offset - 1; // 0-based index of the last line written
    let mut capped = false;
    for (i, line) in lines[offset - 1..end].iter().enumerate() {
        let clipped = clip_line(line);
        if out.len() + clipped.len() + 1 > READ_BYTE_CAP {
            capped = true;
            break;
        }
        out.push_str(&clipped);
        out.push('\n');
        last_emitted = offset - 1 + i;
    }
    let header = format!(
        "{shown_path} lines {}-{} of {total}",
        offset,
        last_emitted + 1
    );
    let mut content = format!("{header}\n{out}");
    if capped {
        content.push_str(&read_byte_cap_marker(last_emitted + 2));
    }
    let shown = last_emitted + 2 - offset;
    Ok(ToolOutcome::ok(
        content,
        format!("read {shown_path} ({shown} of {total} lines)"),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::test_ctx;
    use super::*;
    use serde_json::json;

    fn write(ctx: &ToolCtx, name: &str, content: &str) -> std::path::PathBuf {
        let p = ctx.workspace.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn plain_lines_with_header_and_no_line_numbers() {
        let (_t, ctx) = test_ctx();
        write(&ctx, "f.txt", "alpha\nbeta\ngamma\n");
        let out = run(&ctx, &json!({"path": "f.txt"}));
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(out.content, "f.txt lines 1-3 of 3\nalpha\nbeta\ngamma\n");
        assert_eq!(out.summary, "read f.txt (3 of 3 lines)");
    }

    #[test]
    fn paging_with_offset_and_limit() {
        let (_t, ctx) = test_ctx();
        let body: String = (1..=10).map(|i| format!("line{i}\n")).collect();
        write(&ctx, "f.txt", &body);
        let out = run(&ctx, &json!({"path": "f.txt", "offset": 4, "limit": 2}));
        assert_eq!(out.content, "f.txt lines 4-5 of 10\nline4\nline5\n");
    }

    #[test]
    fn offset_past_end_states_the_line_count() {
        let (_t, ctx) = test_ctx();
        write(&ctx, "f.txt", "one\n");
        let out = run(&ctx, &json!({"path": "f.txt", "offset": 5}));
        assert!(out.is_error);
        assert!(out.content.contains("has 1 lines; offset 5 is past the end"));
    }

    #[test]
    fn missing_file_names_the_remedy() {
        let (_t, ctx) = test_ctx();
        let out = run(&ctx, &json!({"path": "nope.txt"}));
        assert!(out.is_error);
        assert!(out.content.contains("check the path with ls or glob"));
    }

    #[test]
    fn binary_file_is_refused() {
        let (_t, ctx) = test_ctx();
        std::fs::write(ctx.workspace.join("bin"), b"\x00\x01\x02").unwrap();
        let out = run(&ctx, &json!({"path": "bin"}));
        assert!(out.is_error);
        assert!(out.content.contains("looks binary"));
    }

    #[test]
    fn long_lines_are_clipped_and_byte_cap_pages() {
        let (_t, ctx) = test_ctx();
        // 200 lines x ~600 chars: hits the 40 KiB cap well before 200 lines.
        let body: String = (0..200).map(|i| format!("{i:03}{}\n", "x".repeat(600))).collect();
        write(&ctx, "big.txt", &body);
        let out = run(&ctx, &json!({"path": "big.txt"}));
        assert!(!out.is_error);
        assert!(out.content.contains("[line clipped; 603 chars total]"));
        assert!(out.content.contains("[output capped at 40 KiB; continue with offset="));
        assert!(out.content.len() <= READ_BYTE_CAP + 200);
    }

    #[test]
    fn read_records_the_full_file_stamp() {
        let (_t, ctx) = test_ctx();
        let body: String = (1..=800).map(|i| format!("l{i}\n")).collect();
        let p = write(&ctx, "f.txt", &body);
        run(&ctx, &json!({"path": "f.txt", "limit": 5}));
        let stamp = ctx.seen.get(&p).expect("stamp recorded");
        assert_eq!(stamp, FileStamp::of(body.as_bytes()));
    }

    #[test]
    fn empty_file_reads_cleanly() {
        let (_t, ctx) = test_ctx();
        write(&ctx, "empty.txt", "");
        let out = run(&ctx, &json!({"path": "empty.txt"}));
        assert!(!out.is_error);
        assert_eq!(out.content, "empty.txt is empty (0 lines)");
    }
}
