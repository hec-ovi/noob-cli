//! The unix arm of the term contract: termios raw mode with the three
//! restore hooks, SIGINT and SIGWINCH, the window-size ioctl, and classified
//! stdin reads.

use std::sync::atomic::Ordering;

use noob_provider::http::INTERRUPTED;

use super::{StdinRead, WINCH};


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

