//! Rendering for the four surfaces. An interactive REPL at a color terminal
//! gets a themed, display-only surface: a green banner, role-colored activity
//! and notes, and a light tint on streamed assistant text so a human can tell
//! the model's words from their own echoed input. Every other surface, the
//! piped REPL, `exec`, `exec --json`, and `child`, stays byte-for-byte what it
//! was before any of this: model text streams raw, tool activity is a single
//! dim line, and no escape reaches a non-terminal. Rendering is display-only:
//! it never rewrites request bodies, the session log, or the JSONL protocol.

use std::io::{IsTerminal, Write};

use serde_json::{Value, json};

use noob_provider::types::Usage;

pub(crate) mod prompt;
mod style;
mod theme;

pub use prompt::Input;
use style::{ColorDepth, DIM, RESET};
use theme::Theme;

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
    /// stdout is a terminal. Drives the legacy dim lines and reasoning display
    /// exactly as before. Kept separate from `color` so `NO_COLOR` can drop
    /// color without changing exec-mode stderr or hiding reasoning.
    ansi: bool,
    /// Emit the themed color surface: an interactive REPL, color allowed, and a
    /// real color depth. Always false outside `Mode::Repl`.
    color: bool,
    depth: ColorDepth,
    theme: Theme,
    /// The assistant tint is open on stdout and awaits its reset.
    tinted: bool,
    /// Bytes of assistant text printed since the last newline, for prompt
    /// hygiene after streams that do not end in \n.
    mid_line: bool,
    /// Output sinks. Real stdout/stderr in production; in-memory in tests so
    /// the styled path (which the piped suite would otherwise never reach) is
    /// unit-testable.
    out: Box<dyn Write>,
    err: Box<dyn Write>,
    /// Test seam for the confirmation flow: unit tests cannot own a tty.
    #[cfg(test)]
    pub forced_ask: Option<bool>,
}

impl Ui {
    pub fn new(mode: Mode) -> Ui {
        let ansi = std::io::stdout().is_terminal();
        let depth = if ansi { style::detect_depth() } else { ColorDepth::None };
        // A depthless terminal (TERM=dumb) or NO_COLOR falls back to the exact
        // pre-color behavior rather than emitting empty escapes.
        let color = ansi && no_color_allowed() && depth != ColorDepth::None;
        Ui {
            mode,
            ansi,
            color,
            depth,
            theme: Theme::matrix(),
            tinted: false,
            mid_line: false,
            out: Box::new(std::io::stdout()),
            err: Box::new(std::io::stderr()),
            #[cfg(test)]
            forced_ask: None,
        }
    }

    /// True only where the themed color surface renders: an interactive REPL
    /// with color enabled. Every richness gate keys off this, never the bare
    /// terminal flag, so `exec` at a tty stays raw.
    fn styled(&self) -> bool {
        self.mode == Mode::Repl && self.color
    }

    fn out(&mut self, s: &str) {
        let _ = self.out.write_all(s.as_bytes());
        let _ = self.out.flush();
    }

    /// stderr is unbuffered; no flush, matching the prior direct writes.
    fn err_out(&mut self, s: &str) {
        let _ = self.err.write_all(s.as_bytes());
    }

    /// One activity/note/error line: colored when styled, dim at a plain tty,
    /// plain when piped. Routes to stdout in the REPL and to stderr everywhere
    /// else, exactly as before. `opener` is the theme's SGR for this line's
    /// role; an empty opener (depthless) falls through to the legacy path.
    fn styled_line(&mut self, line: &str, opener: &str) {
        self.end_line();
        let rendered = if self.styled() && !opener.is_empty() {
            format!("{opener}{line}{RESET}\n")
        } else if self.ansi {
            format!("{DIM}{line}{RESET}\n")
        } else {
            format!("{line}\n")
        };
        match self.mode {
            Mode::Repl => self.out(&rendered),
            Mode::Exec | Mode::ExecJson | Mode::Child => self.err_out(&rendered),
        }
    }

    fn event(&mut self, v: Value) {
        // JSONL protocol line; only in ExecJson mode. Never colored.
        self.out(&format!("{v}\n"));
    }

    /// Streamed assistant text delta.
    pub fn text_delta(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        match self.mode {
            Mode::ExecJson => self.event(json!({"t": "text", "d": s})),
            // A child's stdout carries exactly one JSON line at the end; its
            // text streams to stderr as progress the parent may relay.
            Mode::Child => {
                self.err_out(s);
                self.mid_line = !s.ends_with('\n');
            }
            _ => {
                if self.styled() && !self.tinted {
                    // Open the tint once and keep the text contiguous: a marker
                    // must never be split by an escape. end_line closes it.
                    let open = self.theme.assistant.sgr(self.depth);
                    if !open.is_empty() {
                        self.out(&open);
                        self.tinted = true;
                    }
                }
                self.out(s);
                self.mid_line = !s.ends_with('\n');
            }
        }
    }

    /// Streamed reasoning delta: dim, inline, REPL only. Reasoning is ephemeral
    /// display; it never lands in transcripts or JSON events. Behavior is
    /// unchanged; it closes any open assistant tint first so the two never nest.
    pub fn reasoning_delta(&mut self, s: &str) {
        if self.mode == Mode::Repl && self.ansi && !s.is_empty() {
            self.close_tint();
            self.out(&format!("{DIM}{s}{RESET}"));
            self.mid_line = !s.ends_with('\n');
        }
    }

    /// Reset the assistant tint if open. Unconditional (not gated on mid_line)
    /// so a message that ends in \n cannot bleed color into the next prompt.
    fn close_tint(&mut self) {
        if self.tinted {
            self.out(RESET);
            self.tinted = false;
        }
    }

    /// Close an unterminated streamed line, if any (on the stream the text went
    /// to: a child's text lives on stderr, everyone else's on stdout).
    pub fn end_line(&mut self) {
        self.close_tint();
        if self.mid_line {
            if self.mode == Mode::Child {
                self.err_out("\n");
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
                    let opener = self.theme.activity.sgr(self.depth);
                    self.styled_line(&line, &opener);
                }
            }
        }
    }

    /// A tool call finished (emission order).
    pub fn tool_done(&mut self, id: &str, summary: &str, is_error: bool) {
        match self.mode {
            Mode::ExecJson => self.event(json!({"t": "result", "id": id, "err": is_error})),
            _ => {
                let token = if is_error { self.theme.error } else { self.theme.activity };
                let opener = token.sgr(self.depth);
                self.styled_line(&format!("* {summary}"), &opener);
            }
        }
    }

    /// Loop / lifecycle note ("cache prefix reset: compaction", nudges).
    pub fn note(&mut self, line: &str) {
        match self.mode {
            Mode::ExecJson => self.err_out(&format!("{line}\n")),
            _ => {
                let opener = self.theme.note.sgr(self.depth);
                self.styled_line(line, &opener);
            }
        }
    }

    /// An error line: red when styled, otherwise identical bytes to a note.
    pub fn error(&mut self, line: &str) {
        match self.mode {
            Mode::ExecJson => self.err_out(&format!("{line}\n")),
            _ => {
                let opener = self.theme.error.sgr(self.depth);
                self.styled_line(line, &opener);
            }
        }
    }

    /// The input prompt marker. Colored when styled; the exact `> ` / `plan> `
    /// bytes otherwise.
    pub fn prompt(&mut self, plan: bool) {
        let glyph = if plan { "plan> " } else { "> " };
        if self.styled() {
            let opener = self.theme.prompt.sgr(self.depth);
            if opener.is_empty() {
                self.out(glyph);
            } else {
                self.out(&format!("{opener}{glyph}{RESET}"));
            }
        } else {
            self.out(glyph);
        }
    }

    /// The startup greeting: a themed banner over the info line when styled;
    /// the plain info note otherwise (byte-identical to before).
    pub fn greeting(&mut self, info: &str) {
        if self.styled() {
            let banner = theme::banner(&self.theme, self.depth);
            self.out(&banner);
            let opener = self.theme.note.sgr(self.depth);
            self.styled_line(info, &opener);
        } else {
            self.note(info);
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

/// Color allowed unless `NO_COLOR` is present and non-empty (the spec: honor it
/// only when set to a non-empty value).
fn no_color_allowed() -> bool {
    match std::env::var("NO_COLOR") {
        Ok(v) => v.is_empty(),
        Err(_) => true,
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
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A `Write` sink that keeps its bytes so a test can read them back.
    #[derive(Clone, Default)]
    struct Buf(Rc<RefCell<Vec<u8>>>);

    impl Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Buf {
        fn text(&self) -> String {
            String::from_utf8(self.0.borrow().clone()).unwrap()
        }
    }

    /// A `Ui` wired to in-memory sinks with mode/color/ansi forced independent
    /// of any real terminal (the suite runs piped, so `is_terminal()` is false
    /// and the styled path would otherwise get zero coverage). Truecolor depth
    /// makes the emitted escapes deterministic.
    fn harness(mode: Mode, color: bool, ansi: bool) -> (Ui, Buf, Buf) {
        let out = Buf::default();
        let err = Buf::default();
        let ui = Ui {
            mode,
            ansi,
            color,
            depth: if color { ColorDepth::Truecolor } else { ColorDepth::None },
            theme: Theme::matrix(),
            tinted: false,
            mid_line: false,
            out: Box::new(out.clone()),
            err: Box::new(err.clone()),
            forced_ask: None,
        };
        (ui, out, err)
    }

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

    // --- the styled surface (only reachable through the seam) --------------

    #[test]
    fn styled_activity_line_is_wrapped_and_reset() {
        // Not "is it the right green" (a theme choice tuned live in the REPL);
        // the invariant is the content survives and the line resets, so nothing
        // bleeds into the next line.
        let (mut ui, out, _) = harness(Mode::Repl, true, true);
        ui.tool_done("id", "done", false);
        let s = out.text();
        assert!(s.contains("* done"), "summary text missing: {s:?}");
        assert!(s.ends_with("\x1b[0m\n"), "line not reset-terminated (bleed risk): {s:?}");
        assert_ne!(s, "* done\n", "styled line must differ from the plain form");
    }

    #[test]
    fn error_bytes_match_note_when_not_styled() {
        // error() is a display-only recolor of note(); on every non-styled
        // surface its bytes must not drift from note()'s.
        let (mut ui_e, out_e, _) = harness(Mode::Repl, false, false);
        ui_e.error("boom");
        let (mut ui_n, out_n, _) = harness(Mode::Repl, false, false);
        ui_n.note("boom");
        assert_eq!(out_e.text(), out_n.text(), "error() drifted from note() when not styled");
        assert_eq!(out_e.text(), "boom\n");
    }

    #[test]
    fn piped_repl_activity_is_plain() {
        let (mut ui, out, _) = harness(Mode::Repl, false, false);
        ui.tool_done("id", "done", false);
        assert_eq!(out.text(), "* done\n");
    }

    #[test]
    fn no_color_repl_is_dim_not_colored() {
        // NO_COLOR at a tty: color off, ansi on. The legacy dim path, no color.
        let (mut ui, out, _) = harness(Mode::Repl, false, true);
        ui.tool_done("id", "done", false);
        assert_eq!(out.text(), "\x1b[2m* done\x1b[0m\n");
    }

    #[test]
    fn exec_at_a_tty_stays_byte_identical() {
        // The exec-leak guard: even with color available, exec is not styled.
        let (mut ui, out, err) = harness(Mode::Exec, true, true);
        ui.text_delta("hello");
        ui.tool_done("id", "done", false);
        // Raw text on stdout (the trailing \n is end_line closing the line
        // before the activity line, exactly as today); no escape anywhere.
        assert_eq!(out.text(), "hello\n", "exec text must be raw on stdout");
        assert!(!out.text().contains('\x1b'), "exec stdout must carry no escapes");
        assert_eq!(err.text(), "\x1b[2m* done\x1b[0m\n", "exec activity must stay legacy dim");
    }

    #[test]
    fn assistant_tint_opens_once_and_keeps_text_contiguous() {
        // The word-split streaming case: a marker must survive across deltas.
        let (mut ui, out, _) = harness(Mode::Repl, true, true);
        ui.text_delta("1. write ");
        ui.text_delta("greeting.txt");
        let s = out.text();
        assert!(s.contains("1. write greeting.txt"), "marker split by an escape: {s:?}");
        // One escape so far (the single opener), no reset yet. Counting escapes
        // rather than a color keeps this green when the theme is retuned.
        assert_eq!(s.matches('\x1b').count(), 1, "tint reopened per delta: {s:?}");
        ui.end_line();
        let s = out.text();
        assert_eq!(s.matches('\x1b').count(), 2, "expected one opener + one reset: {s:?}");
        assert!(s.ends_with("\x1b[0m\n"), "tint not reset before the prompt: {s:?}");
    }

    #[test]
    fn tint_resets_even_when_message_ends_in_newline() {
        // The bleed bug: a \n-terminated message no-ops the old end_line, so
        // the tint must be reset unconditionally or it leaks into the prompt.
        let (mut ui, out, _) = harness(Mode::Repl, true, true);
        ui.text_delta("done\n");
        ui.end_line();
        let s = out.text();
        assert!(s.contains("done\n\x1b[0m"), "tint bled past newline: {s:?}");
    }

    #[test]
    fn prompt_marker_is_exact_when_not_styled() {
        let (mut ui, out, _) = harness(Mode::Repl, false, false);
        ui.prompt(false);
        ui.prompt(true);
        assert_eq!(out.text(), "> plan> ");
    }

    #[test]
    fn styled_prompt_marker_keeps_glyph_and_resets() {
        // The glyph must survive and the marker must reset, so the user's typed
        // input (echoed right after it) is never caught in the marker's color.
        let (mut ui, out, _) = harness(Mode::Repl, true, true);
        ui.prompt(false);
        let s = out.text();
        assert!(s.contains("> "), "prompt glyph missing: {s:?}");
        assert!(s.ends_with("\x1b[0m"), "prompt not reset (would color typed input): {s:?}");
    }

    #[test]
    fn child_text_goes_to_stderr_unstyled() {
        let (mut ui, out, err) = harness(Mode::Child, true, true);
        ui.text_delta("progress");
        assert_eq!(out.text(), "", "child stdout is reserved for the result line");
        assert_eq!(err.text(), "progress");
    }

    #[test]
    fn execjson_events_are_never_colored() {
        let (mut ui, out, _) = harness(Mode::ExecJson, true, true);
        ui.text_delta("hi");
        let s = out.text();
        assert!(s.contains("\"t\":\"text\"") || s.contains("\"text\""), "event missing: {s:?}");
        assert!(!s.contains('\x1b'), "JSONL protocol must carry no escapes: {s:?}");
    }
}
