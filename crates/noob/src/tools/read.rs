//! read: plain text, NO line numbers (number prefixes are the most common
//! contaminant of small-model edit `old` strings). One-line header states
//! the page and the total so the model can page with offset/limit.

use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::Ordering;

use noob_provider::http::INTERRUPTED;
use serde_json::Value;

use super::guard::{FileStamp, fnv1a64, fnv1a64_extend};
use super::truncate::{READ_LINE_CHAR_CAP, read_byte_cap_marker};
use super::{ToolCtx, ToolOutcome, display_path, need_str, opt_u64};

pub fn run(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    match run_inner(ctx, args) {
        Ok(out) => out,
        Err(msg) if msg == "canceled by user" => ToolOutcome::canceled(),
        Err(msg) => ToolOutcome::err(msg),
    }
}

fn run_inner(ctx: &ToolCtx, args: &Value) -> Result<ToolOutcome, String> {
    let raw = need_str(args, "path")?;
    let path = super::guard::resolve_path(&ctx.workspace, raw);
    let offset = opt_u64(args, "offset")?.unwrap_or(1).max(1) as usize;
    let limit = opt_u64(args, "limit")?.unwrap_or(ctx.caps.read_lines as u64) as usize;
    let limit = limit.clamp(1, ctx.caps.read_lines);
    let shown_path = display_path(ctx, &path);

    // O_NONBLOCK prevents a path swapped to a FIFO between lookup and open
    // from waiting for a writer. fstat the opened handle, not the path, so the
    // type check and the bytes below refer to the same object.
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&path)
        .map_err(|e| format!("cannot read {shown_path}: {e}; check the path with ls or glob"))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("cannot read {shown_path}: {e}; check the path with ls or glob"))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "cannot read {shown_path}: it is not a regular file; use read only for text files"
        ));
    }

    let page_end = offset.saturating_add(limit);
    let mut page = String::new();
    let mut last_emitted = offset.saturating_sub(1); // one-based, before page
    let mut capped = false;
    let mut line_no = 1usize;
    let mut preview = LinePreview::with_cap(ctx.caps.line_chars);
    let mut hash = fnv1a64(&[]);
    let mut byte_count = 0u64;
    let mut saw_any = false;
    let mut last_byte = None;
    let mut buf = [0u8; 64 * 1024];

    loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            return Err("canceled by user".to_string());
        }
        let n = file.read(&mut buf).map_err(|e| {
            format!("cannot read {shown_path}: {e}; check the path with ls or glob")
        })?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        let binary_probe_left = 8192usize.saturating_sub(byte_count as usize);
        if chunk[..chunk.len().min(binary_probe_left)].contains(&0) {
            return Err(format!(
                "{shown_path} looks binary ({} bytes); read only works on text files",
                metadata.len()
            ));
        }
        hash = fnv1a64_extend(hash, chunk);
        byte_count = byte_count.saturating_add(n as u64);
        saw_any = true;
        last_byte = chunk.last().copied();

        let mut start = 0usize;
        for (idx, &byte) in chunk.iter().enumerate() {
            if byte != b'\n' {
                continue;
            }
            if line_no >= offset && line_no < page_end && !capped {
                preview.feed(&chunk[start..idx]);
                push_page_line(
                    &mut page,
                    &mut last_emitted,
                    &mut capped,
                    line_no,
                    preview.finish(),
                    ctx.caps.read_bytes,
                );
            }
            preview.clear();
            line_no = line_no.saturating_add(1);
            start = idx + 1;
        }
        if line_no >= offset && line_no < page_end && !capped {
            preview.feed(&chunk[start..]);
        }
    }

    let total = if !saw_any {
        0
    } else if last_byte == Some(b'\n') {
        line_no.saturating_sub(1)
    } else {
        if line_no >= offset && line_no < page_end && !capped {
            push_page_line(
                &mut page,
                &mut last_emitted,
                &mut capped,
                line_no,
                preview.finish(),
                ctx.caps.read_bytes,
            );
        }
        line_no
    };

    // If the model has already seen exactly this content in the current
    // context (a write or a prior read, not pruned by a compaction since) and
    // this read would have shown the whole file, its body is still above:
    // return a short note instead of re-printing it. A page request or a
    // partially-shown file always prints, and so does an immediate repeat of
    // the same read: asking twice is how the model overrides the note.
    let current = FileStamp {
        len: byte_count,
        hash,
    };
    // The whole file is in context only when this read started at line 1 and
    // printed every line (no paging, no byte cap).
    let full = offset == 1 && total > 0 && last_emitted == total;
    // Ask before recording: the question is whether the model held this
    // content in full BEFORE this call, so a first-ever read has nothing to
    // match against and prints.
    let unchanged = full && ctx.seen.stub_unchanged(&path, current);
    // Record the stamp of the full stream, not just the retained page.
    ctx.seen.record(&path, current, full);
    if unchanged {
        return Ok(ToolOutcome::ok(
            format!(
                "{shown_path} unchanged since you last read or wrote it \
                 ({total} lines, {byte_count} bytes); its content is already in the \
                 conversation above. Read it again to print the body."
            ),
            format!("read {shown_path} (unchanged, {total} lines)"),
        ));
    }

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

    let header = format!("{shown_path} lines {offset}-{last_emitted} of {total}");
    let mut content = format!("{header}\n{page}");
    if capped {
        content.push_str(&read_byte_cap_marker(last_emitted + 1));
    }
    let shown = last_emitted.saturating_sub(offset).saturating_add(1);
    Ok(ToolOutcome::ok(
        content,
        format!("read {shown_path} ({shown} of {total} lines)"),
    ))
}

fn push_page_line(
    page: &mut String,
    last_emitted: &mut usize,
    capped: &mut bool,
    line_no: usize,
    line: String,
    byte_cap: usize,
) {
    if page.len().saturating_add(line.len()).saturating_add(1) > byte_cap {
        *capped = true;
        return;
    }
    page.push_str(&line);
    page.push('\n');
    *last_emitted = line_no;
}

/// UTF-8-lossy preview of one selected line. It retains only the first
/// `cap` characters (500 under the default policy; everything when
/// uncapped) while still counting the full line for the clipping marker.
struct LinePreview {
    cap: usize,
    shown: String,
    shown_chars: usize,
    total_chars: usize,
    last_char: Option<char>,
    pending_utf8: Vec<u8>,
}

impl Default for LinePreview {
    fn default() -> Self {
        LinePreview::with_cap(READ_LINE_CHAR_CAP)
    }
}

impl LinePreview {
    fn with_cap(cap: usize) -> Self {
        LinePreview {
            cap,
            shown: String::new(),
            shown_chars: 0,
            total_chars: 0,
            last_char: None,
            pending_utf8: Vec::new(),
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let owned;
        let mut rest = if self.pending_utf8.is_empty() {
            bytes
        } else {
            let mut joined = std::mem::take(&mut self.pending_utf8);
            joined.extend_from_slice(bytes);
            owned = joined;
            owned.as_slice()
        };
        loop {
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    self.push_valid(valid);
                    break;
                }
                Err(error) => {
                    let valid = &rest[..error.valid_up_to()];
                    self.push_valid(std::str::from_utf8(valid).expect("valid_up_to prefix"));
                    rest = &rest[error.valid_up_to()..];
                    match error.error_len() {
                        Some(len) => {
                            self.push_char('�');
                            rest = &rest[len..];
                            if rest.is_empty() {
                                break;
                            }
                        }
                        None => {
                            self.pending_utf8.extend_from_slice(rest);
                            break;
                        }
                    }
                }
            }
        }
    }

    fn push_valid(&mut self, text: &str) {
        for c in text.chars() {
            self.push_char(c);
        }
    }

    fn push_char(&mut self, c: char) {
        self.total_chars = self.total_chars.saturating_add(1);
        self.last_char = Some(c);
        if self.shown_chars < self.cap {
            self.shown.push(c);
            self.shown_chars += 1;
        }
    }

    fn finish(&mut self) -> String {
        if !self.pending_utf8.is_empty() {
            let pending = std::mem::take(&mut self.pending_utf8);
            for c in String::from_utf8_lossy(&pending).chars() {
                self.push_char(c);
            }
        }
        // `str::lines` removes CR only as part of a CRLF terminator.
        if self.last_char == Some('\r') {
            self.total_chars = self.total_chars.saturating_sub(1);
            if self.shown.ends_with('\r') {
                self.shown.pop();
                self.shown_chars = self.shown_chars.saturating_sub(1);
            }
        }
        if self.total_chars > self.cap {
            format!(
                "{} [line clipped; {} chars total]",
                self.shown, self.total_chars
            )
        } else {
            std::mem::take(&mut self.shown)
        }
    }

    fn clear(&mut self) {
        *self = LinePreview::with_cap(self.cap);
    }
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
        assert!(
            out.content
                .contains("has 1 lines; offset 5 is past the end")
        );
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
    fn fifo_is_rejected_without_waiting_for_a_writer() {
        use std::os::unix::ffi::OsStrExt;

        let (_t, ctx) = test_ctx();
        let path = ctx.workspace.join("pipe");
        let raw = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0);
        let started = std::time::Instant::now();
        let out = run(&ctx, &json!({"path": "pipe"}));
        assert!(out.is_error);
        assert!(
            out.content.contains("not a regular file"),
            "{}",
            out.content
        );
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
    }

    #[test]
    fn long_lines_are_clipped_and_byte_cap_pages() {
        use super::super::truncate::READ_BYTE_CAP;

        let (_t, ctx) = test_ctx();
        // 200 lines x ~600 chars: hits the 40 KiB cap well before 200 lines.
        let body: String = (0..200)
            .map(|i| format!("{i:03}{}\n", "x".repeat(600)))
            .collect();
        write(&ctx, "big.txt", &body);
        let out = run(&ctx, &json!({"path": "big.txt"}));
        assert!(!out.is_error);
        assert!(out.content.contains("[line clipped; 603 chars total]"));
        assert!(
            out.content
                .contains("[output capped at 40 KiB; continue with offset=")
        );
        assert!(out.content.len() <= READ_BYTE_CAP + 200);
    }

    #[test]
    fn uncapped_ctx_reads_the_whole_file_with_no_clipping() {
        let (_t, mut ctx) = test_ctx();
        ctx.caps = super::super::truncate::Caps::uncapped();
        // The same fixture that pages and clips under the default policy.
        let body: String = (0..200)
            .map(|i| format!("{i:03}{}\n", "x".repeat(600)))
            .collect();
        write(&ctx, "big.txt", &body);
        let out = run(&ctx, &json!({"path": "big.txt"}));
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.starts_with("big.txt lines 1-200 of 200\n"));
        assert!(!out.content.contains("[line clipped;"));
        assert!(!out.content.contains("[output capped"));
        assert!(out.content.contains(&format!("199{}", "x".repeat(600))));
    }

    #[test]
    fn a_multi_megabyte_line_is_streamed_but_only_its_preview_is_returned() {
        let (_t, ctx) = test_ctx();
        let body = format!("{}\ntail\n", "x".repeat(2 * 1024 * 1024));
        let path = write(&ctx, "huge-line.txt", &body);
        let out = run(&ctx, &json!({"path": "huge-line.txt", "limit": 1}));
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("2097152 chars total"));
        assert!(
            out.content.len() < 1_000,
            "only a bounded preview should survive"
        );
        assert_eq!(ctx.seen.get(&path), Some(FileStamp::of(body.as_bytes())));
    }

    #[test]
    fn streaming_preview_preserves_split_utf8_and_crlf_semantics() {
        let mut line = LinePreview::default();
        let bytes = "aé".as_bytes();
        line.feed(&bytes[..2]);
        line.feed(&bytes[2..]);
        line.feed(b"\r");
        assert_eq!(line.finish(), "aé");

        let mut invalid = LinePreview::default();
        invalid.feed(&[b'a', 0xf0]);
        invalid.feed(&[0x28, b'b']);
        assert_eq!(invalid.finish(), "a�(b");
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

    #[test]
    fn read_right_after_the_write_tool_stubs() {
        let (_t, ctx) = test_ctx();
        let body = "<!DOCTYPE html>\n<html>\nhi\n</html>\n";
        let w = super::super::write::run(&ctx, &json!({"path": "g.html", "content": body}));
        assert!(!w.is_error, "{}", w.content);
        let r = run(&ctx, &json!({"path": "g.html"}));
        assert!(
            r.content.contains("unchanged"),
            "write-then-read must stub, got: {}",
            r.content
        );
    }

    #[test]
    fn read_after_the_edit_tool_stubs() {
        let (_t, ctx) = test_ctx();
        write(&ctx, "g.html", "one\ntwo\nthree\n");
        run(&ctx, &json!({"path": "g.html"})); // establishes a full read
        let e =
            super::super::edit::run(&ctx, &json!({"path": "g.html", "old": "two", "new": "TWO"}));
        assert!(!e.is_error, "{}", e.content);
        let r = run(&ctx, &json!({"path": "g.html"}));
        assert!(
            r.content.contains("unchanged"),
            "read after edit must stub, got: {}",
            r.content
        );
    }

    #[test]
    fn re_reading_unchanged_content_returns_a_stub() {
        let (_t, ctx) = test_ctx();
        write(&ctx, "f.txt", "alpha\nbeta\ngamma\n");
        let first = run(&ctx, &json!({"path": "f.txt"}));
        assert!(first.content.contains("alpha"), "{}", first.content);
        let second = run(&ctx, &json!({"path": "f.txt"}));
        assert!(!second.is_error, "{}", second.content);
        assert!(second.content.contains("unchanged"), "{}", second.content);
        assert!(
            !second.content.contains("alpha"),
            "body should be omitted: {}",
            second.content
        );
    }

    #[test]
    fn asking_again_after_a_stub_prints_the_body() {
        // The escape hatch that replaces a `force` parameter: the model is
        // never stuck without the content, and it costs no schema tokens.
        let (_t, ctx) = test_ctx();
        write(&ctx, "f.txt", "alpha\nbeta\n");
        run(&ctx, &json!({"path": "f.txt"})); // prints
        let stub = run(&ctx, &json!({"path": "f.txt"}));
        assert!(stub.content.contains("unchanged"), "{}", stub.content);
        let insisted = run(&ctx, &json!({"path": "f.txt"}));
        assert!(insisted.content.contains("alpha"), "{}", insisted.content);
        assert!(!insisted.content.contains("unchanged"));
        // And it re-arms: the body is above again, so the next one stubs.
        let again = run(&ctx, &json!({"path": "f.txt"}));
        assert!(again.content.contains("unchanged"), "{}", again.content);
    }

    #[test]
    fn changed_content_is_reprinted_not_stubbed() {
        let (_t, ctx) = test_ctx();
        let p = write(&ctx, "f.txt", "alpha\n");
        run(&ctx, &json!({"path": "f.txt"}));
        std::fs::write(&p, "alpha\nbeta\n").unwrap();
        let out = run(&ctx, &json!({"path": "f.txt"}));
        assert!(
            out.content.contains("beta"),
            "a changed file must reprint: {}",
            out.content
        );
        assert!(!out.content.contains("unchanged"));
    }

    #[test]
    fn a_paged_read_does_not_break_a_later_whole_file_stub() {
        // Regression: the model writes the file (seen in full), then pages
        // through regions with offset/limit (navigation), then reads the whole
        // file. The paged reads must not downgrade the entry, so the final
        // whole-file read still stubs instead of re-printing the body.
        let (_t, ctx) = test_ctx();
        let body: String = (1..=20).map(|i| format!("line{i}\n")).collect();
        super::super::write::run(&ctx, &json!({"path": "g.txt", "content": body}));
        run(&ctx, &json!({"path": "g.txt", "offset": 5, "limit": 3}));
        run(&ctx, &json!({"path": "g.txt", "offset": 12, "limit": 4}));
        let whole = run(&ctx, &json!({"path": "g.txt"}));
        assert!(
            whole.content.contains("unchanged"),
            "whole-file read after paged reads must still stub, got: {}",
            whole.content
        );
    }

    #[test]
    fn a_partially_shown_file_is_not_stubbed_later() {
        let (_t, ctx) = test_ctx();
        let body: String = (1..=10).map(|i| format!("line{i}\n")).collect();
        write(&ctx, "f.txt", &body);
        // First read shows only lines 1-3 but records the full-content stamp.
        run(&ctx, &json!({"path": "f.txt", "limit": 3}));
        // A whole-file read must print, the model never saw lines 4-10.
        let out = run(&ctx, &json!({"path": "f.txt"}));
        assert!(out.content.contains("line10"), "{}", out.content);
        assert!(!out.content.contains("unchanged"));
    }

    #[test]
    fn compaction_invalidates_freshness_and_reprints() {
        let (_t, ctx) = test_ctx();
        write(&ctx, "f.txt", "alpha\nbeta\n");
        run(&ctx, &json!({"path": "f.txt"}));
        // Simulate a compaction pruning the earlier read body from context.
        ctx.seen.invalidate_freshness();
        let out = run(&ctx, &json!({"path": "f.txt"}));
        assert!(
            out.content.contains("alpha"),
            "must reprint after compaction: {}",
            out.content
        );
        assert!(!out.content.contains("unchanged"));
    }
}
