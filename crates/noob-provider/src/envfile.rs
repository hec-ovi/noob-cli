//! Flat `.env` parser: `KEY=VALUE` lines, `#` comments, optional quotes.
//! No interpolation, no escape processing, no multi-line values. Hand-rolled
//! so the binary carries no dotenv crate.

use std::collections::HashMap;
use std::path::Path;

/// Parse `.env` content. Later keys win. Returns a message naming the first
/// bad line on failure.
pub fn parse(src: &str) -> Result<HashMap<String, String>, String> {
    let mut map = HashMap::new();
    for (idx, raw) in src.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!(
                "line {}: expected KEY=VALUE, got {:?}",
                idx + 1,
                raw.trim()
            ));
        };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!(
                "line {}: {:?} is not a valid key name",
                idx + 1,
                key
            ));
        }
        map.insert(key.to_string(), clean_value(value));
    }
    Ok(map)
}

/// Read and parse a `.env` file. IO errors and parse errors both come back
/// as a plain message.
pub fn load(path: &Path) -> Result<HashMap<String, String>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse(&content)
}

fn clean_value(raw: &str) -> String {
    let v = raw.trim();
    // Quoted value: everything up to the matching close quote is the value;
    // anything after it (typically a trailing comment) is ignored. Checking
    // only starts_with/ends_with would keep the quotes whenever a comment
    // follows the closing quote, silently corrupting API keys.
    for quote in ['"', '\''] {
        if let Some(rest) = v.strip_prefix(quote)
            && let Some(end) = rest.find(quote)
        {
            return rest[..end].to_string();
        }
    }
    // Unquoted values may carry a trailing comment: KEY=value  # note
    match v.find(" #") {
        Some(pos) => v[..pos].trim_end().to_string(),
        None => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_plain_pairs() {
        let map = parse("A=1\nB=two words\n").unwrap();
        assert_eq!(map["A"], "1");
        assert_eq!(map["B"], "two words");
    }

    #[test]
    fn skips_comments_and_blanks() {
        let map = parse("# top\n\nA=1\n   # indented comment\n").unwrap();
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn strips_matching_quotes_only() {
        let map = parse("A=\"quoted # not a comment\"\nB='single'\nC=\"mismatch'\n").unwrap();
        assert_eq!(map["A"], "quoted # not a comment");
        assert_eq!(map["B"], "single");
        assert_eq!(map["C"], "\"mismatch'");
    }

    #[test]
    fn quoted_value_with_trailing_comment_loses_neither_quotes_nor_content() {
        let map = parse("A=\"sk-abc\" # from provider\nB='x y'   # note\nC=\"\"\n").unwrap();
        assert_eq!(map["A"], "sk-abc");
        assert_eq!(map["B"], "x y");
        assert_eq!(map["C"], "");
    }

    #[test]
    fn strips_trailing_comment_on_unquoted_values() {
        let map = parse("A=value  # note\nB=no#comment-without-space\n").unwrap();
        assert_eq!(map["A"], "value");
        assert_eq!(map["B"], "no#comment-without-space");
    }

    #[test]
    fn accepts_export_prefix_and_crlf() {
        let map = parse("export A=1\r\nB=2\r\n").unwrap();
        assert_eq!(map["A"], "1");
        assert_eq!(map["B"], "2");
    }

    #[test]
    fn last_key_wins() {
        let map = parse("A=1\nA=2\n").unwrap();
        assert_eq!(map["A"], "2");
    }

    #[test]
    fn no_interpolation() {
        let map = parse("A=1\nB=$A\n").unwrap();
        assert_eq!(map["B"], "$A");
    }

    #[test]
    fn rejects_lines_without_equals() {
        let err = parse("A=1\nnot a pair\n").unwrap_err();
        assert!(err.contains("line 2"), "got: {err}");
    }

    #[test]
    fn rejects_bad_key_names() {
        let err = parse("BAD KEY=1\n").unwrap_err();
        assert!(err.contains("line 1"), "got: {err}");
    }

    #[test]
    fn empty_value_is_kept() {
        let map = parse("A=\n").unwrap();
        assert_eq!(map["A"], "");
    }
}
