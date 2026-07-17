//! Rendering for the four surfaces. An interactive REPL at a color terminal
//! gets a themed, display-only surface: a green banner, role-colored activity
//! and notes, and a light tint on streamed assistant text so a human can tell
//! the model's words from their own echoed input. On that surface each tool
//! activity line tints its leading word (`bash`, `read`, `edit`, a loaded
//! `skill`, ...) its own per-tool accent and pads it to a column, so a scan
//! reads by color before the eye parses a word; a failed line stays red end to
//! end. Every other surface, the piped REPL, `exec`, `exec --json`, and
//! `child`, stays byte-for-byte what it was before any of this: model text
//! streams raw, tool activity is a single dim line, and no escape reaches a
//! non-terminal. Rendering is display-only: it never rewrites request bodies,
//! the session log, or the JSONL protocol.

use std::io::{IsTerminal, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use noob_provider::types::{Item, Usage};

pub(crate) mod commands;
mod dock;
mod markdown;
pub(crate) mod prompt;
mod scanner;
mod style;
mod table;
mod theme;

pub use dock::DockSession;
pub(crate) use dock::WINCH;
use markdown::Markdown;
pub use prompt::Input;
use style::{ColorDepth, DIM, RESET};
use theme::Theme;

pub(crate) fn elapsed_label(elapsed: Duration) -> String {
    let millis = elapsed.as_millis();
    if millis < 60_000 {
        format!("{}.{:01}s", millis / 1_000, (millis % 1_000) / 100)
    } else {
        format!("{}m{:02}s", millis / 60_000, (millis / 1_000) % 60)
    }
}

/// The column the label is padded to on the themed activity line, so the brief
/// after it lines up. Sized to the longest label a summary leads with
/// (`edited`, six columns) plus a trailing space.
const ACTIVITY_LABEL_COL: usize = 7;

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

/// A semantic rendering operation produced by a dock-managed turn. The turn
/// worker never renders terminal bytes: it sends these operations in program
/// order, and the main thread replays adjacent operations through the ordinary
/// `Ui` renderer before performing one terminal repaint. Keeping `Ask` and
/// `End` as separate channel events makes them hard FIFO barriers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TurnEvent {
    Text(String),
    Reasoning(String),
    EndLine,
    ToolStart {
        id: String,
        name: String,
        brief: String,
        read_only: bool,
    },
    ToolDone {
        id: String,
        summary: String,
        is_error: bool,
    },
    /// The `plan` tool's rendered checklist text (header + one glyph line per
    /// item), for the themed REPL's visible block. Off the token path.
    Todos(String),
    /// A multi-agent fan-out panel re-render: the plain checklist block and the
    /// task call ids it covers (so the themed REPL suppresses their redundant
    /// `* task` activity lines). Themed REPL only; off the token path.
    Agents {
        block: String,
        ids: Vec<String>,
    },
    Note(String),
    Error(String),
    Done(Option<Usage>),
}

/// Shared in-memory sink used by the main-thread turn renderer. Stdout and
/// stderr intentionally share one buffer: each semantic operation writes to
/// only one of them, so sharing preserves the event order while still letting
/// the normal `Ui` routing code decide which surface an operation belongs to.
#[derive(Clone, Default)]
struct TurnBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for TurnBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl TurnBuffer {
    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

/// Persistent renderer for one dock turn. It lives on the terminal-owning
/// main thread, so renderer state such as an open assistant tint or a partial
/// line survives across streamed events without allowing worker threads to
/// touch terminal bytes.
pub(super) struct BufferedTurnRenderer {
    ui: Ui,
    buffer: TurnBuffer,
}

impl BufferedTurnRenderer {
    pub(super) fn render(&mut self, event: TurnEvent) {
        match event {
            TurnEvent::Text(s) => self.ui.text_delta(&s),
            TurnEvent::Reasoning(s) => self.ui.reasoning_delta(&s),
            TurnEvent::EndLine => self.ui.end_line(),
            TurnEvent::ToolStart {
                id,
                name,
                brief,
                read_only,
            } => {
                self.ui.render_tool_start(&id, &name, &brief, read_only);
            }
            TurnEvent::ToolDone {
                id,
                summary,
                is_error,
            } => {
                self.ui.tool_done(&id, &summary, is_error);
            }
            TurnEvent::Todos(text) => self.ui.render_checklist(&text),
            TurnEvent::Agents { block, ids } => self.ui.render_agents(&block, &ids),
            TurnEvent::Note(line) => self.ui.note(&line),
            TurnEvent::Error(line) => self.ui.error(&line),
            TurnEvent::Done(usage) => self.ui.done(usage),
        }
    }

    pub(super) fn take(&self) -> Vec<u8> {
        self.buffer.take()
    }

    /// Register the task call ids an open fan-out panel covers, so their
    /// redundant `* task` start/done lines are suppressed on the themed
    /// surface. The dock pins the panel block itself as a live region and never
    /// replays it through this renderer, so the ids must still be recorded here
    /// (the renderer owns the tool lines that consult them).
    pub(super) fn cover_task_ids(&mut self, ids: &[String]) {
        for id in ids {
            self.ui.panel_task_ids.insert(id.clone());
        }
    }
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
    /// Streaming Markdown state for a real interactive terminal. A dock turn
    /// keeps this only on the main-thread buffered renderer; the provider
    /// worker sends semantic text and never parses or styles it.
    markdown: Markdown,
    /// The assistant tint is open on stdout and awaits its reset.
    tinted: bool,
    /// Bytes of assistant text printed since the last newline, for prompt
    /// hygiene after streams that do not end in \n.
    mid_line: bool,
    /// Output sinks. Real stdout/stderr in production; in-memory in tests so
    /// the styled path (which the piped suite would otherwise never reach) is
    /// unit-testable. `Send` because a turn `Ui` (channel sinks) crosses into
    /// the dock's worker thread.
    out: Box<dyn Write + Send>,
    err: Box<dyn Write + Send>,
    /// The thinking scanner while it is sweeping (themed REPL only). Torn down
    /// by the first real output byte and as the end-of-turn bracket.
    scanner: Option<scanner::Scanner>,
    /// Set on a turn `Ui` (see `for_turn`): the dock's render loop owns the
    /// terminal and stdin, so confirmations travel this channel instead of
    /// reading stdin, and the scanner thread never starts (the dock draws
    /// its own liveness). None on every directly-writing `Ui`.
    turn_tx: Option<std::sync::mpsc::SyncSender<dock::Ev>>,
    /// Call ids covered by an open agents fan-out panel. On the themed surface
    /// their per-task `* task` activity lines are suppressed in favor of the
    /// block; consulted only when `styled()`, so byte-identity surfaces still
    /// print the exact `* task ...` lines they always did. Populated as each
    /// panel paints; discarded at the end of the input (per-turn renderers get
    /// a fresh set, and `done` clears the persistent one).
    panel_task_ids: std::collections::HashSet<String>,
    /// Test seam for the confirmation flow: unit tests cannot own a tty.
    #[cfg(test)]
    pub forced_ask: Option<bool>,
}

impl Ui {
    pub fn new(mode: Mode) -> Ui {
        let ansi = std::io::stdout().is_terminal();
        let depth = if ansi {
            style::detect_depth()
        } else {
            ColorDepth::None
        };
        // A depthless terminal (TERM=dumb) or NO_COLOR falls back to the exact
        // pre-color behavior rather than emitting empty escapes.
        let color = ansi && no_color_allowed() && depth != ColorDepth::None;
        Ui {
            mode,
            ansi,
            color,
            depth,
            theme: Theme::from_env(),
            markdown: Markdown::new(),
            tinted: false,
            mid_line: false,
            out: Box::new(std::io::stdout()),
            err: Box::new(std::io::stderr()),
            scanner: None,
            turn_tx: None,
            panel_task_ids: std::collections::HashSet::new(),
            #[cfg(test)]
            forced_ask: None,
        }
    }

    /// A `Ui` for one dock-managed turn. Its public rendering operations ship
    /// semantic events over the dock channel; its sinks are deliberately inert
    /// so a future helper that forgets to intercept cannot become a second
    /// terminal writer. Confirmations reroute through the same channel too.
    pub(crate) fn for_turn(&self, tx: std::sync::mpsc::SyncSender<dock::Ev>) -> Ui {
        Ui {
            mode: self.mode,
            ansi: self.ansi,
            color: self.color,
            depth: self.depth,
            theme: self.theme,
            markdown: Markdown::new(),
            tinted: false,
            mid_line: false,
            out: Box::new(std::io::sink()),
            err: Box::new(std::io::sink()),
            scanner: None,
            turn_tx: Some(tx),
            panel_task_ids: std::collections::HashSet::new(),
            #[cfg(test)]
            forced_ask: None,
        }
    }

    /// Main-thread renderer paired with [`Ui::for_turn`]. It uses the exact
    /// same mode and theme as the production surface, but collects bytes until
    /// the dock chooses a repaint boundary.
    pub(super) fn buffered_turn_renderer(&self) -> BufferedTurnRenderer {
        let buffer = TurnBuffer::default();
        BufferedTurnRenderer {
            ui: Ui {
                mode: self.mode,
                ansi: self.ansi,
                color: self.color,
                depth: self.depth,
                theme: self.theme,
                markdown: Markdown::new(),
                tinted: false,
                mid_line: false,
                out: Box::new(buffer.clone()),
                err: Box::new(buffer.clone()),
                scanner: None,
                turn_tx: None,
                panel_task_ids: std::collections::HashSet::new(),
                #[cfg(test)]
                forced_ask: None,
            },
            buffer,
        }
    }

    /// Send one semantic render operation from a turn Ui. Returning true means
    /// this is a turn surface and the caller must not also render locally. A
    /// closed channel is swallowed because teardown must never panic a worker.
    fn send_turn(&self, event: TurnEvent) -> bool {
        let Some(tx) = &self.turn_tx else {
            return false;
        };
        let _ = tx.send(dock::Ev::Render(event));
        true
    }

    /// Finish a dock turn through the same sender instance that emitted its
    /// semantic operations. The worker calls this only after parking its result,
    /// making `End` a strict FIFO barrier after all rendering from that turn.
    pub(crate) fn turn_end(&self) {
        if let Some(tx) = &self.turn_tx {
            let _ = tx.send(dock::Ev::End);
        }
    }

    /// True only where the themed color surface renders: an interactive REPL
    /// with color enabled. Every richness gate keys off this, never the bare
    /// terminal flag, so `exec` at a tty stays raw.
    fn styled(&self) -> bool {
        self.mode == Mode::Repl && self.color
    }

    /// Rich text is an interactive-terminal feature, not a color feature.
    /// `NO_COLOR` removes SGR while headings, lists, code, tables, and control
    /// sanitization remain readable. Piped and headless surfaces stay raw.
    fn rich_text(&self) -> bool {
        self.mode == Mode::Repl && self.ansi
    }

    /// True at an interactive REPL terminal. Gates the exit session hint, which
    /// a piped REPL neither needs nor should print (it must stay byte-identical).
    pub fn is_interactive(&self) -> bool {
        self.mode == Mode::Repl && self.ansi
    }

    fn out(&mut self, s: &str) {
        // The first real output byte of a turn ends any thinking scanner: it
        // joins the animation thread (which clears its line) before this writes,
        // so the two never interleave. A no-op when none is running.
        self.stop_scanner();
        let _ = self.out.write_all(s.as_bytes());
        let _ = self.out.flush();
    }

    /// Raw bytes to the sink, for the dock's relay of a turn's coalesced
    /// output (whole styled chunks, so the batch is valid UTF-8; bytes here
    /// avoids a pointless lossy round trip).
    pub(super) fn out_raw(&mut self, bytes: &[u8]) {
        self.stop_scanner();
        let _ = self.out.write_all(bytes);
        let _ = self.out.flush();
    }

    /// Open a dock session: the persistent-input REPL driver (fable.md).
    /// Engages only where the raw editor would (interactive REPL, both ends
    /// ttys, and `NOOB_RAW` on). `NOOB_DOCK=0` is the explicit compatibility
    /// opt-out; every other surface keeps the existing line readers.
    pub fn dock_session(&mut self) -> Option<dock::DockSession> {
        if self.use_raw_editor() && dock::enabled_by_env() {
            dock::DockSession::start()
        } else {
            None
        }
    }

    /// Start the thinking scanner: a green square comet that sweeps on its own
    /// line while the model works, until the first output arrives. Themed REPL
    /// surface only, so piped, `exec`, `--json`, and child output are untouched.
    /// Off the inference path: it animates on a side thread while the main
    /// thread blocks on the request, and it is torn down before any reply byte.
    pub fn thinking_start(&mut self) {
        // A turn Ui never spawns the scanner thread: it would be a second
        // writer racing the render loop; the dock draws its own liveness.
        if self.styled() && self.turn_tx.is_none() {
            self.stop_scanner(); // never stack two
            self.scanner = Some(scanner::Scanner::start(self.depth, self.theme.scanner));
        }
    }

    /// Stop the scanner as the explicit end-of-turn bracket, covering a turn
    /// that produced no output at all (nothing routed through `out`). A no-op
    /// when none is running.
    pub fn thinking_stop(&mut self) {
        self.stop_scanner();
    }

    fn stop_scanner(&mut self) {
        if let Some(mut scanner) = self.scanner.take() {
            scanner.stop();
        }
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
        let line = safe_terminal_text(line);
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

    /// One tool-activity line, `* label rest`. On the themed surface the leading
    /// word is tinted its own per-tool accent and padded to a fixed column so
    /// the briefs align down the transcript; a scan then reads by color and by
    /// column before the eye parses a word. Every other surface keeps the exact
    /// prior bytes: `{DIM}* line{RESET}\n` at a plain tty, `* line\n` piped, so
    /// no non-terminal surface gains a byte. `is_error` paints the whole line
    /// the error accent instead of splitting a label, so red stays the single
    /// reserved failure color and a failed line reads red end to end.
    fn activity_line(&mut self, line: &str, is_error: bool) {
        self.end_line();
        let line = safe_terminal_text(line);
        let line = line.as_ref();
        let rendered = if self.styled() {
            // styled() implies a real depth, so every opener below is non-empty.
            if is_error {
                let open = self.theme.error.sgr(self.depth);
                format!("{open}* {line}{RESET}\n")
            } else {
                let (label, rest) = match line.split_once(' ') {
                    Some((l, r)) => (l, r),
                    None => (line, ""),
                };
                let base = self.theme.activity.sgr(self.depth);
                let label_open = self.theme.label_style(label).sgr(self.depth);
                if rest.is_empty() {
                    format!("{base}* {label_open}{label}{RESET}\n")
                } else {
                    // Left-justify the label to a column so the rest lines up; a
                    // label longer than the column keeps a single trailing space
                    // rather than being truncated (never hide the tool name).
                    let width = label.chars().count();
                    let pad = " ".repeat(ACTIVITY_LABEL_COL.saturating_sub(width).max(1));
                    format!("{base}* {label_open}{label}{RESET}{base}{pad}{rest}{RESET}\n")
                }
            }
        } else if self.ansi {
            format!("{DIM}* {line}{RESET}\n")
        } else {
            format!("* {line}\n")
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
        if self.send_turn(TurnEvent::Text(s.to_string())) {
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
            _ if self.rich_text() => {
                let rendered = self
                    .markdown
                    .feed(s, prompt::term_width(), &self.theme, self.depth);
                if !rendered.is_empty() {
                    self.out(&rendered);
                    self.mid_line = !rendered.ends_with('\n');
                }
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
        if s.is_empty() {
            return;
        }
        if self.send_turn(TurnEvent::Reasoning(s.to_string())) {
            return;
        }
        if self.mode == Mode::Repl && self.ansi && !s.is_empty() {
            self.close_tint();
            let safe = safe_terminal_text(s);
            self.out(&format!("{DIM}{safe}{RESET}"));
            self.mid_line = !safe.ends_with('\n');
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
        if self.send_turn(TurnEvent::EndLine) {
            return;
        }
        self.close_tint();
        if self.rich_text() && self.markdown.has_pending() {
            // `finish` also clears CRLF/fence/table state when it currently
            // has no visible bytes, so every assistant message starts clean.
            let rendered = self
                .markdown
                .finish(prompt::term_width(), &self.theme, self.depth);
            if !rendered.is_empty() {
                self.out(&rendered);
                self.mid_line = !rendered.ends_with('\n');
            }
        }
        if self.mid_line {
            if self.mode == Mode::Child {
                self.err_out("\n");
            } else {
                self.out("\n");
            }
            self.mid_line = false;
        }
    }

    /// Record a model-requested call on the JSONL surface. Interactive tool
    /// activity waits for the scheduler's real start transition instead.
    pub fn tool_requested(&mut self, name: &str, args: &Value) {
        if self.mode == Mode::ExecJson {
            self.event(json!({"t": "tool", "name": name, "args": args}));
        }
    }

    /// A tool call is actually about to run.
    pub fn tool_start(&mut self, id: &str, name: &str, brief: &str, read_only: bool) {
        if self.turn_tx.is_some() {
            let _ = self.send_turn(TurnEvent::ToolStart {
                id: id.to_string(),
                name: name.to_string(),
                brief: brief.to_string(),
                read_only,
            });
            return;
        }
        match self.mode {
            Mode::ExecJson => {}
            _ => self.render_tool_start(id, name, brief, read_only),
        }
    }

    fn render_tool_start(&mut self, id: &str, name: &str, brief: &str, read_only: bool) {
        // A fan-out agent's activity line is redundant once the agents panel
        // names it, so the themed surface suppresses it (styled-only: every
        // byte-identity surface still prints the exact `* task ...` line).
        if self.styled() && self.panel_task_ids.contains(id) {
            return;
        }
        // Barrier calls may run long; read-only groups keep their compact
        // completion-only transcript while the dock still names them live.
        if !read_only {
            let line = if brief.is_empty() {
                name.to_string()
            } else {
                format!("{name} {brief}")
            };
            self.activity_line(&line, false);
        }
    }

    /// A tool call finished (emission order).
    pub fn tool_done(&mut self, id: &str, summary: &str, is_error: bool) {
        if self.send_turn(TurnEvent::ToolDone {
            id: id.to_string(),
            summary: summary.to_string(),
            is_error,
        }) {
            return;
        }
        match self.mode {
            Mode::ExecJson => self.event(json!({"t": "result", "id": id, "err": is_error})),
            _ => {
                // A fan-out agent's completion line is redundant with its
                // panel row on the themed surface; suppress it there only.
                if self.styled() && self.panel_task_ids.contains(id) {
                    return;
                }
                self.activity_line(summary, is_error);
            }
        }
    }

    /// Show the `plan` tool's checklist as a visible block. `text` is the
    /// tool's own plain result (header + one glyph line per item), the exact
    /// bytes the model receives. On a dock turn it ships over the channel; the
    /// main-thread renderer replays it through `render_checklist`.
    pub fn checklist(&mut self, text: &str) {
        if self.send_turn(TurnEvent::Todos(text.to_string())) {
            return;
        }
        self.render_checklist(text);
    }

    /// Paint the checklist block on the themed REPL: completed and in-progress
    /// lines in the activity accent, pending lines dim, the header in the note
    /// hue, over the exact same characters. Every byte-identity surface (piped
    /// REPL, exec, exec --json, child) shows nothing here on purpose: the tool
    /// summary line and the transcript already carry the plan, so those
    /// surfaces stay byte-for-byte what they were.
    fn render_checklist(&mut self, text: &str) {
        if !self.styled() {
            return;
        }
        self.end_line();
        // styled() implies a real depth, so every opener below is non-empty.
        let activity = self.theme.activity.sgr(self.depth);
        let error = self.theme.error.sgr(self.depth);
        let note = self.theme.note.sgr(self.depth);
        let mut block = String::new();
        for line in text.lines() {
            let safe = safe_terminal_text(line);
            let open: &str = if safe.starts_with("[ ]") {
                DIM
            } else if safe.starts_with("[!]") {
                &error
            } else if safe.starts_with("[x]") || safe.starts_with("[~]") {
                &activity
            } else {
                &note
            };
            block.push_str(open);
            block.push_str(&safe);
            block.push_str(RESET);
            block.push('\n');
        }
        self.out(&block);
    }

    /// Show a multi-agent fan-out panel: a checklist of the sub-agents a `task`
    /// batch spawned (header + one glyph line per agent), re-rendered as they
    /// run and finish. `ids` are the task call ids the panel covers, so the
    /// themed surface can suppress their redundant per-task activity lines. On
    /// a dock turn it ships over the channel; the main-thread renderer replays
    /// it through `render_agents`.
    pub fn agents(&mut self, block: &str, ids: &[String]) {
        if self.send_turn(TurnEvent::Agents {
            block: block.to_string(),
            ids: ids.to_vec(),
        }) {
            return;
        }
        self.render_agents(block, ids);
    }

    /// Paint the fan-out panel on the themed REPL, reusing the checklist glyph
    /// styling, and register the covered call ids so the redundant `* task`
    /// lines are suppressed. Every byte-identity surface (piped REPL, exec,
    /// exec --json, child) shows nothing here and keeps its exact `* task`
    /// lines: the panel is a themed-REPL affordance, and the sub-agent result
    /// string still enters the transcript unchanged.
    fn render_agents(&mut self, block: &str, ids: &[String]) {
        if !self.styled() {
            return;
        }
        for id in ids {
            self.panel_task_ids.insert(id.clone());
        }
        self.render_checklist(block);
    }

    /// Build the pinned-region rows for a checklist or fan-out block on the
    /// themed dock: one styled physical row per source line, each clamped to a
    /// single terminal row (the same one-row discipline the input line follows)
    /// so the variable-height frame's erase and redraw stay exact. The glyph
    /// styling matches [`Ui::render_checklist`] over the same characters.
    /// Empty off the themed surface, so a no-color or byte-identity dock pins no
    /// region at all, exactly as the scrolled block is silent there.
    pub(super) fn checklist_region_rows(&self, text: &str, width: usize) -> Vec<String> {
        if !self.styled() {
            return Vec::new();
        }
        let activity = self.theme.activity.sgr(self.depth);
        let error = self.theme.error.sgr(self.depth);
        let note = self.theme.note.sgr(self.depth);
        let mut rows = Vec::new();
        for line in text.lines() {
            let safe = safe_terminal_text(line);
            if safe.trim().is_empty() {
                continue;
            }
            let open: &str = if safe.starts_with("[ ]") {
                DIM
            } else if safe.starts_with("[!]") {
                &error
            } else if safe.starts_with("[x]") || safe.starts_with("[~]") {
                &activity
            } else {
                &note
            };
            rows.push(format!("{open}{}{RESET}", clamp_to_row(&safe, width)));
        }
        rows
    }

    pub(super) fn regions_enabled(&self) -> bool {
        self.styled()
    }

    pub(super) fn region_summary_row(&self, text: &str, width: usize, tone: RegionTone) -> String {
        let open = match tone {
            RegionTone::Activity => self.theme.activity.sgr(self.depth),
            RegionTone::Error => self.theme.error.sgr(self.depth),
            RegionTone::Dim => DIM.to_string(),
        };
        format!(
            "{open}{}{RESET}",
            clamp_to_row(&safe_terminal_text(text), width)
        )
    }

    /// Loop / lifecycle note ("cache prefix reset: compaction", nudges).
    pub fn note(&mut self, line: &str) {
        if self.send_turn(TurnEvent::Note(line.to_string())) {
            return;
        }
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
        if self.send_turn(TurnEvent::Error(line.to_string())) {
            return;
        }
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
        // A turn Ui: the render loop owns stdin, so the question travels
        // the event channel and this worker blocks for the human's answer.
        // A closed channel means the turn is being torn down: deny, the
        // same degradation as every other unanswerable surface.
        if let Some(tx) = &self.turn_tx {
            let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
            if tx
                .send(dock::Ev::Ask(question.to_string(), reply_tx))
                .is_err()
            {
                return false;
            }
            return reply_rx.recv().unwrap_or(false);
        }
        if self.mode != Mode::Repl || !std::io::stdin().is_terminal() {
            return false;
        }
        self.end_line();
        unsafe { libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH) };
        self.out(&format!("{} [y/N] ", safe_terminal_text(question)));
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        matches!(line.trim(), "y" | "Y" | "yes")
    }

    /// Redraw a resumed session's prior conversation on the interactive REPL,
    /// reusing the live renderers so it reads the way it did when it streamed.
    /// Display-only: it walks the already-loaded transcript and never touches
    /// the request body, the session log, or the JSONL protocol. Callers gate
    /// on `is_interactive()` and a non-empty transcript, so `exec`, `--json`,
    /// `child`, and a piped REPL never reach it. Synthetic bookkeeping items
    /// (plan toggles, in-band skills notes, compaction summaries, the interrupt
    /// and nudge markers) are filtered so the human sees the dialogue, not the
    /// plumbing, and each tool body is digested to one line so a long session
    /// never floods the screen on resume.
    pub fn replay_transcript(&mut self, items: &[Item]) {
        // call id -> tool name, so a replayed result can lead with its tool.
        let mut names: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for item in items {
            match item {
                Item::User(text) => {
                    if !is_synthetic_replay_item(text) {
                        self.replay_user(text);
                    }
                }
                Item::Assistant {
                    text, tool_calls, ..
                } => {
                    if !text.is_empty() {
                        // The exact streaming Markdown path a live reply uses, so
                        // headings, bold, lists, and tables render identically.
                        self.text_delta(text);
                        self.end_line();
                    }
                    for call in tool_calls {
                        names.insert(call.id.as_str(), call.name.as_str());
                        let args =
                            serde_json::from_str::<Value>(&call.arguments).unwrap_or(Value::Null);
                        let brief = brief_args(&call.name, &args);
                        self.render_tool_start(
                            &call.id,
                            &call.name,
                            &brief,
                            crate::tools::is_read_only(&call.name),
                        );
                    }
                }
                Item::ToolResult { call_id, content } => {
                    let name = names.get(call_id.as_str()).copied();
                    self.replay_tool_result(name, content);
                }
            }
        }
        self.end_line();
    }

    /// Echo one prior human turn on replay: a dim `\u{203a} {text}` line. No
    /// themed user style exists (the dock shows only live input), so this is the
    /// one place a past human line is redrawn; dim keeps it distinct from the
    /// model's words above it.
    fn replay_user(&mut self, text: &str) {
        self.end_line();
        let safe = safe_terminal_text(text);
        let rendered = if self.ansi {
            format!("{DIM}\u{203a} {safe}{RESET}\n")
        } else {
            format!("\u{203a} {safe}\n")
        };
        self.out(&rendered);
    }

    /// One compact done-style line for a replayed tool result: the tool name
    /// (when the paired call is known) plus a single-line digest of the body, so
    /// a resumed transcript never re-dumps a large tool payload.
    fn replay_tool_result(&mut self, name: Option<&str>, content: &str) {
        let digest = replay_result_digest(content);
        let line = match name {
            Some(n) if !digest.is_empty() => format!("{n} {digest}"),
            Some(n) => n.to_string(),
            None if !digest.is_empty() => digest,
            None => return,
        };
        self.activity_line(&line, false);
    }

    /// End of one user input's processing.
    pub fn done(&mut self, usage: Option<Usage>) {
        if self.send_turn(TurnEvent::Done(usage)) {
            return;
        }
        // Fan-out panels belong to the input that spawned them; clear the
        // suppression set so a later input's call ids can never inherit it (a
        // dock turn already gets a fresh renderer per input).
        self.panel_task_ids.clear();
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

#[derive(Clone, Copy)]
pub(super) enum RegionTone {
    Activity,
    Error,
    Dim,
}

/// Make display text harmless before surrounding it with terminal controls.
/// Newlines remain structural, tabs become spaces, carriage returns become
/// newlines, and every other C0/C1 byte becomes visible. Model assistant text
/// uses the stateful Markdown sanitizer; this covers reasoning, tool summaries,
/// notes, endpoint strings, and confirmation questions.
fn safe_terminal_text(input: &str) -> std::borrow::Cow<'_, str> {
    let clean = input
        .chars()
        .all(|c| c == '\n' || !matches!(c as u32, 0x00..=0x1f | 0x7f..=0x9f));
    if clean {
        return std::borrow::Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '\n' => out.push('\n'),
            '\r' => out.push('\n'),
            '\t' => out.push_str("    "),
            c if (c as u32) <= 0x1f => {
                out.push(char::from_u32(0x2400 + c as u32).unwrap_or('�'));
            }
            '\u{7f}' => out.push('␡'),
            c if matches!(c as u32, 0x80..=0x9f) => out.push('�'),
            c => out.push(c),
        }
    }
    std::borrow::Cow::Owned(out)
}

/// One physical row's worth of text: at most `width` terminal columns, counted
/// by display width (so a wide CJK/emoji glyph counts as two, never wrapping the
/// row to a second physical line), with a trailing ellipsis when the source is
/// longer. Keeping a pinned region line to one physical row is what lets the
/// variable-height frame erase and redraw by an exact row count.
fn clamp_to_row(s: &str, width: usize) -> String {
    let width = width.max(1);
    if table::cell_width(s) <= width {
        return s.to_string();
    }
    // Reserve the last column for the ellipsis, then take glyphs while their
    // display width fits, so the result is at most `width` columns wide.
    let budget = width - 1;
    let mut used = 0usize;
    let mut out = String::new();
    for c in s.chars() {
        let w = table::char_width(c);
        if used + w > budget {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Color allowed unless `NO_COLOR` is present and non-empty (the spec: honor it
/// only when set to a non-empty value).
fn no_color_allowed() -> bool {
    match std::env::var("NO_COLOR") {
        Ok(v) => v.is_empty(),
        Err(_) => true,
    }
}

/// Harness-authored transcript items that are plumbing, not conversation: the
/// plan-mode toggles, the in-band skills notes, compaction summaries, and the
/// interrupt/nudge markers. Filtered from the on-screen replay so a resumed
/// session shows the human's dialogue, not internal bookkeeping. This is a
/// display filter only; every one of these items stays in the loaded context.
fn is_synthetic_replay_item(text: &str) -> bool {
    // Prefix, not equality: sessions saved before a wording change carry the
    // old enter message and must stay filtered on replay.
    text.starts_with("[plan mode]")
        || text == crate::agent::PLAN_APPROVED_MSG
        || text == "[interrupted]"
        || text.starts_with("[skills updated]")
        || text.starts_with("[loaded skills:")
        || text.starts_with("[conversation summary]")
        || text.starts_with("[earlier conversation dropped:")
        || text.starts_with("[background sub-agent result ")
        || text.starts_with("[note]")
}

/// A single-line, length-bounded digest of a tool result body for replay, so a
/// resumed session shows a done-style summary instead of re-dumping the payload.
fn replay_result_digest(content: &str) -> String {
    let first = content.lines().next().unwrap_or("").trim();
    let mut out = String::new();
    for (count, ch) in first.chars().enumerate() {
        if count == 72 {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

/// The most telling argument for the one-line activity display.
pub(crate) fn brief_args(name: &str, args: &Value) -> String {
    let s = match name {
        "bash" => args.get("cmd").and_then(Value::as_str).unwrap_or(""),
        "subagent" => args.get("prompt").and_then(Value::as_str).unwrap_or(""),
        _ => args
            .get("path")
            .or_else(|| args.get("pattern"))
            .and_then(Value::as_str)
            .unwrap_or(""),
    };
    let mut shown = String::with_capacity(64);
    let mut chars = 0usize;
    let mut first = true;
    let mut truncated = false;
    'words: for word in s.split_whitespace() {
        if !first {
            if chars == 60 {
                truncated = true;
                break;
            }
            shown.push(' ');
            chars += 1;
        }
        first = false;
        for ch in word.chars() {
            if chars == 60 {
                truncated = true;
                break 'words;
            }
            shown.push(ch);
            chars += 1;
        }
    }
    if truncated {
        shown.push('…');
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A `Write` sink that keeps its bytes so a test can read them back.
    /// Arc+Mutex (not Rc+RefCell) because the sink type must be `Send` now
    /// that a turn `Ui` crosses into the dock's worker thread.
    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);

    impl Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Buf {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
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
            depth: if color {
                ColorDepth::Truecolor
            } else {
                ColorDepth::None
            },
            theme: Theme::matrix(),
            markdown: Markdown::new(),
            tinted: false,
            mid_line: false,
            out: Box::new(out.clone()),
            err: Box::new(err.clone()),
            scanner: None,
            turn_tx: None,
            panel_task_ids: std::collections::HashSet::new(),
            forced_ask: None,
        };
        (ui, out, err)
    }

    #[test]
    fn brief_args_picks_the_telling_field_and_shortens() {
        assert_eq!(
            brief_args("bash", &json!({"cmd": "cargo  test"})),
            "cargo test"
        );
        assert_eq!(brief_args("edit", &json!({"path": "src/a.rs"})), "src/a.rs");
        assert_eq!(
            brief_args("grep", &json!({"pattern": "fn main"})),
            "fn main"
        );
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
        assert!(
            !ui.ask("grant?"),
            "a piped REPL must never grant a confirmation"
        );
    }

    // --- the styled surface (only reachable through the seam) --------------

    /// Drop every SGR escape so a test can assert on the plain text a human
    /// sees, without keying on any color value.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn styled_activity_line_is_wrapped_and_reset() {
        // Not "is it the right green" (a theme choice tuned live in the REPL);
        // the invariant is the content survives and the line resets, so nothing
        // bleeds into the next line.
        let (mut ui, out, _) = harness(Mode::Repl, true, true);
        ui.tool_done("id", "done", false);
        let s = out.text();
        assert_eq!(
            strip_ansi(&s),
            "* done\n",
            "content or marker changed under the color"
        );
        assert!(
            s.ends_with("\x1b[0m\n"),
            "line not reset-terminated (bleed risk): {s:?}"
        );
        assert_ne!(s, "* done\n", "styled line must differ from the plain form");
    }

    #[test]
    fn styled_activity_label_is_padded_to_a_column() {
        // The alignment contract: whatever the label's length, the brief after
        // it starts at the same column, so a column of lines reads cleanly.
        let (mut ui, out, _) = harness(Mode::Repl, true, true);
        ui.tool_start("b", "bash", "cargo test", false); // 4-char label
        ui.tool_done("t", "task done (2 turns)", false); //                4-char label
        ui.tool_done("l", "ls . (3 entries)", false); //                  2-char label
        let lines: Vec<String> = strip_ansi(&out.text())
            .lines()
            .map(str::to_string)
            .collect();
        // Each brief starts at the same column: "* " (2) + the 7-column label.
        assert_eq!(
            lines,
            [
                "* bash   cargo test",
                "* task   done (2 turns)",
                "* ls     . (3 entries)"
            ]
        );
    }

    #[test]
    fn styled_activity_error_stays_red_end_to_end() {
        // A failure must read red the whole line, not split a colored label off
        // a green brief; red is the one reserved accent. Structurally: the
        // marker and text are one contiguous run (no escape between "* " and the
        // word), unlike a normal label line.
        let (mut ui, out, _) = harness(Mode::Repl, true, true);
        ui.tool_done("id", "boom failed", true);
        let s = out.text();
        assert!(
            s.contains("* boom failed"),
            "error line must not split a label out: {s:?}"
        );
        assert!(s.ends_with("\x1b[0m\n"), "error line must reset: {s:?}");
        // A non-error line of the same text does split the label, proving the
        // two paths differ (and that the split is what error suppresses).
        let (mut ui2, out2, _) = harness(Mode::Repl, true, true);
        ui2.tool_done("id", "boom failed", false);
        assert!(
            !out2.text().contains("* boom failed"),
            "non-error line should isolate the label"
        );
    }

    #[test]
    fn piped_tool_start_is_plain_and_unpadded() {
        // Byte-identity for the start line too: piped surfaces get no padding,
        // no color, exactly the pre-v0.2.4 single-space form.
        let (mut ui, out, _) = harness(Mode::Repl, false, false);
        ui.tool_start("b", "bash", "ls -a", false);
        assert_eq!(out.text(), "* bash ls -a\n");
    }

    #[test]
    fn error_bytes_match_note_when_not_styled() {
        // error() is a display-only recolor of note(); on every non-styled
        // surface its bytes must not drift from note()'s.
        let (mut ui_e, out_e, _) = harness(Mode::Repl, false, false);
        ui_e.error("boom");
        let (mut ui_n, out_n, _) = harness(Mode::Repl, false, false);
        ui_n.note("boom");
        assert_eq!(
            out_e.text(),
            out_n.text(),
            "error() drifted from note() when not styled"
        );
        assert_eq!(out_e.text(), "boom\n");
    }

    #[test]
    fn piped_repl_activity_is_plain() {
        let (mut ui, out, _) = harness(Mode::Repl, false, false);
        ui.tool_done("id", "done", false);
        assert_eq!(out.text(), "* done\n");
    }

    #[test]
    fn styled_checklist_shows_every_glyph_and_item() {
        // The visible block: the header and each glyph line survive under the
        // color, so a human reads the plan and its progress. Not "which green":
        // the invariant is the content and glyphs are intact and each line
        // resets so nothing bleeds.
        let (mut ui, out, _) = harness(Mode::Repl, true, true);
        let checklist = "plan (1/3 done):\n[x] research\n[~] build\n[ ] test";
        ui.checklist(checklist);
        let s = out.text();
        let plain = strip_ansi(&s);
        for line in ["plan (1/3 done):", "[x] research", "[~] build", "[ ] test"] {
            assert!(
                plain.contains(line),
                "checklist line {line:?} missing from: {plain:?}"
            );
        }
        assert!(s.contains('\x1b'), "themed checklist must carry styling");
        assert_ne!(
            s,
            format!("{checklist}\n"),
            "styled block must differ from plain"
        );
        // Every opener is reset (paired escapes), so no color leaks past a line.
        assert_eq!(s.matches("\x1b[0m").count() * 2, s.matches("\x1b[").count());
    }

    #[test]
    fn failed_agent_glyph_uses_the_error_tone() {
        let (ui, _out, _err) = harness(Mode::Repl, true, true);
        let rows = ui.checklist_region_rows("agents:\n[!] agent 1: failed", 80);
        assert!(
            rows[1].contains(&ui.theme.error.sgr(ui.depth)),
            "{:?}",
            rows[1]
        );
    }

    #[test]
    fn checklist_region_rows_are_one_clamped_physical_row_each() {
        // The pinned-dock view of a plan: one styled physical row per source
        // line so the variable-height frame erases and redraws by an exact count.
        // Blank lines are dropped (an empty frame row is wasted height), and an
        // over-long line is clamped with an ellipsis so it can never wrap.
        let (ui, _out, _err) = harness(Mode::Repl, true, true);
        let block = "plan (1/3 done):\n[x] short\n[ ] pending\n\n\
                     [~] a very long item that must be clamped to the row";
        let rows = ui.checklist_region_rows(block, 24);
        assert_eq!(rows.len(), 4, "one row per non-empty source line: {rows:?}");
        for row in &rows {
            let plain = strip_ansi(row);
            assert!(
                plain.chars().count() <= 24,
                "row exceeds the width clamp: {plain:?}"
            );
        }
        assert!(strip_ansi(&rows[0]).contains("plan (1/3 done):"));
        assert!(strip_ansi(&rows[1]).contains("[x] short"));
        assert!(
            strip_ansi(&rows[3]).ends_with('…'),
            "the over-long row must be truncated: {:?}",
            strip_ansi(&rows[3])
        );
        assert!(
            rows.iter().all(|r| r.contains('\x1b')),
            "region rows must carry styling"
        );
    }

    #[test]
    fn checklist_region_rows_are_empty_off_the_themed_surface() {
        // The pinned region is a themed-REPL affordance, exactly like the
        // scrolled block: a no-color tty, a piped REPL, and exec all pin nothing.
        for (mode, color, ansi) in [
            (Mode::Repl, false, true),
            (Mode::Repl, false, false),
            (Mode::Exec, true, true),
        ] {
            let (ui, _o, _e) = harness(mode, color, ansi);
            assert!(
                ui.checklist_region_rows("plan (0/1 done):\n[ ] x", 40)
                    .is_empty()
            );
        }
    }

    #[test]
    fn canceled_plan_summary_uses_the_error_tone() {
        let (ui, _, _) = harness(Mode::Repl, true, true);
        let row = ui.region_summary_row("plan canceled · 1/3", 80, RegionTone::Error);
        assert!(row.contains(&ui.theme.error.sgr(ui.depth)), "{row:?}");
        assert!(row.contains("plan canceled · 1/3"));
        assert!(row.ends_with(RESET));
    }

    #[test]
    fn checklist_is_silent_on_byte_identity_surfaces() {
        // The block is a themed-REPL affordance only. Piped REPL, exec, JSON,
        // and child must gain zero bytes here: the tool summary line and the
        // transcript already carry the plan, so those surfaces stay identical.
        let checklist = "plan (0/1 done):\n[ ] test";
        for (label, mode, color, ansi) in [
            ("no_color_repl", Mode::Repl, false, true), // NO_COLOR at a tty
            ("piped_repl", Mode::Repl, false, false),   // piped
            ("exec", Mode::Exec, true, true),           // exec, even at a color tty
            ("exec_json", Mode::ExecJson, true, true),
            ("child", Mode::Child, true, true),
        ] {
            let (mut ui, out, err) = harness(mode, color, ansi);
            ui.checklist(checklist);
            assert!(
                out.text().is_empty(),
                "{label} stdout gained checklist bytes"
            );
            assert!(
                err.text().is_empty(),
                "{label} stderr gained checklist bytes"
            );
        }
    }

    #[test]
    fn styled_agents_panel_shows_every_row_and_differs_from_plain() {
        // The visible fan-out block: the header (with the concurrency cap) and
        // each glyph row survive under the color, so a human reads which agent
        // is doing what and whether it finished. Not "which green": the content
        // and glyphs are intact and each line resets so nothing bleeds.
        let (mut ui, out, _) = harness(Mode::Repl, true, true);
        let block = "agents (1/3 done, up to 4 at once):\n\
                     [x] agent 1: fetch alpha  (7 turns) · Alpha summary line\n\
                     [~] agent 2: fetch beta\n\
                     [~] agent 3: fetch gamma";
        ui.agents(
            block,
            &["f1".to_string(), "f2".to_string(), "f3".to_string()],
        );
        let s = out.text();
        let plain = strip_ansi(&s);
        for line in [
            "agents (1/3 done, up to 4 at once):",
            "[x] agent 1: fetch alpha  (7 turns) · Alpha summary line",
            "[~] agent 2: fetch beta",
            "[~] agent 3: fetch gamma",
        ] {
            assert!(
                plain.contains(line),
                "agents line {line:?} missing from: {plain:?}"
            );
        }
        assert!(s.contains('\x1b'), "themed agents panel must carry styling");
        assert_ne!(
            s,
            format!("{block}\n"),
            "styled block must differ from plain"
        );
        assert_eq!(s.matches("\x1b[0m").count() * 2, s.matches("\x1b[").count());
    }

    #[test]
    fn agents_panel_is_silent_on_byte_identity_surfaces() {
        // The panel is a themed-REPL affordance only. Piped REPL, exec, JSON,
        // and child must gain zero bytes here: the `* task` activity lines and
        // the transcript already carry the fan-out, so those surfaces stay
        // byte-for-byte what they were.
        let block = "agents (0/2 done, up to 4 at once):\n[~] agent 1: a\n[~] agent 2: b";
        for (label, mode, color, ansi) in [
            ("no_color_repl", Mode::Repl, false, true),
            ("piped_repl", Mode::Repl, false, false),
            ("exec", Mode::Exec, true, true),
            ("exec_json", Mode::ExecJson, true, true),
            ("child", Mode::Child, true, true),
        ] {
            let (mut ui, out, err) = harness(mode, color, ansi);
            ui.agents(block, &["f1".to_string(), "f2".to_string()]);
            assert!(out.text().is_empty(), "{label} stdout gained panel bytes");
            assert!(err.text().is_empty(), "{label} stderr gained panel bytes");
        }
    }

    #[test]
    fn styled_panel_suppresses_covered_task_lines_but_not_others() {
        // On the themed surface the panel replaces the redundant `* task` start
        // and `* task done` lines for the agents it covers, while an unrelated
        // tool call still prints its line.
        let (mut ui, out, _) = harness(Mode::Repl, true, true);
        ui.agents(
            "agents (0/1 done, up to 4 at once):\n[~] agent 1: solo",
            &["f1".to_string()],
        );
        ui.tool_start("f1", "subagent", "some prompt", false); // covered: suppressed
        ui.tool_done("f1", "task done (2 turns)", false); //  covered: suppressed
        ui.tool_done("b", "ls . (3 entries)", false); //      unrelated: shown
        let plain = strip_ansi(&out.text());
        assert!(
            !plain.contains("* task"),
            "a covered task line leaked past the panel:\n{plain}"
        );
        assert!(
            plain.contains("agent 1: solo"),
            "the panel itself must render:\n{plain}"
        );
        assert!(
            plain.contains("* ls"),
            "an unrelated tool line must still render:\n{plain}"
        );
    }

    #[test]
    fn piped_subagent_lines_are_unchanged_when_a_panel_is_emitted() {
        // Byte-identity guard: on a piped REPL the panel emits nothing and the
        // subagent calls render their plain `* subagent ...` / summary lines.
        let (mut ui, out, _) = harness(Mode::Repl, false, false);
        ui.agents(
            "agents (0/2 done, up to 4 at once):\n[~] agent 1: a\n[~] agent 2: b",
            &["f1".to_string(), "f2".to_string()],
        );
        ui.tool_start("f1", "subagent", "helper a", false);
        ui.tool_done("f1", "task done (3 turns)", false);
        assert_eq!(out.text(), "* subagent helper a\n* task done (3 turns)\n");
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
        assert!(
            !out.text().contains('\x1b'),
            "exec stdout must carry no escapes"
        );
        assert_eq!(
            err.text(),
            "\x1b[2m* done\x1b[0m\n",
            "exec activity must stay legacy dim"
        );
    }

    #[test]
    fn assistant_markdown_keeps_split_text_contiguous_and_flushes_cleanly() {
        // Line buffering keeps a marker or word split across provider deltas
        // intact, then end_line flushes the final partial line.
        let (mut ui, out, _) = harness(Mode::Repl, true, true);
        ui.text_delta("1. write ");
        ui.text_delta("greeting.txt");
        assert!(
            out.text().is_empty(),
            "partial source line should stay buffered"
        );
        ui.end_line();
        let s = out.text();
        assert!(
            strip_ansi(&s).contains("1. write greeting.txt\n"),
            "split text changed: {s:?}"
        );
        assert_eq!(s.matches("\x1b[0m").count() * 2, s.matches("\x1b[").count());
    }

    #[test]
    fn markdown_resets_even_when_message_ends_in_newline() {
        let (mut ui, out, _) = harness(Mode::Repl, true, true);
        ui.text_delta("**done**\n");
        ui.end_line();
        let s = out.text();
        assert_eq!(strip_ansi(&s), "done\n");
        assert_eq!(s.matches("\x1b[0m").count() * 2, s.matches("\x1b[").count());
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
        assert!(
            s.ends_with("\x1b[0m"),
            "prompt not reset (would color typed input): {s:?}"
        );
    }

    #[test]
    fn child_text_goes_to_stderr_unstyled() {
        let (mut ui, out, err) = harness(Mode::Child, true, true);
        ui.text_delta("progress");
        assert_eq!(
            out.text(),
            "",
            "child stdout is reserved for the result line"
        );
        assert_eq!(err.text(), "progress");
    }

    #[test]
    fn execjson_events_are_never_colored() {
        let (mut ui, out, _) = harness(Mode::ExecJson, true, true);
        ui.text_delta("hi");
        let s = out.text();
        assert!(
            s.contains("\"t\":\"text\"") || s.contains("\"text\""),
            "event missing: {s:?}"
        );
        assert!(
            !s.contains('\x1b'),
            "JSONL protocol must carry no escapes: {s:?}"
        );
    }

    #[test]
    fn replay_transcript_redisplays_turns_and_filters_synthetic_items() {
        use noob_provider::types::ToolCall;
        // color=false, ansi=true: the dim/rich-text path renders Markdown but
        // emits no color, so strip_ansi yields exactly the human-visible text.
        let (mut ui, out, _) = harness(Mode::Repl, false, true);
        let items = vec![
            Item::User("run the tests".into()),
            Item::Assistant {
                text: "Running the tests now.".into(),
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "bash".into(),
                    arguments: r#"{"cmd":"cargo test"}"#.into(),
                }],
                raw_items: vec![],
            },
            Item::ToolResult {
                call_id: "c1".into(),
                content: "running 12 tests\ntest result: ok. 12 passed".into(),
            },
            Item::Assistant {
                text: "All **12 tests** pass.".into(),
                tool_calls: vec![],
                raw_items: vec![],
            },
            // Synthetic plumbing between the real turns: all must be filtered.
            Item::User("[skills updated] now available: pdf: read pdfs.".into()),
            Item::User(crate::agent::PLAN_ENTER_MSG.into()),
            Item::User("[conversation summary]\nwork happened".into()),
            Item::User(
                "[earlier conversation dropped: 8 items removed because summary invalid]".into(),
            ),
            Item::User(
                "[background sub-agent result agent-1]\nstatus: ok\nresult:\ninternal report"
                    .into(),
            ),
            Item::User("great, thanks".into()),
            Item::Assistant {
                text: "You're welcome!".into(),
                tool_calls: vec![],
                raw_items: vec![],
            },
        ];
        ui.replay_transcript(&items);
        let plain = strip_ansi(&out.text());
        assert_eq!(
            plain,
            "\u{203a} run the tests\n\
             Running the tests now.\n\
             * bash cargo test\n\
             * bash running 12 tests\n\
             All 12 tests pass.\n\
             \u{203a} great, thanks\n\
             You're welcome!\n",
            "replay output drifted:\n{plain}"
        );
        // The synthetic markers never reach the screen.
        assert!(
            !plain.contains("[skills updated]"),
            "a skills note leaked into replay"
        );
        assert!(
            !plain.contains("[plan mode]"),
            "the plan toggle leaked into replay"
        );
        assert!(
            !plain.contains("[conversation summary]"),
            "a summary leaked into replay"
        );
        assert!(
            !plain.contains("earlier conversation dropped"),
            "a hard-drop compaction stub leaked into replay"
        );
        assert!(
            !plain.contains("background sub-agent result"),
            "a background report was replayed as human input"
        );
    }

    #[test]
    fn thinking_scanner_never_starts_off_the_themed_surface() {
        // Byte-identity guard: on every non-themed surface (a NO_COLOR or piped
        // repl, exec, json, child) starting the scanner must be a no-op, so
        // those streams gain no animation bytes. Structural: no scanner is
        // created, and stopping one is safe.
        for (mode, color, ansi) in [
            (Mode::Repl, false, true),  // NO_COLOR at a tty
            (Mode::Repl, false, false), // piped
            (Mode::Exec, true, true),   // exec, even at a tty with color
            (Mode::ExecJson, true, true),
            (Mode::Child, true, true),
        ] {
            let (mut ui, _o, _e) = harness(mode, color, ansi);
            ui.thinking_start();
            assert!(
                ui.scanner.is_none(),
                "scanner started on a non-themed surface"
            );
            ui.thinking_stop();
        }
    }

    // --- the turn Ui (dock plumbing) ---------------------------------------

    /// Replay every semantic Render event through the main-thread renderer.
    fn drain_render(
        rx: &std::sync::mpsc::Receiver<dock::Ev>,
        renderer: &mut BufferedTurnRenderer,
    ) -> Vec<u8> {
        while let Ok(ev) = rx.try_recv() {
            if let dock::Ev::Render(event) = ev {
                renderer.render(event);
            }
        }
        renderer.take()
    }

    #[test]
    fn turn_ui_semantics_replay_to_the_exact_direct_bytes() {
        let (mut direct, refout, _) = harness(Mode::Repl, true, true);
        let mut renderer = direct.buffered_turn_renderer();
        let (tx, rx) = std::sync::mpsc::sync_channel(64);
        let mut turn = direct.for_turn(tx);
        for ui in [&mut direct, &mut turn] {
            ui.text_delta("1. write ");
            ui.text_delta("greeting.txt");
            ui.tool_done("id", "done", false);
            ui.note("a note");
            ui.end_line();
        }
        assert_eq!(
            String::from_utf8_lossy(&drain_render(&rx, &mut renderer)),
            refout.text(),
            "semantic replay drifted from the direct Ui"
        );
    }

    #[test]
    fn turn_ui_ask_travels_the_channel_and_denies_on_teardown() {
        let (direct, _, _) = harness(Mode::Repl, true, true);
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        let mut turn = direct.for_turn(tx);
        // A render loop answering "yes".
        let answerer = std::thread::spawn(move || {
            for ev in rx.iter() {
                if let dock::Ev::Ask(q, reply) = ev {
                    assert!(q.contains("proceed"));
                    reply.send(true).unwrap();
                    break;
                }
            }
            rx
        });
        assert!(turn.ask("proceed?"), "channel ask must return the answer");
        let rx = answerer.join().unwrap();
        // "No" travels too.
        let answerer = std::thread::spawn(move || {
            for ev in rx.iter() {
                if let dock::Ev::Ask(_, reply) = ev {
                    reply.send(false).unwrap();
                    break;
                }
            }
            rx
        });
        assert!(!turn.ask("proceed?"));
        let rx = answerer.join().unwrap();
        // Render loop gone entirely: degrade to deny, never hang or panic.
        drop(rx);
        assert!(!turn.ask("proceed?"), "a closed channel must deny");
    }

    #[test]
    fn turn_ui_never_spawns_the_scanner_thread() {
        // The scanner is a second terminal writer; on a turn Ui it must be
        // structurally impossible even on the fully styled surface.
        let (direct, _, _) = harness(Mode::Repl, true, true);
        let (tx, _rx) = std::sync::mpsc::sync_channel(4);
        let mut turn = direct.for_turn(tx);
        turn.thinking_start();
        assert!(turn.scanner.is_none(), "turn Ui must not start a scanner");
        turn.thinking_stop();
    }

    // That the first output byte tears the scanner down (so the reply never
    // interleaves with the comet) is proven end to end in a real pty by the e2e
    // `raw_repl_shows_a_thinking_scanner_while_the_model_works`, which asserts no
    // comet frame lands after the reply begins. It is not unit-tested here
    // because starting a real scanner spawns a thread that writes to the process
    // stdout, which cargo cannot capture and which would litter the runner.
}
