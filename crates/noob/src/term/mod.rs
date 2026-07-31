//! The terminal backend: raw mode with guaranteed restore, the byte-to-key
//! decoder, window size, and the two signal handlers. Every terminal and
//! signal libc call the CLI makes lives here; the dock and the editor build
//! their behavior on this surface. This is the seam a second platform's
//! console implementation stands behind.

use std::sync::atomic::{AtomicBool, Ordering};

use noob_provider::http::INTERRUPTED;

/// Set by the SIGWINCH handler, consumed by the dock's reader thread when its
/// read returns EINTR. The signal is blocked in every thread except the
/// reader, so it always interrupts the read and never races another blocking
/// call. Async-signal-safe: the handler only stores this flag.
pub(crate) static WINCH: AtomicBool = AtomicBool::new(false);

/// One editing action, already decoded from the raw byte stream.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Key {
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
    /// The Tab key. The pure editor ignores it (a no-op, so a `/`-prefixed
    /// command never gets a literal tab); the raw reader intercepts it before
    /// `apply` to run slash-command completion on the draft.
    Tab,
    Interrupt,
    Eof,
    /// A lone ESC press (not the start of a sequence). Only ever produced
    /// by [`Decoder::flush_dangling_esc`], because the byte stream alone
    /// cannot distinguish it from an escape sequence still in flight; the
    /// dock's cancel state machine consumes it, the line editor ignores it.
    Esc,
}

/// Terminal width in columns via the window-size ioctl; 80 when it is
/// unavailable (a startup pty that has not been sized yet reports 0). The box
/// spans the full width, so no upper clamp.
pub(crate) fn term_width() -> usize {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
            (ws.ws_col as usize).max(20)
        } else {
            80
        }
    }
}

/// Terminal height in rows via the window-size ioctl; 24 when it is unavailable
/// (a startup pty that has not been sized yet reports 0). Used only to bound the
/// dock's pinned regions so the live frame never grows taller than the screen,
/// where the relative cursor moves would clamp at the top edge and desync.
pub(crate) fn term_height() -> usize {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_row > 0 {
            ws.ws_row as usize
        } else {
            24
        }
    }
}

// ---------------------------------------------------------------------------
// The decoder: raw bytes -> keys. Stateful only for sequences that can split
// across reads (an incomplete escape or multibyte char) and for bracketed
// paste. Pure and unit-testable.
// ---------------------------------------------------------------------------

#[derive(Default)]
pub(crate) struct Decoder {
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
    /// True when the carried bytes are exactly one bare ESC: the reader
    /// cannot tell a human ESC press from a sequence whose tail is still
    /// in flight, so it polls stdin briefly and, on silence, flushes.
    pub(crate) fn has_dangling_esc(&self) -> bool {
        self.pending == [0x1b]
    }

    /// Resolve a dangling lone ESC after the reader's grace poll timed
    /// out: outside a paste it is the ESC key; inside a paste it is
    /// literal content (escape bytes in a paste are always kept).
    /// A no-op unless [`Self::has_dangling_esc`].
    pub(crate) fn flush_dangling_esc(&mut self) -> Option<Key> {
        if !self.has_dangling_esc() {
            return None;
        }
        self.pending.clear();
        Some(if self.paste {
            Key::Char('\u{1b}')
        } else {
            Key::Esc
        })
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Vec<Key> {
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
                0x09 => keys.push(Key::Tab),
                b if b < 0x20 => {} // ignore other control bytes
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
    if intro == 0x1b {
        // Two rapid human ESC presses can arrive in one read. Emit the first
        // now and leave the second for the normal dangling-ESC grace path;
        // treating ESC+ESC as an unknown chord would silently lose one tap.
        return Some((EscKind::Key(Key::Esc), 1));
    }
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
// can restore it too if a fault occurs outside the guarded editor lifetime.
// ---------------------------------------------------------------------------

/// Restore the terminal to cooked mode if the editor is active. Safe to call
/// from a signal handler: only atomics, `tcsetattr`, and `write`, no
/// allocation. Idempotent, so whichever of the three hooks fires first wins and
/// the rest are no-ops.
pub(crate) fn restore_terminal() {
    raw_state::restore();
}

pub(crate) struct RawGuard;

impl RawGuard {
    pub(crate) fn enter() -> Option<RawGuard> {
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
/// previous hook so the panic message still prints. This is a backstop for
/// faults outside the worker boundaries; normal unwinding also runs RawGuard.
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

/// First Ctrl-C sets the watchdog flag (the in-flight request aborts within
/// one tick); a second Ctrl-C hard-exits. Only async-signal-safe calls here.
pub(crate) fn install_sigint_handler() {
    extern "C" fn on_sigint(_: libc::c_int) {
        if INTERRUPTED.swap(true, Ordering::SeqCst) {
            // A second Ctrl-C hard-exits; restore the terminal first so a raw
            // editor session does not leave the shell garbled. Restore touches
            // only atomics, tcsetattr, and write, all async-signal-safe.
            restore_terminal();
            unsafe { libc::_exit(130) };
        }
    }
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_sigint as *const () as usize;
        // No SA_RESTART: blocked reads return EINTR so the tick loop sees the
        // flag immediately instead of after the socket timeout.
        sa.sa_flags = 0;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
    }
}

/// A terminal resize flips a flag the dock's stdin reader consumes on EINTR, so
/// an idle prompt reflows its box to the new width without a keystroke. SIGWINCH
/// is blocked in this (main) thread and therefore in every thread spawned after
/// this call, so the only thread that can catch it is the reader, which unblocks
/// it for itself: that guarantees the signal interrupts the read rather than
/// racing an unrelated blocking call. Cheap and event-driven: no idle polling.
pub(crate) fn install_sigwinch_handler() {
    extern "C" fn on_sigwinch(_: libc::c_int) {
        WINCH.store(true, Ordering::SeqCst);
    }
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_sigwinch as *const () as usize;
        // No SA_RESTART: the reader's blocked read returns EINTR and injects the
        // resize event instead of resuming as if nothing happened.
        sa.sa_flags = 0;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGWINCH, &sa, std::ptr::null_mut());
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGWINCH);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}

/// Unblock SIGWINCH on the calling thread. The dock's reader calls this so
/// its blocking read is the one place the resize signal lands.
pub(crate) fn unblock_sigwinch() {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGWINCH);
        libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
    }
}

/// One blocking read from stdin, classified for a reader loop.
pub(crate) enum StdinRead {
    Data(usize),
    /// Genuine end of the input stream, not a Ctrl-D byte.
    Eof,
    /// EINTR: a signal landed; check the flags and read again.
    Interrupted,
    /// A real read error (a closed or broken tty).
    Gone,
}

pub(crate) fn read_stdin(buf: &mut [u8]) -> StdinRead {
    let n = unsafe {
        libc::read(
            libc::STDIN_FILENO,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len(),
        )
    };
    if n < 0 {
        if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            StdinRead::Interrupted
        } else {
            StdinRead::Gone
        }
    } else if n == 0 {
        StdinRead::Eof
    } else {
        StdinRead::Data(n as usize)
    }
}

/// Poll stdin for `grace_ms`; the raw poll return (0 on silence, positive
/// when bytes are waiting, negative on error) so the caller's policy sees
/// exactly what poll saw.
pub(crate) fn poll_stdin(grace_ms: i32) -> i32 {
    let mut pfd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut pfd, 1, grace_ms) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(bytes: &[u8]) -> Vec<Key> {
        Decoder::default().feed(bytes)
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
    fn malformed_csi_recovers_instead_of_growing_pending() {
        // A long run with no CSI final byte is dropped past the cap; the
        // decoder recovers and a following key is still seen (pending was not
        // left holding an unbounded junk sequence).
        let mut dec = Decoder::default();
        let mut body = Vec::from(&b"\x1b["[..]);
        body.extend(std::iter::repeat_n(b'0', 200));
        let ks = dec.feed(&body);
        assert!(
            ks.iter().any(|k| matches!(k, Key::Char('0'))),
            "did not recover: {ks:?}"
        );
        assert_eq!(dec.feed(b"a\r"), vec![Key::Char('a'), Key::Enter]);
    }

    #[test]
    fn two_esc_bytes_in_one_read_remain_two_cancel_taps() {
        let mut dec = Decoder::default();
        assert_eq!(dec.feed(b"\x1b\x1b"), vec![Key::Esc]);
        assert!(dec.has_dangling_esc());
        assert_eq!(dec.flush_dangling_esc(), Some(Key::Esc));
    }

    #[test]
    fn split_escape_across_feeds_is_reassembled() {
        let mut dec = Decoder::default();
        assert_eq!(dec.feed(b"\x1b["), vec![]); // incomplete, carried
        assert_eq!(dec.feed(b"C"), vec![Key::Right]);
        // A paste terminator split across feeds still ends the paste.
        let mut dec = Decoder::default();
        assert_eq!(
            dec.feed(b"\x1b[200~ab"),
            vec![Key::Char('a'), Key::Char('b')]
        );
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
}
