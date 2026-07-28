//! A small tokenizer, so the code pane reads as code.
//!
//! Not a parser and not trying to be. It colors the four things that carry
//! nearly all of the legibility of a source line, in the order a reader's eye
//! uses them: comments, strings, numbers, keywords. Everything else is plain.
//!
//! The language comes from the file extension, which is the only signal the
//! harness has, and it is enough: nothing here has to be told what the agent is
//! writing. When the extension is unknown, the text is plain, which is a
//! correct rendering rather than a broken one.
//!
//! This is deliberately not tree-sitter. Real grammars are the right answer for
//! a full editor view and they are also eight crates and several megabytes; the
//! diff lines this pane shows are short and out of context, where a scanner and
//! a parser produce the same result.

/// What a fragment of a line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Plain,
    Comment,
    Str,
    Number,
    Keyword,
    /// A Markdown heading, list bullet or emphasis marker.
    Markup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    /// C-like: braces, `//` and `/* */`, double and single quotes.
    CLike,
    /// `#` comments, triple and single quotes.
    Hash,
    Markdown,
    None,
}

/// Which syntax a path is in, by extension.
pub fn for_path(path: &str) -> Syntax {
    let ext = path
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    for_language(&ext)
}

/// Which syntax a language name is, as a fenced code block writes it. The same
/// table as [`for_path`], because a fence says `python` where a file says `py`
/// and both mean the same thing.
pub fn for_language(name: &str) -> Syntax {
    let ext = match name.trim().to_ascii_lowercase().as_str() {
        "python" | "python3" => "py",
        "rust" => "rs",
        "javascript" | "node" => "js",
        "typescript" => "ts",
        "shell" | "console" | "terminal" => "sh",
        "golang" => "go",
        "c++" | "cxx" => "cpp",
        "csharp" => "cs",
        "yml" => "yaml",
        "text" | "plain" | "" => "txt",
        other => return match_ext(other),
    };
    match_ext(ext)
}

fn match_ext(ext: &str) -> Syntax {
    match ext {
        "rs" | "c" | "h" | "cc" | "cpp" | "hpp" | "go" | "java" | "kt" | "swift" | "cs"
        | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "json" | "css" | "scss" | "php"
        | "zig" | "wgsl" | "glsl" | "proto" => Syntax::CLike,
        "py" | "rb" | "sh" | "bash" | "zsh" | "toml" | "yaml" | "yml" | "ini" | "cfg" | "nix"
        | "r" | "pl" | "ex" | "exs" | "jl" | "dockerfile" | "makefile" | "mk" => Syntax::Hash,
        "md" | "markdown" | "mdx" | "rst" => Syntax::Markdown,
        _ => Syntax::None,
    }
}

/// Keywords worth marking, pooled across the languages this colors.
///
/// One list rather than one per language on purpose: a false positive tints a
/// word that is not a keyword in that file, which costs nothing a reader
/// notices, and a per-language table costs a table per language forever.
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "bool", "break", "case", "catch", "class", "const", "continue",
    "crate", "def", "default", "defer", "del", "dyn", "elif", "else", "end", "enum", "except",
    "export", "extern", "false", "final", "finally", "float", "fn", "for", "from", "func",
    "function", "global", "go", "if", "impl", "import", "in", "int", "interface", "is", "lambda",
    "let", "loop", "match", "mod", "move", "mut", "new", "nil", "none", "not", "null", "or",
    "package", "pass", "priv", "pub", "raise", "ref", "return", "select", "self", "static",
    "struct", "super", "switch", "this", "throw", "trait", "true", "try", "type", "typedef",
    "union", "unsafe", "use", "var", "void", "where", "while", "with", "yield", "and", "do",
    "elseif", "then", "local", "echo", "set", "unset", "readonly", "declare", "source",
];

/// Split one line into colored fragments. Byte ranges are contiguous and cover
/// the whole line, so a caller can reassemble it exactly.
pub fn scan(line: &str, syntax: Syntax) -> Vec<(String, Token)> {
    if syntax == Syntax::None || line.is_empty() {
        return vec![(line.to_string(), Token::Plain)];
    }
    if syntax == Syntax::Markdown {
        return scan_markdown(line);
    }

    // By character, not by byte. A scanner that slices on byte offsets cuts a
    // multi-byte character in half the first time a file has an accent in it,
    // and slicing a `str` there panics.
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut out: Vec<(String, Token)> = Vec::new();
    let mut k = 0;
    let mut plain_from = 0;
    let at = |k: usize| chars.get(k).map_or(line.len(), |(byte, _)| *byte);

    // Flush the plain run that ends just before byte offset `end`.
    macro_rules! flush {
        ($end:expr) => {
            if $end > plain_from {
                out.push((line[plain_from..$end].to_string(), Token::Plain));
            }
        };
    }

    while k < chars.len() {
        let (byte, c) = chars[k];
        let rest = &line[byte..];
        // Comment: everything to the end of the line.
        let comment = match syntax {
            Syntax::CLike => rest.starts_with("//") || rest.starts_with("/*"),
            Syntax::Hash => c == '#',
            _ => false,
        };
        if comment {
            flush!(byte);
            out.push((rest.to_string(), Token::Comment));
            return out;
        }
        // String: to the matching quote, honouring one level of backslash. An
        // unterminated quote takes the rest of the line, which is what it looks
        // like on screen anyway.
        if c == '"' || c == '\'' || c == '`' {
            flush!(byte);
            let mut j = k + 1;
            while j < chars.len() {
                match chars[j].1 {
                    '\\' => j += 2,
                    q if q == c => {
                        j += 1;
                        break;
                    }
                    _ => j += 1,
                }
            }
            let end = at(j.min(chars.len()));
            out.push((line[byte..end].to_string(), Token::Str));
            k = j.min(chars.len());
            plain_from = end;
            continue;
        }
        // Number, only when it starts a word so `a1` is not half a number.
        if c.is_ascii_digit() && (k == 0 || !is_word(chars[k - 1].1)) {
            flush!(byte);
            let mut j = k;
            while j < chars.len() && (is_word(chars[j].1) || chars[j].1 == '.') {
                j += 1;
            }
            let end = at(j);
            out.push((line[byte..end].to_string(), Token::Number));
            k = j;
            plain_from = end;
            continue;
        }
        // Word: keyword or plain.
        if is_word(c) && (k == 0 || !is_word(chars[k - 1].1)) {
            let mut j = k;
            while j < chars.len() && is_word(chars[j].1) {
                j += 1;
            }
            let end = at(j);
            if KEYWORDS.contains(&&line[byte..end]) {
                flush!(byte);
                out.push((line[byte..end].to_string(), Token::Keyword));
                plain_from = end;
            }
            k = j;
            continue;
        }
        k += 1;
    }
    flush!(line.len());
    if out.is_empty() {
        out.push((line.to_string(), Token::Plain));
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn scan_markdown(line: &str) -> Vec<(String, Token)> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return vec![(line.to_string(), Token::Markup)];
    }
    if trimmed.starts_with("```") || trimmed.starts_with('>') || trimmed.starts_with('|') {
        return vec![(line.to_string(), Token::Comment)];
    }
    // A bullet: mark the marker, leave the text.
    let indent = line.len() - trimmed.len();
    for marker in ["- ", "* ", "+ "] {
        if trimmed.starts_with(marker) {
            return vec![
                (line[..indent + marker.len()].to_string(), Token::Markup),
                (line[indent + marker.len()..].to_string(), Token::Plain),
            ];
        }
    }
    vec![(line.to_string(), Token::Plain)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(line: &str, syntax: Syntax) -> String {
        scan(line, syntax).iter().map(|(t, _)| t.as_str()).collect()
    }

    /// Whatever the scanner decides, reassembling its fragments must give back
    /// the line exactly. A colorizer that drops or duplicates a character is
    /// worse than no colorizer.
    #[test]
    fn scanning_never_changes_the_line() {
        let cases = [
            (r#"let x = "hi there"; // done"#, Syntax::CLike),
            ("def add(a, b):  # sums", Syntax::Hash),
            ("## Heading", Syntax::Markdown),
            ("  - a bullet", Syntax::Markdown),
            ("", Syntax::CLike),
            ("     ", Syntax::Hash),
            ("no keywords or quotes at all", Syntax::CLike),
            (r#"s = 'unterminated"#, Syntax::Hash),
            (r#"esc = "a\"b" + 1.5e3"#, Syntax::CLike),
            ("héllo = \"wörld\"  # ünicode", Syntax::Hash),
            ("x", Syntax::None),
        ];
        for (line, syntax) in cases {
            assert_eq!(joined(line, syntax), line, "{line:?} as {syntax:?}");
        }
    }

    #[test]
    fn a_comment_takes_the_rest_of_the_line() {
        let scanned = scan("let x = 1; // and a \"quote\" here", Syntax::CLike);
        let comment = scanned.last().unwrap();
        assert_eq!(comment.1, Token::Comment);
        assert_eq!(comment.0, "// and a \"quote\" here");
    }

    /// A `#` inside a string is not a comment, which is the single most common
    /// way a naive scanner colors half a file grey.
    #[test]
    fn a_hash_inside_a_string_is_not_a_comment() {
        let scanned = scan(r##"color = "#00ff88""##, Syntax::Hash);
        assert!(
            scanned.iter().all(|(_, t)| *t != Token::Comment),
            "{scanned:?}"
        );
        assert!(scanned.iter().any(|(t, k)| *k == Token::Str && t.contains("00ff88")));
    }

    #[test]
    fn keywords_are_whole_words() {
        let scanned = scan("format_int(x)", Syntax::CLike);
        assert!(
            scanned.iter().all(|(_, t)| *t != Token::Keyword),
            "`int` inside `format_int` is not a keyword: {scanned:?}"
        );
        let scanned = scan("return x", Syntax::CLike);
        assert_eq!(scanned[0], (String::from("return"), Token::Keyword));
    }

    #[test]
    fn a_number_must_start_a_word() {
        let scanned = scan("let a1 = 42;", Syntax::CLike);
        let numbers: Vec<&String> = scanned
            .iter()
            .filter(|(_, t)| *t == Token::Number)
            .map(|(s, _)| s)
            .collect();
        assert_eq!(numbers, [&String::from("42")], "{scanned:?}");
    }

    /// A fence says `python` where a file says `py`, and both mean the same
    /// thing, so both reach the same scanner.
    #[test]
    fn a_fence_language_picks_the_same_syntax_as_the_extension() {
        assert_eq!(for_language("python"), for_path("a.py"));
        assert_eq!(for_language("rust"), for_path("a.rs"));
        assert_eq!(for_language("Shell"), for_path("a.sh"));
        assert_eq!(for_language("typescript"), for_path("a.ts"));
        assert_eq!(for_language("toml"), Syntax::Hash);
        assert_eq!(for_language(""), Syntax::None);
        assert_eq!(for_language("brainfuck"), Syntax::None);
    }

    #[test]
    fn the_extension_picks_the_syntax() {
        assert_eq!(for_path("src/main.rs"), Syntax::CLike);
        assert_eq!(for_path("a/b/script.py"), Syntax::Hash);
        assert_eq!(for_path("README.md"), Syntax::Markdown);
        assert_eq!(for_path("Cargo.toml"), Syntax::Hash);
        assert_eq!(for_path("app.TSX"), Syntax::CLike, "case does not matter");
        assert_eq!(for_path("LICENSE"), Syntax::None);
        assert_eq!(for_path("notes.txt"), Syntax::None);
        assert_eq!(for_path("weird.qqq"), Syntax::None);
        // A dot in a directory must not be read as the file's extension.
        assert_eq!(for_path("v1.2/notes"), Syntax::None);
    }

    /// An unterminated quote must consume the rest of the line rather than
    /// running past the end of the buffer.
    #[test]
    fn an_unterminated_string_stops_at_the_end_of_the_line() {
        let scanned = scan(r#"x = "never closed"#, Syntax::CLike);
        assert_eq!(scanned.last().unwrap().1, Token::Str);
        assert_eq!(joined(r#"x = "never closed"#, Syntax::CLike), r#"x = "never closed"#);
    }
}
