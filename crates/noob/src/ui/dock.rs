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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::time::{Duration, Instant};

use noob_provider::http::INTERRUPTED;

use super::prompt::{Decoder, Editor, Input, Key, RawGuard, Step, term_height, term_width};
use super::style::RESET;
use super::{
    BufferedTurnRenderer, RegionTone, TurnEvent, Ui, elapsed_label, safe_terminal_text, scanner,
};

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
    /// The terminal was resized (SIGWINCH). Injected by the reader thread when
    /// its blocked read is interrupted by the signal, so an idle prompt (which
    /// otherwise blocks forever on input) reflows its box to the new width
    /// without waiting for a keystroke. During a turn the render loop already
    /// re-reads the width each tick; this just makes it instant.
    Resize,
}

/// Set by the SIGWINCH handler (installed in `main`), consumed by the reader
/// thread when its read returns EINTR. The signal is blocked in every thread
/// except the reader, so it always interrupts the read and never races another
/// blocking call. Async-signal-safe: the handler only stores this flag.
pub(crate) static WINCH: AtomicBool = AtomicBool::new(false);

/// `NOOB_DOCK=0|false|off|no` opts out of the dock. Unset and every other
/// value leave the default interactive driver enabled.
pub(super) fn enabled_by_env() -> bool {
    match std::env::var("NOOB_DOCK") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        ),
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
/// Hard ceiling on pinned region rows regardless of screen size, so a huge plan
/// never turns the dock into a scroll of its own. The effective cap is the
/// smaller of this and what the terminal height leaves once the three frame
/// chrome rows and a line of output are reserved (see `region_rows`).
const MAX_REGION_ROWS: usize = 24;
/// The plan checklist's own cap, applied before the screen cap: the header
/// plus at most this many step rows, then one "… +N more" summary row. Even
/// on a tall terminal a 20-step plan must not become the whole dock; the
/// window keeps the active step and prefers what comes next over what is
/// already done.
const PLAN_STEP_ROWS: usize = 6;
/// Rows the frame reserves outside its pinned regions: the top status, the input
/// row, the bottom status, and one line of scrolled output kept visible. The
/// region cap is `term_height - this`, so the live frame never exceeds the
/// screen (where relative cursor moves would clamp at the top and desync).
const FRAME_RESERVE_ROWS: usize = 4;

/// The dock's answer to a resize: a viewport reset. A reflowing terminal
/// (VTE, kitty, alacritty, ...) rewraps every logical line longer than the
/// new width into several physical rows the moment the window changes, and
/// no cursor-relative erase can reliably find the old frame afterwards
/// (v0.3.7 walked wrap-aware geometry and real VTE still shredded rows).
/// Clearing the screen and homing the cursor is reflow-agnostic by
/// construction: the frame repaints from the top-left, and VTE pushes the
/// cleared screen into scrollback so the transcript stays in history.
const VIEWPORT_RESET: &[u8] = b"\x1b[2J\x1b[H";

/// Re-read the width until it stops moving: an interactive resize drag
/// delivers a burst of SIGWINCH, and repainting at every intermediate width
/// multiplies the chances of racing the terminal's own reflow. Bounded, so a
/// pathological drag cannot wedge the loop.
fn settled_width() -> usize {
    let mut width = term_width();
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(30));
        let now = term_width();
        if now == width {
            break;
        }
        width = now;
    }
    width
}

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
        OutTracker {
            col: 0,
            fresh: true,
            newline: true,
            esc: EscScan::Normal,
        }
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
    // SIGWINCH is blocked in every other thread (see `install_sigwinch_handler`),
    // so unblock it here: this thread's blocking read is the one that must catch
    // the resize as EINTR and turn it into a `Resize` event.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGWINCH);
        libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
    }
    let mut dec = Decoder::default();
    let mut buf = [0u8; 1024];
    loop {
        let n = unsafe {
            libc::read(
                libc::STDIN_FILENO,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
            )
        };
        if n < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                // A terminal resize: reflow the prompt to the new width even
                // while idle (the read that just unblocked was the only thing
                // waiting). Checked first because a resize is the common EINTR.
                if WINCH.swap(false, Ordering::SeqCst) && tx.send(Ev::Resize).is_err() {
                    return;
                }
                // An external SIGINT (kill -INT; the keyboard's Ctrl-C is a
                // byte in raw mode). The handler set the flag; surface it as
                // the interrupt key so the loop reacts without a keystroke.
                if INTERRUPTED.load(Ordering::SeqCst) && tx.send(Ev::Key(Key::Interrupt)).is_err() {
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
            let mut pfd = libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: libc::POLLIN,
                revents: 0,
            };
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

    /// Commit the cancel and drop the arm. The shipped binary sets the
    /// process-wide flag here; unit tests keep the state local so one parallel
    /// test cannot cancel unrelated tools. PTY tests exercise the real flag.
    fn commit(&mut self) {
        #[cfg(not(test))]
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

/// Double-ESC is the human's stop-everything gesture: committing it also
/// cancels every detached child (each still delivers its one terminal
/// packet). Steering and Ctrl-C keep their narrower contract - they stop
/// only the parent turn and the fleet keeps running.
fn stop_fleet(background: Option<&crate::subagent::BackgroundHub>) {
    if let Some(hub) = background {
        hub.cancel_all();
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
    /// Messages submitted with Enter while a turn ran. Submission queues the
    /// message without touching the running turn; the next prompt dispatches
    /// one message at once, oldest first. Explicit Esc/Ctrl-C cancellation
    /// drains them back into the draft instead of firing them.
    queue: VecDeque<String>,
    /// Tab toggles this persistent view. It survives turn boundaries so a
    /// background child remains inspectable both while the parent runs and at
    /// the otherwise-idle prompt.
    agent_view: bool,
    /// Idle double-ESC arm: a first ESC at the idle prompt while detached
    /// agents run arms this window; a second ESC inside it stops the fleet.
    idle_esc_armed_until: Option<Instant>,
    /// The latest plan checklist, kept across turns so the plan stays pinned
    /// above the input at all times (during turns and at the idle prompt),
    /// updating in place instead of stacking a copy into the transcript at
    /// every turn end. A plan whose every step completes is retired at turn
    /// end: recorded once into the transcript and dropped from here. Cleared
    /// by /clear-plan.
    plan_block: Option<String>,
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
            agent_view: false,
            idle_esc_armed_until: None,
            plan_block: None,
            reader_gone: false,
            _guard: guard,
        })
    }

    /// Dispatch the next queued message, if any, as this prompt's input,
    /// without re-reading stdin: one per prompt = one per turn. The caller
    /// echoes it into the transcript at dispatch; while it waited it was
    /// only a pinned `[queued]` region row, which died with the turn frame.
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

    /// Drop the pinned plan (the /clear-plan command): the next prompt and
    /// turn draw without a plan region until the model posts a new checklist.
    pub fn clear_plan(&mut self) {
        self.plan_block = None;
    }

    /// Read one line at the idle prompt, event-driven with the semantics of the
    /// per-prompt raw editor: a persistent framed box (a plain rule above and
    /// below the editable line, a dim hint when empty) that stays visible the
    /// whole time so the input never collapses to a lone marker between turns,
    /// submit collapses it to a `› message` record, Ctrl-C cancels the line,
    /// Ctrl-D on an empty line is EOF. A draft typed during the previous turn is
    /// already visible and editable. Off the token path entirely: this runs only
    /// while no turn is in flight.
    pub fn read_prompt(
        &mut self,
        ui: &mut Ui,
        plan: bool,
        background: Option<&crate::subagent::BackgroundHub>,
    ) -> Input {
        // A queued message becomes this prompt's input. Its pinned [queued]
        // row disappeared with the turn frame, so echo the plain `› message`
        // record here: the transcript then reads exactly like a typed
        // submission, with no stale [queued] marker anywhere.
        if let Some(input) = self.next_queued() {
            if let Input::Line(line) = &input {
                ui.queued_dispatch_record(line);
            }
            return input;
        }
        if self.reader_gone && self.pending.is_empty() {
            return Input::Eof;
        }
        // The idle input is a persistent box. When the agents view is open its
        // live rows are pinned immediately above that box; the editor remains
        // on the same single input row and stays fully usable.
        let mut width = term_width();
        let mut agent_rows = self.idle_region_rows(ui, background, width);
        self.draw_idle_prompt(ui, plan, width, &agent_rows);
        let mut input_dirty = true;
        loop {
            // Keyboard input wins a race with a completing child. Drain what
            // the reader has already accepted before considering automatic
            // continuation, and never steal a prompt that the human is in the
            // middle of composing. Their normal turn integrates ready packets
            // first, so no result is delayed or lost.
            while let Ok(ev) = self.rx.try_recv() {
                self.absorb_idle(ev);
            }
            if !self.reader_gone
                && self.draft.is_empty()
                && self.pending.is_empty()
                && background.is_some_and(crate::subagent::BackgroundHub::settled_ready)
            {
                self.erase_idle_prompt(ui, agent_rows.len());
                return Input::BackgroundReady;
            }

            if term_width() != width {
                // A resize: wait out the SIGWINCH burst, then reset the
                // viewport and repaint the frame from the top-left (see
                // VIEWPORT_RESET; the terminal's reflow makes any in-place
                // erase of the old frame unreliable).
                width = settled_width();
                agent_rows = self.idle_region_rows(ui, background, width);
                ui.begin_batch();
                ui.out_raw(VIEWPORT_RESET);
                self.draw_idle_prompt(ui, plan, width, &agent_rows);
                ui.end_batch();
                input_dirty = true;
            } else {
                let next_agent_rows = self.idle_region_rows(ui, background, width);
                if next_agent_rows != agent_rows {
                    ui.begin_batch();
                    self.erase_idle_prompt(ui, agent_rows.len());
                    agent_rows = next_agent_rows;
                    self.draw_idle_prompt(ui, plan, width, &agent_rows);
                    ui.end_batch();
                    input_dirty = true;
                }
            }

            while let Some(key) = self.pending.pop_front() {
                input_dirty = true;
                if key == Key::Esc {
                    // Double-ESC at the idle prompt stops every detached
                    // agent (the stop-everything gesture works whether or
                    // not a turn is in flight). Each canceled child still
                    // delivers its one terminal packet, which the idle loop
                    // surfaces on its own. With no agents, ESC stays the
                    // editor no-op it always was.
                    let now = Instant::now();
                    let armed = self
                        .idle_esc_armed_until
                        .take()
                        .is_some_and(|until| now < until);
                    let active = background.map(|hub| hub.snapshot().active).unwrap_or(0);
                    if active > 0 {
                        if armed {
                            stop_fleet(background);
                        } else {
                            self.idle_esc_armed_until = Some(now + CANCEL_WINDOW);
                        }
                    }
                    continue;
                }
                self.idle_esc_armed_until = None;
                if key == Key::Tab {
                    if self.draft.is_empty()
                        && let Some(hub) = background
                    {
                        let snapshot = hub.snapshot();
                        if self.agent_view || !snapshot.rows.is_empty() {
                            self.erase_idle_prompt(ui, agent_rows.len());
                            self.agent_view = !self.agent_view;
                            agent_rows = self.idle_region_rows(ui, background, width);
                            self.draw_idle_prompt(ui, plan, width, &agent_rows);
                            continue;
                        }
                    }
                    super::prompt::complete_editor(&mut self.draft);
                    continue;
                }
                match self.draft.apply(key) {
                    Step::Continue => {}
                    Step::Submit => {
                        if agent_rows.is_empty() {
                            return self.submit(ui, true);
                        }
                        self.erase_idle_prompt(ui, agent_rows.len());
                        return self.submit(ui, false);
                    }
                    Step::Interrupt => {
                        self.erase_idle_prompt(ui, agent_rows.len());
                        self.draft = Editor::default();
                        INTERRUPTED.swap(false, Ordering::SeqCst);
                        return Input::Interrupted;
                    }
                    Step::Eof => {
                        self.erase_idle_prompt(ui, agent_rows.len());
                        return Input::Eof;
                    }
                }
            }
            if self.reader_gone {
                self.erase_idle_prompt(ui, agent_rows.len());
                return Input::Eof;
            }
            if input_dirty {
                ui.redraw_idle_input(&self.draft, width);
                input_dirty = false;
            }
            let received = if let Some(hub) = background {
                match self.rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(ev) => Some(ev),
                    Err(RecvTimeoutError::Timeout)
                        if self.draft.is_empty()
                            && self.pending.is_empty()
                            && hub.settled_ready() =>
                    {
                        self.erase_idle_prompt(ui, agent_rows.len());
                        return Input::BackgroundReady;
                    }
                    // Wake the outer loop to refresh elapsed time and recent
                    // child activity even when the keyboard is untouched.
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => {
                        self.reader_gone = true;
                        None
                    }
                }
            } else {
                match self.rx.recv() {
                    Ok(ev) => Some(ev),
                    Err(_) => {
                        self.reader_gone = true;
                        None
                    }
                }
            };
            let mut gone = match received {
                Some(ev) => self.absorb_idle(ev),
                None => self.reader_gone,
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

    fn draw_idle_prompt(&mut self, ui: &mut Ui, plan: bool, width: usize, rows: &[String]) {
        ui.begin_batch();
        if !rows.is_empty() {
            let mut block = String::new();
            for row in rows {
                block.push_str("\r\x1b[2K");
                block.push_str(row);
                block.push_str("\r\n");
            }
            ui.out_raw(block.as_bytes());
        }
        ui.expand(plan, width);
        ui.end_batch();
    }

    /// Clear the optional agent rows and the three-row idle box, leaving the
    /// cursor at the topmost row they occupied. With no agent rows this emits
    /// exactly the ordinary prompt erasure.
    fn erase_idle_prompt(&mut self, ui: &mut Ui, region_count: usize) {
        ui.erase(true);
        if region_count == 0 {
            return;
        }
        let mut block = format!("\x1b[{region_count}A\r\x1b[2K");
        for _ in 1..region_count {
            block.push_str("\x1b[1B\r\x1b[2K");
        }
        if region_count > 1 {
            block.push_str(&format!("\x1b[{}A\r", region_count - 1));
        }
        ui.out_raw(block.as_bytes());
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
        background: Option<&crate::subagent::BackgroundHub>,
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
                let outcome = crate::agent::sched::catch_unwind_silent(|| f(&mut turn_ui));
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
            self.render_loop(ui, plan, background);
        });
        slot.into_inner()
            .unwrap()
            .expect("the turn worker ended without parking its result")
    }

    /// The render loop: sole terminal writer while a turn runs. Receive order
    /// is semantic order. Only adjacent Render events share a repaint; Ask,
    /// keys, reader loss, and End are strict barriers.
    fn render_loop(
        &mut self,
        ui: &mut Ui,
        plan: bool,
        background: Option<&crate::subagent::BackgroundHub>,
    ) {
        let mut tracker = OutTracker::default();
        let mut width = term_width();
        let mut ask: Option<AskState> = None;
        let mut cancel = Cancel::default();
        let mut renderer = ui.buffered_turn_renderer();
        let mut active_tools: Vec<(String, String)> = Vec::new();
        let started = Instant::now();
        let mut deferred: Option<Ev> = None;

        // The live regions pinned between the top status and the input row: the
        // plan checklist and the fan-out panel. Each is held as its raw block
        // text and re-rendered in place as it changes, so the console never
        // stacks a fresh copy of the plan on every update. The plan text lives
        // on the session (it survives turn boundaries and stays pinned at the
        // idle prompt); the fan-out event block is per turn. `region_rows` is
        // the styled, width-clamped view (one physical row each); `drawn_r` is
        // how many region rows are actually on screen, so an erase always
        // clears the exact height the frame currently occupies even as it
        // grows or shrinks.
        let mut agents_block: Option<String> = None;
        let mut region_rows: Vec<String> =
            self.region_rows(ui, self.plan_block.as_deref(), None, background, width);
        let mut drawn_r: usize = region_rows.len();
        let mut background_revision = background
            .map(crate::subagent::BackgroundHub::revision)
            .unwrap_or_default();

        // A running turn always owns a stable frame: a top status row, any live
        // regions, the editable draft, and a queue/cancel row. They are
        // independent, so typing can never hide liveness again. A plan pinned
        // by an earlier turn is part of the frame from the first paint.
        self.draw_active_frame(
            ui,
            plan,
            width,
            &ask,
            &cancel,
            started,
            &active_tools,
            &region_rows,
        );

        loop {
            cancel.expire(Instant::now());
            if term_width() != width {
                // A resize: wait out the SIGWINCH burst, then reset the
                // viewport and repaint the frame from the top-left (see
                // VIEWPORT_RESET). The partial output line moved into
                // scrollback with the rest of the screen, so the tracker
                // restarts on a fresh row and the next batch never hops up
                // onto a row that no longer exists.
                width = settled_width();
                region_rows = self.region_rows(
                    ui,
                    self.plan_block.as_deref(),
                    agents_block.as_deref(),
                    background,
                    width,
                );
                background_revision = background
                    .map(crate::subagent::BackgroundHub::revision)
                    .unwrap_or_default();
                tracker = OutTracker::default();
                ui.begin_batch();
                ui.out_raw(VIEWPORT_RESET);
                self.draw_active_frame(
                    ui,
                    plan,
                    width,
                    &ask,
                    &cancel,
                    started,
                    &active_tools,
                    &region_rows,
                );
                ui.end_batch();
                drawn_r = region_rows.len();
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
                        // The terminal may have resized while this loop was
                        // blocked; a repaint at the old width would walk the
                        // wrong rows of a reflowed screen. Skip the refresh
                        // and let the width check at the top of the loop
                        // reset the viewport first.
                        if term_width() != width {
                            continue;
                        }
                        let refreshed = self.region_rows(
                            ui,
                            self.plan_block.as_deref(),
                            agents_block.as_deref(),
                            background,
                            width,
                        );
                        background_revision = background
                            .map(crate::subagent::BackgroundHub::revision)
                            .unwrap_or_default();
                        if refreshed != region_rows {
                            region_rows = refreshed;
                            self.redraw_regions(
                                ui,
                                plan,
                                width,
                                &ask,
                                &cancel,
                                started,
                                &active_tools,
                                &region_rows,
                                &mut drawn_r,
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
                                &region_rows,
                            );
                        }
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                },
            };

            match first {
                Ev::Render(event) => {
                    let mut regions_dirty = false;
                    let mut background_dirty = false;
                    Self::absorb_render(
                        event,
                        &mut active_tools,
                        &mut renderer,
                        &mut self.plan_block,
                        &mut agents_block,
                        &mut regions_dirty,
                    );

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
                                Self::absorb_render(
                                    event,
                                    &mut active_tools,
                                    &mut renderer,
                                    &mut self.plan_block,
                                    &mut agents_block,
                                    &mut regions_dirty,
                                );
                            }
                            Ok(barrier) => {
                                deferred = Some(barrier);
                                break;
                            }
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    // Recompute the pinned region view before any redraw, so the
                    // frame that write_above or the region redraw paints already
                    // reflects the new plan/panel at the current width.
                    let next_background_revision = background
                        .map(crate::subagent::BackgroundHub::revision)
                        .unwrap_or_default();
                    let background_changed = next_background_revision != background_revision;
                    if background_changed {
                        background_revision = next_background_revision;
                    }
                    if regions_dirty || background_changed {
                        let refreshed = self.region_rows(
                            ui,
                            self.plan_block.as_deref(),
                            agents_block.as_deref(),
                            background,
                            width,
                        );
                        background_dirty = refreshed != region_rows;
                        region_rows = refreshed;
                    }
                    let batch = renderer.take();
                    if !batch.is_empty() {
                        // Output scrolls above; the erase keys on the on-screen
                        // height and the redraw on the new one, so a region that
                        // changed in the same batch resizes the frame cleanly.
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
                            &region_rows,
                            &mut drawn_r,
                        );
                    } else if regions_dirty || background_dirty {
                        self.redraw_regions(
                            ui,
                            plan,
                            width,
                            &ask,
                            &cancel,
                            started,
                            &active_tools,
                            &region_rows,
                            &mut drawn_r,
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
                            &region_rows,
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
                        &region_rows,
                    );
                }
                Ev::Key(key) => {
                    cancel.expire(Instant::now());
                    let mut regions_changed = false;
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
                                    // Fleet first, then the interrupt flag, so
                                    // the worker's interrupt note already sees
                                    // the children as stopping.
                                    stop_fleet(background);
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
                                    // Fleet first, then the interrupt flag, so
                                    // the worker's interrupt note already sees
                                    // the children as stopping.
                                    stop_fleet(background);
                                    cancel.commit();
                                } else {
                                    cancel.armed_until = Some(Instant::now() + CANCEL_WINDOW);
                                }
                            }
                            Key::Enter => {
                                cancel.disarm();
                                if !cancel.committed && !self.draft.is_empty() {
                                    // Queue only: the running turn (and any
                                    // plan or detached agent it drives) is
                                    // never touched. The message waits as a
                                    // dim pinned `[queued]` row and dispatches
                                    // at the next prompt, after the turn ends
                                    // on its own (where it is echoed into the
                                    // transcript). Esc Esc is the only stop.
                                    self.queue.push_back(self.draft.line());
                                    self.draft = Editor::default();
                                    regions_changed = true;
                                }
                            }
                            Key::Eof => {
                                cancel.disarm();
                            }
                            Key::Tab => {
                                cancel.disarm();
                                if !cancel.committed
                                    && self.draft.is_empty()
                                    && let Some(hub) = background
                                {
                                    let snapshot = hub.snapshot();
                                    if self.agent_view || !snapshot.rows.is_empty() {
                                        self.agent_view = !self.agent_view;
                                        regions_changed = true;
                                    }
                                }
                                if !regions_changed && !cancel.committed {
                                    super::prompt::complete_editor(&mut self.draft);
                                }
                            }
                            other => {
                                cancel.disarm();
                                let _ = self.draft.apply(other);
                            }
                        }
                    }

                    if regions_changed {
                        let refreshed = self.region_rows(
                            ui,
                            self.plan_block.as_deref(),
                            agents_block.as_deref(),
                            background,
                            width,
                        );
                        if refreshed != region_rows {
                            region_rows = refreshed;
                            self.redraw_regions(
                                ui,
                                plan,
                                width,
                                &ask,
                                &cancel,
                                started,
                                &active_tools,
                                &region_rows,
                                &mut drawn_r,
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
                                &region_rows,
                            );
                        }
                    } else {
                        self.refresh_active_frame(
                            ui,
                            plan,
                            width,
                            &ask,
                            &cancel,
                            started,
                            &active_tools,
                            &region_rows,
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
                        &region_rows,
                    );
                }
                Ev::End => {
                    // Tear the live frame down, regions and all, so the exact
                    // height that was on screen is cleared (not a fixed three
                    // rows). An in-progress plan and the hub's agents view are
                    // NOT re-recorded into the transcript: the idle prompt
                    // pins both live, so a static copy per turn would only
                    // stack duplicates. Two records are the exception: a plan
                    // whose every step completed retires into one transcript
                    // summary (and unpins, instead of hugging the input
                    // forever), and a hubless fan-out panel leaves its rows.
                    let plan_record = self.retire_completed_plan(ui, width);
                    let final_rows =
                        self.final_region_rows(ui, agents_block.as_deref(), background, width);
                    ui.begin_batch();
                    self.erase_dock(ui, drawn_r);
                    let mut block = String::new();
                    for row in plan_record.iter().chain(&final_rows) {
                        block.push_str("\r\x1b[2K");
                        block.push_str(row);
                        block.push_str("\r\n");
                    }
                    if !block.is_empty() {
                        ui.out_raw(block.as_bytes());
                    }
                    ui.end_batch();
                    return;
                }
                // A resize just needs to wake the loop: the width re-check at the
                // top of the next iteration erases and redraws the frame at the
                // new width. Nothing to do here.
                Ev::Resize => {}
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

    /// Take in one semantic render op. The plan checklist and the fan-out panel
    /// are diverted into pinned live regions instead of the scrolling
    /// transcript, so they update in place rather than stacking a fresh block on
    /// every change. A panel still records its covered task ids through the
    /// renderer, so the redundant `* task` start/done lines stay suppressed even
    /// though the panel block itself is never replayed there. Every other op
    /// renders through the ordinary buffered renderer exactly as before.
    fn absorb_render(
        event: TurnEvent,
        active: &mut Vec<(String, String)>,
        renderer: &mut BufferedTurnRenderer,
        plan_block: &mut Option<String>,
        agents_block: &mut Option<String>,
        dirty: &mut bool,
    ) {
        match event {
            TurnEvent::Todos(text) => {
                *plan_block = Some(text);
                *dirty = true;
            }
            TurnEvent::Agents { block, ids } => {
                renderer.cover_task_ids(&ids);
                *agents_block = Some(block);
                *dirty = true;
            }
            other => {
                Self::observe_render(&other, active);
                renderer.render(other);
            }
        }
    }

    /// The styled, width-clamped rows for the pinned regions: the plan checklist
    /// above the fan-out panel, each block's lines in program order. Empty off
    /// the themed surface, so a no-color or byte-identity dock pins nothing.
    /// Capped so a very long plan or a wide fan-out cannot grow the frame past
    /// the screen; the hidden rows are summarized in one trailing row.
    fn region_rows(
        &self,
        ui: &Ui,
        plan_block: Option<&str>,
        agents_block: Option<&str>,
        background: Option<&crate::subagent::BackgroundHub>,
        width: usize,
    ) -> Vec<String> {
        if !ui.regions_enabled() {
            return Vec::new();
        }
        let mut rows: Vec<PinnedRegionRow> = Vec::new();
        if let Some(text) = plan_block {
            rows.extend(plan_region_rows(ui, text, width));
        }

        // The event block for detached work captures only its admission-time
        // count. Derive open and closed views from the hub after lifecycle
        // revisions and on the display heartbeat, so completion/cancellation
        // cannot leave a stale "running" row without allocating on every
        // streamed token. The event block remains the non-background fallback.
        let snapshot_block = background.and_then(|hub| {
            let snapshot = hub.snapshot();
            if self.agent_view {
                expanded_agent_snapshot_block(&snapshot)
            } else {
                collapsed_agent_snapshot_block(&snapshot)
            }
        });
        let shown_agents = if background.is_some() {
            snapshot_block.as_deref()
        } else {
            agents_block
        };
        if let Some(block) = shown_agents {
            rows.extend(checklist_pinned_rows(
                ui,
                block,
                width,
                RegionSource::Agents,
            ));
        }
        // Messages queued mid-turn wait as dim pinned rows under the agents
        // row, not as transcript records: the [queued] marker vanishes with
        // the row the moment the message dispatches, so an already-answered
        // message can never keep reading as still queued.
        for message in &self.queue {
            rows.push(queued_region_row(ui, message, width));
        }
        let counts = checklist_counts(plan_block.unwrap_or_default())
            .plus(checklist_counts(shown_agents.unwrap_or_default()));
        self.cap_region_rows(ui, rows, counts, width)
    }

    /// The pinned rows for the idle prompt: the persistent plan checklist,
    /// then the live agents view. Same construction as the in-turn regions,
    /// so the frame reads identically between and during turns and nothing is
    /// ever re-recorded into the scrolling transcript.
    fn idle_region_rows(
        &self,
        ui: &Ui,
        background: Option<&crate::subagent::BackgroundHub>,
        width: usize,
    ) -> Vec<String> {
        if !ui.regions_enabled() {
            return Vec::new();
        }
        let mut rows: Vec<PinnedRegionRow> = Vec::new();
        if let Some(text) = self.plan_block.as_deref() {
            rows.extend(plan_region_rows(ui, text, width));
        }
        // Mirror the in-turn region selection: the expanded panel behind Tab,
        // otherwise the one-line running/ready counter. Closing the panel must
        // fall back to the counter, never to nothing, or live background work
        // becomes invisible at the idle prompt.
        let snapshot = background.map(|hub| hub.snapshot());
        let block = snapshot.as_ref().and_then(|snapshot| {
            if self.agent_view {
                expanded_agent_snapshot_block(snapshot)
            } else {
                collapsed_agent_snapshot_block(snapshot)
            }
        });
        if let Some(block) = block.as_deref() {
            rows.extend(checklist_pinned_rows(
                ui,
                block,
                width,
                RegionSource::Agents,
            ));
        }
        if rows.is_empty() {
            return Vec::new();
        }
        let counts = checklist_counts(self.plan_block.as_deref().unwrap_or_default())
            .plus(checklist_counts(block.as_deref().unwrap_or_default()));
        let mut rows = self.cap_region_rows(ui, rows, counts, width);
        // The armed idle double-ESC shows its own confirmation hint, mirroring
        // the in-turn "press ESC again to cancel" row. The 100ms idle tick
        // re-diffs these rows, so lapse of the window removes it on its own.
        if snapshot.as_ref().is_some_and(|s| s.active > 0)
            && self
                .idle_esc_armed_until
                .is_some_and(|until| Instant::now() < until)
        {
            let open = ui.theme.error.sgr(ui.depth);
            let reset = if open.is_empty() { "" } else { RESET };
            // Clamped to the width like every other pinned row: a wrapped
            // hint would occupy two physical rows while the erase counts one.
            let label: String = "press ESC again to stop all agents"
                .chars()
                .take(width.max(1))
                .collect();
            rows.push(format!("{open}{label}{reset}"));
        }
        rows
    }

    /// Retire a plan whose every step is done: one "plan completed" summary
    /// row for the transcript, and the pinned copy is dropped, so the
    /// finished plan lives in history instead of sticking to the input
    /// forever. An unfinished plan returns None and stays pinned across the
    /// turn boundary.
    fn retire_completed_plan(&mut self, ui: &Ui, width: usize) -> Option<String> {
        if !ui.regions_enabled() {
            return None;
        }
        let label = completed_plan_label(self.plan_block.as_deref()?)?;
        self.plan_block = None;
        Some(ui.region_summary_row(&label, width, RegionTone::Activity))
    }

    /// The static rows a finished turn leaves in the transcript. Nothing when
    /// a background hub exists: the idle prompt pins the live plan and agents
    /// view, so a static copy would only duplicate it (the historical
    /// "[N] agents running" line stacked after every turn). The event-block
    /// fallback still records a hubless fan-out panel, which no idle region
    /// would otherwise keep visible.
    fn final_region_rows(
        &self,
        ui: &Ui,
        agents_block: Option<&str>,
        background: Option<&crate::subagent::BackgroundHub>,
        width: usize,
    ) -> Vec<String> {
        if !ui.regions_enabled() || background.is_some() {
            return Vec::new();
        }
        let Some(block) = agents_block else {
            return Vec::new();
        };
        let rows = checklist_pinned_rows(ui, block, width, RegionSource::Agents);
        let counts = checklist_counts(block);
        self.cap_region_rows(ui, rows, counts, width)
    }

    fn cap_region_rows(
        &self,
        ui: &Ui,
        rows: Vec<PinnedRegionRow>,
        counts: ChecklistCounts,
        width: usize,
    ) -> Vec<String> {
        // Bound the frame to the screen: never taller than the terminal leaves
        // once the chrome and a line of output are reserved. On a terminal too
        // short for any region row the cap is 0 and the dock falls back to the
        // plain three-row frame.
        let cap = term_height()
            .saturating_sub(FRAME_RESERVE_ROWS)
            .min(MAX_REGION_ROWS);
        if rows.len() <= cap {
            return rows.into_iter().map(|row| row.rendered).collect();
        }
        if cap == 0 {
            return Vec::new();
        }

        // Preserve the active plan item and the agent header/summary before
        // filling spare rows in source order. A long plan must never push the
        // only agent indicator below the cap, and a panel must never hide the
        // step that is actually running.
        let plan_row = rows
            .iter()
            .position(|row| row.priority == RegionPriority::Plan);
        let agent_row = rows
            .iter()
            .position(|row| row.priority == RegionPriority::Agents);
        let mut mandatory = Vec::new();
        if let Some(index) = plan_row {
            mandatory.push(index);
        }
        if let Some(index) = agent_row {
            mandatory.push(index);
        }
        mandatory.sort_unstable();
        mandatory.dedup();

        // A pathologically short terminal may leave one region row for two
        // independent indicators. Keep both facts in one compact row.
        if cap == 1 && plan_row.is_some() && agent_row.is_some() {
            return vec![ui.region_summary_row(
                "[~] plan active · agents active",
                width,
                RegionTone::Activity,
            )];
        }

        let summary_slot = usize::from(cap > mandatory.len());
        let content_cap = cap.saturating_sub(summary_slot);
        let mut selected = vec![false; rows.len()];
        for &index in mandatory.iter().take(content_cap) {
            selected[index] = true;
        }
        let mut selected_count = selected.iter().filter(|&&keep| keep).count();
        for keep in &mut selected {
            if selected_count == content_cap {
                break;
            }
            if !*keep {
                *keep = true;
                selected_count += 1;
            }
        }

        let hidden = rows.len().saturating_sub(selected_count);
        let mut visible: Vec<String> = rows
            .into_iter()
            .zip(selected)
            .filter_map(|(row, keep)| keep.then_some(row.rendered))
            .collect();
        if hidden > 0 && summary_slot > 0 {
            visible.push(ui.region_summary_row(
                &format!(
                    "… {} completed · {} active · {} pending · {hidden} hidden",
                    counts.done, counts.active, counts.pending,
                ),
                width,
                RegionTone::Dim,
            ));
        }
        visible
    }

    /// Relay bytes above the frame without losing a partial output line, then
    /// redraw the frame below them. The erase keys on the height currently on
    /// screen (`drawn_r`) and the redraw on the new region rows, so a region that
    /// changed in this same batch resizes the frame in one pass. `drawn_r` is
    /// updated to the height just drawn.
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
        region_rows: &[String],
        drawn_r: &mut usize,
    ) {
        ui.begin_batch();
        self.erase_dock(ui, *drawn_r);
        if !tracker.fresh {
            ui.out_raw(format!("\x1b[1A\x1b[{}G", tracker.col + 1).as_bytes());
        }
        ui.out_raw(bytes);
        tracker.feed(bytes, width);
        if !tracker.newline {
            ui.out_raw(b"\r\n");
        }
        self.draw_active_frame(
            ui,
            plan,
            width,
            ask,
            cancel,
            started,
            active_tools,
            region_rows,
        );
        ui.end_batch();
        *drawn_r = region_rows.len();
    }

    /// Resize and repaint the frame in place when a pinned region's row count
    /// changed: erase the old height, then draw the new one at the same top
    /// position. Growing extends downward (scrolling the transcript up like any
    /// new bottom content); shrinking may leave a blank row below the frame until
    /// the next output batch reclaims it.
    #[allow(clippy::too_many_arguments)]
    fn redraw_regions(
        &mut self,
        ui: &mut Ui,
        plan: bool,
        width: usize,
        ask: &Option<AskState>,
        cancel: &Cancel,
        started: Instant,
        active_tools: &[(String, String)],
        region_rows: &[String],
        drawn_r: &mut usize,
    ) {
        ui.begin_batch();
        self.erase_dock(ui, *drawn_r);
        self.draw_active_frame(
            ui,
            plan,
            width,
            ask,
            cancel,
            started,
            active_tools,
            region_rows,
        );
        ui.end_batch();
        *drawn_r = region_rows.len();
    }

    /// Erase the whole on-screen frame: the top status, `region_count` pinned
    /// region rows, the input row, and the bottom status. Cursor starts on the
    /// input row and ends at column 0 of the top row, where the next output
    /// takes the frame's place. `2K` clears each line whole irrespective of the
    /// cursor column, so this is exact at any height.
    fn erase_dock(&mut self, ui: &mut Ui, region_count: usize) {
        let up = 1 + region_count; // the input row sits this far below the top
        let height = region_count + 3; // top + regions + input + bottom
        let mut s = format!("\r\x1b[{up}A\x1b[2K");
        for _ in 1..height {
            s.push_str("\x1b[1B\r\x1b[2K");
        }
        s.push_str(&format!("\x1b[{}A\r", height - 1));
        ui.out_raw(s.as_bytes());
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
        region_rows: &[String],
    ) {
        let top = self.top_rule(ui, plan, width, started, active_tools);
        let bottom = self.bottom_rule(ui, width, ask, cancel);
        let tick = (started.elapsed().as_millis() / COMET_MS as u128) as usize;
        // Top status, each region row, a blank input row, then the bottom
        // status, advancing into fresh rows with \r\n and clearing each (a no-op
        // on the blank rows write_above just opened, self-healing otherwise).
        let mut s = format!("\r\x1b[2K{top}");
        for row in region_rows {
            s.push_str("\r\n\x1b[2K");
            s.push_str(&animated_region_row(row, tick));
        }
        s.push_str("\r\n\x1b[2K\r\n\x1b[2K");
        s.push_str(&bottom);
        s.push_str("\x1b[1A");
        ui.begin_batch();
        ui.out_raw(s.as_bytes());
        self.redraw_active_input(ui, width, ask);
        ui.end_batch();
    }

    /// Repaint the status rows and pinned regions in place without erasing
    /// committed output. Cursor is parked on the input row before and after. The
    /// on-screen height must equal `region_rows.len() + 3`; a region whose row
    /// count changed is repainted through `redraw_regions` (erase + draw), never
    /// here, so this in-place repaint is always height-exact.
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
        region_rows: &[String],
    ) {
        let top = self.top_rule(ui, plan, width, started, active_tools);
        let bottom = self.bottom_rule(ui, width, ask, cancel);
        let tick = (started.elapsed().as_millis() / COMET_MS as u128) as usize;
        let up = 1 + region_rows.len();
        let mut s = format!("\r\x1b[{up}A\r\x1b[2K{top}");
        for row in region_rows {
            // Clear the whole line before writing the row (one atomic write, so
            // no flash): a trailing clear-to-end would instead sit on the parked
            // deferred-wrap latch of a row clamped to exactly the width and erase
            // its last glyph on every repaint.
            s.push_str("\x1b[1B\r\x1b[2K");
            s.push_str(&animated_region_row(row, tick));
        }
        s.push_str("\x1b[2B\r\x1b[2K");
        s.push_str(&bottom);
        s.push_str("\x1b[1A\r");
        ui.begin_batch();
        ui.out_raw(s.as_bytes());
        self.redraw_active_input(ui, width, ask);
        ui.end_batch();
    }

    fn redraw_active_input(&mut self, ui: &mut Ui, width: usize, ask: &Option<AskState>) {
        if let Some(a) = ask {
            let shown = format!("{} [y/N] {}", safe_terminal_text(&a.question), a.answer);
            let ed = Editor::from_line(&shown);
            ui.redraw_input_row(&ed, width);
        } else {
            ui.redraw_input_row_hint(&self.draft, width, "type a message; Enter queues it");
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
            (
                "press ESC again to cancel".to_string(),
                ui.theme.error.sgr(ui.depth),
            )
        } else if self.reader_gone {
            ("input closed".to_string(), ui.theme.error.sgr(ui.depth))
        } else if ask.is_some() {
            (
                "Enter confirms · Ctrl-C cancels all".to_string(),
                ui.box_color(),
            )
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

}

/// One pinned row per queued message: a dim `› message [queued]`, clamped to
/// one physical row like every region row. It lives only in the pinned region
/// while the message waits; dispatch removes the row (and the [queued] marker
/// with it) and echoes the plain `› message` record into the transcript.
fn queued_region_row(ui: &Ui, message: &str, width: usize) -> PinnedRegionRow {
    let shown: String = message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    PinnedRegionRow::summary(
        ui,
        format!("› {shown} [queued]"),
        width,
        RegionTone::Dim,
        RegionPriority::None,
    )
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RegionPriority {
    None,
    Plan,
    Agents,
}

#[derive(Clone, Copy)]
enum RegionSource {
    Plan,
    Agents,
}

struct PinnedRegionRow {
    rendered: String,
    priority: RegionPriority,
}

impl PinnedRegionRow {
    fn summary(
        ui: &Ui,
        label: String,
        width: usize,
        tone: RegionTone,
        priority: RegionPriority,
    ) -> PinnedRegionRow {
        PinnedRegionRow {
            rendered: ui.region_summary_row(&label, width, tone),
            priority,
        }
    }
}

/// The plan region with the plan's own cap: every non-step row (the header),
/// a contiguous window of at most PLAN_STEP_ROWS steps that contains the
/// active one and prefers what comes next, and one dim "… +N more" row
/// naming what is hidden. A plan at or under the cap renders whole. The
/// screen cap in `cap_region_rows` still applies afterwards, so the active
/// step keeps its reservation there via its row priority.
fn capped_plan_rows(ui: &Ui, text: &str, width: usize) -> Vec<PinnedRegionRow> {
    let rows = checklist_pinned_rows(ui, text, width, RegionSource::Plan);
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let step_lines: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| is_plan_step(line))
        .map(|(index, _)| index)
        .collect();
    let steps: Vec<&str> = step_lines.iter().map(|&index| lines[index]).collect();
    let Some((range, hidden_done, hidden_queued)) = plan_cap_selection(&steps) else {
        return rows;
    };
    let window = &step_lines[range];

    let mut visible: Vec<PinnedRegionRow> = rows
        .into_iter()
        .enumerate()
        .filter_map(|(index, row)| {
            (!is_plan_step(lines[index]) || window.contains(&index)).then_some(row)
        })
        .collect();
    visible.push(PinnedRegionRow::summary(
        ui,
        plan_cap_label(hidden_done, hidden_queued),
        width,
        RegionTone::Dim,
        RegionPriority::None,
    ));
    visible
}

fn is_plan_step(line: &str) -> bool {
    line.starts_with("[x]")
        || line.starts_with("[!]")
        || line.starts_with("[~]")
        || line.starts_with("[ ]")
}

/// The pinned rows for one plan checklist: the capped step window while it
/// runs, collapsing to a one-line "plan completed" summary once every step is
/// done (the summary stays pinned only until the turn ends; then
/// `retire_completed_plan` moves it into the transcript). Shared by the
/// in-turn regions and the idle prompt so the plan looks the same wherever it
/// is pinned.
fn plan_region_rows(ui: &Ui, text: &str, width: usize) -> Vec<PinnedRegionRow> {
    if let Some(label) = completed_plan_label(text) {
        vec![PinnedRegionRow::summary(
            ui,
            label,
            width,
            RegionTone::Activity,
            RegionPriority::Plan,
        )]
    } else {
        capped_plan_rows(ui, text, width)
    }
}

/// The one-line summary of a plan whose every step is done, shared by the
/// mid-turn pinned row and the turn-end transcript record so both read
/// identically. None while any step is still open.
fn completed_plan_label(text: &str) -> Option<String> {
    let counts = checklist_counts(text);
    if !counts.is_complete() {
        return None;
    }
    let mut label = format!("plan completed · {}/{}", counts.done, counts.total());
    if let Some(plan_elapsed) = plan_elapsed(text) {
        label.push_str(" · ");
        label.push_str(plan_elapsed);
    }
    Some(label)
}

/// Pure window math for the plan cap. Given the step glyph lines in order,
/// the contiguous PLAN_STEP_ROWS window to show and the hidden done/queued
/// counts; None when the plan already fits. The window anchors on the active
/// step (falling back to the first pending one) so it shows the active step
/// plus what comes next, shifting back only when the tail runs short.
fn plan_cap_selection(steps: &[&str]) -> Option<(std::ops::Range<usize>, usize, usize)> {
    if steps.len() <= PLAN_STEP_ROWS {
        return None;
    }
    let anchor = steps
        .iter()
        .position(|step| step.starts_with("[~]"))
        .or_else(|| steps.iter().position(|step| step.starts_with("[ ]")))
        .unwrap_or(0);
    let start = anchor.min(steps.len() - PLAN_STEP_ROWS);
    let range = start..start + PLAN_STEP_ROWS;
    let mut hidden_done = 0usize;
    let mut hidden_queued = 0usize;
    for (position, step) in steps.iter().enumerate() {
        if range.contains(&position) {
            continue;
        }
        if step.starts_with("[ ]") {
            hidden_queued += 1;
        } else {
            hidden_done += 1;
        }
    }
    Some((range, hidden_done, hidden_queued))
}

fn plan_cap_label(hidden_done: usize, hidden_queued: usize) -> String {
    let hidden = hidden_done + hidden_queued;
    let mut label = format!(
        "… +{hidden} more step{}",
        if hidden == 1 { "" } else { "s" }
    );
    if hidden_done > 0 {
        label.push_str(&format!(" · {hidden_done} done"));
    }
    if hidden_queued > 0 {
        label.push_str(&format!(" · {hidden_queued} queued"));
    }
    label
}

fn checklist_pinned_rows(
    ui: &Ui,
    text: &str,
    width: usize,
    source: RegionSource,
) -> Vec<PinnedRegionRow> {
    let source_lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    ui.checklist_region_rows(text, width)
        .into_iter()
        .zip(source_lines)
        .enumerate()
        .map(|(index, (rendered, line))| {
            let priority = match source {
                RegionSource::Plan if line.starts_with("[~]") => RegionPriority::Plan,
                RegionSource::Agents if index == 0 => RegionPriority::Agents,
                _ => RegionPriority::None,
            };
            PinnedRegionRow { rendered, priority }
        })
        .collect()
}

fn agent_snapshot_block(snapshot: &crate::subagent::JobsSnapshot) -> String {
    let mut block = format!(
        "agents ({} active, {} ready): · {} queued · {} running · Tab to close",
        snapshot.active, snapshot.ready, snapshot.queued, snapshot.running,
    );
    for row in &snapshot.rows {
        let glyph = if row.contains(" · queued · ") {
            "[ ]"
        } else if row.contains(" · ready · ") {
            "[x]"
        } else {
            "[~]"
        };
        block.push('\n');
        block.push_str(glyph);
        block.push(' ');
        block.push_str(row);

        let id = row.split(" · ").next().unwrap_or_default();
        if let Some(progress) = snapshot
            .recent_progress
            .iter()
            .find(|progress| progress.id == id)
        {
            for line in &progress.lines {
                block.push('\n');
                block.push_str("    ");
                block.push_str(id);
                block.push_str(" │ ");
                block.push_str(line);
            }
        }
    }
    block
}

fn expanded_agent_snapshot_block(snapshot: &crate::subagent::JobsSnapshot) -> Option<String> {
    (!snapshot.rows.is_empty()).then(|| agent_snapshot_block(snapshot))
}

fn collapsed_agent_snapshot_block(snapshot: &crate::subagent::JobsSnapshot) -> Option<String> {
    if snapshot.active > 0 {
        // After an explicit stop-everything cancel the whole fleet is
        // winding down; "running" would misread as the cancel having been
        // ignored while the workers reap the children.
        if snapshot.stopping == snapshot.active {
            return Some(format!(
                "[{}] agents stopping (Tab to view)",
                snapshot.active
            ));
        }
        Some(format!(
            "[{}] agents running (Tab to view)",
            snapshot.active
        ))
    } else if snapshot.ready > 0 {
        Some(format!("[{}] agents ready (Tab to view)", snapshot.ready))
    } else {
        None
    }
}

/// New plan payloads append lifecycle time after the compatible header,
/// `plan (x/y done): · 1.2s`. Old payloads have no suffix and fall back to the
/// dock turn duration in final summaries.
fn plan_elapsed(text: &str) -> Option<&str> {
    let header = text.lines().next()?;
    let (_, elapsed) = header.split_once("): · ")?;
    let elapsed = elapsed.trim();
    (!elapsed.is_empty()).then_some(elapsed)
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct ChecklistCounts {
    done: usize,
    active: usize,
    pending: usize,
}

impl ChecklistCounts {
    fn total(self) -> usize {
        self.done + self.active + self.pending
    }

    fn is_complete(self) -> bool {
        self.total() > 0 && self.done == self.total()
    }

    fn plus(self, other: ChecklistCounts) -> ChecklistCounts {
        ChecklistCounts {
            done: self.done + other.done,
            active: self.active + other.active,
            pending: self.pending + other.pending,
        }
    }
}

fn checklist_counts(text: &str) -> ChecklistCounts {
    let mut counts = ChecklistCounts::default();
    for line in text.lines() {
        if line.starts_with("[x]") || line.starts_with("[!]") {
            counts.done += 1;
        } else if line.starts_with("[~]") {
            counts.active += 1;
        } else if line.starts_with("[ ]") {
            counts.pending += 1;
        }
    }
    counts
}

fn animated_region_row(row: &str, tick: usize) -> String {
    const FRAMES: [&str; 4] = ["[|]", "[/]", "[-]", "[\\]"];
    row.replacen("[~]", FRAMES[tick % FRAMES.len()], 1)
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
        assert_eq!(
            (t.col, t.fresh),
            (0, false),
            "\\r returns onto used content"
        );
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
    fn checklist_counts_drive_completed_and_overflow_summaries() {
        let counts = checklist_counts("plan (2/4 done):\n[x] one\n[x] two\n[~] three\n[ ] four");
        assert_eq!(
            counts,
            ChecklistCounts {
                done: 2,
                active: 1,
                pending: 1
            }
        );
        assert!(!counts.is_complete());
        assert!(checklist_counts("plan (2/2 done):\n[x] one\n[x] two").is_complete());
        assert_eq!(checklist_counts("agents:\n[!] failed").done, 1);
    }

    #[test]
    fn plan_cap_windows_on_the_active_step_and_counts_the_hidden_rest() {
        // At or under the cap: untouched.
        let fits: Vec<&str> = vec!["[x] a", "[~] b", "[ ] c"];
        assert!(plan_cap_selection(&fits).is_none());

        // Active mid-list: the window starts at the active step and runs
        // forward; everything hidden before it is done, after it queued.
        let steps: Vec<&str> = vec![
            "[x] 1", "[x] 2", "[x] 3", "[~] 4", "[ ] 5", "[ ] 6", "[ ] 7", "[ ] 8", "[ ] 9",
            "[ ] 10",
        ];
        let (range, done, queued) = plan_cap_selection(&steps).expect("over the cap");
        assert_eq!(range, 3..9, "active step leads the window");
        assert_eq!((done, queued), (3, 1));

        // Active near the end: the window shifts back instead of running
        // past the list, and the hidden side is all completed work.
        let late: Vec<&str> = vec![
            "[x] 1", "[x] 2", "[x] 3", "[x] 4", "[x] 5", "[x] 6", "[x] 7", "[x] 8", "[~] 9",
            "[ ] 10",
        ];
        let (range, done, queued) = plan_cap_selection(&late).expect("over the cap");
        assert_eq!(range, 4..10, "window clamps to the tail");
        assert_eq!((done, queued), (4, 0));

        // No active step yet: anchor on the first pending one.
        let fresh: Vec<&str> = vec![
            "[ ] 1", "[ ] 2", "[ ] 3", "[ ] 4", "[ ] 5", "[ ] 6", "[ ] 7", "[ ] 8",
        ];
        let (range, done, queued) = plan_cap_selection(&fresh).expect("over the cap");
        assert_eq!(range, 0..6);
        assert_eq!((done, queued), (0, 2));

        assert_eq!(plan_cap_label(3, 1), "… +4 more steps · 3 done · 1 queued");
        assert_eq!(plan_cap_label(0, 1), "… +1 more step · 1 queued");
        assert_eq!(plan_cap_label(4, 0), "… +4 more steps · 4 done");
    }

    #[test]
    fn agent_snapshot_view_keeps_compact_row_and_separate_recent_activity() {
        let snapshot = crate::subagent::JobsSnapshot {
            active: 1,
            queued: 0,
            running: 1,
            ready: 0,
            active_ids: vec!["agent-1".into()],
            undelivered_ids: vec!["agent-1".into()],
            rows: vec!["agent-1 · running · 1.2s · code one file · * bash make".into()],
            recent_progress: vec![crate::subagent::JobProgressSnapshot {
                id: "agent-1".into(),
                lines: vec!["* read src/lib.rs".into(), "* write src/lib.rs".into()],
            }],
            ..crate::subagent::JobsSnapshot::default()
        };
        let block = agent_snapshot_block(&snapshot);
        let lines: Vec<&str> = block.lines().collect();
        assert!(lines[0].starts_with("agents (1 active, 0 ready):"));
        assert!(lines[1].contains("[~] agent-1 · running · 1.2s · code one file"));
        assert_eq!(lines[2], "    agent-1 │ * read src/lib.rs");
        assert_eq!(lines[3], "    agent-1 │ * write src/lib.rs");

        assert_eq!(
            collapsed_agent_snapshot_block(&snapshot).as_deref(),
            Some("[1] agents running (Tab to view)")
        );
        let ready = crate::subagent::JobsSnapshot {
            active: 0,
            ready: 1,
            ..crate::subagent::JobsSnapshot::default()
        };
        assert_eq!(
            collapsed_agent_snapshot_block(&ready).as_deref(),
            Some("[1] agents ready (Tab to view)")
        );
        // A stop-everything cancel flips the whole fleet to "stopping"; a
        // single targeted cancel among others keeps the running label.
        let stopping = crate::subagent::JobsSnapshot {
            active: 2,
            stopping: 2,
            ..crate::subagent::JobsSnapshot::default()
        };
        assert_eq!(
            collapsed_agent_snapshot_block(&stopping).as_deref(),
            Some("[2] agents stopping (Tab to view)")
        );
        let partial = crate::subagent::JobsSnapshot {
            active: 2,
            stopping: 1,
            ..crate::subagent::JobsSnapshot::default()
        };
        assert_eq!(
            collapsed_agent_snapshot_block(&partial).as_deref(),
            Some("[2] agents running (Tab to view)")
        );
        assert!(
            collapsed_agent_snapshot_block(&crate::subagent::JobsSnapshot::default()).is_none()
        );
        assert!(
            expanded_agent_snapshot_block(&crate::subagent::JobsSnapshot::default()).is_none(),
            "an open temporal view must disappear once its last job is drained"
        );
    }

    #[test]
    fn plan_elapsed_reads_new_header_and_old_headers_still_fall_back() {
        assert_eq!(
            plan_elapsed("plan (1/2 done): · 3.4s\n[x] first · 1.0s\n[~] second"),
            Some("3.4s")
        );
        assert_eq!(
            plan_elapsed("plan (1/2 done):\n[x] first\n[~] second"),
            None
        );
    }

    #[test]
    fn active_plan_glyph_animates_without_changing_other_rows() {
        assert_eq!(animated_region_row("[~] active", 0), "[|] active");
        assert_eq!(animated_region_row("[~] active", 1), "[/] active");
        assert_eq!(animated_region_row("[~] active", 2), "[-] active");
        assert_eq!(animated_region_row("[~] active", 3), "[\\] active");
        assert_eq!(animated_region_row("[x] done", 3), "[x] done");
    }

    #[test]
    fn cancel_is_two_tap_but_ctrl_c_can_commit_directly() {
        let mut cancel = Cancel {
            armed_until: Some(Instant::now() + CANCEL_WINDOW),
            ..Cancel::default()
        };
        assert!(cancel.disarm(), "first ESC must arm without canceling");
        assert!(!cancel.committed);
        cancel.commit();
        assert!(cancel.committed);
    }
}
