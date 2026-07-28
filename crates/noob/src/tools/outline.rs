//! Naming the parts of a file, so a change to one function costs one function.
//!
//! Without this the agent has two options for "rewrite `add`": read the whole
//! file and send the whole file back, or guess a unique `old` string and hope
//! it matches exactly one spot. The first is paid in tokens on every edit of
//! every large file; the second is where a small model thrashes.
//!
//! Deliberately not tree-sitter. The grammars are large C tables and eight of
//! them would multiply a 4 MiB binary several times over, against a hard 8 MiB
//! budget. This is a brace and indentation scanner: it is not a parser and
//! cannot be, but it does not have to be. It only proposes a line span, and
//! every caller re-reads the file and validates before writing, so a wrong
//! span fails loudly instead of corrupting anything. CLIppy can carry real
//! tree-sitter for highlighting, where binary size is not the constraint.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: &'static str,
    /// 1-based, inclusive.
    pub start: usize,
    pub end: usize,
}

impl Symbol {
    pub fn lines(&self) -> usize {
        self.end + 1 - self.start
    }
}

/// Keywords that introduce something worth addressing by name. The union
/// across supported languages rather than a table per language: a keyword that
/// does not exist in a given language simply never matches, and a shared list
/// is one thing to keep true instead of eight.
const DEFINERS: &[&str] = &[
    "fn",
    "func",
    "function",
    "def",
    "class",
    "struct",
    "enum",
    "impl",
    "trait",
    "interface",
    "type",
    "mod",
    "module",
    "namespace",
];

/// Modifiers that may sit in front of a definer and must not hide it.
const LEADERS: &[&str] = &[
    "pub",
    "pub(crate)",
    "export",
    "default",
    "async",
    "static",
    "public",
    "private",
    "protected",
    "final",
    "abstract",
    "override",
    "unsafe",
    "const",
    "extern",
    "declare",
    "inline",
    "virtual",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Markdown,
    Indented,
    Braced,
    Unknown,
}

fn family(path: &Path) -> Family {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "md" | "markdown" => Family::Markdown,
        "py" | "pyi" | "rb" | "yaml" | "yml" => Family::Indented,
        "rs" | "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" | "go" | "java" | "c" | "h" | "cc"
        | "cpp" | "hpp" | "cs" | "kt" | "swift" | "php" | "scala" | "dart" => Family::Braced,
        _ => Family::Unknown,
    }
}

/// Addressable parts of `text`, outermost first, in file order.
pub fn outline(path: &Path, text: &str) -> Vec<Symbol> {
    match family(path) {
        Family::Markdown => markdown(text),
        Family::Indented => indented(text),
        Family::Braced => braced(text),
        Family::Unknown => Vec::new(),
    }
}

/// Find the one symbol `wanted` names.
///
/// Exact, then case-insensitive, then a unique substring. Several matches are
/// an error rather than a pick: guessing which function to overwrite is the
/// failure this whole module exists to avoid.
pub fn find<'a>(symbols: &'a [Symbol], wanted: &str) -> Result<&'a Symbol, String> {
    let wanted = wanted.trim();
    let exact: Vec<&Symbol> = symbols.iter().filter(|s| s.name == wanted).collect();
    if exact.len() == 1 {
        return Ok(exact[0]);
    }
    if exact.len() > 1 {
        return Err(ambiguous(wanted, &exact));
    }
    let lower = wanted.to_lowercase();
    let folded: Vec<&Symbol> = symbols
        .iter()
        .filter(|s| s.name.to_lowercase() == lower)
        .collect();
    if folded.len() == 1 {
        return Ok(folded[0]);
    }
    if folded.len() > 1 {
        return Err(ambiguous(wanted, &folded));
    }
    let partial: Vec<&Symbol> = symbols
        .iter()
        .filter(|s| s.name.to_lowercase().contains(&lower))
        .collect();
    match partial.len() {
        1 => Ok(partial[0]),
        0 => Err(format!(
            "no symbol named {wanted:?}{}",
            if symbols.is_empty() {
                "; this file has no addressable symbols, so use old/new".to_string()
            } else {
                format!(
                    "; this file has: {}",
                    symbols
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        )),
        _ => Err(ambiguous(wanted, &partial)),
    }
}

fn ambiguous(wanted: &str, hits: &[&Symbol]) -> String {
    format!(
        "{wanted:?} matches {} symbols: {}. Name one exactly.",
        hits.len(),
        hits.iter()
            .map(|s| format!("{} (lines {}-{})", s.name, s.start, s.end))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Render the table the model reads instead of the file.
pub fn render(symbols: &[Symbol], total_lines: usize) -> String {
    if symbols.is_empty() {
        return "no addressable symbols found; read the file normally".to_string();
    }
    let mut out = format!("{} symbols, {total_lines} lines:\n", symbols.len());
    for symbol in symbols {
        out.push_str(&format!(
            "{}-{} {} {}\n",
            symbol.start, symbol.end, symbol.kind, symbol.name
        ));
    }
    out
}

fn markdown(text: &str) -> Vec<Symbol> {
    let lines: Vec<&str> = text.lines().collect();
    let mut heads: Vec<(usize, usize, String)> = Vec::new();
    let mut fenced = false;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced || !trimmed.starts_with('#') {
            continue;
        }
        let level = trimmed.chars().take_while(|c| *c == '#').count();
        if level > 6 {
            continue;
        }
        let title = trimmed[level..].trim();
        if title.is_empty() {
            continue;
        }
        heads.push((index, level, title.to_string()));
    }
    let mut out = Vec::new();
    for (position, (index, level, title)) in heads.iter().enumerate() {
        // A section runs until the next heading at the same level or shallower;
        // a nested subsection belongs to it.
        let end = heads[position + 1..]
            .iter()
            .find(|(_, next_level, _)| next_level <= level)
            .map(|(next_index, _, _)| *next_index)
            .unwrap_or(lines.len());
        out.push(Symbol {
            name: title.clone(),
            kind: "section",
            start: index + 1,
            end: end.max(index + 1),
        });
    }
    out
}

fn indented(text: &str) -> Vec<Symbol> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let indent = line.len() - line.trim_start().len();
        let Some((kind, name)) = definition(line.trim_start()) else {
            continue;
        };
        // The body is every following line indented deeper, plus the blank
        // lines between them; trailing blanks belong to the file, not the
        // symbol.
        let mut end = index + 1;
        for (offset, later) in lines[index + 1..].iter().enumerate() {
            if later.trim().is_empty() {
                continue;
            }
            if later.len() - later.trim_start().len() <= indent {
                break;
            }
            end = index + 1 + offset + 1;
        }
        out.push(Symbol {
            name,
            kind,
            start: index + 1,
            end,
        });
    }
    out
}

fn braced(text: &str) -> Vec<Symbol> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut in_block_comment = false;
    let mut pending: Option<(usize, &'static str, String)> = None;
    for (index, line) in lines.iter().enumerate() {
        let (opens, closes, comment_state) = braces(line, in_block_comment);
        in_block_comment = comment_state;
        let starting_depth = depth;
        if pending.is_none()
            && let Some((kind, name)) = definition(line.trim_start())
        {
            pending = Some((index, kind, name));
        }
        depth = (depth + opens).saturating_sub(closes);
        if let Some((start, kind, name)) = pending.take() {
            if opens > 0 && depth <= starting_depth {
                // Opened and closed on one line: a one-line body or a
                // declaration such as `struct Point { x: f64 }`.
                out.push(Symbol {
                    name,
                    kind,
                    start: start + 1,
                    end: index + 1,
                });
            } else if opens > 0 {
                // Body is open; find the line that closes it.
                let end = close_line(&lines, index, depth, starting_depth, in_block_comment);
                out.push(Symbol {
                    name,
                    kind,
                    start: start + 1,
                    end: end + 1,
                });
            } else if line.trim_end().ends_with(';') {
                // A declaration with no body (`fn f();`, `type A = B;`).
                out.push(Symbol {
                    name,
                    kind,
                    start: start + 1,
                    end: index + 1,
                });
            } else {
                // A signature spanning several lines: keep waiting.
                pending = Some((start, kind, name));
            }
        }
    }
    out
}

/// The line where a body opened on `from` returns to `target` depth.
fn close_line(
    lines: &[&str],
    from: usize,
    mut depth: usize,
    target: usize,
    mut in_block_comment: bool,
) -> usize {
    for (offset, line) in lines[from + 1..].iter().enumerate() {
        let (opens, closes, state) = braces(line, in_block_comment);
        in_block_comment = state;
        depth = (depth + opens).saturating_sub(closes);
        if depth <= target {
            return from + 1 + offset;
        }
    }
    lines.len().saturating_sub(1)
}

/// Braces that actually open and close code, ignoring those inside strings,
/// character literals and comments. Not a lexer: it handles the cases that
/// otherwise miscount a span, and a miscount is caught by the caller
/// re-reading before it writes.
fn braces(line: &str, mut in_block_comment: bool) -> (usize, usize, bool) {
    let (mut opens, mut closes) = (0usize, 0usize);
    let mut chars = line.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        if in_block_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment = false;
            }
            continue;
        }
        if let Some(open) = quote {
            if c == '\\' {
                chars.next();
            } else if c == open {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' | '`' => quote = Some(c),
            '/' if chars.peek() == Some(&'/') => break,
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                in_block_comment = true;
            }
            '#' => break, // a shell or python style comment; harmless elsewhere
            '{' => opens += 1,
            '}' => closes += 1,
            _ => {}
        }
    }
    (opens, closes, in_block_comment)
}

/// Does this line introduce something addressable, and what is it called?
fn definition(trimmed: &str) -> Option<(&'static str, String)> {
    if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with('*') {
        return None;
    }
    let mut words = trimmed.split_whitespace().peekable();
    let mut kind: Option<&'static str> = None;
    for word in words.by_ref() {
        let bare = word.trim_end_matches(['(', '<', '{', ':']);
        if LEADERS.contains(&bare) || bare.starts_with("pub(") || bare.starts_with('@') {
            continue;
        }
        if let Some(found) = DEFINERS.iter().find(|d| **d == bare) {
            kind = Some(found);
            break;
        }
        // A modifier we do not know ends the search: this is not a definition
        // line, and scanning on would match a keyword deep inside an
        // expression.
        return None;
    }
    let kind = kind?;
    // `impl Display for Point` is only meaningful whole; everything else is
    // named by the identifier that follows the keyword.
    if kind == "impl" {
        let rest: String = words.collect::<Vec<_>>().join(" ");
        let rest = rest
            .split('{')
            .next()
            .unwrap_or("")
            .trim()
            .trim_end_matches("where")
            .trim()
            .to_string();
        return (!rest.is_empty()).then_some(("impl", rest));
    }
    let name: String = words
        .next()?
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
        .collect();
    (!name.is_empty()).then_some((kind, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn of(name: &str, text: &str) -> Vec<Symbol> {
        outline(&PathBuf::from(name), text)
    }

    /// The case the user described: ten functions in a file, address one.
    #[test]
    fn a_rust_function_spans_exactly_its_body() {
        let text = "\
use std::fmt;

fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn multiply(a: i64, b: i64) -> i64 {
    if a == 0 {
        return 0;
    }
    a * b
}
";
        let symbols = of("calc.rs", text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["add", "multiply"]);
        assert_eq!((symbols[0].start, symbols[0].end), (3, 5));
        // The nested `if` brace must not end the function early.
        assert_eq!((symbols[1].start, symbols[1].end), (7, 12));
    }

    #[test]
    fn rust_types_impls_and_declarations_are_named() {
        let text = "\
pub struct Point { x: f64, y: f64 }

impl fmt::Display for Point {
    fn fmt(&self) -> String {
        String::new()
    }
}

pub trait Shape {
    fn area(&self) -> f64;
}
";
        let symbols = of("shape.rs", text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Point"), "{names:?}");
        assert!(names.contains(&"fmt::Display for Point"), "{names:?}");
        assert!(names.contains(&"Shape"), "{names:?}");
    }

    #[test]
    fn python_bodies_end_when_the_indentation_returns() {
        let text = "\
import sys


def add(a, b):
    return a + b


class Store:
    def get(self, key):
        return self.data[key]


def main():
    print(add(1, 2))
";
        let symbols = of("store.py", text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["add", "Store", "get", "main"]);
        assert_eq!((symbols[0].start, symbols[0].end), (4, 5));
        // The class covers its method, and the method is addressable too.
        assert_eq!((symbols[1].start, symbols[1].end), (8, 10));
        assert_eq!((symbols[2].start, symbols[2].end), (9, 10));
    }

    #[test]
    fn markdown_sections_cover_their_subsections() {
        let text = "\
# Title

intro

## First

body

### Nested

more

## Second

end
";
        let symbols = of("doc.md", text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Title", "First", "Nested", "Second"]);
        // "First" runs through its nested subsection and stops before
        // "Second" on line 13.
        assert_eq!(symbols[1].start, 5);
        assert_eq!(symbols[1].end, 12);
        // The lone level-1 heading owns the whole document, which is what
        // makes "rewrite the Title section" mean the obvious thing.
        assert_eq!((symbols[0].start, symbols[0].end), (1, 15));
    }

    /// A brace inside a string or a comment is not structure. Getting this
    /// wrong silently swallows the rest of a file into one symbol.
    #[test]
    fn braces_in_strings_and_comments_do_not_count() {
        let text = "\
function render() {
    const template = \"a { b\";
    // closing } in a comment
    /* and } in a block */
    return template;
}

function after() {
    return 1;
}
";
        let symbols = of("render.js", text);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["render", "after"]);
        assert_eq!((symbols[0].start, symbols[0].end), (1, 6));
    }

    #[test]
    fn a_signature_spanning_lines_still_resolves() {
        let text = "\
func Process(
    input string,
    count int,
) error {
    return nil
}
";
        let symbols = of("main.go", text);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Process");
        assert_eq!((symbols[0].start, symbols[0].end), (1, 6));
    }

    /// Control flow is not a definition. `if (x) {` used to open a symbol
    /// named after whatever followed it.
    #[test]
    fn control_flow_is_not_a_symbol() {
        let text = "\
fn run() {
    if ready {
        go();
    }
    while true {
        stop();
    }
}
";
        let symbols = of("run.rs", text);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "run");
    }

    #[test]
    fn an_unknown_extension_yields_nothing_rather_than_guessing() {
        assert!(of("data.bin", "fn add() {}\n").is_empty());
    }

    #[test]
    fn find_prefers_exact_then_folds_then_refuses_to_guess() {
        let symbols = of(
            "x.rs",
            "fn load_records() {}\nfn load_records_cached() {}\nfn Save() {}\n",
        );
        assert_eq!(find(&symbols, "load_records").unwrap().name, "load_records");
        assert_eq!(find(&symbols, "save").unwrap().name, "Save");
        // A substring hitting two symbols is a question, not a pick.
        let error = find(&symbols, "load").unwrap_err();
        assert!(error.contains("matches 2 symbols"), "{error}");
        assert!(error.contains("load_records_cached"), "{error}");
    }

    #[test]
    fn a_missing_symbol_lists_what_the_file_actually_has() {
        let symbols = of("x.rs", "fn alpha() {}\nfn beta() {}\n");
        let error = find(&symbols, "gamma").unwrap_err();
        assert!(error.contains("alpha, beta"), "{error}");
    }

    #[test]
    fn the_rendered_table_names_every_span() {
        let symbols = of("x.rs", "fn alpha() {\n    1\n}\n");
        let table = render(&symbols, 3);
        assert!(table.contains("1 symbols, 3 lines"), "{table}");
        assert!(table.contains("1-3 fn alpha"), "{table}");
        assert!(render(&[], 10).contains("no addressable symbols"));
    }
}
