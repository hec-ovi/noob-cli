//! The interactive REPL slash commands, in one place. This const list is the
//! single source of truth: the greeting banner, the unknown-command notice, and
//! the input-editor's Tab completion all read from it, so a command can never be
//! listed one place and dropped from another.
//!
//! Pure and display-agnostic: the functions here take and return plain strings
//! (no terminal bytes, no `Editor`), so they are unit-testable and the raw-mode
//! reader in `prompt.rs`/`dock.rs` stays the only place that touches the draft.
//! Completion is off the inference path entirely: it is input-side editing that
//! runs between keystrokes, never during a turn's token stream.

/// One REPL slash command: its canonical name (no leading `/`), a short
/// description, and whether it takes arguments (so Tab can append a space).
pub(crate) struct Command {
    pub name: &'static str,
    pub desc: &'static str,
    pub takes_args: bool,
}

/// The commands a `/`-prefixed line dispatches to (see the match in `main.rs`).
/// Order is the display order used by the banner. Aliases the match also accepts
/// (`q`, `exit`) are intentionally omitted: this is the discoverable set.
pub(crate) const COMMANDS: &[Command] = &[
    Command {
        name: "plan",
        desc: "enter plan mode (read-only tools until /go)",
        takes_args: false,
    },
    Command {
        name: "clear-plan",
        desc: "remove completed plan payloads from context",
        takes_args: false,
    },
    Command {
        name: "go",
        desc: "approve the plan and run it",
        takes_args: false,
    },
    Command {
        name: "status",
        desc: "endpoint, model, session, and usage",
        takes_args: false,
    },
    Command {
        name: "context",
        desc: "context use and the compaction threshold",
        takes_args: false,
    },
    Command {
        name: "sessions",
        desc: "list saved sessions newest first",
        takes_args: false,
    },
    Command {
        name: "agents",
        desc: "list or cancel background sub-agents",
        takes_args: true,
    },
    Command {
        name: "config",
        desc: "show or set non-secret configuration",
        takes_args: true,
    },
    Command {
        name: "compact",
        desc: "summarize and shrink the context",
        takes_args: false,
    },
    Command {
        name: "skills",
        desc: "list, add, remove, or reload skills",
        takes_args: true,
    },
    Command {
        name: "mcp",
        desc: "list, add, remove, or connect MCP servers",
        takes_args: true,
    },
    Command {
        name: "quit",
        desc: "leave the REPL",
        takes_args: false,
    },
];

/// The command list as the banner and the unknown-command notice show it,
/// every registered command in order (`/plan /clear-plan /go /status
/// /context /sessions /agents /config /compact /skills /mcp /quit`). Both
/// callers read this, so the two can never drift from the list completion
/// uses.
pub(crate) fn banner() -> String {
    COMMANDS
        .iter()
        .map(|c| format!("/{}", c.name))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The command token being typed, if completion applies to this line: the line
/// starts with `/` and contains no whitespace yet, i.e. the user is typing the
/// command itself, not its arguments. Returns the token WITHOUT the leading `/`.
/// `None` for a non-slash line or once a space is present (`/skills add x`), so
/// completion never fires on an argument.
pub(crate) fn command_token(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('/')?;
    if rest.contains(char::is_whitespace) {
        return None;
    }
    Some(rest)
}

/// Command names whose start matches `token` (every command for an empty token,
/// i.e. a bare `/`). In `COMMANDS` order.
pub(crate) fn candidates(token: &str) -> Vec<&'static str> {
    COMMANDS
        .iter()
        .map(|c| c.name)
        .filter(|n| n.starts_with(token))
        .collect()
}

/// The longest common prefix of a set of names, for advancing an ambiguous Tab
/// as far as it unambiguously can (e.g. two commands sharing `st` stop at `st`).
fn longest_common_prefix(names: &[&str]) -> String {
    let Some(first) = names.first() else {
        return String::new();
    };
    let mut end = first.len();
    for name in &names[1..] {
        end = end.min(name.len());
        while !first.is_char_boundary(end) || first[..end] != name[..end.min(name.len())] {
            end -= 1;
        }
    }
    first[..end].to_string()
}

/// Compute a Tab completion for `line`. Returns the replacement line, or `None`
/// when there is nothing to do (not a command token, no match, or the token is
/// already the full command). Semantics:
/// - exactly one match: the full `/command`, plus a trailing space when the
///   command takes arguments (so the next keystroke is its first argument);
/// - multiple matches: the longest common `/prefix`, so Tab advances as far as
///   it can without guessing which command was meant.
pub(crate) fn complete(line: &str) -> Option<String> {
    let token = command_token(line)?;
    let names = candidates(token);
    let completed = match names.as_slice() {
        [] => return None,
        [one] => {
            let takes_args = COMMANDS.iter().any(|c| c.name == *one && c.takes_args);
            if takes_args {
                format!("/{one} ")
            } else {
                format!("/{one}")
            }
        }
        many => format!("/{}", longest_common_prefix(many)),
    };
    (completed != line).then_some(completed)
}

/// The dim hint trailing the typed token on the one input row, or `None` when
/// there is nothing worth hinting (not a command token, no match, or the token
/// is already exactly one full command). A single candidate shows the
/// completion and its description; several show the candidate `/names`. The two
/// leading spaces separate it from the token. Display text only: it never enters
/// the buffer.
pub(crate) fn hint(line: &str) -> Option<String> {
    let token = command_token(line)?;
    let names = candidates(token);
    match names.as_slice() {
        [] => None,
        [one] if *one == token => None,
        [one] => {
            let desc = COMMANDS
                .iter()
                .find(|c| c.name == *one)
                .map(|c| c.desc)
                .unwrap_or("");
            Some(format!("  /{one}  {desc}"))
        }
        many => {
            let list = many
                .iter()
                .map(|n| format!("/{n}"))
                .collect::<Vec<_>>()
                .join(" ");
            Some(format!("  {list}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_lists_every_command_with_a_slash() {
        assert_eq!(
            banner(),
            "/plan /clear-plan /go /status /context /sessions /agents /config /compact /skills /mcp /quit"
        );
    }

    #[test]
    fn command_token_only_when_typing_the_command_not_an_argument() {
        assert_eq!(command_token("/pl"), Some("pl"));
        assert_eq!(command_token("/"), Some(""));
        assert_eq!(command_token("/skills"), Some("skills"));
        // A space means arguments have started: no command completion.
        assert_eq!(command_token("/skills add"), None);
        assert_eq!(command_token("/skills "), None);
        // A non-slash line is never a command token.
        assert_eq!(command_token("hello"), None);
        assert_eq!(command_token(""), None);
    }

    #[test]
    fn a_unique_prefix_completes_to_the_full_command() {
        assert_eq!(complete("/pl").as_deref(), Some("/plan"));
        assert_eq!(complete("/g").as_deref(), Some("/go"));
        assert_eq!(complete("/c"), None);
        assert_eq!(complete("/co"), None);
        assert_eq!(complete("/com").as_deref(), Some("/compact"));
        // /con is shared by config and context: Tab holds at the prefix.
        assert_eq!(complete("/con"), None);
        assert_eq!(complete("/conf").as_deref(), Some("/config "));
        assert_eq!(complete("/cont").as_deref(), Some("/context"));
        assert_eq!(complete("/cl").as_deref(), Some("/clear-plan"));
        assert_eq!(complete("/q").as_deref(), Some("/quit"));
        assert_eq!(complete("/st").as_deref(), Some("/status"));
        assert_eq!(complete("/se").as_deref(), Some("/sessions"));
        assert_eq!(complete("/ag").as_deref(), Some("/agents "));
        assert_eq!(complete("/m").as_deref(), Some("/mcp "));
    }

    #[test]
    fn an_arg_taking_command_completes_with_a_trailing_space() {
        // Argument-taking commands append a space so the next key starts arguments.
        assert_eq!(complete("/sk").as_deref(), Some("/skills "));
        assert_eq!(complete("/conf").as_deref(), Some("/config "));
        assert_eq!(complete("/ag").as_deref(), Some("/agents "));
        // /status takes none, so no trailing space.
        assert_eq!(complete("/stat").as_deref(), Some("/status"));
    }

    #[test]
    fn an_ambiguous_prefix_stops_at_the_common_prefix_never_guessing() {
        // /s matches skills and status; their only shared prefix is "s", which
        // is already typed, so Tab is a no-op (it must NOT pick one).
        assert_eq!(complete("/s"), None);
        assert_eq!(candidates("s"), vec!["status", "sessions", "skills"]);
    }

    #[test]
    fn complete_is_a_noop_for_a_full_command_a_miss_or_a_non_command() {
        assert_eq!(complete("/plan"), None); // already complete
        assert_eq!(complete("/zzz"), None); // no such command
        assert_eq!(complete("hello"), None); // not a slash line
        assert_eq!(complete("/skills add"), None); // an argument, not the command
    }

    #[test]
    fn the_hint_lists_candidates_or_a_single_description() {
        // Ambiguous: the candidates, each with its slash.
        let h = hint("/s").unwrap();
        assert!(
            h.contains("/skills") && h.contains("/status"),
            "candidates missing: {h:?}"
        );
        // Unique partial: the completion and its description.
        let h = hint("/pl").unwrap();
        assert!(h.contains("/plan"), "the completion is missing: {h:?}");
        assert!(h.contains("plan mode"), "the description is missing: {h:?}");
        // Nothing to hint once the command is fully typed, or for a non-command.
        assert_eq!(hint("/plan"), None);
        assert_eq!(hint("hello"), None);
        assert_eq!(hint("/skills add"), None);
    }

    #[test]
    fn longest_common_prefix_is_bounded_by_the_shortest_name() {
        assert_eq!(longest_common_prefix(&["status", "skills"]), "s");
        assert_eq!(longest_common_prefix(&["go", "go"]), "go");
        assert_eq!(longest_common_prefix(&["plan"]), "plan");
        assert_eq!(longest_common_prefix(&[]), "");
    }
}
