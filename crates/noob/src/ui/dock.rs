//! The dock: a persistent input frame that stays live while a turn streams,
//! output scrolling above it into native scrollback. One thread (the render
//! loop, on main) is the only terminal writer; the turn worker renders
//! through its own `Ui` whose sinks ship every styled byte here, and a
//! reader thread ships decoded keys, all over one channel (fable.md).
//!
//! Nothing in this module knows the agent: the worker is a closure the
//! caller hands in, and the turn's outcome travels back opaquely. Opt-in
//! via `NOOB_DOCK=1` while the driver is being proven; without it the REPL
//! keeps the exact per-prompt raw editor.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::time::{Duration, Instant};

use noob_provider::http::INTERRUPTED;

use super::prompt::{Decoder, Editor, Input, Key, RawGuard, Step, term_width};
use super::style::{ColorDepth, DIM, RESET};
use super::{Ui, scanner};

/// One event on the dock channel. Producers: the turn worker's `Ui` sinks
/// (`Out`, `Ask`), the run wrapper (`End`), and the stdin reader (`Key`).
pub(crate) enum Ev {
    /// Styled bytes from the turn `Ui`, relayed to the terminal verbatim.
    Out(Vec<u8>),
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

/// A `Write` sink that ships bytes to the render loop. A send error means
/// the render loop is gone (the turn is being torn down); the write is
/// swallowed so a worker mid-stream can never panic the process over it.
pub(crate) struct ChannelWriter(pub(crate) SyncSender<Ev>);

impl std::io::Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if !buf.is_empty() {
            let _ = self.0.send(Ev::Out(buf.to_vec()));
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `NOOB_DOCK=1|true|on|yes` opts the interactive REPL into the dock driver.
/// Default off until the driver has survived its real-REPL shakedown; the
/// flag then flips to an opt-out.
pub(super) fn enabled_by_env() -> bool {
    match std::env::var("NOOB_DOCK") {
        Ok(v) => matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes"),
        Err(_) => false,
    }
}

/// How long the reader waits for the tail of an escape sequence before a
/// dangling lone ESC is flushed as the ESC key.
const ESC_GRACE_MS: i32 = 50;
/// Comet cadence while a request is in flight and nothing has streamed yet.
const COMET_MS: u64 = 120;
/// How long a first ESC arms the cancel: a second ESC inside this window
/// cancels the turn, and the window lapsing reverts the dock to normal.
const CANCEL_WINDOW: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// The output column tracker
// ---------------------------------------------------------------------------

/// Escape-sequence scanner state, kept across feeds because a sequence can
/// straddle two `Ev::Out` chunks (a model is free to emit a split escape).
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
            if ready == 0 {
                if let Some(key) = dec.flush_dangling_esc() {
                    if tx.send(Ev::Key(key)).is_err() {
                        return;
                    }
                }
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
        // A message queued during the previous turn dispatches first, one per
        // prompt. Echo it as an ordinary `› message` record so history reads
        // the same as a typed submit, then hand it straight to the agent.
        if let Some(Input::Line(msg)) = self.next_queued() {
            let ed = Editor::from_line(&msg);
            ui.collapse_to_message(&ed, false);
            return Input::Line(msg);
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
                ui.erase(expanded);
                return Input::Eof;
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
            Ev::ReaderGone => true,
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

    /// Run one turn with the dock live: the worker runs `f` with a
    /// channel-sinked `Ui` on a scoped thread while this thread renders its
    /// output above the frame, keeps the draft editable, answers asks, and
    /// tears the frame down when the turn ends. Generic over the outcome so
    /// this module never learns the agent's types.
    pub fn run_turn<R: Send>(
        &mut self,
        ui: &mut Ui,
        plan: bool,
        f: impl FnOnce(&mut Ui) -> R + Send,
    ) -> R {
        let mut turn_ui = ui.for_turn(self.tx.clone());
        let slot: Mutex<Option<R>> = Mutex::new(None);
        std::thread::scope(|s| {
            let tx = self.tx.clone();
            let slot = &slot;
            s.spawn(move || {
                let end = f(&mut turn_ui);
                *slot.lock().unwrap() = Some(end);
                let _ = tx.send(Ev::End);
            });
            self.render_loop(ui, plan);
        });
        slot.into_inner()
            .unwrap()
            .expect("the turn worker ended without parking its result")
    }

    /// The render loop: sole terminal writer while a turn runs. Blocks on
    /// the channel (a timed wake exists only while the comet is sweeping),
    /// coalesces output bursts into one repaint, and keeps the cursor
    /// parked on the dock's input row between flushes.
    fn render_loop(&mut self, ui: &mut Ui, plan: bool) {
        let mut tracker = OutTracker::default();
        let mut width = term_width();
        let mut expanded = !self.draft.is_empty();
        let mut ask: Option<AskState> = None;
        let mut cancel = Cancel::default();
        let mut awaiting_first = true;
        let mut comet = 0usize;
        let row = self.row(ui, awaiting_first, expanded, &ask, &cancel, comet);
        self.draw_dock(ui, plan, expanded, width, &ask, row);

        loop {
            // The soonest timed wake: the comet frame while awaiting the first
            // byte, and the cancel-arm window expiry. Neither runs otherwise,
            // so an idle turn still blocks with no polling.
            let comet_active =
                awaiting_first && !expanded && ask.is_none() && !cancel.is_armed();
            let mut wait: Option<Duration> =
                comet_active.then(|| Duration::from_millis(COMET_MS));
            if let Some(deadline) = cancel.armed_until {
                let rem = deadline.saturating_duration_since(Instant::now());
                wait = Some(wait.map_or(rem, |w| w.min(rem)));
            }
            let first = match wait {
                Some(w) => match self.rx.recv_timeout(w) {
                    Ok(ev) => ev,
                    Err(RecvTimeoutError::Timeout) => {
                        // The cancel window lapsed: revert the dock to normal.
                        if cancel.armed_until.is_some_and(|d| Instant::now() >= d) {
                            cancel.armed_until = None;
                        }
                        if comet_active {
                            comet += 1;
                        }
                        let row = self.row(ui, awaiting_first, expanded, &ask, &cancel, comet);
                        self.redraw_input(ui, width, &ask, row);
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                },
                None => match self.rx.recv() {
                    Ok(ev) => ev,
                    Err(_) => return,
                },
            };

            // Drain the burst: batch output bytes, queue keys, catch the
            // rest. Order across kinds is irrelevant; within Out it is
            // preserved by the concatenation.
            let mut batch: Vec<u8> = Vec::new();
            let mut keys: VecDeque<Key> = VecDeque::new();
            let mut ended = false;
            let mut dirty = false;
            let mut reader_gone = false;
            let mut ev = Some(first);
            loop {
                match ev.take() {
                    Some(Ev::Out(b)) => batch.extend_from_slice(&b),
                    Some(Ev::Key(k)) => keys.push_back(k),
                    Some(Ev::Ask(question, reply)) => {
                        ask = Some(AskState { question, answer: String::new(), reply });
                        // The modal must paint even when nothing else is
                        // happening: the worker is now blocked on the answer
                        // and a human cannot answer an invisible question.
                        dirty = true;
                    }
                    Some(Ev::ReaderGone) => reader_gone = true,
                    Some(Ev::End) => ended = true,
                    None => {}
                }
                match self.rx.try_recv() {
                    Ok(e) => ev = Some(e),
                    Err(_) => break,
                }
            }

            // The reader has exited (real EOF/error) mid-turn. If a y/N modal
            // is open the worker is blocked on its reply and no keystroke can
            // ever arrive to answer it; deny so the worker unblocks, finishes,
            // and sends `End`. Without this the render loop and the worker
            // both wait forever and the scope never joins.
            if reader_gone {
                if let Some(a) = &ask {
                    let _ = a.reply.send(false);
                }
                ask = None;
            }

            // Flush the output batch above the dock: erase the frame, put
            // the cursor back on the stream's insertion point, relay the
            // bytes (they scroll into native scrollback), open a fresh row
            // if the stream stopped mid-line, redraw the frame below.
            if !batch.is_empty() {
                awaiting_first = false;
                width = term_width();
                ui.erase(expanded);
                if !tracker.fresh {
                    ui.out_raw(format!("\x1b[1A\x1b[{}G", tracker.col + 1).as_bytes());
                }
                ui.out_raw(&batch);
                tracker.feed(&batch, width);
                // Advance to a fresh row before the dock unless the batch ended
                // with a real newline. The exact-fill case reports fresh (col 0)
                // but has NOT advanced a row (deferred-wrap latch), so it needs
                // the `\r\n` too, or the dock redraw would erase the filled line.
                if !tracker.newline {
                    ui.out_raw(b"\r\n");
                }
                let row = self.row(ui, awaiting_first, expanded, &ask, &cancel, comet);
                self.draw_dock(ui, plan, expanded, width, &ask, row);
            }

            // Keys: the ask modal consumes them while open; otherwise ESC drives
            // the double-tap cancel and any other key edits the draft (Enter is
            // inert during a turn until the queue milestone).
            while let Some(key) = keys.pop_front() {
                if let Some(a) = &mut ask {
                    match key {
                        Key::Enter => {
                            let yes = matches!(a.answer.trim(), "y" | "Y" | "yes");
                            let _ = a.reply.send(yes);
                            ask = None;
                        }
                        // Ctrl-C and Ctrl-D at a confirmation both deny: the
                        // contract is that anything but an explicit yes is No.
                        Key::Interrupt | Key::Eof => {
                            let _ = a.reply.send(false);
                            ask = None;
                        }
                        Key::Char(c) => a.answer.push(c),
                        Key::Backspace => {
                            a.answer.pop();
                        }
                        _ => {}
                    }
                    dirty = true;
                    continue;
                }
                match key {
                    // Ctrl-C mid-turn cancels immediately (no arming ceremony):
                    // the shared flag every existing checkpoint polls, so the
                    // agent's own `[interrupted]` note is the feedback.
                    Key::Interrupt => {
                        cancel.commit();
                        dirty = true;
                    }
                    Key::Esc => {
                        if cancel.committed {
                            // already canceling; further ESC is a no-op
                        } else if cancel.disarm() {
                            // second ESC inside the window: commit the cancel.
                            cancel.commit();
                        } else {
                            // first ESC: arm the window (repaints the red hint).
                            cancel.armed_until = Some(Instant::now() + CANCEL_WINDOW);
                        }
                        dirty = true;
                    }
                    // Enter queues the draft for the next turn: it is not a
                    // second ESC (so it disarms), and while canceling it is
                    // inert. An empty draft queues nothing.
                    Key::Enter => {
                        cancel.disarm();
                        if !cancel.committed && !self.draft.is_empty() {
                            self.queue.push_back(self.draft.line());
                            self.draft = Editor::default();
                        }
                        dirty = true;
                    }
                    // Ctrl-D is inert mid-turn; a stray one still disarms.
                    Key::Eof => {
                        if cancel.disarm() {
                            dirty = true;
                        }
                    }
                    other => {
                        // Any edit key means the user did not double-tap ESC.
                        cancel.disarm();
                        let _ = self.draft.apply(other);
                        if !expanded {
                            // First draft key mid-turn: grow marker to frame.
                            ui.erase(false);
                            expanded = true;
                            width = term_width();
                            let row = self.row(ui, awaiting_first, expanded, &ask, &cancel, comet);
                            self.draw_dock(ui, plan, expanded, width, &ask, row);
                        }
                        dirty = true;
                    }
                }
            }
            if dirty {
                let row = self.row(ui, awaiting_first, expanded, &ask, &cancel, comet);
                self.redraw_input(ui, width, &ask, row);
            }

            if ended {
                // The worker is done (End is its last event). Remove the
                // frame; the stream already ended its own line (`end_line`
                // flowed through as bytes), so the caller resumes on a
                // clean row. The draft stays for the next prompt.
                ui.erase(expanded);
                return;
            }
        }
    }

    /// Draw the whole dock at the cursor (assumed: column 0 of a fresh
    /// row): the frame when expanded, then the input row content, parking
    /// the cursor there.
    fn draw_dock(
        &mut self,
        ui: &mut Ui,
        plan: bool,
        expanded: bool,
        width: usize,
        ask: &Option<AskState>,
        row: Row,
    ) {
        if expanded {
            ui.expand(plan, width);
        }
        self.redraw_input(ui, width, ask, row);
    }

    /// Redraw only the input row in place. A confirmation always wins the row;
    /// otherwise the cancel status, the comet, or the draft, by priority.
    fn redraw_input(&mut self, ui: &mut Ui, width: usize, ask: &Option<AskState>, row: Row) {
        if let Some(a) = ask {
            let shown = format!("{} [y/N] {}", a.question, a.answer);
            let ed = Editor::from_line(&shown);
            ui.redraw_input_row(&ed, width);
            return;
        }
        match row {
            Row::Comet(frame) => ui.out_raw(format!("\r\x1b[2K{frame}").as_bytes()),
            Row::Armed => self.status_row(ui, ui.theme.error.sgr(ui.depth), "press ESC again to cancel"),
            Row::Canceling => self.status_row(ui, ui.theme.error.sgr(ui.depth), "canceling…"),
            Row::Queued(n) => {
                let dim = if ui.depth == ColorDepth::None { String::new() } else { DIM.to_string() };
                self.status_row(ui, dim, &format!("› {n} queued"));
            }
            Row::Draft => ui.redraw_input_row(&self.draft, width),
        }
    }

    /// What the input row should show this frame, by priority: the committed
    /// cancel status, the armed hint, the request-to-first-byte comet, else
    /// the editable draft.
    fn row(
        &self,
        ui: &Ui,
        awaiting_first: bool,
        expanded: bool,
        ask: &Option<AskState>,
        cancel: &Cancel,
        tick: usize,
    ) -> Row {
        if cancel.committed {
            Row::Canceling
        } else if cancel.is_armed() {
            Row::Armed
        } else if awaiting_first && !expanded && ask.is_none() {
            // The dock row is the liveness indicator (the scanner thread is
            // retired in dock mode); once a draft expands the frame it takes
            // the row instead.
            Row::Comet(scanner::frame(tick, ui.depth, &ui.theme.scanner))
        } else if self.draft.is_empty() && !self.queue.is_empty() {
            // Nothing half-typed but messages are waiting: show the count so
            // the human sees their type-ahead landed.
            Row::Queued(self.queue.len())
        } else {
            Row::Draft
        }
    }

    /// Paint the input row as a one-line status with the given SGR opener (an
    /// empty opener stays plain, for a no-color terminal): the red ESC-cancel
    /// hint and canceling notice, and the dim queued-count.
    fn status_row(&mut self, ui: &mut Ui, open: String, msg: &str) {
        let reset = if open.is_empty() { "" } else { RESET };
        ui.out_raw(format!("\r\x1b[2K{open}{msg}{reset}").as_bytes());
    }
}

/// The input-row content for one repaint (the ask modal is handled ahead of
/// this, so it is not a variant here).
enum Row {
    /// The request-to-first-byte comet animation frame.
    Comet(String),
    /// A first ESC is armed: the red "press ESC again to cancel" hint.
    Armed,
    /// The cancel is committed: the red "canceling…" notice.
    Canceling,
    /// Messages are queued and nothing is half-typed: the dim count.
    Queued(usize),
    /// The editable draft (the default).
    Draft,
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
        junk.extend(std::iter::repeat(b'0').take(200));
        junk.extend_from_slice(b"abc");
        let mut t = OutTracker::default();
        t.feed(&junk, 80);
        assert!(t.col > 0, "tracker wedged in a malformed CSI");
        // Same for an unterminated OSC: past the cap the tracker returns
        // to text (the exact cut inside the junk run is unimportant; what
        // matters is it counts again instead of staying swallowed).
        let mut junk = Vec::from(&b"\x1b]"[..]);
        junk.extend(std::iter::repeat(b'x').take(OSC_CAP + 10));
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
    fn channel_writer_relays_bytes_and_survives_a_dropped_receiver() {
        use std::io::Write;
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let mut w = ChannelWriter(tx);
        w.write_all(b"hello").unwrap();
        w.write_all(b"").unwrap(); // empty writes send nothing
        match rx.try_recv() {
            Ok(Ev::Out(b)) => assert_eq!(b, b"hello"),
            _ => panic!("expected one Out event"),
        }
        assert!(rx.try_recv().is_err(), "empty write must not send");
        drop(rx);
        // The render loop is gone: writes are swallowed, never a panic.
        w.write_all(b"late").unwrap();
    }
}
