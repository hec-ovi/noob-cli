//! Rendering for the three surfaces: cooked-mode REPL, plain `exec`, and
//! `exec --json` (one JSONL event per loop step on stdout). Model text
//! streams raw; no markdown rendering (small local models produce cleaner
//! plain text when nothing reformats them). Tool activity is a single dim
//! line; ANSI is disabled automatically when stdout is not a terminal.

use std::io::{IsTerminal, Write};

use serde_json::{Value, json};

use noob_provider::types::Usage;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Interactive REPL: text to stdout, activity dim to stdout.
    Repl,
    /// `noob exec`: text to stdout, activity to stderr.
    Exec,
    /// `noob exec --json`: JSONL events on stdout, activity to stderr.
    ExecJson,
    /// `noob child` (P6): stdout is reserved for the single JSON result
    /// line, so text AND activity stream to stderr as parent-relayable
    /// progress. Never a TTY; asks always deny.
    Child,
}

pub struct Ui {
    pub mode: Mode,
    ansi: bool,
    /// Bytes of assistant text printed since the last newline, for prompt
    /// hygiene after streams that do not end in \n.
    mid_line: bool,
    /// Test seam for the confirmation flow: unit tests cannot own a tty.
    #[cfg(test)]
    pub forced_ask: Option<bool>,
}

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

impl Ui {
    pub fn new(mode: Mode) -> Ui {
        Ui {
            mode,
            ansi: std::io::stdout().is_terminal(),
            mid_line: false,
            #[cfg(test)]
            forced_ask: None,
        }
    }

    fn out(&mut self, s: &str) {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(s.as_bytes());
        let _ = stdout.flush();
    }

    fn dim_line(&mut self, line: &str) {
        self.end_line();
        let rendered = if self.ansi {
            format!("{DIM}{line}{RESET}\n")
        } else {
            format!("{line}\n")
        };
        match self.mode {
            Mode::Repl => self.out(&rendered),
            Mode::Exec | Mode::ExecJson | Mode::Child => {
                let mut err = std::io::stderr().lock();
                let _ = err.write_all(rendered.as_bytes());
            }
        }
    }

    fn event(&mut self, v: Value) {
        // JSONL protocol line; only in ExecJson mode.
        self.out(&format!("{v}\n"));
    }

    /// Streamed assistant text delta.
    pub fn text_delta(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        match self.mode {
            Mode::ExecJson => self.event(json!({"t": "text", "d": s})),
            // A child's stdout carries exactly one JSON line at the end;
            // its text streams to stderr as progress the parent may relay.
            Mode::Child => {
                let mut err = std::io::stderr().lock();
                let _ = err.write_all(s.as_bytes());
                self.mid_line = !s.ends_with('\n');
            }
            _ => {
                self.out(s);
                self.mid_line = !s.ends_with('\n');
            }
        }
    }

    /// Streamed reasoning delta: dim, inline, REPL only. Reasoning is
    /// ephemeral display; it never lands in transcripts or JSON events.
    pub fn reasoning_delta(&mut self, s: &str) {
        if self.mode == Mode::Repl && self.ansi && !s.is_empty() {
            self.out(&format!("{DIM}{s}{RESET}"));
            self.mid_line = !s.ends_with('\n');
        }
    }

    /// Close an unterminated streamed line, if any (on the stream the text
    /// went to: a child's text lives on stderr, everyone else's on stdout).
    pub fn end_line(&mut self) {
        if self.mid_line {
            if self.mode == Mode::Child {
                let mut err = std::io::stderr().lock();
                let _ = err.write_all(b"\n");
            } else {
                self.out("\n");
            }
            self.mid_line = false;
        }
    }

    /// A tool call is about to run (emission order).
    pub fn tool_start(&mut self, name: &str, args: &Value, read_only: bool) {
        match self.mode {
            Mode::ExecJson => self.event(json!({"t": "tool", "name": name, "args": args})),
            _ => {
                // Barrier calls (bash, edit, ...) may run long; announce them.
                // Read-only groups print only their completion line.
                if !read_only {
                    let brief = brief_args(name, args);
                    let line = if brief.is_empty() {
                        format!("* {name}")
                    } else {
                        format!("* {name} {brief}")
                    };
                    self.dim_line(&line);
                }
            }
        }
    }

    /// A tool call finished (emission order).
    pub fn tool_done(&mut self, id: &str, summary: &str, is_error: bool) {
        match self.mode {
            Mode::ExecJson => self.event(json!({"t": "result", "id": id, "err": is_error})),
            _ => self.dim_line(&format!("* {summary}")),
        }
    }

    /// Loop / lifecycle note ("cache prefix reset: compaction", nudges).
    pub fn note(&mut self, line: &str) {
        match self.mode {
            Mode::ExecJson => {
                let mut err = std::io::stderr().lock();
                let _ = err.write_all(format!("{line}\n").as_bytes());
            }
            _ => self.dim_line(line),
        }
    }

    /// One-line y/N question. Only a human at a real terminal can grant:
    /// headless surfaces AND a REPL fed from a pipe degrade to No (the spec
    /// says no TTY means deny), and queued type-ahead is flushed so a line
    /// typed while the agent was working cannot satisfy the prompt.
    pub fn ask(&mut self, question: &str) -> bool {
        #[cfg(test)]
        if let Some(forced) = self.forced_ask {
            return forced;
        }
        if self.mode != Mode::Repl || !std::io::stdin().is_terminal() {
            return false;
        }
        self.end_line();
        unsafe { libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH) };
        self.out(&format!("{question} [y/N] "));
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        matches!(line.trim(), "y" | "Y" | "yes")
    }

    /// End of one user input's processing.
    pub fn done(&mut self, usage: Option<Usage>) {
        if self.mode == Mode::ExecJson {
            let u = usage.map(|u| {
                json!({
                    "prompt": u.prompt_tokens,
                    "completion": u.completion_tokens,
                    "cached_prompt": u.cached_prompt_tokens,
                })
            });
            self.event(json!({"t": "done", "usage": u}));
        } else {
            self.end_line();
        }
    }
}

/// The most telling argument for the one-line activity display.
fn brief_args(name: &str, args: &Value) -> String {
    let s = match name {
        "bash" => args.get("cmd").and_then(Value::as_str).unwrap_or(""),
        "task" => args.get("prompt").and_then(Value::as_str).unwrap_or(""),
        _ => args
            .get("path")
            .or_else(|| args.get("pattern"))
            .and_then(Value::as_str)
            .unwrap_or(""),
    };
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 60 {
        let cut: String = one.chars().take(60).collect();
        format!("{cut}…")
    } else {
        one
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_args_picks_the_telling_field_and_shortens() {
        assert_eq!(brief_args("bash", &json!({"cmd": "cargo  test"})), "cargo test");
        assert_eq!(brief_args("edit", &json!({"path": "src/a.rs"})), "src/a.rs");
        assert_eq!(brief_args("grep", &json!({"pattern": "fn main"})), "fn main");
        let long = brief_args("bash", &json!({"cmd": "x".repeat(200)}));
        assert_eq!(long.chars().count(), 61);
    }

    #[test]
    fn headless_ask_degrades_to_no() {
        let mut ui = Ui::new(Mode::Exec);
        assert!(!ui.ask("continue?"));
        let mut ui = Ui::new(Mode::ExecJson);
        assert!(!ui.ask("continue?"));
    }

    #[test]
    fn repl_ask_without_a_tty_denies_instead_of_eating_stdin() {
        // Only meaningful when the suite itself runs headless (dev.sh runs
        // docker without -t); with a live tty this would block on input.
        if std::io::stdin().is_terminal() {
            return;
        }
        let mut ui = Ui::new(Mode::Repl);
        assert!(!ui.ask("grant?"), "a piped REPL must never grant a confirmation");
    }
}
