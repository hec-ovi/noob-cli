//! The dock: a persistent input frame that stays live while a turn streams,
//! output scrolling above it into native scrollback. One thread (the render
//! loop, on main) is the only terminal writer; the turn worker ships semantic
//! rendering operations and a reader thread ships decoded keys over one
//! strictly ordered channel.
//!
//! Nothing in this module knows the agent: the worker is a closure the caller
//! hands in, and the turn's outcome travels back opaquely. The dock is the
//! default interactive driver; `NOOB_DOCK=0` keeps the classic prompt.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::time::{Duration, Instant};

use noob_provider::http::INTERRUPTED;

use super::prompt::{Decoder, Editor, Input, Key, RawGuard, Step, term_width};
use super::style::{ColorDepth, RESET};
use super::{TurnEvent, Ui, safe_terminal_text, scanner};

/// One event on the dock channel. Its receive order is the behavioral order:
/// only adjacent `Render` events may be coalesced, while Ask/Key/ReaderGone/End
/// are hard barriers that are handled before anything after them.
pub(crate) enum Ev {
    /// A semantic rendering operation from the turn worker. The main thread
    /// replays it through the normal renderer and owns the resulting bytes.
    Render(TurnEvent),
    /// A y/N confirmation from agent code. The worker blocks on the reply,
    /// so the render loop must answer (or drop the sender to deny).
    Ask(String, SyncSender<bool>),
    /// One decoded keystroke from the stdin reader.
    Key(Key),
    /// The stdin reader thread has exited: the input stream reached real EOF
    /// or errored (a closed tty, an SSH/pty drop). Distinct from `Key(Eof)`
    /// (a decoded Ctrl-D byte, after which the reader keeps running): once
    /// this arrives no further keys can, so any consumer blocked on input
    /// must give up rather than wait forever on a channel this session keeps
    /// alive (`DockSession` holds a `Sender`, so `recv` never disconnects).
    ReaderGone,
    /// The worker's final event: the turn is over, its result is parked in
    /// the run wrapper. Carries no payload so this module stays agent-free.
    End,
}

/// `NOOB_DOCK=0|false|off|no` opts out of the dock. Unset and every other
/// value leave the default interactive driver enabled.
pub(super) fn enabled_by_env() -> bool {
    match std::env::var("NOOB_DOCK") {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no"),
        Err(_) => true,
    }
}

/// How long the reader waits for the tail of an escape sequence before a
/// dangling lone ESC is flushed as the ESC key.
const ESC_GRACE_MS: i32 = 50;
/// Liveness repaint cadence for the whole active turn.
const COMET_MS: u64 = 120;
/// Small frame budget used to fold a sparse run of adjacent semantic render
/// operations into one terminal repaint. A non-render event ends it early.
const COALESCE_MS: u64 = 8;
/// How long a first ESC arms the cancel: a second ESC inside this window
/// cancels the turn, and the window lapsing reverts the dock to normal.
const CANCEL_WINDOW: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// The output column tracker
// ---------------------------------------------------------------------------

/// Escape-sequence scanner state, kept across feeds because a sequence can
/// straddle two `Ev::Render` chunks (a model is free to emit a split escape).
enum EscScan {
    Normal,
    /// Saw ESC, waiting for the introducer.
    SawEsc,
    /// Inside `CSI ... final`; the count bounds a malformed run.
    Csi(usize),
    /// Inside an OSC string, until BEL or ST; the count bounds it.
    Osc(usize),
    /// Inside an OSC, saw ESC (a potential `ESC \` terminator).
    OscEsc(usize),
}

/// Longest parameter run a CSI may consume before the tracker gives up and
/// returns to text (mirrors the decoder's cap, so junk cannot wedge it).
const CSI_CAP: usize = 64;
/// Bound on an OSC payload for the same reason.
const OSC_CAP: usize = 4096;

/// Tracks where the streamed output's insertion point sits, so the dock can
/// be erased and the cursor put back exactly where the next bytes belong.
///
/// `col` is the insertion column on the current row. `fresh` means the
/// insertion point is column 0 of a row with no content yet (the last write
/// ended in a newline, or exactly filled its row and the terminal's wrap
/// will open the next one): the dock's top row IS the insertion row, so the
/// restore needs no cursor-up. When not fresh, the insertion point is one
/// row above the dock top at `col`.
///
/// Widths count one column per character (continuation bytes are free), the
/// same single-width simplification as the editor's `input_window`: runs of
/// double-width CJK or emoji can drift the estimate, which at worst moves a
/// dock repaint, never the transcript. SGR/CSI/OSC sequences are zero-width;
/// `\n`, `\r`, `\t`, and backspace move the point the way a terminal does.
///
/// `newline` splits the `fresh` state into its two physically different
/// causes. Both leave `col == 0`, but on a DECAWM terminal a real `\n` has
/// advanced to a new empty row, whereas exactly filling the last column
/// leaves the cursor parked in that column with the deferred-wrap latch set
/// and NO row advance. `newline` is true only in the former case. The render
/// loop keys the leading cursor-restore on `fresh` (both cases resume at
/// column 0) but the trailing row-advance on `newline`, so the exact-fill
/// case emits a `\r\n` before the dock is redrawn: without it the redraw's
/// `\r`-prefixed erase would cancel the latch, land on the filled row, and
/// wipe that line of streamed output.
pub(crate) struct OutTracker {
    pub(crate) col: usize,
    pub(crate) fresh: bool,
    pub(crate) newline: bool,
    esc: EscScan,
}

impl Default for OutTracker {
    fn default() -> Self {
        // A turn starts on a fresh row: the prompt collapsed to its
        // `› message` record and ended the line with a real newline.
        OutTracker { col: 0, fresh: true, newline: true, esc: EscScan::Normal }
    }
}

impl OutTracker {
    /// Advance the insertion point over `bytes` at the given terminal
    /// width. Never panics on any input; a width under 2 degrades to
    /// "every glyph fills the row".
    pub(crate) fn feed(&mut self, bytes: &[u8], width: usize) {
        for &b in bytes {
            match self.esc {
                EscScan::SawEsc => {
                    self.esc = match b {
                        b'[' => EscScan::Csi(0),
                        b']' => EscScan::Osc(0),
                        // Two-byte escape (ESC 7, ESC =, ...): consumed.
                        _ => EscScan::Normal,
                    };
                }
                EscScan::Csi(n) => {
                    if (0x40..=0x7e).contains(&b) || n >= CSI_CAP {
                        self.esc = EscScan::Normal;
                    } else {
                        self.esc = EscScan::Csi(n + 1);
                    }
                }
                EscScan::Osc(n) => {
                    self.esc = match b {
                        0x07 => EscScan::Normal,
                        0x1b => EscScan::OscEsc(n),
                        _ if n >= OSC_CAP => EscScan::Normal,
                        _ => EscScan::Osc(n + 1),
                    };
                }
                EscScan::OscEsc(n) => {
                    self.esc = match b {
                        b'\\' | 0x07 => EscScan::Normal,
                        0x1b => EscScan::OscEsc(n),
                        _ if n >= OSC_CAP => EscScan::Normal,
                        _ => EscScan::Osc(n + 1),
                    };
                }
                EscScan::Normal => match b {
                    0x1b => self.esc = EscScan::SawEsc,
                    b'\n' => {
                        self.col = 0;
                        self.fresh = true;
                        self.newline = true;
                    }
                    b'\r' => {
                        self.col = 0;
                        // Column 0 but on a row that still holds content: not
                        // a fresh empty row, so the dock must still advance.
                        self.newline = false;
                    }
                    b'\t' => {
                        // Tabs advance to the next 8-stop and clamp at the
                        // last column; a tab never wraps the row.
                        let next = (self.col / 8 + 1) * 8;
                        self.col = next.min(width.saturating_sub(1));
                        self.fresh = false;
                        self.newline = false;
                    }
                    0x08 => {
                        self.col = self.col.saturating_sub(1);
                        self.newline = false;
                    }
                    // Other C0 controls and DEL are zero-width.
                    _ if b < 0x20 || b == 0x7f => {}
                    // UTF-8 continuation bytes are width-free; the lead
                    // byte already counted the character's single cell.
                    _ if b & 0xc0 == 0x80 => {}
                    _ => self.print_cell(width),
                },
            }
        }
    }

    /// One printed cell. Filling the last column opens a fresh row (the
    /// terminal's deferred wrap will place the next glyph at column 0 of
    /// the next line, which is exactly the `fresh` contract).
    fn print_cell(&mut self, width: usize) {
        // A printed cell is never a fresh empty row: even the exact-fill
        // wrap leaves the cursor in the last column with the latch set until
        // the next glyph, so `newline` is false in every branch here.
        self.newline = false;
        if width <= 1 {
            self.col = 0;
            self.fresh = true;
            return;
        }
        self.col += 1;
        if self.col >= width {
            self.col = 0;
            self.fresh = true;
        } else {
            self.fresh = false;
        }
    }
}

// ---------------------------------------------------------------------------
// The stdin reader thread
// ---------------------------------------------------------------------------

/// Blocking reads on stdin, decoded and shipped as key events. Exits when
/// the channel closes (the session ended) or stdin reaches EOF. A dangling
/// lone ESC is resolved by a single bounded poll: if no sequence tail
/// arrives within the grace window, it was a human ESC press.
fn reader(tx: SyncSender<Ev>) {
    let mut dec = Decoder::default();
    let mut buf = [0u8; 1024];
    loop {
        let n = unsafe {
            libc::read(libc::STDIN_FILENO, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
        };
        if n < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                // An external SIGINT (kill -INT; the keyboard's Ctrl-C is a
                // byte in raw mode). The handler set the flag; surface it as
                // the interrupt key so the loop reacts without a keystroke.
                if INTERRUPTED.load(Ordering::SeqCst)
                    && tx.send(Ev::Key(Key::Interrupt)).is_err()
                {
                    return;
                }
                continue;
            }
            // A real read error (a closed/broken tty): the reader is done.
            let _ = tx.send(Ev::ReaderGone);
            return;
        }
        if n == 0 {
            // Genuine end of the input stream, not a Ctrl-D byte.
            let _ = tx.send(Ev::ReaderGone);
            return;
        }
        for key in dec.feed(&buf[..n as usize]) {
            if tx.send(Ev::Key(key)).is_err() {
                return;
            }
        }
        if dec.has_dangling_esc() {
            let mut pfd = libc::pollfd { fd: libc::STDIN_FILENO, events: libc::POLLIN, revents: 0 };
            let ready = unsafe { libc::poll(&mut pfd, 1, ESC_GRACE_MS) };
            if ready == 0
                && let Some(key) = dec.flush_dangling_esc()
                && tx.send(Ev::Key(key)).is_err()
            {
                return;
            }
            // ready > 0: the sequence tail is waiting, the next read joins
            // it; ready < 0 (EINTR): the next read's error path handles it.
        }
    }
}

// ---------------------------------------------------------------------------
// The session driver
// ---------------------------------------------------------------------------

/// The y/N modal while agent code waits on a confirmation: the question
/// replaces the input row, fresh keys type the answer, and anything typed
/// before the ask arrived is untouchable draft (so type-ahead structurally
/// cannot satisfy a confirmation).
struct AskState {
    question: String,
    answer: String,
    reply: SyncSender<bool>,
}

/// The double-ESC cancel state machine for one turn. First ESC arms (until
/// the window lapses); a second ESC inside the window, or a Ctrl-C at any
/// time, commits by setting the shared INTERRUPTED flag the watchdog and the
/// agent loop already poll. Any other key, or the window lapsing, disarms.
#[derive(Default)]
struct Cancel {
    /// Armed until this instant; None once it lapses or commits.
    armed_until: Option<Instant>,
    /// Committed: the interrupt is set, awaiting the worker's `End`.
    committed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InterruptAction {
    Cancel,
    HardExit,
}

impl Cancel {
    fn is_armed(&self) -> bool {
        self.armed_until.is_some()
    }

    /// Commit the cancel: set the process-wide flag (the watchdog trips the
    /// in-flight read within one tick) and drop the arm.
    fn commit(&mut self) {
        INTERRUPTED.store(true, Ordering::SeqCst);
        self.committed = true;
        self.armed_until = None;
    }

    /// Disarm a pending first-ESC without committing. Returns whether it had
    /// been armed, so the caller knows to repaint.
    fn disarm(&mut self) -> bool {
        self.armed_until.take().is_some()
    }

    fn expire(&mut self, now: Instant) -> bool {
        if self.armed_until.is_some_and(|deadline| now >= deadline) {
            self.armed_until = None;
            true
        } else {
            false
        }
    }

    fn interrupt(&mut self) -> InterruptAction {
        if self.committed {
            InterruptAction::HardExit
        } else {
            self.commit();
            InterruptAction::Cancel
        }
    }
}

fn hard_exit() -> ! {
    super::prompt::restore_terminal();
    unsafe { libc::_exit(130) }
}

/// One dock REPL session: raw mode held for its lifetime, a persistent
/// stdin reader, and the draft editor that survives across turns (typing
/// while the agent works lands here and is waiting at the next prompt).
pub struct DockSession {
    tx: SyncSender<Ev>,
    rx: Receiver<Ev>,
    /// Keys decoded but not yet applied (a multi-line paste submits one
    /// line per turn; the tail waits here, replacing the old CARRYOVER).
    pending: VecDeque<Key>,
    draft: Editor,
    /// Messages typed and submitted with Enter WHILE a turn ran: they are
    /// dispatched one per turn once the agent is free (FIFO), and drained back
    /// into the draft if the turn is interrupted rather than fired blindly.
    queue: VecDeque<String>,
    /// Permanent input-stream state. The channel itself cannot disconnect
    /// while this session owns `tx`, so ReaderGone must survive the turn where
    /// it was observed and make every later prompt return EOF immediately.
    reader_gone: bool,
    _guard: RawGuard,
}

impl DockSession {
    /// Enter raw mode for the session and start the reader. None when the
    /// terminal refuses raw (the caller then uses the ordinary readers).
    pub(super) fn start() -> Option<DockSession> {
        let guard = RawGuard::enter()?;
        let (tx, rx) = sync_channel(128);
        let reader_tx = tx.clone();
        std::thread::spawn(move || reader(reader_tx));
        Some(DockSession {
            tx,
            rx,
            pending: VecDeque::new(),
            draft: Editor::default(),
            queue: VecDeque::new(),
            reader_gone: false,
            _guard: guard,
        })
    }

    /// Dispatch the next queued message, if any, as this prompt's input: it
    /// was already accepted (echoed on Enter during the turn), so it is
    /// returned without re-reading stdin, one per prompt = one per turn.
    fn next_queued(&mut self) -> Option<Input> {
        self.queue.pop_front().map(Input::Line)
    }

    /// A turn ended in an interrupt: move any still-queued messages back into
    /// the editable draft (newline-joined, oldest first, before any half-typed
    /// draft) instead of firing them, since a cancel means "stop, I'll drive".
    pub fn drain_queue_to_draft(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let mut parts: Vec<String> = self.queue.drain(..).collect();
        let half_typed = self.draft.line();
        if !half_typed.is_empty() {
            parts.push(half_typed);
        }
        self.draft = Editor::from_line(&parts.join("\n"));
    }

    /// Read one line at the idle prompt, event-driven but with the exact
    /// semantics of the per-prompt raw editor: bare marker until the first
    /// key expands the frame, submit collapses to a `› message` record,
    /// Ctrl-C cancels the line, Ctrl-D on an empty line is EOF. A draft
    /// typed during the previous turn is already visible and editable.
    pub fn read_prompt(&mut self, ui: &mut Ui, plan: bool) -> Input {
        // Queued messages were echoed at acceptance time during the turn. Do
        // not print them a second time when they are dispatched.
        if let Some(input) = self.next_queued() {
            return input;
        }
        if self.reader_gone && self.pending.is_empty() {
            return Input::Eof;
        }
        let mut width = term_width();
        let mut expanded = false;
        if !self.draft.is_empty() {
            expanded = true;
            ui.expand(plan, width);
        }
        loop {
            let mut acted = false;
            while let Some(key) = self.pending.pop_front() {
                acted = true;
                match self.draft.apply(key) {
                    Step::Continue => {}
                    Step::Submit => return self.submit(ui, expanded),
                    Step::Interrupt => {
                        ui.erase(expanded);
                        self.draft = Editor::default();
                        INTERRUPTED.swap(false, Ordering::SeqCst);
                        return Input::Interrupted;
                    }
                    Step::Eof => {
                        ui.erase(expanded);
                        return Input::Eof;
                    }
                }
            }
            if self.reader_gone {
                ui.erase(expanded);
                return Input::Eof;
            }
            if acted && !expanded {
                expanded = true;
                width = term_width();
                ui.expand(plan, width);
            } else if expanded {
                ui.refit(plan, &mut width);
            }
            ui.redraw_input_row(&self.draft, width);
            let mut gone = match self.rx.recv() {
                Ok(ev) => self.absorb_idle(ev),
                // Every Sender dropped: the session is torn down.
                Err(_) => true,
            };
            while let Ok(ev) = self.rx.try_recv() {
                gone |= self.absorb_idle(ev);
            }
            // The reader has exited (real EOF/error): no key will ever arrive
            // again and `recv` would block forever (this session still holds a
            // Sender), so end the REPL. Any complete line already in `pending`
            // was submitted by the drain above before this point.
            if gone {
                self.reader_gone = true;
                // Loop once more: keys sent before ReaderGone are ahead of it
                // in the channel and may contain a final complete submission.
                continue;
            }
        }
    }

    /// File an idle-time event: keys queue for the editor, a straggler from a
    /// finished turn is dropped. Returns true when the reader is gone, so the
    /// caller stops waiting for input that can never come.
    fn absorb_idle(&mut self, ev: Ev) -> bool {
        match ev {
            Ev::Key(k) => {
                self.pending.push_back(k);
                false
            }
            Ev::ReaderGone => {
                self.reader_gone = true;
                true
            }
            _ => false,
        }
    }

    /// Finish a submitted line: reconcile a stray interrupt (same rule as
    /// the per-prompt editor), collapse the frame to the message record,
    /// and hand the line out. The draft resets for the next prompt.
    fn submit(&mut self, ui: &mut Ui, expanded: bool) -> Input {
        if INTERRUPTED.swap(false, Ordering::SeqCst) {
            ui.erase(expanded);
            self.draft = Editor::default();
            return Input::Interrupted;
        }
        ui.collapse_to_message(&self.draft, expanded);
        let line = self.draft.line();
        self.draft = Editor::default();
        Input::Line(line)
    }

    /// Run one turn with the dock live: the worker emits semantic events while
    /// this main-thread loop owns all rendering and input. Generic over the
    /// outcome so this module never learns the agent's types.
    pub fn run_turn<R: Send>(
        &mut self,
        ui: &mut Ui,
        plan: bool,
        f: impl FnOnce(&mut Ui) -> R + Send,
    ) -> R {
        let mut turn_ui = ui.for_turn(self.tx.clone());
        let slot: Mutex<Option<R>> = Mutex::new(None);
        std::thread::scope(|s| {
            let slot = &slot;
            s.spawn(move || {
                // Always send the End barrier, including a panic path, so the
                // terminal-owning thread cannot wait forever. The scoped
                // thread rethrows after the render loop has restored the dock.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    f(&mut turn_ui)
                }));
                match outcome {
                    Ok(end) => {
                        *slot.lock().unwrap() = Some(end);
                        turn_ui.turn_end();
                    }
                    Err(payload) => {
                        turn_ui.turn_end();
                        std::panic::resume_unwind(payload);
                    }
                }
            });
            self.render_loop(ui, plan);
        });
        slot.into_inner()
            .unwrap()
            .expect("the turn worker ended without parking its result")
    }

    /// The render loop: sole terminal writer while a turn runs. Receive order
    /// is semantic order. Only adjacent Render events share a repaint; Ask,
    /// keys, reader loss, and End are strict barriers.
    fn render_loop(&mut self, ui: &mut Ui, plan: bool) {
        let mut tracker = OutTracker::default();
        let mut width = term_width();
        let mut ask: Option<AskState> = None;
        let mut cancel = Cancel::default();
        let mut renderer = ui.buffered_turn_renderer();
        let mut active_tools: Vec<(String, String)> = Vec::new();
        let started = Instant::now();
        let mut deferred: Option<Ev> = None;

        // A running turn always owns a stable three-row frame. Its top status,
        // editable draft, and queue/cancel line are independent, so typing can
        // never hide liveness again.
        self.draw_active_frame(
            ui,
            plan,
            width,
            &ask,
            &cancel,
            started,
            &active_tools,
        );

        loop {
            cancel.expire(Instant::now());
            let now_width = term_width();
            if now_width != width {
                ui.erase(true);
                width = now_width;
                self.draw_active_frame(
                    ui,
                    plan,
                    width,
                    &ask,
                    &cancel,
                    started,
                    &active_tools,
                );
            }

            // Whole-turn liveness wakes only this display loop. Tool work,
            // provider I/O, and the transcript remain untouched.
            let mut wait = Duration::from_millis(COMET_MS);
            if let Some(deadline) = cancel.armed_until {
                let rem = deadline.saturating_duration_since(Instant::now());
                wait = wait.min(rem);
            }
            let first = match deferred.take() {
                Some(ev) => ev,
                None => match self.rx.recv_timeout(wait) {
                    Ok(ev) => ev,
                    Err(RecvTimeoutError::Timeout) => {
                        cancel.expire(Instant::now());
                        self.refresh_active_frame(
                            ui,
                            plan,
                            width,
                            &ask,
                            &cancel,
                            started,
                            &active_tools,
                        );
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                },
            };

            match first {
                Ev::Render(event) => {
                    Self::observe_render(&event, &mut active_tools);
                    renderer.render(event);

                    // Coalesce only adjacent rendering operations, for at most
                    // one small frame budget. The first non-render event is
                    // parked and handled next, never reordered behind output.
                    let deadline = Instant::now() + Duration::from_millis(COALESCE_MS);
                    loop {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        if remaining.is_zero() {
                            break;
                        }
                        match self.rx.recv_timeout(remaining) {
                            Ok(Ev::Render(event)) => {
                                Self::observe_render(&event, &mut active_tools);
                                renderer.render(event);
                            }
                            Ok(barrier) => {
                                deferred = Some(barrier);
                                break;
                            }
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    let batch = renderer.take();
                    if !batch.is_empty() {
                        self.write_above(
                            ui,
                            &mut tracker,
                            &batch,
                            plan,
                            width,
                            &ask,
                            &cancel,
                            started,
                            &active_tools,
                        );
                    } else {
                        self.refresh_active_frame(
                            ui,
                            plan,
                            width,
                            &ask,
                            &cancel,
                            started,
                            &active_tools,
                        );
                    }
                }
                Ev::Ask(question, reply) => {
                    route_ask(self.reader_gone, &mut ask, question, reply);
                    self.refresh_active_frame(
                        ui,
                        plan,
                        width,
                        &ask,
                        &cancel,
                        started,
                        &active_tools,
                    );
                }
                Ev::Key(key) => {
                    cancel.expire(Instant::now());
                    let mut queued_record = None;
                    if ask.is_some() {
                        match key {
                            Key::Enter => {
                                let a = ask.take().expect("ask exists");
                                let yes = matches!(a.answer.trim(), "y" | "Y" | "yes");
                                let _ = a.reply.send(yes);
                                cancel.disarm();
                            }
                            Key::Interrupt => {
                                let a = ask.take().expect("ask exists");
                                let _ = a.reply.send(false);
                                if cancel.interrupt() == InterruptAction::HardExit {
                                    hard_exit();
                                }
                            }
                            Key::Esc => {
                                if !cancel.committed && cancel.disarm() {
                                    let a = ask.take().expect("ask exists");
                                    let _ = a.reply.send(false);
                                    cancel.commit();
                                } else if !cancel.committed {
                                    cancel.armed_until = Some(Instant::now() + CANCEL_WINDOW);
                                }
                            }
                            Key::Eof => {
                                let a = ask.take().expect("ask exists");
                                let _ = a.reply.send(false);
                                cancel.disarm();
                            }
                            Key::Char(c) => {
                                cancel.disarm();
                                ask.as_mut().expect("ask exists").answer.push(c);
                            }
                            Key::Backspace => {
                                cancel.disarm();
                                ask.as_mut().expect("ask exists").answer.pop();
                            }
                            _ => {}
                        }
                    } else {
                        match key {
                            Key::Interrupt => {
                                if cancel.interrupt() == InterruptAction::HardExit {
                                    hard_exit();
                                }
                            }
                            Key::Esc => {
                                if cancel.committed {
                                    // Already canceling.
                                } else if cancel.disarm() {
                                    cancel.commit();
                                } else {
                                    cancel.armed_until = Some(Instant::now() + CANCEL_WINDOW);
                                }
                            }
                            Key::Enter => {
                                cancel.disarm();
                                if !cancel.committed && !self.draft.is_empty() {
                                    let message = self.draft.line();
                                    self.queue.push_back(message.clone());
                                    self.draft = Editor::default();
                                    queued_record = Some(self.queued_record(ui, &message));
                                }
                            }
                            Key::Eof => {
                                cancel.disarm();
                            }
                            other => {
                                cancel.disarm();
                                let _ = self.draft.apply(other);
                            }
                        }
                    }

                    if let Some(record) = queued_record {
                        self.write_above(
                            ui,
                            &mut tracker,
                            record.as_bytes(),
                            plan,
                            width,
                            &ask,
                            &cancel,
                            started,
                            &active_tools,
                        );
                    } else {
                        self.refresh_active_frame(
                            ui,
                            plan,
                            width,
                            &ask,
                            &cancel,
                            started,
                            &active_tools,
                        );
                    }
                }
                Ev::ReaderGone => {
                    self.reader_gone = true;
                    if let Some(a) = ask.take() {
                        let _ = a.reply.send(false);
                    }
                    self.refresh_active_frame(
                        ui,
                        plan,
                        width,
                        &ask,
                        &cancel,
                        started,
                        &active_tools,
                    );
                }
                Ev::End => {
                    ui.erase(true);
                    return;
                }
            }
        }
    }

    fn observe_render(event: &TurnEvent, active: &mut Vec<(String, String)>) {
        match event {
            TurnEvent::ToolStart { id, name, .. } => {
                active.retain(|(active_id, _)| active_id != id);
                active.push((id.clone(), frame_label(name)));
            }
            TurnEvent::ToolDone { id, .. } => active.retain(|(active_id, _)| active_id != id),
            TurnEvent::Done(_) => active.clear(),
            _ => {}
        }
    }

    /// Relay bytes above the frame without losing a partial output line, then
    /// redraw the active frame below it.
    #[allow(clippy::too_many_arguments)]
    fn write_above(
        &mut self,
        ui: &mut Ui,
        tracker: &mut OutTracker,
        bytes: &[u8],
        plan: bool,
        width: usize,
        ask: &Option<AskState>,
        cancel: &Cancel,
        started: Instant,
        active_tools: &[(String, String)],
    ) {
        ui.erase(true);
        if !tracker.fresh {
            ui.out_raw(format!("\x1b[1A\x1b[{}G", tracker.col + 1).as_bytes());
        }
        ui.out_raw(bytes);
        tracker.feed(bytes, width);
        if !tracker.newline {
            ui.out_raw(b"\r\n");
        }
        self.draw_active_frame(ui, plan, width, ask, cancel, started, active_tools);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_active_frame(
        &mut self,
        ui: &mut Ui,
        plan: bool,
        width: usize,
        ask: &Option<AskState>,
        cancel: &Cancel,
        started: Instant,
        active_tools: &[(String, String)],
    ) {
        let top = self.top_rule(ui, plan, width, started, active_tools);
        let bottom = self.bottom_rule(ui, width, ask, cancel);
        ui.out_raw(format!("\r\x1b[2K{top}\r\n\r\n{bottom}\x1b[1A").as_bytes());
        self.redraw_active_input(ui, width, ask);
    }

    /// Repaint the two status rows without erasing committed output. Cursor is
    /// parked on the input row before and after this operation.
    #[allow(clippy::too_many_arguments)]
    fn refresh_active_frame(
        &mut self,
        ui: &mut Ui,
        plan: bool,
        width: usize,
        ask: &Option<AskState>,
        cancel: &Cancel,
        started: Instant,
        active_tools: &[(String, String)],
    ) {
        let top = self.top_rule(ui, plan, width, started, active_tools);
        let bottom = self.bottom_rule(ui, width, ask, cancel);
        ui.out_raw(
            format!(
                "\r\x1b[1A\r\x1b[2K{top}\x1b[2B\r\x1b[2K{bottom}\x1b[1A\r"
            )
            .as_bytes(),
        );
        self.redraw_active_input(ui, width, ask);
    }

    fn redraw_active_input(&mut self, ui: &mut Ui, width: usize, ask: &Option<AskState>) {
        if let Some(a) = ask {
            let shown = format!("{} [y/N] {}", safe_terminal_text(&a.question), a.answer);
            let ed = Editor::from_line(&shown);
            ui.redraw_input_row(&ed, width);
        } else {
            ui.redraw_input_row(&self.draft, width);
        }
    }

    fn top_rule(
        &self,
        ui: &Ui,
        plan: bool,
        width: usize,
        started: Instant,
        active_tools: &[(String, String)],
    ) -> String {
        let elapsed = elapsed_label(started.elapsed());
        let mut label = format!("Working {elapsed}");
        if plan {
            label.push_str(" · plan");
        }
        if active_tools.len() == 1 {
            label.push_str(" · ");
            label.push_str(&active_tools[0].1);
        } else if active_tools.len() > 1 {
            label.push_str(&format!(" · {} tools", active_tools.len()));
        }

        let open = ui.box_color();
        let reset = if open.is_empty() { "" } else { RESET };
        let tick = (started.elapsed().as_millis() / COMET_MS as u128) as usize;
        let track = scanner::track(tick, ui.depth, &ui.theme.scanner);
        let fixed = 3 + scanner::TRACK + 1 + label.chars().count() + 1;
        if width >= fixed {
            let fill = "─".repeat(width - fixed);
            format!("{open}── {reset}{track}{open} {label} {fill}{reset}")
        } else {
            styled_rule(&label, width, &open)
        }
    }

    fn bottom_rule(
        &self,
        ui: &Ui,
        width: usize,
        ask: &Option<AskState>,
        cancel: &Cancel,
    ) -> String {
        let (label, open) = if cancel.committed {
            ("canceling".to_string(), ui.theme.error.sgr(ui.depth))
        } else if cancel.is_armed() {
            ("press ESC again to cancel".to_string(), ui.theme.error.sgr(ui.depth))
        } else if self.reader_gone {
            ("input closed".to_string(), ui.theme.error.sgr(ui.depth))
        } else if ask.is_some() {
            ("Enter confirms · Ctrl-C cancels all".to_string(), ui.box_color())
        } else if self.queue.is_empty() {
            ("Esc Esc to cancel".to_string(), ui.box_color())
        } else {
            (
                format!("{} queued · Esc Esc to cancel", self.queue.len()),
                ui.box_color(),
            )
        };
        styled_rule(&label, width, &open)
    }

    fn queued_record(&self, ui: &Ui, message: &str) -> String {
        let shown: String = message
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let prompt = ui.box_color();
        let reset = if prompt.is_empty() { "" } else { RESET };
        let note = if ui.depth == ColorDepth::None || !ui.color {
            String::new()
        } else {
            ui.theme.note.sgr(ui.depth)
        };
        let note_reset = if note.is_empty() { "" } else { RESET };
        format!("{prompt}› {reset}{shown} {note}[queued]{note_reset}\r\n")
    }
}

fn frame_label(input: &str) -> String {
    let mut shown = String::new();
    let mut chars = input.chars();
    for ch in chars.by_ref().take(80) {
        shown.push(if ch.is_control() { ' ' } else { ch });
    }
    if chars.next().is_some() {
        shown.push('…');
    }
    shown
}

/// Start a confirmation only while keyboard input can still arrive. A reader
/// may disappear before a later tool reaches its write gate; denying that ask
/// immediately keeps the worker from waiting forever on an impossible reply.
fn route_ask(
    reader_gone: bool,
    ask: &mut Option<AskState>,
    question: String,
    reply: SyncSender<bool>,
) {
    if reader_gone {
        let _ = reply.send(false);
    } else {
        *ask = Some(AskState {
            question,
            answer: String::new(),
            reply,
        });
    }
}

fn elapsed_label(elapsed: Duration) -> String {
    let millis = elapsed.as_millis();
    if millis < 60_000 {
        format!("{}.{:01}s", millis / 1_000, (millis % 1_000) / 100)
    } else {
        format!("{}m{:02}s", millis / 60_000, (millis / 1_000) % 60)
    }
}

fn styled_rule(label: &str, width: usize, open: &str) -> String {
    let reset = if open.is_empty() { "" } else { RESET };
    let max_label = width.saturating_sub(4);
    let mut shown: String = label.chars().take(max_label).collect();
    if label.chars().count() > max_label && max_label > 0 {
        shown.pop();
        shown.push('…');
    }
    let used = (3 + shown.chars().count() + 1).min(width);
    let fill = "─".repeat(width.saturating_sub(used));
    format!("{open}── {shown} {fill}{reset}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fed(bytes: &[u8], width: usize) -> OutTracker {
        let mut t = OutTracker::default();
        t.feed(bytes, width);
        t
    }

    #[test]
    fn confirmation_after_input_closes_is_denied_without_waiting() {
        let (tx, rx) = sync_channel(1);
        let mut ask = None;
        route_ask(true, &mut ask, "write?".to_string(), tx);
        assert!(ask.is_none());
        assert_eq!(rx.recv_timeout(Duration::from_millis(50)), Ok(false));
    }

    #[test]
    fn confirmation_with_live_input_enters_the_modal() {
        let (tx, _rx) = sync_channel(1);
        let mut ask = None;
        route_ask(false, &mut ask, "write?".to_string(), tx);
        let ask = ask.expect("live input must retain the confirmation");
        assert_eq!(ask.question, "write?");
    }

    #[test]
    fn expired_escape_arm_cannot_turn_a_late_tap_into_cancellation() {
        let mut cancel = Cancel {
            armed_until: Some(Instant::now() - Duration::from_millis(1)),
            committed: false,
        };
        assert!(cancel.expire(Instant::now()));
        assert!(!cancel.disarm(), "the late tap must become a new first tap");
        assert!(!cancel.committed);
    }

    #[test]
    fn second_ctrl_c_chooses_the_hard_exit_path() {
        let mut cancel = Cancel::default();
        assert_eq!(cancel.interrupt(), InterruptAction::Cancel);
        assert!(cancel.committed);
        assert_eq!(cancel.interrupt(), InterruptAction::HardExit);
    }

    #[test]
    fn active_tool_labels_cannot_inject_terminal_controls() {
        let mut active = Vec::new();
        DockSession::observe_render(
            &TurnEvent::ToolStart {
                id: "call".into(),
                name: "bad\x1b]52;c;secret\x07\nname".into(),
                brief: String::new(),
                read_only: false,
            },
            &mut active,
        );
        assert_eq!(active.len(), 1);
        assert!(!active[0].1.chars().any(char::is_control));
        assert!(active[0].1.contains("bad ]52;c;secret  name"));
    }

    #[test]
    fn plain_text_advances_and_wraps_exactly_at_the_width() {
        let t = fed(b"hello", 80);
        assert_eq!((t.col, t.fresh), (5, false));
        // Nine chars at width 10: one short of the wrap.
        let t = fed(b"123456789", 10);
        assert_eq!((t.col, t.fresh), (9, false));
        // Exactly filling the row opens a fresh one (deferred wrap).
        let t = fed(b"1234567890", 10);
        assert_eq!((t.col, t.fresh), (0, true));
        // One past: the wrap happened, one cell on the new row.
        let t = fed(b"12345678901", 10);
        assert_eq!((t.col, t.fresh), (1, false));
        // A long stream lands where total % width says.
        let t = fed(&[b'x'; 205], 10);
        assert_eq!((t.col, t.fresh), (5, false));
    }

    #[test]
    fn newline_is_fresh_and_carriage_return_is_not() {
        let t = fed(b"abc\n", 80);
        assert_eq!((t.col, t.fresh), (0, true), "\\n opens a fresh row");
        let t = fed(b"abc\r", 80);
        assert_eq!((t.col, t.fresh), (0, false), "\\r returns onto used content");
        // \r on a fresh row keeps it fresh (nothing was printed there).
        let t = fed(b"abc\n\r", 80);
        assert!(t.fresh);
    }

    #[test]
    fn exact_fill_is_fresh_but_not_a_newline() {
        // The deferred-wrap trap: filling the last column reports col 0 /
        // fresh (the next glyph wraps), but the cursor has NOT advanced a row,
        // so `newline` must stay false. A real newline sets both. The render
        // loop keys its trailing row-advance on `newline`, so this distinction
        // is what stops the dock redraw from erasing a width-boundary line.
        let t = fed(b"1234567890", 10); // exactly fills the row
        assert_eq!((t.col, t.fresh, t.newline), (0, true, false));
        let t = fed(b"line\n", 80);
        assert_eq!((t.col, t.fresh, t.newline), (0, true, true));
        // A newline followed by more text is mid-line, neither fresh nor newline.
        let t = fed(b"line\nmore", 80);
        assert_eq!((t.col, t.fresh, t.newline), (4, false, false));
        // Carriage return, tab, and backspace are never a fresh newline.
        assert!(!fed(b"ab\r", 80).newline);
        assert!(!fed(b"ab\t", 80).newline);
        assert!(!fed(b"ab\x08", 80).newline);
    }

    #[test]
    fn sgr_and_csi_sequences_are_zero_width() {
        // A real themed line: opener + text + reset.
        let t = fed(b"\x1b[38;2;104;211;145mhello\x1b[0m", 80);
        assert_eq!((t.col, t.fresh), (5, false));
        // Cursor-movement CSIs in relayed bytes are not counted either
        // (the dock never emits them into the stream, but a model could).
        let t = fed(b"\x1b[2Kabc", 80);
        assert_eq!(t.col, 3);
    }

    #[test]
    fn escape_split_across_feeds_stays_zero_width() {
        let mut t = OutTracker::default();
        t.feed(b"ab\x1b[38;2;1", 80);
        t.feed(b"0;10mcd", 80);
        assert_eq!((t.col, t.fresh), (4, false));
    }

    #[test]
    fn osc_titles_are_zero_width_with_both_terminators() {
        let t = fed(b"\x1b]0;title\x07ab", 80);
        assert_eq!(t.col, 2);
        let t = fed(b"\x1b]0;title\x1b\\ab", 80);
        assert_eq!(t.col, 2);
    }

    #[test]
    fn multibyte_chars_count_one_column() {
        let t = fed("café".as_bytes(), 80);
        assert_eq!(t.col, 4);
        let t = fed("中文".as_bytes(), 80);
        assert_eq!(t.col, 2, "single-width simplification, by design");
    }

    #[test]
    fn tab_backspace_and_controls() {
        let t = fed(b"ab\t", 80);
        assert_eq!(t.col, 8, "tab advances to the next 8-stop");
        let t = fed(b"\t\t", 80);
        assert_eq!(t.col, 16);
        // A tab near the edge clamps at the last column and never wraps.
        let mut t = fed(&[b'x'; 78], 80);
        t.feed(b"\t", 80);
        assert_eq!((t.col, t.fresh), (79, false));
        // Backspace steps back and clamps at 0.
        let t = fed(b"ab\x08\x08\x08", 80);
        assert_eq!(t.col, 0);
        // Other control bytes are ignored.
        let t = fed(b"a\x00\x01b", 80);
        assert_eq!(t.col, 2);
    }

    #[test]
    fn malformed_escapes_recover_instead_of_eating_the_stream() {
        // A CSI that never sends its final byte gives up at the cap and
        // the text after it is counted again.
        let mut junk = Vec::from(&b"\x1b["[..]);
        junk.extend(std::iter::repeat_n(b'0', 200));
        junk.extend_from_slice(b"abc");
        let mut t = OutTracker::default();
        t.feed(&junk, 80);
        assert!(t.col > 0, "tracker wedged in a malformed CSI");
        // Same for an unterminated OSC: past the cap the tracker returns
        // to text (the exact cut inside the junk run is unimportant; what
        // matters is it counts again instead of staying swallowed).
        let mut junk = Vec::from(&b"\x1b]"[..]);
        junk.extend(std::iter::repeat_n(b'x', OSC_CAP + 10));
        junk.extend_from_slice(b"ab");
        let mut t = OutTracker::default();
        t.feed(&junk, 200);
        assert!(t.col >= 2, "tracker wedged in a malformed OSC");
    }

    #[test]
    fn tracker_never_panics_and_col_stays_inside_the_row() {
        // Pseudo-random byte soup at several widths: the invariant is
        // col < width (or the (0, fresh) wrap state), and no panic.
        let mut seed: u64 = 0x9e3779b97f4a7c15;
        let mut bytes = Vec::with_capacity(4096);
        for _ in 0..4096 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            bytes.push((seed & 0xff) as u8);
        }
        for width in [0usize, 1, 2, 10, 80, 500] {
            let mut t = OutTracker::default();
            for chunk in bytes.chunks(7) {
                t.feed(chunk, width);
                assert!(
                    t.col < width.max(1) || t.col == 0,
                    "col {} escaped width {width}",
                    t.col
                );
            }
        }
    }

    #[test]
    fn status_rules_fit_narrow_and_wide_terminals() {
        for width in [20, 40, 120] {
            let rule = styled_rule("Working 12.3s · several tools", width, "");
            assert_eq!(rule.chars().count(), width, "bad width {width}: {rule:?}");
        }
    }

    #[test]
    fn elapsed_status_switches_to_minutes_without_jittery_milliseconds() {
        assert_eq!(elapsed_label(Duration::from_millis(123)), "0.1s");
        assert_eq!(elapsed_label(Duration::from_millis(12_345)), "12.3s");
        assert_eq!(elapsed_label(Duration::from_secs(125)), "2m05s");
    }

    #[test]
    fn cancel_is_two_tap_but_ctrl_c_can_commit_directly() {
        INTERRUPTED.store(false, Ordering::SeqCst);
        let mut cancel = Cancel {
            armed_until: Some(Instant::now() + CANCEL_WINDOW),
            ..Cancel::default()
        };
        assert!(cancel.disarm(), "first ESC must arm without canceling");
        assert!(!INTERRUPTED.load(Ordering::SeqCst));
        cancel.commit();
        assert!(INTERRUPTED.swap(false, Ordering::SeqCst));
    }
}
