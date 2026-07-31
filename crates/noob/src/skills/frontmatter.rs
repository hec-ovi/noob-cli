//! The SKILL.md frontmatter scanner: a bounded reader that stops at the
//! closing fence, plain and quoted scalars, `|`/`>` blocks, and the
//! agentskills.io field validation. Hand-rolled so the crate gate never pays
//! for a YAML dependency.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

pub(super) const FRONTMATTER_BYTE_CAP: usize = 64 * 1024;

/// Read only the fenced metadata needed for discovery and validation. The
/// file must be a real regular file: following a FIFO, device, or symlink here
/// could block startup or consume unbounded memory before cancellation exists.
/// The check is on the opened fd, not the path (the tools/read.rs pattern):
/// O_NOFOLLOW fails a symlink at open, O_NONBLOCK keeps a FIFO swapped in
/// after the lookup from blocking on a writer, and the fstat below judges the
/// same object the bytes come from, so no TOCTOU window remains.
pub(super) fn read_frontmatter_file(path: &Path) -> std::io::Result<String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    let not_regular = || {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "SKILL.md must be a regular non-symlink file",
        )
    };
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| {
            if e.raw_os_error() == Some(libc::ELOOP) {
                not_regular()
            } else {
                e
            }
        })?;
    if !file.metadata()?.file_type().is_file() {
        return Err(not_regular());
    }
    // A verified regular file: clear O_NONBLOCK so the reads below are
    // ordinary blocking file reads.
    let fd = file.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags >= 0 {
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) };
    }
    let mut kept = Vec::with_capacity(8 * 1024);
    let mut chunk = [0u8; 8 * 1024];
    let mut line_start = 0usize;
    let mut line_count = 0usize;
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        for &byte in &chunk[..n] {
            if kept.len() >= FRONTMATTER_BYTE_CAP {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "frontmatter exceeds 64 KiB or has no closing `---` fence",
                ));
            }
            kept.push(byte);
            if byte != b'\n' {
                continue;
            }
            line_count += 1;
            if line_count > 1 && fence_line(&kept[line_start..kept.len() - 1])? {
                return String::from_utf8(kept)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e));
            }
            line_start = kept.len();
        }
    }
    if line_start < kept.len() && fence_line(&kept[line_start..])? {
        return String::from_utf8(kept)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e));
    }
    String::from_utf8(kept).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

fn fence_line(bytes: &[u8]) -> std::io::Result<bool> {
    let line = std::str::from_utf8(bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(line.trim_end() == "---")
}

#[derive(Debug)]
pub struct Parsed {
    pub fields: HashMap<String, String>,
    /// Number of frontmatter lines (== 0-based line index where the body
    /// starts; file line numbers are this + 1).
    pub body_start: usize,
}

/// Scan the `---`-fenced frontmatter: `key: value` plain scalars, quoted
/// strings, and `|`/`>` block scalars. Indented lines under an unknown key
/// (nested metadata) are ignored; a top-level line that is not `key: value`
/// is an error. All values land in the map trimmed.
pub fn parse(text: &str) -> Result<Parsed, String> {
    // YAML permits a BOM at stream start and Windows editors add one; it
    // must not hide the opening fence. Line indexing is unchanged (the BOM
    // is not a newline), so body_of's byte math over the original text
    // stays exact.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split('\n').enumerate().peekable();
    if lines.next().map(|(_, line)| line.trim_end()) != Some("---") {
        return Err("no frontmatter: the file must start with a `---` line".to_string());
    }
    let mut fields = HashMap::new();
    while let Some((index, raw_line)) = lines.next() {
        let line = raw_line.trim_end();
        if line == "---" {
            return Ok(Parsed {
                fields,
                body_start: index + 1,
            });
        }
        let line_number = index + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            continue; // nested content under an ignored key
        }
        let Some(colon) = line.find(':') else {
            return Err(format!(
                "line {line_number}: expected `key: value`, got {line:?}"
            ));
        };
        let key = line[..colon].trim_end();
        if key.is_empty()
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(format!("line {line_number}: invalid key {key:?}"));
        }
        let value = line[colon + 1..].trim();
        // A block header may carry a trailing YAML comment (`description: | #
        // keep newlines`), so test only the first token for the indicator.
        let indicator = value.split_whitespace().next().unwrap_or("");
        let parsed = if is_block_indicator(indicator) {
            scan_block(&mut lines, indicator.starts_with('>'))
        } else {
            scalar(value)?
        };
        fields.insert(key.to_string(), parsed.trim().to_string());
    }
    Err("unterminated frontmatter: no closing `---` line".to_string())
}

/// `|` or `>` with optional chomping (`+`/`-`) and an optional explicit
/// indentation digit, in either order (YAML block headers like `|2` or
/// `>-2`). The digit is accepted but indentation stays auto-detected:
/// values are trimmed for validation, so the distinction cannot matter.
fn is_block_indicator(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('|') | Some('>'))
        && value.len() <= 3
        && chars.all(|c| c == '+' || c == '-' || c.is_ascii_digit())
}

/// Collect a block scalar's indented lines starting at `i`; returns the
/// joined text and the index of the first line after the block. Literal
/// (`|`) keeps line breaks; folded (`>`) joins lines with spaces and keeps
/// blank lines as breaks. Chomping is irrelevant here: values are trimmed.
fn scan_block<'a>(
    lines: &mut std::iter::Peekable<impl Iterator<Item = (usize, &'a str)>>,
    folded: bool,
) -> String {
    let mut out = String::new();
    let mut indent = None;
    let mut first = true;
    while let Some((_, raw_line)) = lines.peek() {
        let line = raw_line.trim_end();
        if line == "---" {
            break;
        }
        if !line.is_empty() && !line.starts_with(' ') && !line.starts_with('\t') {
            break;
        }
        let (_, raw_line) = lines.next().expect("peeked line exists");
        let line = raw_line.trim_end();
        let line_indent = line
            .bytes()
            .take_while(|&byte| byte == b' ' || byte == b'\t')
            .count();
        let indent = match indent {
            Some(indent) => indent,
            None if !line.trim().is_empty() => {
                indent = Some(line_indent);
                line_indent
            }
            None => 0,
        };
        let bytes = line.as_bytes();
        let mut cut = 0;
        while cut < indent && cut < bytes.len() && matches!(bytes[cut], b' ' | b'\t') {
            cut += 1;
        }
        let stripped = &line[cut..];
        if folded {
            if stripped.trim().is_empty() {
                out.push('\n');
            } else {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str(stripped.trim_end());
            }
        } else {
            if !first {
                out.push('\n');
            }
            out.push_str(stripped);
        }
        first = false;
    }
    out
}

/// One-line scalar: double-quoted (with `\"` `\\` `\n` `\t` escapes),
/// single-quoted (`''` is a literal quote), or plain.
fn scalar(value: &str) -> Result<String, String> {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let inner = &value[1..value.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(other) => out.push(other), // \" \\ and anything else
                None => return Err("dangling backslash in a quoted value".to_string()),
            }
        }
        return Ok(out);
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return Ok(value[1..value.len() - 1].replace("''", "'"));
    }
    Ok(value.to_string())
}

/// Validate the agentskills.io required fields; the error becomes the
/// stderr skip warning.
pub(super) fn validate(fields: &HashMap<String, String>) -> Result<(String, String), String> {
    let name = fields
        .get("name")
        .filter(|n| !n.is_empty())
        .ok_or("missing required field `name`")?;
    if name.len() > 64 {
        return Err(format!("name is {} chars (max 64)", name.len()));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(format!(
            "name {name:?} must contain only lowercase letters, digits, and hyphens"
        ));
    }
    let description = fields
        .get("description")
        .filter(|d| !d.is_empty())
        .ok_or("missing required field `description`")?;
    let chars = description.chars().count();
    if chars > 1024 {
        return Err(format!("description is {chars} chars (max 1024)"));
    }
    Ok((name.clone(), description.clone()))
}

/// The body with the frontmatter stripped (byte-exact suffix of the file)
/// plus the number of leading lines removed. Lenient: a file whose
/// frontmatter no longer parses is returned whole, because at `skill`-tool
/// call time a stale file should degrade, not error.
pub fn body_of(text: &str) -> (Cow<'_, str>, usize) {
    match parse(text) {
        Ok(p) => {
            let mut offset = 0usize;
            for (n, line) in text.split('\n').enumerate() {
                if n == p.body_start {
                    break;
                }
                offset += line.len() + 1;
            }
            (Cow::Borrowed(&text[offset.min(text.len())..]), p.body_start)
        }
        Err(_) => (Cow::Borrowed(text), 0),
    }
}
