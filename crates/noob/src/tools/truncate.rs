//! Truncation policy: applied once at emission, then byte-frozen in the
//! transcript (append-only cache discipline). Marker phrasing is API surface
//! for small models, so every marker names the next action and the exact
//! strings are frozen in the golden tests below.

use std::borrow::Cow;

pub const READ_LINE_CAP: usize = 500; // default lines per page
pub const READ_LINE_CHAR_CAP: usize = 500; // chars per line
pub const READ_BYTE_CAP: usize = 40 * 1024; // hard cap per read result
pub const BASH_HEAD: usize = 8 * 1024;
pub const BASH_TAIL: usize = 16 * 1024;
pub const GREP_MATCH_CAP: usize = 100;
pub const GREP_BYTE_CAP: usize = 16 * 1024;
pub const LIST_ENTRY_CAP: usize = 200; // glob and ls

/// Largest byte index <= `at` that is a char boundary of `s`.
pub fn floor_char_boundary(s: &str, at: usize) -> usize {
    if at >= s.len() {
        return s.len();
    }
    let mut i = at;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Smallest byte index >= `at` that is a char boundary of `s`.
pub fn ceil_char_boundary(s: &str, at: usize) -> usize {
    if at >= s.len() {
        return s.len();
    }
    let mut i = at;
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Clip one line to `READ_LINE_CHAR_CAP` characters.
pub fn clip_line(line: &str) -> Cow<'_, str> {
    let total = line.chars().count();
    if total <= READ_LINE_CHAR_CAP {
        return Cow::Borrowed(line);
    }
    let cut: usize = line
        .char_indices()
        .nth(READ_LINE_CHAR_CAP)
        .map(|(i, _)| i)
        .unwrap_or(line.len());
    Cow::Owned(format!(
        "{} [line clipped; {total} chars total]",
        &line[..cut]
    ))
}

/// Head+tail truncation for bash output: keep the first `head` and last
/// `tail` bytes (rounded to char boundaries) with an omission marker between.
/// Tail-heavy because compilers and test runners put the verdict last.
pub fn head_tail(s: &str, head: usize, tail: usize) -> Cow<'_, str> {
    if s.len() <= head + tail {
        return Cow::Borrowed(s);
    }
    let head_end = floor_char_boundary(s, head);
    let tail_start = ceil_char_boundary(s, s.len() - tail);
    let omitted = tail_start - head_end;
    Cow::Owned(format!(
        "{}\n[output truncated: {omitted} bytes omitted from the middle; the start and \
         end are shown; narrow the command if you need the omitted part]\n{}",
        &s[..head_end],
        &s[tail_start..]
    ))
}

/// Marker for a `read` that hit the 40 KiB hard cap mid-file.
pub fn read_byte_cap_marker(next_line: usize) -> String {
    format!("[output capped at 40 KiB; continue with offset={next_line}]")
}

/// Trailer for grep: always states the total count; when capped it names the
/// next action (phrasing from the architecture spec, frozen).
pub fn grep_trailer(total: usize, shown: usize) -> String {
    if total > shown {
        format!("{total} matches, showing {shown}; narrow the pattern or add a glob")
    } else if total == 1 {
        "1 match".to_string()
    } else {
        format!("{total} matches")
    }
}

/// Trailer for glob and ls when the entry cap bites.
pub fn list_trailer(kind: &str, total: usize, shown: usize) -> Option<String> {
    (total > shown).then(|| format!("{total} {kind}, showing {shown}; narrow the pattern"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Golden tests: these freeze the exact marker phrasing. A failure here
    // means the tool-result API surface changed; that is a decision, not a
    // refactor.

    #[test]
    fn golden_clip_line() {
        let long: String = "x".repeat(600);
        let clipped = clip_line(&long);
        assert_eq!(
            clipped,
            format!("{} [line clipped; 600 chars total]", "x".repeat(500))
        );
        assert!(matches!(clip_line("short"), Cow::Borrowed("short")));
    }

    #[test]
    fn golden_head_tail_marker() {
        let s = format!("{}{}{}", "a".repeat(10), "b".repeat(30), "c".repeat(10));
        let out = head_tail(&s, 10, 10);
        assert_eq!(
            out,
            format!(
                "{}\n[output truncated: 30 bytes omitted from the middle; the start and \
                 end are shown; narrow the command if you need the omitted part]\n{}",
                "a".repeat(10),
                "c".repeat(10)
            )
        );
    }

    #[test]
    fn head_tail_returns_borrowed_when_it_fits() {
        assert!(matches!(head_tail("small", 8192, 16384), Cow::Borrowed(_)));
    }

    #[test]
    fn head_tail_respects_char_boundaries() {
        // A multibyte char straddling both cut points must not split.
        let s = format!("{}é{}é{}", "a".repeat(9), "b".repeat(30), "c".repeat(9));
        let out = head_tail(&s, 10, 10);
        assert!(out.contains("[output truncated:"));
        // The é at byte 9..11 straddles head=10: floor to 9, so it is dropped
        // from the head, never split.
        assert!(out.starts_with(&"a".repeat(9)));
        assert!(std::str::from_utf8(out.as_bytes()).is_ok());
    }

    #[test]
    fn golden_read_cap_marker() {
        assert_eq!(
            read_byte_cap_marker(213),
            "[output capped at 40 KiB; continue with offset=213]"
        );
    }

    #[test]
    fn golden_grep_trailer() {
        assert_eq!(
            grep_trailer(312, 100),
            "312 matches, showing 100; narrow the pattern or add a glob"
        );
        assert_eq!(grep_trailer(12, 12), "12 matches");
        assert_eq!(grep_trailer(1, 1), "1 match");
        assert_eq!(grep_trailer(0, 0), "0 matches");
    }

    #[test]
    fn golden_list_trailer() {
        assert_eq!(
            list_trailer("files", 431, 200),
            Some("431 files, showing 200; narrow the pattern".to_string())
        );
        assert_eq!(list_trailer("entries", 5, 5), None);
    }
}
