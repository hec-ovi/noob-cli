# term

contractVersion: 1.0.0

## Purpose

The terminal backend: raw mode with guaranteed restore, bytes to keys,
window size, stdin reads, and the two signal handlers. Every terminal and
signal libc call the CLI makes lives here; this is the seam a second
platform's console implementation stands behind.

## Public surface

```rust
pub(crate) enum Key;            // Char, Backspace, Delete, Left, Right,
                                // Home, End, the three kills, Enter, Tab,
                                // Interrupt, Eof, Esc
pub(crate) struct Decoder;      // stateful bytes -> keys
impl Decoder {
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Vec<Key>;
    pub(crate) fn has_dangling_esc(&self) -> bool;
    pub(crate) fn flush_dangling_esc(&mut self) -> Option<Key>;
}

pub(crate) struct RawGuard;     // raw mode for its lifetime
impl RawGuard { pub(crate) fn enter() -> Option<RawGuard> }
pub(crate) fn restore_terminal();   // async-signal-safe, idempotent

pub(crate) fn term_width() -> usize;    // 80 when unavailable, floor 20
pub(crate) fn term_height() -> usize;   // 24 when unavailable

pub(crate) fn install_sigint_handler();
pub(crate) fn install_sigwinch_handler();
pub(crate) static WINCH: AtomicBool;    // set by SIGWINCH, consumed by the
                                        // dock's reader on EINTR
pub(crate) fn unblock_sigwinch();       // reader thread only
pub(crate) enum StdinRead { Data(usize), Eof, Interrupted, Gone }
pub(crate) fn read_stdin(buf: &mut [u8]) -> StdinRead;
pub(crate) fn poll_stdin(grace_ms: i32) -> i32;   // raw poll return
```

## The capability, platform-neutral

- Keys arrive decoded whatever the read chunking: escape sequences, CRLF,
  and multibyte characters split across reads reassemble; a bracketed paste
  delivers its newlines as text; a lone ESC is distinguishable from a
  sequence in flight via the dangling-ESC grace protocol.
- Raw mode always ends: normal drop, panic, and a second Ctrl-C all restore
  the cooked terminal, so no exit path leaves the shell garbled.
- A terminal resize wakes the input path without a keystroke; a first
  Ctrl-C interrupts work, a second hard-exits with the terminal restored.
- Width and height are always answerable, with stated fallbacks.

The implementation here is unix: termios raw mode, TIOCGWINSZ, sigaction
without SA_RESTART so blocked reads see EINTR, SIGWINCH blocked everywhere
except the one reader thread that opts in.

## Errors

`RawGuard::enter` returns `None` when stdin is not a terminal it can
configure; `StdinRead` is the closed classification of a read. Nothing else
fails: the decoder drops what it cannot classify and always makes progress,
and size fns fall back rather than error.

## Invariants

1. Decoder state is bounded: a stream that never completes an escape
   sequence cannot grow the carried bytes past the parameter cap.
2. Restore is idempotent and async-signal-safe (atomics, `tcsetattr`,
   `write` only), so whichever of the three hooks fires first wins.
3. Bracketed paste can never wedge the editor: Ctrl-C and Ctrl-D break out
   even when the terminator never arrives.
4. Inside a paste, escape bytes are literal content; only the terminator is
   control.

## Dependencies

Contracts: [`crates/noob-provider/CONTRACT.md`](../../../noob-provider/CONTRACT.md)
(the shared interrupt flag the SIGINT handler sets). The editor, the dock's
reader policy, and rendering stay in the ui box; they consume this surface.

## Tests

Inline: the decoder's sequence, split, and recovery cases. The raw-mode and
signal behavior is proven end to end by the pty suites
(`crates/noob/tests/ui_editor.rs`, `ui_dock.rs`, `ui_screen.rs`), which
drive the real binary through a pseudo terminal and assert exact screens.
