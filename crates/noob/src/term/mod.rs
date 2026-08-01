//! The terminal backend: raw mode with guaranteed restore, the byte-to-key
//! decoder, window size, and the two signal handlers. Every terminal and
//! signal libc call the CLI makes lives here; the dock and the editor build
//! their behavior on this surface. This is the seam a second platform's
//! console implementation stands behind.

use std::sync::atomic::AtomicBool;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::*;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;

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
