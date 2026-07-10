//! edit: exact string replace with a deterministic fallback ladder and hard
//! ambiguity rejection. No similarity-score fuzzing, ever: a fuzzy match can
//! corrupt a file silently, a rejection teaches the model to retry correctly.
//!
//! Ladder: exact bytes -> A: trailing whitespace ignored -> B: typographic
//! normalization (smart quotes, unicode dashes, NBSP) -> C: uniform indent
//! shift. Matching happens on a shadow view with a byte-offset map back to
//! the original; splicing always happens on the original bytes; `new` goes
//! in verbatim (stage C re-indents it by the same delta, losslessly).
//! Every stage independently enforces uniqueness and never falls through on
//! ambiguity.

use serde_json::Value;

use super::guard::{FileStamp, atomic_write, check_write_allowed, fnv1a64, resolve_path};
use super::{ToolCtx, ToolOutcome, display_path, need_str, opt_bool};

pub fn run(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    match run_inner(ctx, args) {
        Ok(out) => out,
        Err(msg) => ToolOutcome::err(msg),
    }
}

fn run_inner(ctx: &ToolCtx, args: &Value) -> Result<ToolOutcome, String> {
    let raw = need_str(args, "path")?;
    let old = need_str(args, "old")?;
    let new = need_str(args, "new")?;
    let all = opt_bool(args, "all")?.unwrap_or(false);
    if let Some(refusal) = ctx.skills_write_refusal(raw) {
        return Err(refusal);
    }
    let path = resolve_path(&ctx.workspace, raw);
    check_write_allowed(ctx.sandbox, &ctx.workspace, &path)?;
    let shown = display_path(ctx, &path);

    if old.is_empty() {
        return Err("old is empty; to create a new file use write".to_string());
    }
    if old == new {
        return Err("old and new are identical; nothing would change".to_string());
    }

    let bytes = std::fs::read(&path).map_err(|e| {
        format!("cannot read {shown}: {e}; check the path with ls or glob")
    })?;
    match ctx.seen.get(&path) {
        None => {
            return Err(format!(
                "you have not read {shown} yet; edit needs the current content, read it first"
            ));
        }
        Some(stamp) if stamp != FileStamp::of(&bytes) => {
            return Err(format!(
                "{shown} changed on disk since your last read; re-read it"
            ));
        }
        Some(_) => {}
    }
    let text = String::from_utf8(bytes).map_err(|_| {
        format!("{shown} is not valid UTF-8; edit only works on text files, use write")
    })?;

    let fail_key = (path.clone(), fnv1a64(old.as_bytes()));
    match ladder(&text, old, new, all) {
        Ok(applied) => {
            atomic_write(&path, applied.content.as_bytes())?;
            ctx.seen.record(&path, FileStamp::of(applied.content.as_bytes()));
            ctx.edit_failures.lock().unwrap().remove(&fail_key);
            let n = applied.count;
            let mut msg = if n == 1 {
                format!("edited {shown} (1 replacement")
            } else {
                format!("edited {shown} ({n} replacements")
            };
            msg.push_str(match applied.stage {
                Stage::Exact => ")",
                Stage::Ws => "; matched after ignoring trailing whitespace)",
                Stage::Typo => "; matched after normalizing typographic characters)",
                Stage::Indent => {
                    "; matched at a different indent depth, new was re-indented to match)"
                }
            });
            Ok(ToolOutcome::ok(msg.clone(), msg))
        }
        Err(LadderFail::Ambiguous { stage, n }) => {
            *ctx.edit_failures.lock().unwrap().entry(fail_key).or_insert(0) += 1;
            let qual = match stage {
                Stage::Exact => "",
                Stage::Ws => " (ignoring trailing whitespace)",
                Stage::Typo => " (after normalizing typographic characters)",
                Stage::Indent => " (at a uniform indent shift)",
            };
            Err(format!(
                "old matched {n} locations in {shown}{qual}; add surrounding lines to \
                 make it unique, or pass all:true to replace every match"
            ))
        }
        Err(LadderFail::Miss) => {
            let mut fails = ctx.edit_failures.lock().unwrap();
            let count = fails.entry(fail_key).or_insert(0);
            *count += 1;
            let attempt = *count;
            drop(fails);
            Err(teach(&shown, &text, old, attempt))
        }
    }
}

// --- the ladder --------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Exact,
    Ws,
    Typo,
    Indent,
}

struct Applied {
    content: String,
    count: usize,
    stage: Stage,
}

enum LadderFail {
    Ambiguous { stage: Stage, n: usize },
    Miss,
}

fn ladder(text: &str, old: &str, new: &str, all: bool) -> Result<Applied, LadderFail> {
    // Exact byte match.
    let ranges: Vec<(usize, usize)> = text
        .match_indices(old)
        .map(|(i, _)| (i, i + old.len()))
        .collect();
    if let Some(applied) = settle(text, new, &ranges, all, Stage::Exact)? {
        return Ok(applied);
    }

    // Stage A: per-line trailing whitespace stripped on both sides.
    let f_ws = shadow(text, false);
    let o_ws = shadow(old, false);
    if !o_ws.text.is_empty() {
        let ranges = shadow_ranges(&f_ws, &o_ws);
        if let Some(applied) = settle(text, new, &ranges, all, Stage::Ws)? {
            return Ok(applied);
        }
    }

    // Stage B: typographic normalization on top of A.
    let f_ty = shadow(text, true);
    let o_ty = shadow(old, true);
    if !o_ty.text.is_empty() {
        let ranges = shadow_ranges(&f_ty, &o_ty);
        if let Some(applied) = settle(text, new, &ranges, all, Stage::Typo)? {
            return Ok(applied);
        }
    }

    // Stage C: uniform indent shift over whole lines.
    stage_indent(text, old, new, all)
}

/// Uniqueness rules shared by exact and the shadow stages: 1 match applies,
/// >1 without `all` rejects and does NOT fall through, 0 descends.
fn settle(
    text: &str,
    new: &str,
    ranges: &[(usize, usize)],
    all: bool,
    stage: Stage,
) -> Result<Option<Applied>, LadderFail> {
    match ranges.len() {
        0 => Ok(None),
        1 => Ok(Some(Applied {
            content: splice(text, ranges, new),
            count: 1,
            stage,
        })),
        n if all => Ok(Some(Applied {
            content: splice(text, ranges, new),
            count: n,
            stage,
        })),
        n => Err(LadderFail::Ambiguous { stage, n }),
    }
}

/// Replace every (start, end) byte range (ascending, non-overlapping) with `new`.
fn splice(text: &str, ranges: &[(usize, usize)], new: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for &(s, e) in ranges {
        out.push_str(&text[at..s]);
        out.push_str(new);
        at = e;
    }
    out.push_str(&text[at..]);
    out
}

// --- shadow views ------------------------------------------------------------

/// A transformed view of the file plus, for every shadow byte, the original
/// byte offset it came from (one trailing sentinel maps the end).
struct Shadow {
    text: String,
    map: Vec<usize>,
    /// Horizontal whitespace was dropped at end-of-input (no final newline).
    /// For a needle this means its matches must end at a line boundary:
    /// old "foo " must never rewrite the "foo" inside "foobar".
    eof_ws_dropped: bool,
}

/// Fold one typographic character to its ASCII intent.
fn fold(c: char) -> char {
    match c {
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
        '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
        | '\u{2212}' => '-',
        '\u{00A0}' | '\u{2007}' | '\u{202F}' => ' ',
        other => other,
    }
}

fn shadow(s: &str, typo: bool) -> Shadow {
    let mut text = String::with_capacity(s.len());
    let mut map = Vec::with_capacity(s.len() + 1);
    // Pending horizontal whitespace: dropped at line end, flushed otherwise.
    // '\r' counts as line-end whitespace so CRLF files behave exactly like
    // LF files in every stage (the splice still happens on original bytes).
    let mut pending: Vec<(usize, char)> = Vec::new();
    let push = |text: &mut String, map: &mut Vec<usize>, at: usize, c: char| {
        let start = text.len();
        text.push(c);
        for _ in start..text.len() {
            map.push(at);
        }
    };
    for (i, raw) in s.char_indices() {
        let c = if typo { fold(raw) } else { raw };
        if c == ' ' || c == '\t' || c == '\r' {
            pending.push((i, c));
            continue;
        }
        if c == '\n' {
            pending.clear(); // trailing whitespace vanishes from the shadow
            push(&mut text, &mut map, i, '\n');
            continue;
        }
        for (j, w) in pending.drain(..) {
            push(&mut text, &mut map, j, w);
        }
        push(&mut text, &mut map, i, c);
    }
    let eof_ws_dropped = !pending.is_empty();
    map.push(s.len());
    Shadow { text, map, eof_ws_dropped }
}

/// Find the needle shadow in the file shadow and map every hit back to
/// original byte ranges. A needle whose trailing whitespace was dropped at
/// end-of-input claimed whitespace after its last character, so the hit
/// must be followed by whitespace or a line end: old "foo " must never
/// rewrite the "foo" inside "foobar", but may match "foo z" (the file has
/// the claimed space) and "foo\n" (a real line-trailing space).
fn shadow_ranges(sh: &Shadow, needle: &Shadow) -> Vec<(usize, usize)> {
    sh.text
        .match_indices(&needle.text)
        .filter(|(i, _)| {
            if !needle.eof_ws_dropped {
                return true;
            }
            let end = i + needle.text.len();
            end == sh.text.len()
                || matches!(sh.text.as_bytes()[end], b'\n' | b' ' | b'\t' | b'\r')
        })
        .map(|(i, _)| (sh.map[i], sh.map[i + needle.text.len()]))
        .collect()
}

// --- stage C: uniform indent shift -------------------------------------------

/// (byte start, byte end excluding newline, byte end including newline)
fn line_spans(text: &str) -> Vec<(usize, usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'\n' {
            spans.push((start, i, i + 1));
            start = i + 1;
        }
    }
    if start < bytes.len() {
        spans.push((start, bytes.len(), bytes.len()));
    }
    spans
}

fn split_indent(line: &str) -> (&str, &str) {
    let end = line
        .char_indices()
        .find(|(_, c)| *c != ' ' && *c != '\t')
        .map(|(i, _)| i)
        .unwrap_or(line.len());
    line.split_at(end)
}

/// Normalized comparison body: typographic fold + trailing whitespace strip
/// ('\r' included so CRLF lines compare equal to their LF originals).
fn norm_body(s: &str) -> String {
    s.trim_end_matches([' ', '\t', '\r']).chars().map(fold).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Delta {
    Add(String),
    Remove(String),
}

/// The uniform indent delta for one window, or None when it does not match.
fn window_delta(file_lines: &[&str], old_lines: &[&str], i: usize) -> Option<Delta> {
    let mut delta: Option<Delta> = None;
    for (j, old_line) in old_lines.iter().enumerate() {
        let (fi, fb) = split_indent(file_lines[i + j]);
        let (oi, ob) = split_indent(old_line);
        let fb = norm_body(fb);
        let ob = norm_body(ob);
        if fb.is_empty() && ob.is_empty() {
            continue; // blank lines match at any indent
        }
        if fb != ob {
            return None;
        }
        let pair = if let Some(p) = fi.strip_suffix(oi) {
            Delta::Add(p.to_string())
        } else if let Some(p) = oi.strip_suffix(fi) {
            Delta::Remove(p.to_string())
        } else {
            return None;
        };
        match &delta {
            None => delta = Some(pair),
            Some(d) if *d == pair => {}
            Some(_) => return None,
        }
    }
    // All-blank windows carry no signal; never match on them.
    delta
}

fn stage_indent(text: &str, old: &str, new: &str, all: bool) -> Result<Applied, LadderFail> {
    let old_lines: Vec<&str> = old.lines().collect();
    let n = old_lines.len();
    if n == 0 {
        return Err(LadderFail::Miss);
    }
    let spans = line_spans(text);
    if spans.len() < n {
        return Err(LadderFail::Miss);
    }
    let file_lines: Vec<&str> = spans.iter().map(|&(s, e, _)| &text[s..e]).collect();

    // Non-overlapping greedy collection, the same semantics match_indices
    // gives the exact stage: after a hit at line i the search resumes at
    // i + n. Overlapping windows would corrupt content when applied and can
    // index out of bounds after a shrinking replacement.
    let mut candidates: Vec<(usize, Delta)> = Vec::new();
    let mut i = 0;
    while i + n <= file_lines.len() {
        match window_delta(&file_lines, &old_lines, i) {
            Some(d) => {
                candidates.push((i, d));
                i += n;
            }
            None => i += 1,
        }
    }

    match candidates.len() {
        0 => Err(LadderFail::Miss),
        1 => Ok(apply_indent(text, &spans, old, new, &candidates[0], 1)),
        len if all => {
            // Replace back-to-front so earlier spans stay valid.
            let mut content = text.to_string();
            for cand in candidates.iter().rev() {
                content = apply_indent(&content, &line_spans(&content), old, new, cand, 1)
                    .content;
            }
            Ok(Applied {
                content,
                count: len,
                stage: Stage::Indent,
            })
        }
        len => Err(LadderFail::Ambiguous {
            stage: Stage::Indent,
            n: len,
        }),
    }
}

fn apply_indent(
    text: &str,
    spans: &[(usize, usize, usize)],
    old: &str,
    new: &str,
    cand: &(usize, Delta),
    count: usize,
) -> Applied {
    let (i, delta) = cand;
    let n = old.lines().count();
    let start = spans[*i].0;
    let end = if old.ends_with('\n') {
        spans[i + n - 1].2 // include the last line's newline
    } else {
        spans[i + n - 1].1
    };
    let replacement = reindent(new, delta);
    Applied {
        content: splice(text, &[(start, end)], &replacement),
        count,
        stage: Stage::Indent,
    }
}

/// Apply the window's indent delta to `new`, so the replacement lands at the
/// file's real depth even though the model wrote it at the depth it imagined.
fn reindent(new: &str, delta: &Delta) -> String {
    let ends_nl = new.ends_with('\n');
    let mut out: Vec<String> = Vec::new();
    for line in new.lines() {
        if line.trim_matches([' ', '\t']).is_empty() {
            out.push(line.to_string());
            continue;
        }
        out.push(match delta {
            Delta::Add(p) => format!("{p}{line}"),
            Delta::Remove(p) => line.strip_prefix(p.as_str()).unwrap_or(line).to_string(),
        });
    }
    let mut joined = out.join("\n");
    if ends_nl {
        joined.push('\n');
    }
    joined
}

// --- failure teaching ---------------------------------------------------------

/// The whole ladder missed: locate the closest region by anchor-line match
/// and put the ground truth in the error. Escalates with repeated failures
/// of the same (path, old): 2nd returns up to 40 file lines around the
/// anchor, 3rd tells the model to re-read and rewrite the region.
fn teach(shown: &str, text: &str, old: &str, attempt: u32) -> String {
    if attempt >= 3 {
        return format!(
            "edit failed {attempt} times: old still matches nothing in {shown}; \
             re-read the file with read, then replace the whole region with one \
             write or one larger edit copied exactly from the file"
        );
    }
    let old_lines: Vec<&str> = old.lines().collect();
    let file_lines: Vec<&str> = text.lines().collect();
    let Some((anchor, score)) = best_window(&file_lines, &old_lines) else {
        return format!(
            "old matched nothing in {shown} and no similar region was found; \
             re-read the file, the content may differ from what you expect"
        );
    };
    if attempt == 2 {
        let n = old_lines.len();
        let extra = 40usize.saturating_sub(n);
        let from = anchor.saturating_sub(extra / 2);
        let to = (anchor + n + (extra - extra / 2)).min(file_lines.len());
        return format!(
            "edit failed again: old still matches nothing in {shown}. The actual file \
             content around the closest match is:\n---\n{}\n---\nmake old an exact \
             copy of the lines to replace",
            file_lines[from..to].join("\n")
        );
    }
    let mut msg = format!(
        "old matched nothing in {shown} (tried exact, whitespace, punctuation, and \
         indent matching). Closest region ({score} of {} lines match):\n---\n{}\n---",
        old_lines.len(),
        file_lines[anchor..(anchor + old_lines.len()).min(file_lines.len())].join("\n"),
    );
    if let Some((j, col, o, f)) = first_difference(&file_lines[anchor..], &old_lines) {
        msg.push_str(&format!(
            "\nfirst difference on line {} of old: old has {o:?} but the file has \
             {f:?} (they differ at character {col})",
            j + 1
        ));
    }
    msg.push_str("\nfix old to match the file exactly, then retry");
    msg
}

/// Best window of `file` lines vs `old` lines, scored by fully-trimmed
/// equality (content match regardless of indent or trailing whitespace).
fn best_window(file_lines: &[&str], old_lines: &[&str]) -> Option<(usize, usize)> {
    let n = old_lines.len();
    if n == 0 || file_lines.is_empty() {
        return None;
    }
    let trim = |s: &str| -> String { norm_body(s.trim_matches([' ', '\t'])) };
    let old_t: Vec<String> = old_lines.iter().map(|l| trim(l)).collect();
    let mut best: Option<(usize, usize)> = None;
    for i in 0..file_lines.len() {
        let mut score = 0;
        for (j, o) in old_t.iter().enumerate() {
            if o.is_empty() {
                continue;
            }
            if let Some(f) = file_lines.get(i + j) {
                if trim(f) == *o {
                    score += 1;
                }
            }
        }
        if score > best.map(|(_, s)| s).unwrap_or(0) {
            best = Some((i, score));
        }
    }
    best
}

/// First line pair that differs, with the first differing character column
/// (1-based, counted in characters).
fn first_difference<'a>(
    file_lines: &[&'a str],
    old_lines: &[&'a str],
) -> Option<(usize, usize, &'a str, &'a str)> {
    for (j, o) in old_lines.iter().enumerate() {
        let f = file_lines.get(j).copied().unwrap_or("");
        if f == *o {
            continue;
        }
        let col = f
            .chars()
            .zip(o.chars())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| f.chars().count().min(o.chars().count()));
        return Some((j, col + 1, o, f));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::test_ctx;
    use super::*;
    use serde_json::json;

    fn seed(ctx: &ToolCtx, name: &str, content: &str) {
        std::fs::write(ctx.workspace.join(name), content).unwrap();
        super::super::read::run(ctx, &json!({"path": name}));
    }

    fn file(ctx: &ToolCtx, name: &str) -> String {
        std::fs::read_to_string(ctx.workspace.join(name)).unwrap()
    }

    #[test]
    fn exact_single_replacement() {
        let (_t, ctx) = test_ctx();
        seed(&ctx, "f.rs", "fn a() {}\nfn b() {}\n");
        let out = run(&ctx, &json!({"path": "f.rs", "old": "fn b()", "new": "fn c()"}));
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(out.content, "edited f.rs (1 replacement)");
        assert_eq!(file(&ctx, "f.rs"), "fn a() {}\nfn c() {}\n");
    }

    #[test]
    fn ambiguity_is_rejected_with_the_location_count() {
        let (_t, ctx) = test_ctx();
        seed(&ctx, "f.rs", "x = 1;\nx = 1;\nx = 1;\n");
        let out = run(&ctx, &json!({"path": "f.rs", "old": "x = 1;", "new": "x = 2;"}));
        assert!(out.is_error);
        assert!(out.content.contains("old matched 3 locations"));
        assert!(out.content.contains("add surrounding lines to make it unique"));
        assert_eq!(file(&ctx, "f.rs"), "x = 1;\nx = 1;\nx = 1;\n");
    }

    #[test]
    fn all_true_replaces_every_match() {
        let (_t, ctx) = test_ctx();
        seed(&ctx, "f.rs", "old_name(1);\nold_name(2);\n");
        let out = run(
            &ctx,
            &json!({"path": "f.rs", "old": "old_name", "new": "new_name", "all": true}),
        );
        assert_eq!(out.content, "edited f.rs (2 replacements)");
        assert_eq!(file(&ctx, "f.rs"), "new_name(1);\nnew_name(2);\n");
    }

    #[test]
    fn stage_a_ignores_trailing_whitespace_on_both_sides() {
        let (_t, ctx) = test_ctx();
        seed(&ctx, "f.py", "def f():   \n    pass\t\n");
        let out = run(
            &ctx,
            &json!({"path": "f.py", "old": "def f():\n    pass", "new": "def f():\n    return 1"}),
        );
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("ignoring trailing whitespace"));
        assert_eq!(file(&ctx, "f.py"), "def f():\n    return 1\n");
    }

    #[test]
    fn stage_b_folds_smart_quotes_dashes_and_nbsp() {
        let (_t, ctx) = test_ctx();
        // File has plain ASCII; the model pasted typographic characters.
        seed(&ctx, "f.md", "say \"hi\" - it's free\n");
        let out = run(
            &ctx,
            &json!({"path": "f.md",
                "old": "say \u{201C}hi\u{201D} \u{2014} it\u{2019}s\u{00A0}free",
                "new": "say \"bye\""}),
        );
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("normalizing typographic characters"));
        assert_eq!(file(&ctx, "f.md"), "say \"bye\"\n");
    }

    #[test]
    fn stage_c_adds_the_missing_indent_and_reindents_new() {
        let (_t, ctx) = test_ctx();
        seed(
            &ctx,
            "f.rs",
            "fn outer() {\n    if x {\n        do_it();\n    }\n}\n",
        );
        // The dominant qwen failure: old written at the wrong depth.
        let out = run(
            &ctx,
            &json!({"path": "f.rs",
                "old": "if x {\n    do_it();\n}",
                "new": "if x {\n    do_it();\n    log();\n}"}),
        );
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("different indent depth"));
        assert_eq!(
            file(&ctx, "f.rs"),
            "fn outer() {\n    if x {\n        do_it();\n        log();\n    }\n}\n"
        );
    }

    #[test]
    fn stage_c_removes_excess_indent_from_new_too() {
        let (_t, ctx) = test_ctx();
        seed(&ctx, "f.rs", "a();\nb();\n");
        let out = run(
            &ctx,
            &json!({"path": "f.rs", "old": "    a();\n    b();", "new": "    c();"}),
        );
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(file(&ctx, "f.rs"), "c();\n");
    }

    #[test]
    fn stage_c_rejects_two_candidate_windows() {
        let (_t, ctx) = test_ctx();
        // The same block exists at two different depths; a multi-line old at
        // a third depth reaches stage C and must reject, not pick one.
        seed(
            &ctx,
            "f.rs",
            "mod a {\n    if x {\n        go();\n    }\n}\n\
             mod b {\n        if x {\n            go();\n        }\n}\n",
        );
        let out = run(
            &ctx,
            &json!({"path": "f.rs", "old": "if x {\n    go();\n}", "new": "stop();"}),
        );
        assert!(out.is_error);
        assert!(out.content.contains("old matched 2 locations"));
        assert!(out.content.contains("uniform indent shift"));
    }

    #[test]
    fn stage_a_ambiguity_does_not_fall_through_to_b_or_c() {
        let (_t, ctx) = test_ctx();
        // "word\n" exact-misses (both lines carry trailing whitespace), then
        // matches twice at stage A; the ladder must stop there, not descend.
        seed(&ctx, "f.txt", "word \nword\t\n");
        let out = run(&ctx, &json!({"path": "f.txt", "old": "word\n", "new": "sword\n"}));
        assert!(out.is_error);
        assert!(out.content.contains("old matched 2 locations"));
        assert!(out.content.contains("ignoring trailing whitespace"));
    }

    #[test]
    fn never_read_is_rejected_with_the_read_first_remedy() {
        let (_t, ctx) = test_ctx();
        std::fs::write(ctx.workspace.join("f.rs"), "x\n").unwrap();
        let out = run(&ctx, &json!({"path": "f.rs", "old": "x", "new": "y"}));
        assert!(out.is_error);
        assert!(out.content.contains("edit needs the current content, read it first"));
    }

    #[test]
    fn stale_file_is_rejected() {
        let (_t, ctx) = test_ctx();
        seed(&ctx, "f.rs", "x\n");
        std::fs::write(ctx.workspace.join("f.rs"), "changed elsewhere\n").unwrap();
        let out = run(&ctx, &json!({"path": "f.rs", "old": "x", "new": "y"}));
        assert!(out.is_error);
        assert!(out.content.contains("changed on disk since your last read"));
    }

    #[test]
    fn miss_teaching_returns_the_closest_region_and_first_difference() {
        let (_t, ctx) = test_ctx();
        seed(&ctx, "f.rs", "let count = compute(items);\nreturn count;\n");
        let out = run(
            &ctx,
            &json!({"path": "f.rs",
                "old": "let count = compute(item);\nreturn count;",
                "new": "let count = 0;"}),
        );
        assert!(out.is_error);
        assert!(out.content.contains("Closest region (1 of 2 lines match)"));
        assert!(out.content.contains("let count = compute(items);"));
        assert!(out.content.contains("first difference on line 1 of old"));
        assert!(out.content.contains("fix old to match the file exactly"));
    }

    #[test]
    fn second_miss_returns_the_file_region_third_suggests_reread() {
        let (_t, ctx) = test_ctx();
        let body: String = (1..=60).map(|i| format!("line number {i}\n")).collect();
        seed(&ctx, "f.txt", &body);
        // Line 1 anchors ("line number 30" exists); line 2 never matches.
        let call = json!({"path": "f.txt", "old": "line number 30\nEXTRA JUNK", "new": "x"});
        let first = run(&ctx, &call);
        assert!(first.content.contains("Closest region"), "{}", first.content);
        let second = run(&ctx, &call);
        assert!(second.content.contains("The actual file content around the closest match"));
        // Up to 40 lines of ground truth, verbatim.
        let shown = second.content.lines().filter(|l| l.starts_with("line number")).count();
        assert!((35..=40).contains(&shown), "shown {shown} lines");
        let third = run(&ctx, &call);
        assert!(third.content.contains("re-read the file with read"));
        // A later success on this file removes its failure counter.
        let ok = run(&ctx, &json!({"path": "f.txt", "old": "line number 30\n", "new": "thirty\n"}));
        assert!(!ok.is_error, "{}", ok.content);
    }

    #[test]
    fn empty_old_and_identical_old_new_are_rejected() {
        let (_t, ctx) = test_ctx();
        seed(&ctx, "f.rs", "x\n");
        let out = run(&ctx, &json!({"path": "f.rs", "old": "", "new": "y"}));
        assert!(out.content.contains("old is empty"));
        let out = run(&ctx, &json!({"path": "f.rs", "old": "x", "new": "x"}));
        assert!(out.content.contains("old and new are identical"));
    }

    #[test]
    fn deletion_via_empty_new_works() {
        let (_t, ctx) = test_ctx();
        seed(&ctx, "f.rs", "keep\ndrop me\nkeep too\n");
        let out = run(&ctx, &json!({"path": "f.rs", "old": "drop me\n", "new": ""}));
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(file(&ctx, "f.rs"), "keep\nkeep too\n");
    }

    #[test]
    fn exact_match_wins_before_the_shadow_stages() {
        let (_t, ctx) = test_ctx();
        // "word" appears exactly once as exact bytes, but the shadow view
        // would also match the "word  " line; exact must win with 1 hit.
        seed(&ctx, "f.txt", "word\nother word  here\n");
        let out = run(&ctx, &json!({"path": "f.txt", "old": "word\n", "new": "sword\n"}));
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(out.content, "edited f.txt (1 replacement)");
        assert_eq!(file(&ctx, "f.txt"), "sword\nother word  here\n");
    }

    #[test]
    fn stage_c_all_true_windows_never_overlap() {
        let (_t, ctx) = test_ctx();
        // Overlapping candidate windows (lines 0-2 and 2-4) must resolve
        // greedily like match_indices, not corrupt the unmatched lines.
        seed(&ctx, "f.txt", "    a\n    b\n    a\n    b\n    a\n");
        let out = run(
            &ctx,
            &json!({"path": "f.txt", "old": "a\nb\na", "new": "z", "all": true}),
        );
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("1 replacement"), "{}", out.content);
        assert_eq!(file(&ctx, "f.txt"), "    z\n    b\n    a\n");
    }

    #[test]
    fn stage_c_all_true_shrinking_replacement_does_not_panic() {
        let (_t, ctx) = test_ctx();
        seed(&ctx, "f.txt", &"    x\n".repeat(5));
        let out = run(
            &ctx,
            &json!({"path": "f.txt", "old": "x\nx\nx\n", "new": "", "all": true}),
        );
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(file(&ctx, "f.txt"), "    x\n    x\n");
    }

    #[test]
    fn trailing_space_in_old_never_matches_mid_line() {
        let (_t, ctx) = test_ctx();
        // "a " occurs nowhere in "ab": the dropped trailing space must not
        // let the shadow match rewrite content the file does not contain.
        seed(&ctx, "f.txt", "ab\n");
        let out = run(&ctx, &json!({"path": "f.txt", "old": "a ", "new": "X"}));
        assert!(out.is_error, "false match applied: {}", out.content);
        assert_eq!(file(&ctx, "f.txt"), "ab\n");
        // But the legitimate case (trailing whitespace at a real line end)
        // still matches.
        seed(&ctx, "g.txt", "foo\n");
        let out = run(&ctx, &json!({"path": "g.txt", "old": "foo ", "new": "bar"}));
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(file(&ctx, "g.txt"), "bar\n");
    }

    #[test]
    fn trailing_space_in_old_matches_when_the_file_has_the_space() {
        let (_t, ctx) = test_ctx();
        // The file genuinely contains whitespace after the match; old's
        // trailing space claims exactly that, so the edit must apply
        // (over-rejecting here burns the model's retry budget for nothing).
        seed(&ctx, "f.md", "x \u{2014} y z\n");
        let out = run(&ctx, &json!({"path": "f.md", "old": "x - y ", "new": "x-y"}));
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(file(&ctx, "f.md"), "x-y z\n");

        seed(&ctx, "g.py", "def f():\n    pass # comment\n");
        let out = run(
            &ctx,
            &json!({"path": "g.py", "old": "def f():  \n    pass ", "new": "def g():\n    pass"}),
        );
        assert!(!out.is_error, "{}", out.content);
        // The file's own space after the match is kept, not consumed.
        assert_eq!(file(&ctx, "g.py"), "def g():\n    pass # comment\n");
    }

    #[test]
    fn crlf_files_work_through_stage_a() {
        let (_t, ctx) = test_ctx();
        seed(&ctx, "f.txt", "foo  \r\nbar\r\nrest\r\n");
        // old copied without the \r (what read/teach show): must match.
        // The last matched line's \r is consumed like any trailing
        // whitespace (new goes in verbatim); untouched lines keep CRLF.
        let out = run(&ctx, &json!({"path": "f.txt", "old": "foo\nbar", "new": "baz"}));
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(file(&ctx, "f.txt"), "baz\nrest\r\n");
    }

    #[test]
    fn crlf_files_work_through_stage_c() {
        let (_t, ctx) = test_ctx();
        seed(&ctx, "f.txt", "if x {\r\n    go();\r\n    stop();\r\n}\r\n");
        let out = run(
            &ctx,
            &json!({"path": "f.txt", "old": "go();\nstop();", "new": "go();\nwait();\nstop();"}),
        );
        assert!(!out.is_error, "{}", out.content);
        let content = file(&ctx, "f.txt");
        assert!(content.contains("wait();"), "{content}");
        assert!(content.starts_with("if x {\r\n"), "untouched lines keep CRLF: {content}");
    }

    #[test]
    fn stage_c_handles_old_with_trailing_newline() {
        let (_t, ctx) = test_ctx();
        seed(&ctx, "f.rs", "{\n    a();\n    b();\n}\n");
        let out = run(&ctx, &json!({"path": "f.rs", "old": "a();\nb();\n", "new": "c();\n"}));
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("different indent depth"));
        assert_eq!(file(&ctx, "f.rs"), "{\n    c();\n}\n");
    }
}
