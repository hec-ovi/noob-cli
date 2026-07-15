//! A zero-dependency, streaming markdown-lite renderer for the styled REPL.
//!
//! `Markdown` buffers at most one ordinary source line so syntax may straddle
//! arbitrary provider deltas. Tables and fenced blocks have separate explicit
//! bounds; crossing one flushes content and degrades to a streaming/plain form
//! rather than dropping any model output. All input terminal controls are made
//! visible before parsing. Every generated style span is locally reset, so a
//! malformed response cannot leak SGR state into a tool line or prompt.
//!
//! Intended integration in `Ui`: keep one `Markdown` per `Ui`, route only the
//! styled REPL branch of `text_delta` through `feed`, and drain `finish` before
//! the existing end-of-message newline/reset. Do not wrap the returned bytes in
//! the old one-shot assistant tint: this renderer supplies balanced styles.

use super::style::{ColorDepth, RESET, Style, paint};
use super::table::{self, Table};
use super::theme::Theme;

/// An adversarial response cannot retain an unbounded line while withholding a
/// newline. Reaching this bound inserts a display line break and streams the
/// continuation. It does not truncate the source.
pub(crate) const MAX_LINE_CHARS: usize = 16 * 1024;
/// Fences buffer briefly so an incomplete fence can be returned as legible
/// source. Past either bound they switch to rendered streaming mode.
pub(crate) const MAX_FENCE_BYTES: usize = 32 * 1024;
pub(crate) const MAX_FENCE_LINES: usize = 512;

#[derive(Default)]
pub(crate) struct Markdown {
    line: String,
    line_chars: usize,
    /// A CR was already normalized to a newline; suppress a following LF even
    /// when the CRLF pair straddles provider deltas.
    skip_lf: bool,
    /// The current logical line crossed MAX_LINE_CHARS and was displayed in
    /// chunks. Its tail must remain literal until the real newline arrives.
    spilled_line: bool,
    pending_table_header: Option<SourceLine>,
    table: Option<TableState>,
    fence: Option<Fence>,
}

#[derive(Clone)]
struct SourceLine {
    text: String,
    newline: bool,
}

enum TableState {
    Buffered(Table),
    /// The table hit a buffer bound. Subsequent pipe rows stream as readable
    /// source until a non-row ends the region.
    Raw {
        columns: usize,
    },
}

struct Fence {
    marker: char,
    marker_len: usize,
    language: String,
    opener: SourceLine,
    lines: Vec<SourceLine>,
    buffered_bytes: usize,
    streaming: bool,
}

impl Markdown {
    pub(crate) fn new() -> Markdown {
        Markdown::default()
    }

    /// Consume one provider text delta and return any complete rendered region.
    /// An empty result only means the parser is retaining the current line or a
    /// bounded table/fence, never that source was discarded.
    pub(crate) fn feed(
        &mut self,
        delta: &str,
        width: usize,
        theme: &Theme,
        depth: ColorDepth,
    ) -> String {
        let mut out = String::new();
        for ch in delta.chars() {
            if self.skip_lf {
                self.skip_lf = false;
                if ch == '\n' {
                    continue;
                }
            }
            match ch {
                '\r' => {
                    out.push_str(&self.complete_line(true, width, theme, depth));
                    self.skip_lf = true;
                }
                '\n' => out.push_str(&self.complete_line(true, width, theme, depth)),
                '\t' => self.push_safe("    "),
                c if is_c0_or_del(c) => {
                    let shown = control_picture(c);
                    self.push_safe(&shown);
                }
                c if is_c1(c) => self.push_safe("�"),
                c => {
                    self.line.push(c);
                    self.line_chars += 1;
                }
            }
            if self.line_chars >= MAX_LINE_CHARS {
                out.push_str(&self.flush_long_line_chunk(width, theme, depth));
            }
        }
        out
    }

    /// Flush the final partial line and every bounded construct. Call once at
    /// assistant-message end; the renderer is then clean for the next message.
    pub(crate) fn finish(&mut self, width: usize, theme: &Theme, depth: ColorDepth) -> String {
        let mut out = String::new();
        if !self.line.is_empty() {
            out.push_str(&self.complete_line(false, width, theme, depth));
        }
        if let Some(fence) = self.fence.take() {
            out.push_str(&fence.render_incomplete(theme, depth));
        }
        out.push_str(&self.flush_table(width, theme, depth));
        if let Some(line) = self.pending_table_header.take() {
            out.push_str(&render_normal_line(&line, theme, depth));
        }
        self.line.clear();
        self.line_chars = 0;
        self.skip_lf = false;
        self.spilled_line = false;
        out
    }

    /// Useful to an integrating `Ui` deciding whether its end-of-message path
    /// has parser state to drain.
    pub(crate) fn has_pending(&self) -> bool {
        !self.line.is_empty()
            || self.skip_lf
            || self.spilled_line
            || self.pending_table_header.is_some()
            || self.table.is_some()
            || self.fence.is_some()
    }

    fn push_safe(&mut self, s: &str) {
        self.line.push_str(s);
        self.line_chars += s.chars().count();
    }

    fn complete_line(
        &mut self,
        newline: bool,
        width: usize,
        theme: &Theme,
        depth: ColorDepth,
    ) -> String {
        let text = std::mem::take(&mut self.line);
        self.line_chars = 0;
        if self.spilled_line {
            self.spilled_line = false;
            if text.is_empty() {
                return String::new();
            }
            let line = SourceLine { text, newline };
            if let Some(mut fence) = self.fence.take() {
                let out = fence.push_code(line, theme, depth);
                self.fence = Some(fence);
                return out;
            }
            return render_literal_line(&line, theme, depth);
        }
        self.process_line(SourceLine { text, newline }, width, theme, depth)
    }

    fn flush_long_line_chunk(&mut self, width: usize, theme: &Theme, depth: ColorDepth) -> String {
        let text = std::mem::take(&mut self.line);
        self.line_chars = 0;
        self.spilled_line = true;
        let line = SourceLine {
            text,
            newline: true,
        };
        if let Some(mut fence) = self.fence.take() {
            let out = fence.push_code(line, theme, depth);
            self.fence = Some(fence);
            return out;
        }

        let mut out = self.flush_table(width, theme, depth);
        if let Some(header) = self.pending_table_header.take() {
            out.push_str(&render_normal_line(&header, theme, depth));
        }
        out.push_str(&render_literal_line(&line, theme, depth));
        out
    }

    fn process_line(
        &mut self,
        line: SourceLine,
        width: usize,
        theme: &Theme,
        depth: ColorDepth,
    ) -> String {
        if let Some(mut fence) = self.fence.take() {
            if is_fence_close(&line.text, fence.marker, fence.marker_len) {
                return fence.render_complete(line.newline, theme, depth);
            }
            let out = fence.push_code(line, theme, depth);
            self.fence = Some(fence);
            return out;
        }
        self.process_normal_state(line, width, theme, depth)
    }

    fn process_normal_state(
        &mut self,
        line: SourceLine,
        width: usize,
        theme: &Theme,
        depth: ColorDepth,
    ) -> String {
        let mut out = String::new();
        let current = line;
        if let Some(state) = self.table.take() {
            match state {
                TableState::Buffered(mut table) => {
                    if table.accepts_row(&current.text) {
                        if table.push_row(&current.text).is_ok() {
                            self.table = Some(TableState::Buffered(table));
                        } else {
                            let columns = table.columns();
                            out.push_str(&table.render(width, theme, depth));
                            out.push_str(&render_literal_line(&current, theme, depth));
                            self.table = Some(TableState::Raw { columns });
                        }
                        return out;
                    }
                    out.push_str(&table.render(width, theme, depth));
                }
                TableState::Raw { columns } => {
                    if table::looks_like_body_row(&current.text, columns) {
                        out.push_str(&render_literal_line(&current, theme, depth));
                        self.table = Some(TableState::Raw { columns });
                        return out;
                    }
                }
            }
        }

        if let Some(header) = self.pending_table_header.take() {
            if let Some(table) = Table::new(&header.text, &current.text) {
                self.table = Some(TableState::Buffered(table));
                return out;
            }
            out.push_str(&render_normal_line(&header, theme, depth));
        }

        if let Some((marker, marker_len, language)) = parse_fence_open(&current.text) {
            self.fence = Some(Fence {
                marker,
                marker_len,
                language,
                buffered_bytes: current.text.len(),
                opener: current,
                lines: Vec::new(),
                streaming: false,
            });
            return out;
        }

        if table::could_be_header(&current.text) {
            self.pending_table_header = Some(current);
            return out;
        }

        out.push_str(&render_normal_line(&current, theme, depth));
        out
    }

    fn flush_table(&mut self, width: usize, theme: &Theme, depth: ColorDepth) -> String {
        match self.table.take() {
            Some(TableState::Buffered(table)) => table.render(width, theme, depth),
            Some(TableState::Raw { .. }) | None => String::new(),
        }
    }
}

impl Fence {
    fn push_code(&mut self, line: SourceLine, theme: &Theme, depth: ColorDepth) -> String {
        if !self.streaming
            && self.lines.len() < MAX_FENCE_LINES
            && self.buffered_bytes.saturating_add(line.text.len()) <= MAX_FENCE_BYTES
        {
            self.buffered_bytes += line.text.len();
            self.lines.push(line);
            return String::new();
        }

        let mut out = String::new();
        if !self.streaming {
            self.streaming = true;
            out.push_str(&render_fence_top(&self.language, theme, depth));
            for buffered in self.lines.drain(..) {
                out.push_str(&render_code_line(&buffered, &self.language, theme, depth));
            }
        }
        out.push_str(&render_code_line(&line, &self.language, theme, depth));
        out
    }

    fn render_complete(mut self, final_newline: bool, theme: &Theme, depth: ColorDepth) -> String {
        let mut out = String::new();
        if !self.streaming {
            out.push_str(&render_fence_top(&self.language, theme, depth));
            for line in self.lines.drain(..) {
                out.push_str(&render_code_line(&line, &self.language, theme, depth));
            }
        }
        out.push_str(&render_fence_bottom(final_newline, None, theme, depth));
        out
    }

    fn render_incomplete(mut self, theme: &Theme, depth: ColorDepth) -> String {
        if self.streaming {
            return render_fence_bottom(false, Some("unclosed code fence"), theme, depth);
        }
        // Nothing escaped yet: preserve the malformed construct as source. It
        // is already sanitized, so literal fence markers are safe and clearest.
        let mut out = render_literal_line(&self.opener, theme, depth);
        for line in self.lines.drain(..) {
            out.push_str(&render_literal_line(&line, theme, depth));
        }
        out
    }
}

fn parse_fence_open(line: &str) -> Option<(char, usize, String)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len().saturating_sub(trimmed.len()) > 3 {
        return None;
    }
    let marker = trimmed.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let marker_len = trimmed.chars().take_while(|c| *c == marker).count();
    if marker_len < 3 {
        return None;
    }
    let marker_bytes = marker.len_utf8() * marker_len;
    let info = trimmed[marker_bytes..].trim();
    if marker == '`' && info.contains('`') {
        return None;
    }
    let language = info
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    Some((marker, marker_len, language))
}

fn is_fence_close(line: &str, marker: char, minimum: usize) -> bool {
    let trimmed = line.trim_start_matches(' ');
    if line.len().saturating_sub(trimmed.len()) > 3 {
        return false;
    }
    let count = trimmed.chars().take_while(|c| *c == marker).count();
    if count < minimum {
        return false;
    }
    trimmed[marker.len_utf8() * count..].trim().is_empty()
}

fn render_fence_top(language: &str, theme: &Theme, depth: ColorDepth) -> String {
    let label = if language.is_empty() {
        "code"
    } else {
        language
    };
    let mut out = paint(theme.activity, depth, "┌─ ");
    out.push_str(&paint(theme.label_style(label), depth, label));
    out.push('\n');
    out
}

fn render_fence_bottom(
    newline: bool,
    note: Option<&str>,
    theme: &Theme,
    depth: ColorDepth,
) -> String {
    let text = note.map_or_else(|| "└─".to_string(), |n| format!("└─ [{n}]"));
    let mut out = paint(theme.activity, depth, &text);
    if newline {
        out.push('\n');
    }
    out
}

fn render_code_line(line: &SourceLine, language: &str, theme: &Theme, depth: ColorDepth) -> String {
    let mut out = paint(theme.activity, depth, "│ ");
    if matches!(language, "json" | "jsonc" | "json5") {
        out.push_str(&render_json(&line.text, theme, depth));
    } else {
        out.push_str(&paint(theme.assistant, depth, &line.text));
    }
    if line.newline {
        out.push('\n');
    }
    out
}

fn render_json(line: &str, theme: &Theme, depth: ColorDepth) -> String {
    let mut out = String::new();
    let mut plain = String::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            flush_painted(&mut out, &mut plain, theme.assistant, depth);
            let start = i;
            i += 1;
            let mut escaped = false;
            while i < bytes.len() {
                let c = bytes[i];
                i += 1;
                if escaped {
                    escaped = false;
                } else if c == b'\\' {
                    escaped = true;
                } else if c == b'"' {
                    break;
                }
            }
            let mut next = i;
            while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                next += 1;
            }
            let style = if bytes.get(next) == Some(&b':') {
                Style::new(theme.activity_palette[0], false)
            } else {
                Style::new(theme.activity_palette[2], false)
            };
            out.push_str(&paint(style, depth, &line[start..i]));
            continue;
        }
        if b.is_ascii_digit() || (b == b'-' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            flush_painted(&mut out, &mut plain, theme.assistant, depth);
            let start = i;
            i += 1;
            while i < bytes.len()
                && matches!(bytes[i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
            {
                i += 1;
            }
            out.push_str(&paint(
                Style::new(theme.activity_palette[4], false),
                depth,
                &line[start..i],
            ));
            continue;
        }
        let remaining = &line[i..];
        let keyword = ["true", "false", "null"]
            .into_iter()
            .find(|word| remaining.starts_with(word));
        if let Some(word) = keyword {
            flush_painted(&mut out, &mut plain, theme.assistant, depth);
            let style = if word == "null" {
                theme.note
            } else {
                Style::new(theme.activity_palette[8], false)
            };
            out.push_str(&paint(style, depth, word));
            i += word.len();
            continue;
        }
        let ch = remaining.chars().next().unwrap();
        if matches!(ch, '{' | '}' | '[' | ']' | ':' | ',') {
            flush_painted(&mut out, &mut plain, theme.assistant, depth);
            out.push_str(&paint(theme.activity, depth, &ch.to_string()));
        } else {
            plain.push(ch);
        }
        i += ch.len_utf8();
    }
    flush_painted(&mut out, &mut plain, theme.assistant, depth);
    out
}

fn render_normal_line(line: &SourceLine, theme: &Theme, depth: ColorDepth) -> String {
    if line.text.is_empty() {
        return if line.newline {
            "\n".to_string()
        } else {
            String::new()
        };
    }
    let indent_bytes = line.text.len() - line.text.trim_start_matches(' ').len();
    let indent = &line.text[..indent_bytes];
    let mut content = &line.text[indent_bytes..];
    let mut out = String::new();
    out.push_str(indent);

    // Quotes may nest. Keep their semantic marker visible independently of
    // color so NO_COLOR/depthless rendering remains understandable.
    while let Some(rest) = content.strip_prefix('>') {
        if !rest.is_empty() && !rest.starts_with(' ') {
            break;
        }
        out.push_str(&paint(theme.note, depth, "│ "));
        content = rest.strip_prefix(' ').unwrap_or(rest);
    }

    if let Some((level, rest)) = heading(content) {
        let glyph = if level <= 2 { "▌ " } else { "▪ " };
        out.push_str(&paint(theme.prompt, depth, glyph));
        out.push_str(&render_inline(rest, theme, depth, Some(theme.prompt)));
    } else if let Some(rest) = unordered_item(content) {
        out.push_str(&paint(theme.prompt, depth, "• "));
        out.push_str(&render_inline(rest, theme, depth, None));
    } else if let Some((marker, rest)) = ordered_item(content) {
        out.push_str(&paint(theme.prompt, depth, marker));
        out.push(' ');
        out.push_str(&render_inline(rest, theme, depth, None));
    } else {
        out.push_str(&render_inline(content, theme, depth, None));
    }
    if line.newline {
        out.push('\n');
    }
    out
}

fn render_literal_line(line: &SourceLine, theme: &Theme, depth: ColorDepth) -> String {
    let mut out = paint(theme.assistant, depth, &line.text);
    if line.newline {
        out.push('\n');
    }
    out
}

fn heading(s: &str) -> Option<(usize, &str)> {
    let count = s.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&count) || s.as_bytes().get(count) != Some(&b' ') {
        return None;
    }
    Some((count, &s[count + 1..]))
}

fn unordered_item(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    (bytes.len() >= 2 && matches!(bytes[0], b'-' | b'*' | b'+') && bytes[1] == b' ')
        .then(|| &s[2..])
}

fn ordered_item(s: &str) -> Option<(&str, &str)> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    let bytes = s.as_bytes();
    if !matches!(bytes.get(digits), Some(b'.' | b')')) || bytes.get(digits + 1) != Some(&b' ') {
        return None;
    }
    Some((&s[..=digits], &s[digits + 2..]))
}

fn render_inline(s: &str, theme: &Theme, depth: ColorDepth, base: Option<Style>) -> String {
    let base = base.unwrap_or(theme.assistant);
    let mut out = String::new();
    let mut plain = String::new();
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if let Some(escaped) = rest.strip_prefix('\\')
            && let Some(next) = escaped.chars().next()
            && matches!(next, '*' | '_' | '`' | '\\' | '|')
        {
            plain.push(next);
            i += 1 + next.len_utf8();
            continue;
        }
        if let Some((marker, style)) = [("**", theme.prompt), ("__", theme.prompt)]
            .into_iter()
            .find(|(marker, _)| rest.starts_with(marker))
        {
            let from = marker.len();
            if let Some(end) = rest[from..].find(marker).filter(|end| *end > 0) {
                flush_painted(&mut out, &mut plain, base, depth);
                out.push_str(&paint(style, depth, &rest[from..from + end]));
                i += from + end + marker.len();
                continue;
            }
        }
        if let Some(code) = rest.strip_prefix('`')
            && let Some(end) = code.find('`').filter(|end| *end > 0)
        {
            flush_painted(&mut out, &mut plain, base, depth);
            out.push_str(&paint(
                Style::new(theme.activity_palette[4], false),
                depth,
                &code[..end],
            ));
            i += end + 2;
            continue;
        }
        if rest.starts_with('*') || rest.starts_with('_') {
            let marker = &rest[..1];
            if let Some(end) = rest[1..].find(marker).filter(|end| *end > 0) {
                flush_painted(&mut out, &mut plain, base, depth);
                let content = &rest[1..1 + end];
                let open = italic_sgr(theme.note, depth);
                if open.is_empty() {
                    out.push_str(content);
                } else {
                    out.push_str(&open);
                    out.push_str(content);
                    out.push_str(RESET);
                }
                i += end + 2;
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        plain.push(ch);
        i += ch.len_utf8();
    }
    flush_painted(&mut out, &mut plain, base, depth);
    out
}

fn flush_painted(out: &mut String, plain: &mut String, style: Style, depth: ColorDepth) {
    if !plain.is_empty() {
        out.push_str(&paint(style, depth, plain));
        plain.clear();
    }
}

fn italic_sgr(style: Style, depth: ColorDepth) -> String {
    let mut open = style.sgr(depth);
    if open.is_empty() {
        return open;
    }
    debug_assert!(open.ends_with('m'));
    open.pop();
    open.push_str(";3m");
    open
}

fn is_c0_or_del(c: char) -> bool {
    matches!(c as u32, 0x00..=0x1f | 0x7f)
}

fn is_c1(c: char) -> bool {
    matches!(c as u32, 0x80..=0x9f)
}

fn control_picture(c: char) -> String {
    if c == '\u{7f}' {
        return "␡".to_string();
    }
    char::from_u32(0x2400 + c as u32).unwrap_or('�').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(s: &str) -> String {
        let mut out = String::new();
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'[') {
                i += 2;
                while i < bytes.len() {
                    let b = bytes[i];
                    i += 1;
                    if (0x40..=0x7e).contains(&b) {
                        break;
                    }
                }
            } else {
                let ch = s[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
        out
    }

    fn render(parts: &[&str], width: usize, depth: ColorDepth) -> String {
        let theme = Theme::matrix();
        let mut md = Markdown::new();
        let mut out = String::new();
        for part in parts {
            out.push_str(&md.feed(part, width, &theme, depth));
        }
        out.push_str(&md.finish(width, &theme, depth));
        out
    }

    #[test]
    fn block_and_inline_markdown_render_semantically() {
        let source = "# Head **bold**\n- one *em* and `code`\n12. ordered\n> quote\n";
        let shown = plain(&render(&[source], 80, ColorDepth::Truecolor));
        assert!(shown.contains("▌ Head bold"));
        assert!(shown.contains("• one em and code"));
        assert!(shown.contains("12. ordered"));
        assert!(shown.contains("│ quote"));
        assert!(!shown.contains("**") && !shown.contains("`code`"));
    }

    #[test]
    fn every_split_of_inline_markers_matches_one_chunk() {
        let source = "## A **strong** and *soft* plus `code`\n";
        let expected = render(&[source], 80, ColorDepth::Truecolor);
        for split in 0..=source.len() {
            let parts = [&source[..split], &source[split..]];
            assert_eq!(
                render(&parts, 80, ColorDepth::Truecolor),
                expected,
                "split {split}"
            );
        }
    }

    #[test]
    fn unmatched_inline_markers_remain_literal() {
        let shown = plain(&render(
            &["plain ** open and ` code\n"],
            80,
            ColorDepth::Truecolor,
        ));
        assert!(shown.contains("plain ** open and ` code"));
    }

    #[test]
    fn hostile_terminal_controls_are_visible_not_executable() {
        let source = "safe\x1b]52;c;SECRET\x07\x08\u{0085}end\r\nnext\n";
        let shown = render(&[source], 80, ColorDepth::None);
        assert!(!shown.contains('\x1b'));
        assert!(!shown.contains('\x07'));
        assert!(!shown.contains('\x08'));
        assert!(!shown.contains('\u{0085}'));
        assert!(shown.contains("␛]52;c;SECRET␇␈�end"));
        assert_eq!(shown.matches("next").count(), 1, "CRLF must be one newline");
    }

    #[test]
    fn complete_json_fence_has_label_gutter_and_syntax_accents() {
        let source = "```json\n{\"name\": \"noob\", \"n\": 42, \"ok\": true, \"x\": null}\n```\n";
        let rendered = render(&[source], 100, ColorDepth::Truecolor);
        let shown = plain(&rendered);
        assert!(shown.contains("┌─ json"));
        assert!(shown.contains("│ {\"name\": \"noob\""));
        assert!(shown.contains("└─"));
        assert!(!shown.contains("```"));
        // Key, string, number, boolean and null use several semantic spans.
        assert!(rendered.matches("\x1b[38;2;").count() >= 8);
    }

    #[test]
    fn fence_markers_and_json_may_split_anywhere() {
        let source = "```json\n{\"a\":1}\n```";
        let expected = render(&[source], 80, ColorDepth::Truecolor);
        for split in 0..=source.len() {
            assert_eq!(
                render(
                    &[&source[..split], &source[split..]],
                    80,
                    ColorDepth::Truecolor
                ),
                expected,
                "split {split}"
            );
        }
    }

    #[test]
    fn incomplete_fence_flushes_as_legible_source() {
        let shown = plain(&render(
            &["```rust\nfn main() {}"],
            80,
            ColorDepth::Truecolor,
        ));
        assert!(shown.contains("```rust"));
        assert!(shown.contains("fn main() {}"));
        assert!(
            !shown.contains("unclosed code fence"),
            "small malformed fence stays source"
        );
    }

    #[test]
    fn oversized_fence_streams_without_losing_output() {
        let theme = Theme::matrix();
        let mut md = Markdown::new();
        let mut output = md.feed("```text\n", 80, &theme, ColorDepth::None);
        let line = format!("{}\n", "x".repeat(256));
        let count = MAX_FENCE_BYTES / line.len() + 4;
        for _ in 0..count {
            output.push_str(&md.feed(&line, 80, &theme, ColorDepth::None));
        }
        assert!(
            !output.is_empty(),
            "fence must switch to streaming at its bound"
        );
        output.push_str(&md.finish(80, &theme, ColorDepth::None));
        let code_xs: usize = output
            .lines()
            .filter_map(|line| line.strip_prefix("│ "))
            .map(|line| line.matches('x').count())
            .sum();
        assert_eq!(code_xs, count * 256);
        assert!(output.contains("unclosed code fence"));
        assert!(!md.has_pending());
    }

    #[test]
    fn wide_and_narrow_tables_choose_the_right_layout() {
        let source = "| name | description |\n| :--- | ---: |\n| alpha | words that wrap in a cell |\n| beta | short |\n";
        let wide = plain(&render(&[source], 48, ColorDepth::Truecolor));
        assert!(wide.contains('┬') && wide.contains("alpha"));
        let narrow = plain(&render(&[source], 14, ColorDepth::Truecolor));
        assert!(
            narrow.contains("row 1") && narrow.contains("name:"),
            "{narrow:?}"
        );
        assert!(!narrow.contains('┬'));
    }

    #[test]
    fn table_detection_survives_delta_boundaries() {
        let source = "a | b\n--- | ---\none | two\nend\n";
        let expected = render(&[source], 40, ColorDepth::Truecolor);
        for split in 0..=source.len() {
            assert_eq!(
                render(
                    &[&source[..split], &source[split..]],
                    40,
                    ColorDepth::Truecolor
                ),
                expected,
                "split {split}"
            );
        }
    }

    #[test]
    fn table_bound_degrades_to_source_and_preserves_every_row() {
        let mut source = String::from("a | b\n--- | ---\n");
        for i in 0..=table::MAX_TABLE_ROWS {
            source.push_str(&format!("row-{i} | value-{i}\n"));
        }
        source.push_str("after\n");
        let shown = plain(&render(&[&source], 40, ColorDepth::Truecolor));
        assert!(shown.contains("row-0"));
        assert!(shown.contains(&format!("row-{}", table::MAX_TABLE_ROWS)));
        assert!(shown.contains("after"));
    }

    #[test]
    fn long_line_bound_flushes_without_an_output_cap() {
        let source = "x".repeat(MAX_LINE_CHARS + 777);
        let shown = plain(&render(&[&source], 80, ColorDepth::Truecolor));
        assert_eq!(shown.matches('x').count(), source.len());
        assert!(
            shown.contains('\n'),
            "bounded line flush creates a display continuation"
        );
    }

    #[test]
    fn generated_sgr_is_balanced_and_none_depth_is_escape_free() {
        let source = "# **head**\n- *item* `code`\n";
        let styled = render(&[source], 80, ColorDepth::Truecolor);
        let opens = styled.matches("\x1b[").count() - styled.matches(RESET).count();
        assert_eq!(opens, styled.matches(RESET).count());
        let unstyled = render(&[source], 80, ColorDepth::None);
        assert!(!unstyled.contains('\x1b'));
    }

    #[test]
    fn one_instance_resets_cleanly_between_messages() {
        let theme = Theme::matrix();
        let mut md = Markdown::new();
        assert!(md.feed("held", 80, &theme, ColorDepth::None).is_empty());
        assert!(md.has_pending());
        assert_eq!(md.finish(80, &theme, ColorDepth::None), "held");
        assert!(!md.has_pending());
        assert_eq!(md.feed("next\n", 80, &theme, ColorDepth::None), "next\n");
        assert_eq!(md.finish(80, &theme, ColorDepth::None), "");
    }
}
