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
pub const SKILL_BYTE_CAP: usize = 24 * 1024; // skill body per load
pub const MCP_HEAD: usize = 8 * 1024; // mcp results: 20 KiB head+tail,
pub const MCP_TAIL: usize = 12 * 1024; // tail-heavy like bash

/// The session's truncation policy, resolved once at bootstrap from
/// NOOB_TOOL_CAPS. `default()` is the shipped policy above; `uncapped()`
/// (NOOB_TOOL_CAPS=0/off) sets every limit to usize::MAX so tool results
/// flow through whole and no truncation marker can ever render. Uncapped
/// mode is for large-context setups; on a small window it trades the
/// bounded pages for raw output the compactor then has to swallow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub read_lines: usize,
    pub line_chars: usize,
    pub read_bytes: usize,
    pub bash_head: usize,
    pub bash_tail: usize,
    pub grep_matches: usize,
    pub grep_bytes: usize,
    pub list_entries: usize,
    pub skill_bytes: usize,
    pub mcp_head: usize,
    pub mcp_tail: usize,
}

impl Default for Caps {
    fn default() -> Self {
        Caps {
            read_lines: READ_LINE_CAP,
            line_chars: READ_LINE_CHAR_CAP,
            read_bytes: READ_BYTE_CAP,
            bash_head: BASH_HEAD,
            bash_tail: BASH_TAIL,
            grep_matches: GREP_MATCH_CAP,
            grep_bytes: GREP_BYTE_CAP,
            list_entries: LIST_ENTRY_CAP,
            skill_bytes: SKILL_BYTE_CAP,
            mcp_head: MCP_HEAD,
            mcp_tail: MCP_TAIL,
        }
    }
}

impl Caps {
    pub fn uncapped() -> Self {
        Caps {
            read_lines: usize::MAX,
            line_chars: usize::MAX,
            read_bytes: usize::MAX,
            bash_head: usize::MAX,
            bash_tail: usize::MAX,
            grep_matches: usize::MAX,
            grep_bytes: usize::MAX,
            list_entries: usize::MAX,
            skill_bytes: usize::MAX,
            mcp_head: usize::MAX,
            mcp_tail: usize::MAX,
        }
    }
}


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

/// Clip one line to `cap` characters (READ_LINE_CHAR_CAP under the default
/// policy; usize::MAX when uncapped, which never clips).
pub fn clip_line(line: &str, cap: usize) -> Cow<'_, str> {
    let total = line.chars().count();
    if total <= cap {
        return Cow::Borrowed(line);
    }
    let cut: usize = line
        .char_indices()
        .nth(cap)
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
#[cfg(test)]
pub fn head_tail(s: &str, head: usize, tail: usize) -> Cow<'_, str> {
    head_tail_with(
        s,
        head,
        tail,
        "narrow the command if you need the omitted part",
    )
}

/// Same shape with a caller-supplied next action, because the marker is API
/// surface: "narrow the command" teaches nothing on an MCP result.
pub fn head_tail_with<'a>(s: &'a str, head: usize, tail: usize, next_action: &str) -> Cow<'a, str> {
    if s.len() <= head.saturating_add(tail) {
        return Cow::Borrowed(s);
    }
    let head_end = floor_char_boundary(s, head);
    let tail_start = ceil_char_boundary(s, s.len() - tail);
    let omitted = tail_start - head_end;
    Cow::Owned(format!(
        "{}\n[output truncated: {omitted} bytes omitted from the middle; the start and \
         end are shown; {next_action}]\n{}",
        &s[..head_end],
        &s[tail_start..]
    ))
}

/// The MCP result cap: 20 KiB head+tail by default, pass-through when
/// uncapped, with an MCP-appropriate next action.
pub fn mcp_cap<'a>(s: &'a str, caps: &Caps) -> Cow<'a, str> {
    head_tail_with(
        s,
        caps.mcp_head,
        caps.mcp_tail,
        "ask the tool for less data if you need the omitted part",
    )
}

/// Marker for a `read` that hit the 40 KiB hard cap mid-file.
pub fn read_byte_cap_marker(next_line: usize) -> String {
    format!("[output capped at 40 KiB; continue with offset={next_line}]")
}

/// Marker for a skill body that hit the 24 KiB cap; points at the ordinary
/// read tool for the remainder.
pub fn skill_cap_marker(path: &str, next_line: usize) -> String {
    format!("[skill body capped at 24 KiB; read the rest with read {path} offset={next_line}]")
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

/// Trailer for glob when the entry cap bites ("narrow the pattern" is the
/// right remedy only for a pattern-taking tool; ls passes its own).
pub fn list_trailer(kind: &str, total: usize, shown: usize) -> Option<String> {
    list_trailer_with(kind, total, shown, "narrow the pattern")
}

/// Same shape with a caller-supplied remedy, because the trailer is API
/// surface: ls has no pattern to narrow.
pub fn list_trailer_with(kind: &str, total: usize, shown: usize, remedy: &str) -> Option<String> {
    (total > shown).then(|| format!("{total} {kind}, showing {shown}; {remedy}"))
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
        let clipped = clip_line(&long, READ_LINE_CHAR_CAP);
        assert_eq!(
            clipped,
            format!("{} [line clipped; 600 chars total]", "x".repeat(500))
        );
        assert!(matches!(
            clip_line("short", READ_LINE_CHAR_CAP),
            Cow::Borrowed("short")
        ));
        // Uncapped: never clips, whatever the length.
        assert!(matches!(clip_line(&long, usize::MAX), Cow::Borrowed(_)));
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
    fn golden_mcp_cap_marker() {
        let s = format!(
            "{}{}{}",
            "a".repeat(MCP_HEAD),
            "b".repeat(64),
            "c".repeat(MCP_TAIL)
        );
        let out = mcp_cap(&s, &Caps::default());
        assert!(
            out.contains(
                "[output truncated: 64 bytes omitted from the middle; the start and \
                 end are shown; ask the tool for less data if you need the omitted part]"
            ),
            "{}",
            &out[MCP_HEAD..MCP_HEAD + 200]
        );
        assert!(matches!(
            mcp_cap("small", &Caps::default()),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn uncapped_mcp_results_pass_through_whole() {
        let s = format!("{}{}", "a".repeat(MCP_HEAD + MCP_TAIL), "b".repeat(4096));
        let out = mcp_cap(&s, &Caps::uncapped());
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, s);
    }

    #[test]
    fn uncapped_head_tail_never_truncates_and_never_overflows() {
        let s = "x".repeat(64 * 1024);
        let out = head_tail_with(&s, usize::MAX, usize::MAX, "unused");
        assert!(matches!(out, Cow::Borrowed(_)));
    }


    #[test]
    fn caps_defaults_mirror_the_constants() {
        let caps = Caps::default();
        assert_eq!(caps.read_lines, READ_LINE_CAP);
        assert_eq!(caps.line_chars, READ_LINE_CHAR_CAP);
        assert_eq!(caps.read_bytes, READ_BYTE_CAP);
        assert_eq!(caps.bash_head, BASH_HEAD);
        assert_eq!(caps.bash_tail, BASH_TAIL);
        assert_eq!(caps.grep_matches, GREP_MATCH_CAP);
        assert_eq!(caps.grep_bytes, GREP_BYTE_CAP);
        assert_eq!(caps.list_entries, LIST_ENTRY_CAP);
        assert_eq!(caps.skill_bytes, SKILL_BYTE_CAP);
        assert_eq!(caps.mcp_head, MCP_HEAD);
        assert_eq!(caps.mcp_tail, MCP_TAIL);
    }

    #[test]
    fn golden_read_cap_marker() {
        assert_eq!(
            read_byte_cap_marker(213),
            "[output capped at 40 KiB; continue with offset=213]"
        );
    }

    #[test]
    fn golden_skill_cap_marker() {
        assert_eq!(
            skill_cap_marker(".claude/skills/pdf/SKILL.md", 812),
            "[skill body capped at 24 KiB; read the rest with read \
             .claude/skills/pdf/SKILL.md offset=812]"
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
        assert_eq!(
            list_trailer_with("entries", 250, 200, "list a subdirectory instead"),
            Some("250 entries, showing 200; list a subdirectory instead".to_string())
        );
        assert_eq!(
            list_trailer_with("entries", 5, 5, "list a subdirectory instead"),
            None
        );
    }
}
