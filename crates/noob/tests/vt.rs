//! A tiny, test-only terminal SCREEN emulator, hand-rolled (zero new crates).
//!
//! noob draws its bottom dock with RELATIVE cursor moves and never tracks the
//! cursor row: it relies on the terminal to scroll on newline (see
//! `ui/dock.rs`). The existing PTY tests only assert on the raw output BYTES,
//! so a scroll-at-bottom cursor-math desync is invisible to them. This grid
//! replays noob's exact captured bytes into a fixed rows x cols screen and
//! lets a test inspect what a human would actually see, dock and all.
//!
//! Scope: enough of xterm to render noob's own output faithfully, nothing more.
//!  - printable UTF-8: one column per character (continuation bytes are free),
//!    the same single-width simplification the dock's own tracker makes.
//!  - DECAWM deferred wrap (the xterm default and exactly what noob's tracker
//!    assumes): a glyph in the last column parks a pending-wrap latch; the NEXT
//!    glyph wraps to column 0 of the next row. Any cursor move or CR/LF clears
//!    the latch. Modeling the real deferred wrap (rather than a naive clamp)
//!    matters, because a mismatch there would manufacture a desync that noob
//!    does not actually have.
//!  - LF is treated as CR+LF (carriage return + line feed + scroll-at-bottom).
//!    The slave pty runs with OPOST|ONLCR (openpty's default; noob's RawGuard
//!    only touches the input/local flags), so a bare `\n` written by noob is
//!    turned into `\r\n` before the master sees it, and noob's own column
//!    tracker treats `\n` as a fresh column-0 row. Making LF reset the column
//!    reproduces that and is robust whether the captured bytes carry `\n` or
//!    an already-translated `\r\n`.
//!  - CSI cursor moves A/B/C/D (clamped), G (CHA, 1-based column), H/f (CUP),
//!    erases K/0K/1K/2K and J/0J/2J, and every other CSI (SGR `m`, private
//!    `?...h/l`, ...) parsed to its final byte and ignored. OSC (`ESC ] ... BEL`
//!    or `ESC ] ... ST`) is swallowed.
//!
//! `render()` returns the visible rows with trailing blanks trimmed.

#![allow(dead_code)]

pub struct Vt {
    rows: usize,
    cols: usize,
    grid: Vec<Vec<char>>,
    row: usize,
    col: usize,
    /// DECAWM deferred-wrap latch: the last glyph filled the final column and
    /// the cursor is parked there; the next glyph wraps first.
    wrap_pending: bool,
    state: State,
    /// CSI parameter/intermediate bytes accumulated until the final byte.
    csi: Vec<u8>,
    /// In-flight UTF-8 lead+continuation bytes for one printable character.
    utf8: Vec<u8>,
    utf8_left: usize,
}

enum State {
    Ground,
    Esc,
    Csi,
    Osc,
    /// Inside an OSC string, saw ESC: a potential `ESC \` (ST) terminator.
    OscEsc,
}

impl Vt {
    pub fn new(rows: usize, cols: usize) -> Vt {
        Vt {
            rows,
            cols,
            grid: vec![vec![' '; cols]; rows],
            row: 0,
            col: 0,
            wrap_pending: false,
            state: State::Ground,
            csi: Vec::new(),
            utf8: Vec::new(),
            utf8_left: 0,
        }
    }

    /// Apply a run of bytes to the screen. Safe on any input; never panics.
    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match self.state {
                State::Ground => self.ground(b),
                State::Esc => self.esc(b),
                State::Csi => self.csi(b),
                State::Osc => match b {
                    0x07 => self.state = State::Ground,
                    0x1b => self.state = State::OscEsc,
                    _ => {}
                },
                State::OscEsc => match b {
                    b'\\' | 0x07 => self.state = State::Ground,
                    0x1b => {}
                    _ => self.state = State::Osc,
                },
            }
        }
    }

    fn ground(&mut self, b: u8) {
        if self.utf8_left > 0 {
            if b & 0xc0 == 0x80 {
                self.utf8.push(b);
                self.utf8_left -= 1;
                if self.utf8_left == 0 {
                    let ch = std::str::from_utf8(&self.utf8)
                        .ok()
                        .and_then(|s| s.chars().next())
                        .unwrap_or('\u{fffd}');
                    self.utf8.clear();
                    self.put(ch);
                }
                return;
            }
            // A malformed sequence: abandon the partial char and reprocess.
            self.utf8.clear();
            self.utf8_left = 0;
        }
        match b {
            0x1b => self.state = State::Esc,
            b'\r' => {
                self.col = 0;
                self.wrap_pending = false;
            }
            b'\n' => self.line_feed(),
            0x08 => {
                self.col = self.col.saturating_sub(1);
                self.wrap_pending = false;
            }
            _ if b < 0x20 || b == 0x7f => {} // other C0 / DEL: zero width
            _ if b < 0x80 => self.put(b as char),
            _ => {
                // UTF-8 lead byte.
                let len = utf8_len(b);
                if len <= 1 {
                    self.put('\u{fffd}');
                } else {
                    self.utf8.push(b);
                    self.utf8_left = len - 1;
                }
            }
        }
    }

    fn esc(&mut self, b: u8) {
        match b {
            b'[' => {
                self.csi.clear();
                self.state = State::Csi;
            }
            b']' => self.state = State::Osc,
            _ => self.state = State::Ground, // two-byte escape (ESC 7, ESC =, ...)
        }
    }

    fn csi(&mut self, b: u8) {
        if (0x40..=0x7e).contains(&b) {
            self.dispatch_csi(b);
            self.state = State::Ground;
        } else if self.csi.len() < 64 {
            self.csi.push(b);
        } else {
            // Malformed run past the cap: give up and return to text.
            self.state = State::Ground;
        }
    }

    fn dispatch_csi(&mut self, fin: u8) {
        // Ignore private sequences (`?...h/l` etc.) wholesale.
        if self.csi.first() == Some(&b'?') {
            return;
        }
        let params = String::from_utf8_lossy(&self.csi);
        let nums: Vec<usize> = params
            .split(';')
            .map(|p| p.parse::<usize>().unwrap_or(0))
            .collect();
        let n1 = nums.first().copied().filter(|&n| n > 0).unwrap_or(1);
        match fin {
            b'A' => {
                self.wrap_pending = false;
                self.row = self.row.saturating_sub(n1);
            }
            b'B' => {
                self.wrap_pending = false;
                self.row = (self.row + n1).min(self.rows - 1);
            }
            b'C' => {
                self.wrap_pending = false;
                self.col = (self.col + n1).min(self.cols - 1);
            }
            b'D' => {
                self.wrap_pending = false;
                self.col = self.col.saturating_sub(n1);
            }
            b'G' => {
                self.wrap_pending = false;
                self.col = (n1 - 1).min(self.cols - 1);
            }
            b'H' | b'f' => {
                self.wrap_pending = false;
                let r = nums.first().copied().filter(|&n| n > 0).unwrap_or(1);
                let c = nums.get(1).copied().filter(|&n| n > 0).unwrap_or(1);
                self.row = (r - 1).min(self.rows - 1);
                self.col = (c - 1).min(self.cols - 1);
            }
            b'K' => {
                let mode = nums.first().copied().unwrap_or(0);
                let (a, b) = match mode {
                    1 => (0, self.col + 1),
                    2 => (0, self.cols),
                    _ => (self.col, self.cols),
                };
                for c in a..b.min(self.cols) {
                    self.grid[self.row][c] = ' ';
                }
            }
            b'J' => {
                let mode = nums.first().copied().unwrap_or(0);
                match mode {
                    2 | 3 => {
                        for r in 0..self.rows {
                            for c in 0..self.cols {
                                self.grid[r][c] = ' ';
                            }
                        }
                    }
                    _ => {
                        // Cursor to end of screen: rest of this line, then below.
                        for c in self.col..self.cols {
                            self.grid[self.row][c] = ' ';
                        }
                        for r in (self.row + 1)..self.rows {
                            for c in 0..self.cols {
                                self.grid[r][c] = ' ';
                            }
                        }
                    }
                }
            }
            _ => {} // SGR 'm', mode 'h'/'l', and everything else: no effect
        }
    }

    /// Place one printed glyph, honoring the deferred-wrap latch.
    fn put(&mut self, ch: char) {
        if self.wrap_pending {
            self.line_feed();
            self.col = 0;
            self.wrap_pending = false;
        }
        self.grid[self.row][self.col] = ch;
        if self.col + 1 >= self.cols {
            // Fill the last column: park the latch, do not advance the row yet.
            self.wrap_pending = true;
        } else {
            self.col += 1;
        }
    }

    /// CR + LF: return to column 0 and move down one row, scrolling the whole
    /// grid up when already on the bottom row (ONLCR + a full screen).
    fn line_feed(&mut self) {
        self.col = 0;
        self.wrap_pending = false;
        if self.row + 1 < self.rows {
            self.row += 1;
        } else {
            self.grid.remove(0);
            self.grid.push(vec![' '; self.cols]);
        }
    }

    /// The visible screen, one String per row, trailing blanks trimmed.
    pub fn render(&self) -> Vec<String> {
        self.grid
            .iter()
            .map(|r| r.iter().collect::<String>().trim_end().to_string())
            .collect()
    }

    /// The last `n` rows of the screen (the dock lives here).
    pub fn bottom(&self, n: usize) -> Vec<String> {
        let all = self.render();
        let start = all.len().saturating_sub(n);
        all[start..].to_vec()
    }

    /// A framed dump for `--nocapture` inspection: every row inside a ruler so
    /// trailing spaces and blank rows are visible.
    pub fn dump(&self, label: &str) -> String {
        let mut s = format!("┌── {label} ({}x{}) ", self.rows, self.cols);
        while s.chars().count() < self.cols + 6 {
            s.push('─');
        }
        s.push('\n');
        for (i, line) in self.render().iter().enumerate() {
            s.push_str(&format!("{i:>2}│{line}│\n"));
        }
        s.push('└');
        for _ in 0..self.cols + 5 {
            s.push('─');
        }
        s.push('\n');
        s
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(vt: &Vt) -> Vec<String> {
        vt.render()
    }

    #[test]
    fn prints_and_wraps_with_deferred_wrap() {
        let mut vt = Vt::new(4, 5);
        vt.feed(b"abcde"); // exactly fills row 0; latch parked, no row advance
        assert_eq!(lines(&vt)[0], "abcde");
        assert_eq!((vt.row, vt.col, vt.wrap_pending), (0, 4, true));
        vt.feed(b"f"); // the next glyph wraps to row 1
        assert_eq!(lines(&vt)[1], "f");
        assert_eq!((vt.row, vt.col), (1, 1));
    }

    #[test]
    fn newline_is_carriage_return_plus_line_feed() {
        let mut vt = Vt::new(4, 8);
        vt.feed(b"ab\ncd");
        assert_eq!(lines(&vt)[0], "ab");
        assert_eq!(lines(&vt)[1], "cd"); // cd starts at column 0, not staircased
    }

    #[test]
    fn scrolls_up_when_line_feeding_off_the_bottom_row() {
        let mut vt = Vt::new(3, 8);
        vt.feed(b"one\ntwo\nthree\nfour");
        // one scrolled off the top; the newest line sits on the bottom row.
        assert_eq!(lines(&vt), vec!["two", "three", "four"]);
    }

    #[test]
    fn cursor_up_then_overwrite_lands_on_the_right_row() {
        let mut vt = Vt::new(4, 8);
        vt.feed(b"top\r\nmid\r\nbot");
        vt.feed(b"\x1b[2A"); // up two rows
        vt.feed(b"\rX"); // column 0, overwrite
        assert_eq!(lines(&vt)[0], "Xop");
        assert_eq!(lines(&vt)[2], "bot");
    }

    #[test]
    fn erase_line_and_cha_column_move() {
        let mut vt = Vt::new(2, 10);
        vt.feed(b"hello");
        vt.feed(b"\r\x1b[K"); // CR then erase to end of line
        assert_eq!(lines(&vt)[0], "");
        vt.feed(b"abc\x1b[1G_"); // CHA to column 1 (1-based), overwrite
        assert_eq!(lines(&vt)[0], "_bc");
    }

    #[test]
    fn sgr_private_modes_and_osc_are_swallowed() {
        let mut vt = Vt::new(2, 20);
        vt.feed(b"\x1b[38;2;1;2;3mhi\x1b[0m");
        vt.feed(b"\x1b[?2004h!\x1b[?2004l");
        vt.feed(b"\x1b]0;window title\x07?");
        assert_eq!(lines(&vt)[0], "hi!?");
    }
}
