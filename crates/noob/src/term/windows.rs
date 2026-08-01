//! The Windows console arm of the term contract. Raw mode is the pair of
//! console modes (raw keys in, VT processing out), keys arrive as input
//! records translated to the same byte language the shared decoder speaks,
//! and a resize is a WINDOW_BUFFER_SIZE event rather than a signal, so the
//! reader learns about it on the same channel as everything else.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use noob_provider::http::INTERRUPTED;

use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Console::{
    CONSOLE_SCREEN_BUFFER_INFO, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetConsoleScreenBufferInfo, GetStdHandle,
    INPUT_RECORD, KEY_EVENT, ReadConsoleInputW, SetConsoleCtrlHandler, SetConsoleMode,
    STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, WINDOW_BUFFER_SIZE_EVENT,
};
use windows_sys::Win32::System::Threading::{ExitProcess, WaitForSingleObject};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_DELETE, VK_DOWN, VK_END, VK_HOME, VK_LEFT, VK_RIGHT, VK_UP,
};

use super::{StdinRead, WINCH};

fn stdin_handle() -> HANDLE {
    unsafe { GetStdHandle(STD_INPUT_HANDLE) }
}

fn stdout_handle() -> HANDLE {
    unsafe { GetStdHandle(STD_OUTPUT_HANDLE) }
}

/// The cooked modes, saved while raw is active so every restore hook can put
/// them back. Zero is never a real saved state (a console always has some
/// mode bits), so it doubles as "nothing to restore".
static SAVED_IN: AtomicU32 = AtomicU32::new(0);
static SAVED_OUT: AtomicU32 = AtomicU32::new(0);
static ACTIVE: AtomicBool = AtomicBool::new(false);

pub(crate) struct RawGuard;

impl RawGuard {
    pub(crate) fn enter() -> Option<RawGuard> {
        unsafe {
            let hin = stdin_handle();
            let hout = stdout_handle();
            let mut min = 0u32;
            let mut mout = 0u32;
            if GetConsoleMode(hin, &mut min) == 0 || GetConsoleMode(hout, &mut mout) == 0 {
                return None; // not a console (piped): the cooked path serves
            }
            SAVED_IN.store(min, Ordering::SeqCst);
            SAVED_OUT.store(mout, Ordering::SeqCst);
            ACTIVE.store(true, Ordering::SeqCst);
            // Raw keys: no line buffering, no echo, and no PROCESSED_INPUT so
            // Ctrl-C arrives as the 0x03 byte the decoder maps, exactly as
            // ISIG off does on unix.
            let raw_in = min & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT);
            if SetConsoleMode(hin, raw_in) == 0 {
                restore_terminal();
                return None;
            }
            // VT processing out, so the dock's escape sequences draw instead
            // of printing.
            SetConsoleMode(hout, mout | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
        Some(RawGuard)
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Restore the cooked console modes. Idempotent and callable from the ctrl
/// handler; only atomics and SetConsoleMode.
pub(crate) fn restore_terminal() {
    if ACTIVE.swap(false, Ordering::SeqCst) {
        unsafe {
            SetConsoleMode(stdin_handle(), SAVED_IN.load(Ordering::SeqCst));
            SetConsoleMode(stdout_handle(), SAVED_OUT.load(Ordering::SeqCst));
        }
    }
}

fn screen() -> Option<CONSOLE_SCREEN_BUFFER_INFO> {
    unsafe {
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
        (GetConsoleScreenBufferInfo(stdout_handle(), &mut info) != 0).then_some(info)
    }
}

/// Terminal width in columns; 80 when unavailable, floor 20, like the unix
/// arm.
pub(crate) fn term_width() -> usize {
    screen()
        .map(|i| (i.srWindow.Right - i.srWindow.Left + 1).max(0) as usize)
        .filter(|&w| w > 0)
        .map(|w| w.max(20))
        .unwrap_or(80)
}

/// Terminal height in rows; 24 when unavailable.
pub(crate) fn term_height() -> usize {
    screen()
        .map(|i| (i.srWindow.Bottom - i.srWindow.Top + 1).max(0) as usize)
        .filter(|&h| h > 0)
        .unwrap_or(24)
}

/// First Ctrl-C sets the shared flag (PROCESSED_INPUT is off while the raw
/// editor runs, so this path serves the cooked surfaces and external
/// CTRL_CLOSE style events); a second hard-exits with the console restored.
pub(crate) fn install_sigint_handler() {
    unsafe extern "system" fn on_ctrl(_kind: u32) -> i32 {
        if INTERRUPTED.swap(true, Ordering::SeqCst) {
            restore_terminal();
            unsafe { ExitProcess(130) };
        }
        1
    }
    unsafe {
        SetConsoleCtrlHandler(Some(on_ctrl), 1);
    }
}

/// No SIGWINCH on Windows: the resize arrives as a console input record and
/// the reader turns it into the same WINCH flag.
pub(crate) fn install_sigwinch_handler() {}

/// The unix arm unblocks the signal on the reader thread; here there is
/// nothing to unblock.
pub(crate) fn unblock_sigwinch() {}

/// One blocking read from the console, classified for the reader loop. Key
/// records become the byte language the shared decoder speaks: printable
/// UTF-16 re-encodes to UTF-8 (surrogate pairs included), and the navigation
/// keys become the CSI sequences the decoder already maps. A buffer-size
/// record sets WINCH and reports Interrupted, so the reader's resize path is
/// the same on every platform.
pub(crate) fn read_stdin(buf: &mut [u8]) -> StdinRead {
    let hin = stdin_handle();
    loop {
        match unsafe { WaitForSingleObject(hin, 200) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => {
                if INTERRUPTED.load(Ordering::SeqCst) || WINCH.load(Ordering::SeqCst) {
                    return StdinRead::Interrupted;
                }
                continue;
            }
            _ => return StdinRead::Gone,
        }
        let mut records: [INPUT_RECORD; 16] = unsafe { std::mem::zeroed() };
        let mut got = 0u32;
        if unsafe { ReadConsoleInputW(hin, records.as_mut_ptr(), 16, &mut got) } == 0 {
            return StdinRead::Gone;
        }
        let mut wrote = 0usize;
        let mut pending_high: Option<u16> = None;
        for record in records.iter().take(got as usize) {
            match record.EventType as u32 {
                WINDOW_BUFFER_SIZE_EVENT => {
                    WINCH.store(true, Ordering::SeqCst);
                }
                KEY_EVENT => {
                    let key = unsafe { record.Event.KeyEvent };
                    if key.bKeyDown == 0 {
                        continue;
                    }
                    let unit = unsafe { key.uChar.UnicodeChar };
                    if unit != 0 {
                        // A surrogate pair spans two records; hold the high
                        // half until its partner arrives.
                        let mut push = |c: char| {
                            let s = c.encode_utf8(&mut [0u8; 4]).len();
                            if wrote + s <= buf.len() {
                                c.encode_utf8(&mut buf[wrote..]);
                                wrote += s;
                            }
                        };
                        if (0xD800..0xDC00).contains(&unit) {
                            pending_high = Some(unit);
                        } else if let Some(high) = pending_high.take() {
                            if let Some(c) = char::from_u32(
                                0x10000
                                    + ((high as u32 - 0xD800) << 10)
                                    + (unit as u32 - 0xDC00),
                            ) {
                                push(c);
                            }
                        } else if let Some(c) = char::from_u32(unit as u32) {
                            push(c);
                        }
                    } else {
                        let seq: &[u8] = match key.wVirtualKeyCode {
                            k if k == VK_LEFT => b"\x1b[D",
                            k if k == VK_RIGHT => b"\x1b[C",
                            k if k == VK_UP => b"\x1b[A",
                            k if k == VK_DOWN => b"\x1b[B",
                            k if k == VK_HOME => b"\x1b[H",
                            k if k == VK_END => b"\x1b[F",
                            k if k == VK_DELETE => b"\x1b[3~",
                            _ => b"",
                        };
                        if wrote + seq.len() <= buf.len() {
                            buf[wrote..wrote + seq.len()].copy_from_slice(seq);
                            wrote += seq.len();
                        }
                    }
                }
                _ => {}
            }
        }
        if WINCH.load(Ordering::SeqCst) && wrote == 0 {
            return StdinRead::Interrupted;
        }
        if wrote > 0 {
            return StdinRead::Data(wrote);
        }
        // Only focus or mouse records: wait again.
    }
}

/// Poll the console for `grace_ms`, with the raw poll shape the unix arm
/// returns: positive when input waits, 0 on silence, negative on error.
pub(crate) fn poll_stdin(grace_ms: i32) -> i32 {
    match unsafe { WaitForSingleObject(stdin_handle(), grace_ms.max(0) as u32) } {
        WAIT_OBJECT_0 => 1,
        WAIT_TIMEOUT => 0,
        _ => -1,
    }
}
