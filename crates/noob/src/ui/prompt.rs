//! The REPL line reader. At an interactive terminal it runs a small termios
//! line editor that draws a two-line green input box and offers real editing
//! (insert, backspace across a multibyte char, word/line kills, cursor moves,
//! bracketed paste). Piped or headless, it falls back to the exact cooked
//! `read_line` so those surfaces stay byte-for-byte what they were.
//!
//! The editor is off the inference path: raw mode is entered only while the
//! human is typing and restored to cooked before the agent runs, so keystrokes
//! never reach the model (it sees the message once, on Enter) and prefill and
//! decode throughput are untouched. Three hooks restore the terminal so a
//! crash never leaves the shell raw: the RAII guard on the normal return, the
//! panic hook (release is `panic = "abort"`, so `Drop` does not run on a
//! panic), and the SIGINT handler before its `_exit(130)`.
//!
//! Display-only, like everything under `ui/`: the reader never rewrites request
//! bodies, the session log, or the wire protocol. The submitted line is handed
//! to `run_input`, which persists it (`push_item`) before the model replies, so
//! a crash after Enter is resumable; only an unsubmitted in-progress line is
//! lost, which is acceptable.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{BufRead, IsTerminal};
use std::sync::atomic::Ordering;

use noob_provider::http::INTERRUPTED;

use super::style::RESET;
use super::{Mode, Ui};

thread_local! {
    /// Keys decoded past a submitted Enter within a single read (a multi-line
    /// paste on a terminal that ignores bracketed paste). Replayed on the next
    /// prompt so each pasted line becomes its own turn instead of being lost.
    static CARRYOVER: RefCell<VecDeque<Key>> = const { RefCell::new(VecDeque::new()) };
}

/// The outcome of reading one prompt. EOF (Ctrl-D or a closed stream) and a
/// Ctrl-C at the prompt are kept distinct: EOF exits, an interrupt reprompts.
pub enum Input {
    Line(String),
    Interrupted,
    Eof,
}

impl Ui {
    /// Read one line of user input, drawing the boxed editor at an interactive
    /// terminal and falling back to cooked `read_line` everywhere else.
    pub fn read_prompt(&mut self, plan: bool) -> Input {
        if self.use_raw_editor() {
            self.read_raw(plan)
        } else {
            self.read_cooked(plan)
        }
    }

    /// The raw editor is for an interactive REPL only: both ends must be a
    /// terminal (you cannot line-edit a pipe), and `NOOB_RAW=0` forces the
    /// cooked reader as an escape hatch if a terminal misbehaves.
    fn use_raw_editor(&self) -> bool {
        self.mode == Mode::Repl
            && std::io::stdin().is_terminal()
            && std::io::stdout().is_terminal()
            && raw_enabled_by_env()
    }

    /// Byte-identical to the pre-editor reader: write the plain marker, read a
    /// cooked line. A Ctrl-C delivered during the read reprompts (matching the
    /// old loop, which checked the flag after `read_line`).
    fn read_cooked(&mut self, plan: bool) -> Input {
        self.prompt(plan);
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) => Input::Eof,
            Ok(_) => {
                if INTERRUPTED.swap(false, Ordering::SeqCst) {
                    Input::Interrupted
                } else {
                    Input::Line(line)
                }
            }
            Err(_) => Input::Eof,
        }
    }

    /// The termios editor. Restores the terminal on every exit path.
    fn read_raw(&mut self, plan: bool) -> Input {
        let Some(_guard) = RawGuard::enter() else {
            // tcgetattr/tcsetattr failed (not a real tty after all): degrade.
            return self.read_cooked(plan);
        };
        let mut ed = Editor::default();
        let mut dec = Decoder::default();
        // Seed `width` from the exact value the rule was drawn at (not a second
        // ioctl), so a size that lands between the two reads cannot leave refit
        // comparing against a width the rule was never drawn at.
        let mut width = self.draw_box_top(plan);

        // Replay any keys carried over from a previous multi-line submit before
        // reading new input, so a pasted script runs one line per turn.
        let mut queue: VecDeque<Key> = CARRYOVER.with(|c| std::mem::take(&mut *c.borrow_mut()));

        let mut buf = [0u8; 1024];
        loop {
            while let Some(key) = queue.pop_front() {
                match ed.apply(key) {
                    Step::Continue => {}
                    Step::Submit => return self.submit(&ed, queue),
                    Step::Interrupt => {
                        self.erase_box();
                        return self.interrupted();
                    }
                    Step::Eof => {
                        self.erase_box();
                        return Input::Eof;
                    }
                }
            }
            // Snap the top rule to the current terminal width, then paint the
            // input line. A freshly spawned pty reports width 0 for the first
            // draw and its real size lands a moment later; re-fitting here means
            // the box reaches full width on the first keystroke. Just a cheap
            // ioctl per key on the read path already taken: no idle loop, no
            // extra signal, nothing listening.
            self.refit(plan, &mut width);
            self.redraw_line(&ed, width);
            let n = unsafe {
                libc::read(libc::STDIN_FILENO, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
            };
            if n < 0 {
                // EINTR: a signal landed. A first Ctrl-C set the flag (treat as
                // an interrupt); any other EINTR is benign, so retry. A second
                // Ctrl-C already exited via the handler.
                if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                    if INTERRUPTED.swap(false, Ordering::SeqCst) {
                        self.erase_box();
                        return Input::Interrupted;
                    }
                    continue;
                }
                self.erase_box();
                return Input::Eof;
            }
            if n == 0 {
                self.erase_box();
                return Input::Eof;
            }
            for key in dec.feed(&buf[..n as usize]) {
                queue.push_back(key);
            }
        }
    }

    /// Finish a submitted line. Reconcile a stray interrupt first: a Ctrl-C
    /// that landed during the cooked gap before this prompt went raw set
    /// INTERRUPTED without exiting, and would otherwise phantom-cancel the
    /// fresh turn, so consume it and reprompt instead. Otherwise show the final
    /// line and carry any keys decoded past the Enter to the next prompt.
    fn submit(&mut self, ed: &Editor, rest: VecDeque<Key>) -> Input {
        if INTERRUPTED.swap(false, Ordering::SeqCst) {
            self.erase_box();
            return Input::Interrupted;
        }
        self.collapse_to_message(ed);
        if !rest.is_empty() {
            CARRYOVER.with(|c| *c.borrow_mut() = rest);
        }
        Input::Line(ed.line())
    }

    /// Reprompt after a Ctrl-C, clearing any pending interrupt so it cannot
    /// leak into the next turn.
    fn interrupted(&mut self) -> Input {
        INTERRUPTED.swap(false, Ordering::SeqCst);
        Input::Interrupted
    }

    /// The border/prompt SGR, empty when color is off (a raw editor still runs
    /// at a `NO_COLOR` or depthless terminal, just without the green).
    fn box_color(&self) -> String {
        if self.color {
            self.theme.prompt.sgr(self.depth)
        } else {
            String::new()
        }
    }

    /// The top rule (`╭───...───╮`, no wordmark; the banner carries it, plan
    /// mode is the only label), then a newline, leaving the cursor at column 0
    /// of the input line below it. Returns the width it drew at so the caller can
    /// track it without a second ioctl.
    fn draw_box_top(&mut self, plan: bool) -> usize {
        let width = term_width();
        self.draw_rule(plan, width);
        width
    }

    /// Emit the top rule at a given width plus the newline down to the input row.
    fn draw_rule(&mut self, plan: bool, width: usize) {
        let color = self.box_color();
        let reset = if color.is_empty() { "" } else { RESET };
        let rule = box_top_rule(plan, width);
        self.out(&format!("{color}{rule}{reset}\r\n"));
    }

    /// Re-fit the box to the current terminal width, called once per keystroke
    /// on the read path. A freshly spawned pty often reports width 0 for the
    /// very first draw and only reports its real size a moment later; without
    /// this the first box would stay stuck at the fallback width, so it snaps to
    /// full width on the first keystroke (and tracks a later resize the same
    /// way). When the width changed it repaints the box cleanly (erase both
    /// rows, redraw the rule) and reports true so the caller repaints the input
    /// line under it; otherwise a bare ioctl and nothing else. Sound because the
    /// input line never wraps (redraw_line windows it to one row), so the box is
    /// exactly two rows and erase_box targets them exactly. The one exception is
    /// a terminal narrower than the 80-column fallback used before the pty
    /// reports its size, where the fallback rule itself wraps for a moment; the
    /// next key redraws it clean, and it only ever leaves a stray rule fragment
    /// above the box (the input and the transcript stay correct). No timer, no
    /// signal: the width is only re-read when a key is already being handled.
    fn refit(&mut self, plan: bool, width: &mut usize) -> bool {
        let now = term_width();
        if now == *width {
            return false;
        }
        *width = now;
        self.erase_box();
        self.draw_rule(plan, now);
        true
    }

    /// Redraw the input line in place at the given width: return to column 0,
    /// clear it, print the framed prompt plus a one-row window of the buffer,
    /// then park the cursor. The window keeps the input to exactly one physical
    /// row (a long line scrolls horizontally instead of wrapping), so there is
    /// never more than one input row and every in-place redraw (this, erase_box,
    /// refit) stays exact.
    fn redraw_line(&mut self, ed: &Editor, width: usize) {
        let color = self.box_color();
        let reset = if color.is_empty() { "" } else { RESET };
        let avail = width.saturating_sub(PREFIX_CELLS).max(1);
        let (shown, cur) = input_window(&ed.buf, ed.cursor, avail);
        let mut s = format!("\r\x1b[K{color}{PREFIX}{reset}{shown}");
        // Cursor column = the prefix width plus its offset within the window.
        let col = PREFIX_CELLS + cur;
        s.push('\r');
        if col > 0 {
            s.push_str(&format!("\x1b[{col}C"));
        }
        self.out(&s);
    }

    /// Wipe the two box lines, leaving the cursor at the box's origin so the
    /// next output takes its place. `2K` clears each whole line irrespective of
    /// the cursor column.
    fn erase_box(&mut self) {
        self.out("\r\x1b[2K\x1b[1A\x1b[2K\r");
    }

    /// On submit, collapse the box to a compact record of the message: a green
    /// arrow and the text, then a newline so the reply flows below. The box
    /// frame is not left behind, so history reads as `› message` lines, not a
    /// stack of frames.
    fn collapse_to_message(&mut self, ed: &Editor) {
        self.erase_box();
        let shown: String = ed
            .buf
            .iter()
            .map(|&c| if c.is_control() { ' ' } else { c })
            .collect();
        let color = self.box_color();
        let reset = if color.is_empty() { "" } else { RESET };
        self.out(&format!("{color}› {reset}{shown}\r\n"));
    }
}

/// The framed prompt: left border, a space, the marker glyph, a space.
const PREFIX: &str = "│ › ";
/// Its display width in columns (each of the four glyphs is single-width).
const PREFIX_CELLS: usize = 4;

/// `NOOB_RAW=0|false|off|no` forces the cooked reader; anything else (including
/// unset) leaves the editor on. A rebuild-free escape hatch for odd terminals.
fn raw_enabled_by_env() -> bool {
    match std::env::var("NOOB_RAW") {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no"),
        Err(_) => true,
    }
}

/// The top border string at a given width: `╭───...───╮`, or `╭─ plan ──...─╮`
/// in plan mode. Shared by the first draw and the resize re-fit so they never
/// disagree.
fn box_top_rule(plan: bool, width: usize) -> String {
    let head = if plan { "╭─ plan " } else { "╭" };
    let mut rule = String::from(head);
    for _ in head.chars().count()..width.saturating_sub(1) {
        rule.push('─');
    }
    rule.push('╮');
    rule
}

/// A one-physical-row view of the input buffer: the visible slice (control
/// chars, including any pasted newline, shown as spaces so nothing wraps or
/// moves the cursor unexpectedly) and the cursor's column within it. `avail` is
/// the number of columns available for text. The window holds the cursor: it
/// stays left-anchored until the cursor would fall off the right edge, then
/// scrolls so the cursor rides the right. Keeping the input to one row is what
/// lets every in-place redraw assume a two-row box. Pure, so unit-testable.
///
/// Widths are counted in `char`s, i.e. one column per character: this carries no
/// unicode-width table (a deliberate zero-dependency choice), so a run of
/// double-width CJK or emoji glyphs is the one case that can still spill past the
/// row and nudge the cursor. Plain single-width text (paths, code, prose) is
/// exact, and the buffer and the submitted line are always correct regardless.
fn input_window(buf: &[char], cursor: usize, avail: usize) -> (String, usize) {
    let avail = avail.max(1);
    let start = if cursor >= avail { cursor - avail + 1 } else { 0 };
    let end = (start + avail).min(buf.len());
    let shown: String = buf[start..end]
        .iter()
        .map(|&c| if c.is_control() { ' ' } else { c })
        .collect();
    (shown, cursor - start)
}

/// Terminal width in columns via the window-size ioctl; 80 when it is
/// unavailable (a startup pty that has not been sized yet reports 0). The box
/// spans the full width, so no upper clamp.
fn term_width() -> usize {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
            (ws.ws_col as usize).max(20)
        } else {
            80
        }
    }
}

// ---------------------------------------------------------------------------
// The editor: a pure state machine over decoded keys. No I/O, so it is fully
// unit-testable without owning a terminal.
// ---------------------------------------------------------------------------

/// One editing action, already decoded from the raw byte stream.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Key {
    Char(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    KillToStart,
    KillToEnd,
    KillWord,
    Enter,
    Interrupt,
    Eof,
}

/// What the loop should do after applying a key.
enum Step {
    Continue,
    Submit,
    Interrupt,
    Eof,
}

/// The line buffer as `char`s (not bytes) so the cursor and backspace operate
/// on whole codepoints: one backspace deletes a whole multibyte character.
#[derive(Default)]
struct Editor {
    buf: Vec<char>,
    /// Cursor position in chars, `0..=buf.len()`.
    cursor: usize,
}

impl Editor {
    fn line(&self) -> String {
        self.buf.iter().collect()
    }

    fn apply(&mut self, key: Key) -> Step {
        match key {
            Key::Char(c) => {
                self.buf.insert(self.cursor, c);
                self.cursor += 1;
            }
            Key::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.buf.remove(self.cursor);
                }
            }
            Key::Delete => {
                if self.cursor < self.buf.len() {
                    self.buf.remove(self.cursor);
                }
            }
            Key::Left => self.cursor = self.cursor.saturating_sub(1),
            Key::Right => {
                if self.cursor < self.buf.len() {
                    self.cursor += 1;
                }
            }
            Key::Home => self.cursor = 0,
            Key::End => self.cursor = self.buf.len(),
            Key::KillToStart => {
                self.buf.drain(0..self.cursor);
                self.cursor = 0;
            }
            Key::KillToEnd => self.buf.truncate(self.cursor),
            Key::KillWord => self.kill_word(),
            Key::Enter => return Step::Submit,
            Key::Interrupt => return Step::Interrupt,
            // Ctrl-D exits only on an empty line; with text it is a no-op, so a
            // stray Ctrl-D never truncates a message mid-edit.
            Key::Eof => {
                if self.buf.is_empty() {
                    return Step::Eof;
                }
            }
        }
        Step::Continue
    }

    /// Delete the whitespace-delimited word before the cursor (Ctrl-W).
    fn kill_word(&mut self) {
        let mut i = self.cursor;
        while i > 0 && self.buf[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !self.buf[i - 1].is_whitespace() {
            i -= 1;
        }
        self.buf.drain(i..self.cursor);
        self.cursor = i;
    }
}

// ---------------------------------------------------------------------------
// The decoder: raw bytes -> keys. Stateful only for sequences that can split
// across reads (an incomplete escape or multibyte char) and for bracketed
// paste. Pure and unit-testable.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Decoder {
    /// An incomplete escape or UTF-8 sequence carried to the next feed.
    pending: Vec<u8>,
    /// Inside a bracketed paste: newlines are literal text, not Enter.
    paste: bool,
}

/// The classification of an escape sequence.
enum EscKind {
    Key(Key),
    PasteStart,
    PasteEnd,
    /// Recognized-but-unhandled (arrows we do not bind yet, a lone ESC): drop.
    Ignore,
}

/// One decoded printable character, or a signal to skip/wait.
enum Decoded {
    Char(char, usize),
    Skip(usize),
    Incomplete,
}

impl Decoder {
    fn feed(&mut self, bytes: &[u8]) -> Vec<Key> {
        let mut data = std::mem::take(&mut self.pending);
        data.extend_from_slice(bytes);
        let mut keys = Vec::new();
        let mut i = 0;
        while i < data.len() {
            let b = data[i];
            if b == 0x1b {
                match match_esc(&data[i..]) {
                    None => {
                        // Incomplete escape: wait for the rest.
                        self.pending = data[i..].to_vec();
                        return keys;
                    }
                    Some((kind, used)) => {
                        let used = used.max(1);
                        if self.paste {
                            // Inside a paste only the terminator is a control
                            // sequence; every other escape is literal content,
                            // preserved byte-for-byte (the ESC here, its tail
                            // as ordinary chars next).
                            match kind {
                                EscKind::PasteEnd => {
                                    self.paste = false;
                                    i += used;
                                }
                                _ => {
                                    keys.push(Key::Char('\u{1b}'));
                                    i += 1;
                                }
                            }
                        } else {
                            match kind {
                                EscKind::PasteStart => self.paste = true,
                                EscKind::PasteEnd => {} // stray terminator: drop
                                EscKind::Key(k) => keys.push(k),
                                EscKind::Ignore => {}
                            }
                            i += used;
                        }
                    }
                }
                continue;
            }
            if self.paste {
                match b {
                    // Ctrl-C and Ctrl-D always break out, even from a paste
                    // with no terminator, so a truncated paste can never wedge
                    // the editor (ISIG is off, so there is no other exit).
                    0x03 => {
                        self.paste = false;
                        keys.push(Key::Interrupt);
                        i += 1;
                    }
                    0x04 => {
                        keys.push(Key::Eof);
                        i += 1;
                    }
                    0x0d => {
                        // A CRLF can straddle a read boundary. If the CR is the
                        // last byte, wait so the following LF can be collapsed
                        // instead of emitting two newlines.
                        if i + 1 == data.len() {
                            self.pending = data[i..].to_vec();
                            return keys;
                        }
                        keys.push(Key::Char('\n'));
                        if data[i + 1] == 0x0a {
                            i += 1;
                        }
                        i += 1;
                    }
                    0x0a => {
                        keys.push(Key::Char('\n'));
                        i += 1;
                    }
                    b if b < 0x20 => i += 1, // drop other control bytes in a paste
                    _ => match take_char(&data, i) {
                        Decoded::Incomplete => {
                            self.pending = data[i..].to_vec();
                            return keys;
                        }
                        Decoded::Skip(n) => i += n,
                        Decoded::Char(c, n) => {
                            keys.push(Key::Char(c));
                            i += n;
                        }
                    },
                }
                continue;
            }
            match b {
                b'\r' | b'\n' => keys.push(Key::Enter),
                0x7f | 0x08 => keys.push(Key::Backspace),
                0x01 => keys.push(Key::Home),
                0x02 => keys.push(Key::Left),
                0x05 => keys.push(Key::End),
                0x06 => keys.push(Key::Right),
                0x03 => keys.push(Key::Interrupt),
                0x04 => keys.push(Key::Eof),
                0x0b => keys.push(Key::KillToEnd),
                0x15 => keys.push(Key::KillToStart),
                0x17 => keys.push(Key::KillWord),
                b if b < 0x20 => {} // ignore other control (Tab, etc.)
                _ => match take_char(&data, i) {
                    Decoded::Incomplete => {
                        self.pending = data[i..].to_vec();
                        return keys;
                    }
                    Decoded::Skip(n) => {
                        i += n;
                        continue;
                    }
                    Decoded::Char(c, n) => {
                        keys.push(Key::Char(c));
                        i += n;
                        continue;
                    }
                },
            }
            i += 1;
        }
        keys
    }
}

/// Byte length of a UTF-8 sequence from its lead byte; 1 for a bad lead (so the
/// decoder makes progress and drops it).
fn utf8_len(lead: u8) -> usize {
    if lead < 0x80 {
        1
    } else if lead >> 5 == 0b110 {
        2
    } else if lead >> 4 == 0b1110 {
        3
    } else if lead >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

/// Decode one character at `data[i]`, or report that the sequence is split
/// across reads (`Incomplete`) or invalid (`Skip`).
fn take_char(data: &[u8], i: usize) -> Decoded {
    let len = utf8_len(data[i]);
    if i + len > data.len() {
        return Decoded::Incomplete;
    }
    match std::str::from_utf8(&data[i..i + len]) {
        Ok(s) => match s.chars().next() {
            Some(c) => Decoded::Char(c, len),
            None => Decoded::Skip(len),
        },
        Err(_) => Decoded::Skip(1),
    }
}

/// Match an escape sequence beginning at `data[0] == 0x1b`. Returns the kind
/// and the number of bytes it consumes, or `None` if more bytes are needed.
fn match_esc(data: &[u8]) -> Option<(EscKind, usize)> {
    if data.len() < 2 {
        return None; // just ESC so far
    }
    let intro = data[1];
    if intro != b'[' && intro != b'O' {
        // ESC + anything else (a lone ESC, an Alt-chord): drop the ESC only.
        return Some((EscKind::Ignore, 1));
    }
    // Scan parameter bytes (0x20..=0x3f) to the final byte (0x40..=0x7e). A
    // real CSI is short; bound the scan so a stream that never sends a final
    // byte cannot grow `pending` without bound. Past the cap, drop the run.
    const MAX_PARAMS: usize = 64;
    let mut j = 2;
    while j < data.len() {
        let c = data[j];
        if (0x40..=0x7e).contains(&c) {
            return Some((classify_csi(&data[2..j], c), j + 1));
        }
        if j - 2 >= MAX_PARAMS {
            return Some((EscKind::Ignore, j));
        }
        j += 1;
    }
    None // no final byte yet (still within the cap): wait for more
}

fn classify_csi(params: &[u8], fin: u8) -> EscKind {
    match (params, fin) {
        (b"", b'C') => EscKind::Key(Key::Right),
        (b"", b'D') => EscKind::Key(Key::Left),
        (b"", b'H') => EscKind::Key(Key::Home),
        (b"", b'F') => EscKind::Key(Key::End),
        (b"1", b'~') | (b"7", b'~') => EscKind::Key(Key::Home),
        (b"4", b'~') | (b"8", b'~') => EscKind::Key(Key::End),
        (b"3", b'~') => EscKind::Key(Key::Delete),
        (b"200", b'~') => EscKind::PasteStart,
        (b"201", b'~') => EscKind::PasteEnd,
        // Arrows we do not bind yet (Up/Down) and any other sequence: drop.
        _ => EscKind::Ignore,
    }
}

// ---------------------------------------------------------------------------
// Raw mode: entry/exit and the three restore hooks. The saved terminal state
// lives in a signal-reachable global so the panic hook and the SIGINT handler
// can restore it too (a `Drop` alone is not enough under `panic = "abort"`).
// ---------------------------------------------------------------------------

/// Restore the terminal to cooked mode if the editor is active. Safe to call
/// from a signal handler: only atomics, `tcsetattr`, and `write`, no
/// allocation. Idempotent, so whichever of the three hooks fires first wins and
/// the rest are no-ops.
pub fn restore_terminal() {
    raw_state::restore();
}

struct RawGuard;

impl RawGuard {
    fn enter() -> Option<RawGuard> {
        install_panic_hook();
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut saved) } != 0 {
            return None;
        }
        // Arm the restore state BEFORE touching the terminal, so a signal in
        // the tiny window still finds a valid saved state to restore.
        raw_state::arm(saved);
        let mut raw = saved;
        // Char-at-a-time, no echo (we draw the line), no signal keys (Ctrl-C
        // arrives as a byte we handle), no XON/XOFF freeze, CR left as CR.
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
        raw.c_iflag &= !(libc::IXON | libc::ICRNL);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) } != 0 {
            raw_state::restore();
            return None;
        }
        // Bracketed paste: a multi-line paste arrives wrapped, so its newlines
        // are literal text instead of premature submits. Mark it before the
        // write so a signal in the tiny gap disables a not-yet-enabled mode (a
        // harmless no-op) rather than leaking an enabled one past exit.
        raw_state::set_paste(true);
        write_stdout(b"\x1b[?2004h");
        Some(RawGuard)
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        raw_state::restore();
    }
}

/// Install the panic hook exactly once: restore the terminal, then run the
/// previous hook so the panic message still prints. Needed because
/// `panic = "abort"` skips `Drop`.
fn install_panic_hook() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            raw_state::restore();
            prev(info);
        }));
    });
}

/// A direct, unbuffered write to stdout for the paste-mode toggles, so their
/// ordering relative to the terminal-mode changes is exact.
fn write_stdout(bytes: &[u8]) {
    unsafe {
        libc::write(
            libc::STDOUT_FILENO,
            bytes.as_ptr() as *const libc::c_void,
            bytes.len(),
        );
    }
}

mod raw_state {
    use std::cell::UnsafeCell;
    use std::mem::MaybeUninit;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Holds the saved termios for the signal path. Single-threaded REPL input,
    /// written before `ACTIVE` is set and read only while `ACTIVE`, so the
    /// unsynchronized cell is sound.
    struct Cell(UnsafeCell<MaybeUninit<libc::termios>>);
    unsafe impl Sync for Cell {}

    static SAVED: Cell = Cell(UnsafeCell::new(MaybeUninit::uninit()));
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    static PASTE: AtomicBool = AtomicBool::new(false);

    /// Record the cooked termios and mark the editor active. Ordered so the
    /// signal handler that sees `ACTIVE` also sees a fully written `SAVED`.
    pub(super) fn arm(saved: libc::termios) {
        unsafe { (*SAVED.0.get()).write(saved) };
        PASTE.store(false, Ordering::SeqCst);
        ACTIVE.store(true, Ordering::SeqCst);
    }

    pub(super) fn set_paste(on: bool) {
        PASTE.store(on, Ordering::SeqCst);
    }

    /// Restore cooked mode and disable bracketed paste. Async-signal-safe, and
    /// re-entrant-safe: the terminal work happens BEFORE `ACTIVE` is cleared,
    /// so if a signal preempts this mid-restore and the handler re-enters, it
    /// re-issues the same idempotent `tcsetattr` (leaving the tty cooked)
    /// instead of short-circuiting on an already-cleared flag and exiting with
    /// the terminal still raw. `tcsetattr` is idempotent; the `PASTE` swap
    /// keeps the disable write at-most-once.
    pub(super) fn restore() {
        if ACTIVE.load(Ordering::SeqCst) {
            unsafe {
                let saved = (*SAVED.0.get()).assume_init_ref();
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, saved);
            }
            if PASTE.swap(false, Ordering::SeqCst) {
                const OFF: &[u8] = b"\x1b[?2004l";
                unsafe {
                    libc::write(
                        libc::STDOUT_FILENO,
                        OFF.as_ptr() as *const libc::c_void,
                        OFF.len(),
                    );
                }
            }
            ACTIVE.store(false, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed a whole byte string and collect the keys (single chunk).
    fn keys(bytes: &[u8]) -> Vec<Key> {
        Decoder::default().feed(bytes)
    }

    /// Drive an editor from an empty buffer with decoded keys; return the final
    /// line and the last step (default Continue if no key was terminal).
    fn run(bytes: &[u8]) -> (String, &'static str) {
        let mut ed = Editor::default();
        let mut last = "continue";
        for k in keys(bytes) {
            match ed.apply(k) {
                Step::Continue => last = "continue",
                Step::Submit => last = "submit",
                Step::Interrupt => last = "interrupt",
                Step::Eof => last = "eof",
            }
        }
        (ed.line(), last)
    }

    #[test]
    fn typing_and_submit() {
        let (line, step) = run(b"hello\r");
        assert_eq!(line, "hello");
        assert_eq!(step, "submit");
        // LF submits too (what a pty write of "\n" delivers).
        assert_eq!(run(b"hi\n").1, "submit");
    }

    #[test]
    fn backspace_deletes_a_whole_multibyte_char() {
        // "café" then one backspace removes the whole 'é' (2 bytes), not a byte.
        let mut ed = Editor::default();
        for k in keys("café".as_bytes()) {
            ed.apply(k);
        }
        assert_eq!(ed.line(), "café");
        ed.apply(Key::Backspace);
        assert_eq!(ed.line(), "caf");
        assert_eq!(ed.cursor, 3);
    }

    #[test]
    fn cursor_moves_and_mid_line_insert() {
        let mut ed = Editor::default();
        for k in keys(b"ac") {
            ed.apply(k);
        }
        ed.apply(Key::Left); // between a and c
        ed.apply(Key::Char('b'));
        assert_eq!(ed.line(), "abc");
        // Left past the start clamps; Right past the end clamps.
        for _ in 0..9 {
            ed.apply(Key::Left);
        }
        assert_eq!(ed.cursor, 0);
        for _ in 0..9 {
            ed.apply(Key::Right);
        }
        assert_eq!(ed.cursor, 3);
    }

    #[test]
    fn home_end_and_line_kills() {
        let mut ed = Editor::default();
        for k in keys(b"hello world") {
            ed.apply(k);
        }
        ed.apply(Key::Home);
        assert_eq!(ed.cursor, 0);
        ed.apply(Key::End);
        assert_eq!(ed.cursor, 11);
        // Ctrl-U kills to start from the cursor.
        ed.apply(Key::Left); // before 'd'
        ed.apply(Key::KillToStart);
        assert_eq!(ed.line(), "d");
        // Ctrl-K kills to end.
        let mut ed = Editor::default();
        for k in keys(b"keep drop") {
            ed.apply(k);
        }
        ed.apply(Key::Home);
        for _ in 0..4 {
            ed.apply(Key::Right);
        }
        ed.apply(Key::KillToEnd);
        assert_eq!(ed.line(), "keep");
    }

    #[test]
    fn kill_word_removes_the_word_before_the_cursor() {
        let mut ed = Editor::default();
        for k in keys(b"alpha beta gamma") {
            ed.apply(k);
        }
        ed.apply(Key::KillWord);
        assert_eq!(ed.line(), "alpha beta ");
        ed.apply(Key::KillWord);
        assert_eq!(ed.line(), "alpha ");
    }

    #[test]
    fn control_bytes_map_to_editing_keys() {
        assert_eq!(keys(&[0x7f]), vec![Key::Backspace]);
        assert_eq!(keys(&[0x08]), vec![Key::Backspace]);
        assert_eq!(keys(&[0x01]), vec![Key::Home]);
        assert_eq!(keys(&[0x05]), vec![Key::End]);
        assert_eq!(keys(&[0x15]), vec![Key::KillToStart]);
        assert_eq!(keys(&[0x0b]), vec![Key::KillToEnd]);
        assert_eq!(keys(&[0x17]), vec![Key::KillWord]);
        assert_eq!(keys(&[0x03]), vec![Key::Interrupt]);
        assert_eq!(keys(&[0x04]), vec![Key::Eof]);
        // Tab and other stray control bytes are dropped, not inserted.
        assert_eq!(keys(&[0x09]), vec![]);
    }

    #[test]
    fn arrow_home_end_delete_escapes() {
        assert_eq!(keys(b"\x1b[C"), vec![Key::Right]);
        assert_eq!(keys(b"\x1b[D"), vec![Key::Left]);
        assert_eq!(keys(b"\x1b[H"), vec![Key::Home]);
        assert_eq!(keys(b"\x1b[F"), vec![Key::End]);
        assert_eq!(keys(b"\x1b[1~"), vec![Key::Home]);
        assert_eq!(keys(b"\x1b[4~"), vec![Key::End]);
        assert_eq!(keys(b"\x1b[3~"), vec![Key::Delete]);
        // SS3-introduced arrows (application cursor mode) decode too.
        assert_eq!(keys(b"\x1bOC"), vec![Key::Right]);
        // Up/Down are recognized but unbound: dropped, never inserted.
        assert_eq!(keys(b"\x1b[A"), vec![]);
        assert_eq!(keys(b"\x1b[B"), vec![]);
    }

    #[test]
    fn interrupt_and_eof_are_distinct() {
        assert_eq!(run(b"\x03").1, "interrupt");
        // Ctrl-D on an empty line is EOF; with text it is a no-op.
        assert_eq!(run(b"\x04").1, "eof");
        let (line, step) = run(b"typed\x04");
        assert_eq!(line, "typed");
        assert_eq!(step, "continue");
    }

    #[test]
    fn bracketed_paste_holds_newlines_until_a_real_enter() {
        // A pasted multi-line block: its newlines are literal text, no submit.
        let (line, step) = run(b"\x1b[200~one\ntwo\x1b[201~");
        assert_eq!(line, "one\ntwo");
        assert_eq!(step, "continue");
        // A real Enter after the paste submits the whole thing.
        let (line, step) = run(b"\x1b[200~a\nb\x1b[201~\r");
        assert_eq!(line, "a\nb");
        assert_eq!(step, "submit");
        // CRLF inside a paste collapses to one newline.
        assert_eq!(run(b"\x1b[200~x\r\ny\x1b[201~").0, "x\ny");
    }

    #[test]
    fn crlf_split_across_feeds_in_paste_collapses_to_one_newline() {
        // The CRLF straddles a read boundary (CR is the last byte of feed 1);
        // it must be held so the LF starting feed 2 collapses into it rather
        // than emitting a second newline.
        let mut dec = Decoder::default();
        let k1 = dec.feed(b"\x1b[200~x\r");
        let k2 = dec.feed(b"\ny\x1b[201~");
        let mut ed = Editor::default();
        for k in k1.into_iter().chain(k2) {
            ed.apply(k);
        }
        assert_eq!(ed.line(), "x\ny", "CRLF split across feeds doubled the newline");
    }

    #[test]
    fn ctrl_c_and_ctrl_d_break_out_of_an_unterminated_paste() {
        // A paste-start with no terminator must never wedge the editor: with
        // ISIG off, Ctrl-C is the only way out, so it must reach the editor
        // even mid-paste.
        assert_eq!(run(b"\x1b[200~hello\x03").1, "interrupt");
        assert_eq!(Decoder::default().feed(b"\x1b[200~\x04"), vec![Key::Eof]);
    }

    #[test]
    fn escape_bytes_inside_a_paste_are_kept_literally() {
        // Pasted content that contains a raw escape keeps every byte; only the
        // paste-end terminator is honored, so nothing is silently deleted.
        assert_eq!(run(b"\x1b[200~a\x1b[Db\x1b[201~").0, "a\x1b[Db");
    }

    #[test]
    fn malformed_csi_recovers_instead_of_growing_pending() {
        // A long run with no CSI final byte is dropped past the cap; the
        // decoder recovers and a following key is still seen (pending was not
        // left holding an unbounded junk sequence).
        let mut dec = Decoder::default();
        let mut body = Vec::from(&b"\x1b["[..]);
        body.extend(std::iter::repeat(b'0').take(200));
        let ks = dec.feed(&body);
        assert!(ks.iter().any(|k| matches!(k, Key::Char('0'))), "did not recover: {ks:?}");
        assert_eq!(dec.feed(b"a\r"), vec![Key::Char('a'), Key::Enter]);
    }

    #[test]
    fn split_escape_across_feeds_is_reassembled() {
        let mut dec = Decoder::default();
        assert_eq!(dec.feed(b"\x1b["), vec![]); // incomplete, carried
        assert_eq!(dec.feed(b"C"), vec![Key::Right]);
        // A paste terminator split across feeds still ends the paste.
        let mut dec = Decoder::default();
        assert_eq!(dec.feed(b"\x1b[200~ab"), vec![Key::Char('a'), Key::Char('b')]);
        assert_eq!(dec.feed(b"\x1b[20"), vec![]); // incomplete terminator
        assert_eq!(dec.feed(b"1~cd"), vec![Key::Char('c'), Key::Char('d')]);
        // 'c'/'d' are outside the paste now, so a newline would submit.
    }

    #[test]
    fn split_multibyte_char_across_feeds() {
        let bytes = "é".as_bytes(); // two bytes
        let mut dec = Decoder::default();
        assert_eq!(dec.feed(&bytes[..1]), vec![]); // first byte carried
        assert_eq!(dec.feed(&bytes[1..]), vec![Key::Char('é')]);
    }

    #[test]
    fn enter_ends_a_batch_and_the_editor_stops_there() {
        // Everything up to Enter is the line; the loop returns on Submit.
        let mut ed = Editor::default();
        let mut submitted = None;
        for k in keys(b"ab\rcd") {
            if let Step::Submit = ed.apply(k) {
                submitted = Some(ed.line());
                break;
            }
        }
        assert_eq!(submitted.as_deref(), Some("ab"));
    }

    #[test]
    fn box_top_rule_spans_the_width_and_is_bracketed() {
        // The rule fills the terminal exactly: ╭ + dashes + ╮ == width glyphs.
        let r = box_top_rule(false, 80);
        assert!(r.starts_with('╭') && r.ends_with('╮'));
        assert_eq!(r.chars().count(), 80, "rule must span the full width");
        assert_eq!(r.chars().filter(|&c| c == '─').count(), 78);
        // Plan mode keeps the label and still fills the width.
        let p = box_top_rule(true, 120);
        assert!(p.starts_with("╭─ plan "));
        assert_eq!(p.chars().count(), 120);
    }

    #[test]
    fn input_window_keeps_the_line_to_one_row_and_holds_the_cursor() {
        // Short line: the whole buffer shows and the cursor is where it is.
        let buf: Vec<char> = "hello world".chars().collect();
        let (shown, cur) = input_window(&buf, 5, 20);
        assert_eq!(shown, "hello world");
        assert_eq!(cur, 5);
        // Long line at any width never exceeds `avail` cells (so it cannot wrap),
        // and the cursor stays inside the window at every position.
        let long: Vec<char> = (0..200u32).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        for &avail in &[1usize, 5, 16, 40] {
            for cursor in [0, 1, 50, 199, 200] {
                let (shown, cur) = input_window(&long, cursor, avail);
                assert!(shown.chars().count() <= avail, "window exceeds avail {avail}: {shown:?}");
                assert!(cur < avail, "cursor {cur} not inside window (avail {avail})");
                assert!(cur <= shown.chars().count(), "cursor past the shown text");
            }
        }
    }

    #[test]
    fn input_window_shows_control_chars_as_spaces() {
        // A pasted newline (or tab) in the buffer must render as a space so the
        // single input row never wraps and the cursor math stays right.
        let buf: Vec<char> = vec!['a', '\n', 'b', '\t', 'c'];
        let (shown, _) = input_window(&buf, 5, 20);
        assert_eq!(shown, "a b c");
    }

    #[test]
    fn large_paste_stays_linear_enough() {
        // Guard against an accidental blow-up in the decode path: a big paste
        // decodes without hanging (kept well within a paste a human would do).
        let mut body = Vec::from(&b"\x1b[200~"[..]);
        body.extend(std::iter::repeat(b'x').take(20_000));
        body.extend_from_slice(b"\x1b[201~");
        let ks = Decoder::default().feed(&body);
        assert_eq!(ks.len(), 20_000);
        assert!(ks.iter().all(|k| matches!(k, Key::Char('x'))));
    }
}
