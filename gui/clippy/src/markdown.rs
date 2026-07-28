//! The model writes Markdown. This renders it instead of showing the marks.
//!
//! A local model answers in headings, bold, bullets and fenced code whether or
//! not anything asked it to, so a transcript that prints `**read**` and
//! ```` ```python ```` verbatim is showing its working rather than its answer.
//!
//! Deliberately a formatter and not a parser. It works one line at a time, with
//! one bit of carried state (whether a fence is open), because a transcript is
//! rendered from a scrolling window and re-parsing six thousand lines to draw
//! forty is the wrong shape. Everything it does not recognise passes through
//! unchanged, which is the correct rendering for prose.

use noob_draw::Run;

use crate::skin::Skin;
use crate::syntax;

/// A fence that is open, and the language it named.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Fence(pub Option<String>);

impl Fence {
    /// Whether a block is open here.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn open(&self) -> bool {
        self.0.is_some()
    }

    /// Toggle on a fence line, returning whether this line was one.
    fn toggle(&mut self, line: &str) -> bool {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("```") && !trimmed.starts_with("~~~") {
            return false;
        }
        self.0 = match self.0 {
            Some(_) => None,
            None => Some(trimmed[3..].trim().to_ascii_lowercase()),
        };
        true
    }
}

/// Where a fence stands after `lines`, so a window that starts mid-block knows
/// it is looking at code.
pub fn fence_after<'a>(lines: impl Iterator<Item = &'a str>) -> Fence {
    let mut fence = Fence::default();
    for line in lines {
        fence.toggle(line);
    }
    fence
}

/// Append one line as runs, updating the fence state.
pub fn line(text: &str, fence: &mut Fence, skin: &Skin, out: &mut Vec<Run>) {
    if fence.toggle(text) {
        // The fence itself is structure, not content: show the language it
        // named rather than the backticks.
        let label = match &fence.0 {
            Some(lang) if !lang.is_empty() => format!("── {lang} "),
            Some(_) => String::from("── code "),
            None => String::from("──"),
        };
        out.push(Run::tinted(label, skin.comment));
        return;
    }
    if let Some(lang) = &fence.0 {
        let syntax = syntax::for_language(lang);
        for (fragment, token) in syntax::scan(text, syntax) {
            out.push(Run::tinted(fragment, skin.token(token).unwrap_or(skin.body)));
        }
        return;
    }

    let trimmed = text.trim_start();
    let indent = &text[..text.len() - trimmed.len()];

    // A heading: the hashes are the mark, not the text.
    if let Some(rest) = trimmed.strip_prefix('#') {
        let level = 1 + rest.chars().take_while(|c| *c == '#').count();
        let title = rest.trim_start_matches('#').trim_start();
        if !title.is_empty() || level > 1 {
            out.push(Run::tinted(indent, skin.body));
            out.push(Run::tinted(
                if level <= 2 { "▌ " } else { "  " },
                skin.markup,
            ));
            inline(title, if level <= 2 { skin.bright } else { skin.markup }, skin, out);
            return;
        }
    }
    // A horizontal rule, drawn rather than spelled.
    if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-' || c == '=' || c == '*') {
        out.push(Run::tinted(
            "─".repeat(trimmed.len().min(60)),
            skin.comment,
        ));
        return;
    }
    // A quote.
    if let Some(rest) = trimmed.strip_prefix("> ") {
        out.push(Run::tinted(format!("{indent}│ "), skin.comment));
        inline(rest, skin.dim, skin, out);
        return;
    }
    // A bullet or a numbered item: the marker is structure.
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            out.push(Run::tinted(format!("{indent}• "), skin.markup));
            inline(rest, skin.body, skin, out);
            return;
        }
    }
    if let Some((number, rest)) = numbered(trimmed) {
        out.push(Run::tinted(format!("{indent}{number} "), skin.markup));
        inline(rest, skin.body, skin, out);
        return;
    }

    out.push(Run::tinted(indent, skin.body));
    inline(trimmed, skin.body, skin, out);
}

/// `1. text` or `12) text`, returning the marker and the rest.
fn numbered(line: &str) -> Option<(&str, &str)> {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 3 {
        return None;
    }
    let after = &line[digits..];
    let rest = after.strip_prefix(". ").or_else(|| after.strip_prefix(") "))?;
    Some((&line[..digits + 1], rest))
}

/// `**bold**`, `*emphasis*` and `` `code` `` inside one line.
///
/// Scanned rather than parsed, so an unmatched marker is text. That matters:
/// prose is full of lone asterisks and stray backticks, and a parser that
/// insists on pairing them swallows the rest of the paragraph.
fn inline(text: &str, base: [u8; 4], skin: &Skin, out: &mut Vec<Run>) {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let at = |k: usize| chars.get(k).map_or(text.len(), |(byte, _)| *byte);
    let mut k = 0;
    let mut plain_from = 0;

    macro_rules! flush {
        ($end:expr) => {
            if $end > plain_from {
                out.push(Run::tinted(&text[plain_from..$end], base));
            }
        };
    }

    while k < chars.len() {
        let (byte, c) = chars[k];
        let rest = &text[byte..];
        // Longest marker first, or `**bold**` reads as two emphases.
        for (marker, color) in [
            ("**", skin.bright),
            ("`", skin.string),
            ("__", skin.bright),
            ("*", skin.markup),
        ] {
            if !rest.starts_with(marker) {
                continue;
            }
            let width = marker.chars().count();
            let Some(close) = find(&chars, text, k + width, marker) else {
                continue;
            };
            // Tight, the way CommonMark requires: the opener is followed by a
            // non-space and the closer preceded by one. Without this, `a * b *
            // c` reads as an emphasis and comes out as `a  b  c`, and prose is
            // full of multiplication signs and bullets in the middle of lines.
            let opens = chars.get(k + width).is_some_and(|(_, c)| !c.is_whitespace());
            let closes = close > 0 && chars[close - 1].1 != ' ';
            if !opens || !closes {
                continue;
            }
            flush!(byte);
            let inner = &text[at(k + width)..at(close)];
            if inner.is_empty() {
                continue;
            }
            out.push(Run::tinted(inner, color));
            k = close + width;
            plain_from = at(k);
            break;
        }
        if plain_from <= byte {
            k += 1;
        }
        let _ = c;
    }
    flush!(text.len());
    if out.is_empty() {
        out.push(Run::tinted(text, base));
    }
}

/// The char index where `marker` next occurs at or after `from`.
fn find(chars: &[(usize, char)], text: &str, from: usize, marker: &str) -> Option<usize> {
    (from..chars.len()).find(|k| text[chars[*k].0..].starts_with(marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn skin() -> Skin {
        Skin::from(&Config::default())
    }

    fn render(text: &str) -> (String, Vec<Run>) {
        let skin = skin();
        let mut fence = Fence::default();
        let mut runs = Vec::new();
        for line in text.lines() {
            self::line(line, &mut fence, &skin, &mut runs);
            runs.push(Run::plain("\n"));
        }
        let flat: String = runs.iter().map(|r| r.text.as_str()).collect();
        (flat, runs)
    }

    /// The marks are the formatting, so they must not survive into the text.
    #[test]
    fn the_marks_are_replaced_by_the_formatting() {
        let (text, _) = render("## Notable Features\n- **read** a file\n`inline` code\n");
        assert!(!text.contains("##"), "{text}");
        assert!(!text.contains("**"), "{text}");
        assert!(!text.contains('`'), "{text}");
        assert!(text.contains("Notable Features"), "{text}");
        assert!(text.contains("read"), "{text}");
        assert!(text.contains("inline"), "{text}");
    }

    #[test]
    fn bold_and_code_get_their_own_colors() {
        let skin = skin();
        let (_, runs) = render("- **write** creates `a.txt`");
        let colors: Vec<Option<[u8; 4]>> = runs.iter().map(|r| r.color).collect();
        assert!(colors.contains(&Some(skin.bright)), "bold");
        assert!(colors.contains(&Some(skin.string)), "code");
        assert!(colors.contains(&Some(skin.markup)), "the bullet");
    }

    /// A fenced block is code, colored by the language the fence named.
    #[test]
    fn a_fenced_block_is_syntax_colored_by_its_language() {
        let skin = skin();
        let (text, runs) = render("```python\nx = \"hi\"  # note\n```\n");
        assert!(!text.contains("```"), "{text}");
        assert!(text.contains("python"), "the language is still named: {text}");
        assert!(text.contains("x = \"hi\"  # note"), "{text}");
        let colors: Vec<Option<[u8; 4]>> = runs.iter().map(|r| r.color).collect();
        assert!(colors.contains(&Some(skin.string)), "the string is tinted");
        assert!(colors.contains(&Some(skin.comment)), "the comment is tinted");
    }

    /// Inside a fence, a `#` is a comment and a `-` is not a bullet.
    #[test]
    fn markdown_marks_do_not_apply_inside_code() {
        let (text, _) = render("```sh\n- not a bullet\n# a comment\n```\n");
        assert!(text.contains("- not a bullet"), "{text}");
        assert!(text.contains("# a comment"), "{text}");
        assert!(!text.contains('•'), "{text}");
    }

    /// A window that starts inside a block has to know it is inside one.
    #[test]
    fn the_fence_state_survives_a_scrolling_window() {
        let above = ["prose", "```rust", "let x = 1;"];
        let fence = fence_after(above.into_iter());
        assert!(fence.open());
        assert_eq!(fence.0.as_deref(), Some("rust"));
        let closed = fence_after(["```py", "x=1", "```"].into_iter());
        assert!(!closed.open());
    }

    /// Prose is full of lone asterisks and stray backticks. A parser that
    /// insists on pairing them swallows the rest of the paragraph.
    #[test]
    fn an_unmatched_marker_is_just_text() {
        for line in [
            "2 * 3 = 6",
            "an unclosed `backtick here",
            "**never closed",
            "a * b * c",
            "3 * 4 * 5 = 60",
            "`",
            "spaced ` backtick ` pair",
        ] {
            let (text, _) = render(line);
            assert_eq!(text.trim_end(), line, "{line:?} was mangled");
        }
    }

    /// Whatever it does, reassembling the runs must give the text back, minus
    /// only the marks it deliberately consumed.
    #[test]
    fn plain_prose_passes_through_untouched() {
        for line in [
            "",
            "Just a sentence.",
            "   indented prose",
            "a line with 3 - 1 = 2 in it",
            "héllo wörld 日本語",
        ] {
            let (text, _) = render(line);
            assert_eq!(text.trim_end_matches('\n'), line, "{line:?}");
        }
    }

    /// A line of nothing but rule characters is a thematic break, including
    /// one made of asterisks, which is why it is not in the unmatched-marker
    /// list above.
    #[test]
    fn a_line_of_only_rule_characters_is_a_rule() {
        for rule in ["---", "***", "====", "****"] {
            let (text, _) = render(rule);
            assert!(text.contains('─'), "{rule:?} rendered as {text:?}");
            assert!(!text.contains('*') && !text.contains('='), "{text:?}");
        }
    }

    #[test]
    fn lists_quotes_and_rules_all_render() {
        let (text, _) = render("- one\n* two\n1. three\n> quoted\n---\n");
        assert_eq!(text.matches('•').count(), 2, "{text}");
        assert!(text.contains("1. three"), "{text}");
        assert!(text.contains("│ quoted"), "{text}");
        assert!(text.contains('─'), "the rule is drawn: {text}");
        assert!(!text.contains("---"), "{text}");
    }

    #[test]
    fn indentation_is_kept_so_nested_lists_stay_nested() {
        let (text, _) = render("- top\n  - nested\n");
        assert!(text.contains("\n  • nested"), "{text:?}");
    }

    /// A heading with nothing after the hashes is prose, not an empty heading.
    #[test]
    fn a_bare_hash_is_not_a_heading() {
        let (text, _) = render("# \nplain");
        assert!(text.contains("plain"));
    }
}
