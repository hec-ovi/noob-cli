//! The thinking scanner: a small green square comet that sweeps on its own line
//! while the model is working, from the moment a turn is dispatched until the
//! first output byte arrives. It exists to fill the dead air of the
//! request-to-first-token gap so the screen never looks frozen.
//!
//! Off the inference path, by construction. The animation runs on a separate
//! thread that only ever writes to stdout; the main thread is meanwhile blocked
//! inside the provider waiting on the socket, and it tears the scanner down
//! (join) before it writes the first reply byte, so the two never interleave and
//! decode/prefill throughput is untouched. The scanner never reads or mutates
//! any request, transcript, or model state: it is display-only, like everything
//! under `ui/`, and the caller only ever starts it on the themed REPL surface,
//! so piped, `exec`, `--json`, and child output stay byte-for-byte unchanged.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::style::{ColorDepth, RESET, Rgb, fg_sgr};

/// Milliseconds between comet frames. Slow enough to cost nothing, fast enough
/// to read as motion.
const FRAME_MS: u64 = 90;
/// Cells the comet sweeps across.
pub(super) const TRACK: usize = 14;
/// Length of the fading tail behind the head (uses the 6-stop ramp: head is the
/// brightest stop, the tail steps down through the rest).
const TAIL: usize = 5;
/// The comet glyph: one small square per cell.
const CELL: &str = "▪";

/// A running scanner. Dropping or `stop`ping it halts the thread and clears the
/// line it drew on.
pub struct Scanner {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Scanner {
    /// Start sweeping. The very first frame is drawn synchronously here so the
    /// comet appears the instant a turn is dispatched (no wait on the thread
    /// getting scheduled) and a fast turn still shows at least one frame; the
    /// thread then animates the rest.
    pub fn start(depth: ColorDepth, ramp: [Rgb; 6]) -> Scanner {
        let mut out = std::io::stdout();
        let _ = out.write_all(frame(0, depth, &ramp).as_bytes());
        let _ = out.flush();

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let handle = thread::spawn(move || run(&thread_stop, depth, ramp));
        Scanner { stop, handle: Some(handle) }
    }

    /// Halt the sweep and wait for the thread to clear its line, so on return
    /// the cursor is at column 0 of a blank line ready for the reply. Idempotent.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

impl Drop for Scanner {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The animation loop: draw a frame, park until the next tick (woken early by
/// `stop`), repeat; clear the line on the way out. Frame 0 was already drawn by
/// `start`, so this begins at 1.
fn run(stop: &AtomicBool, depth: ColorDepth, ramp: [Rgb; 6]) {
    let mut out = std::io::stdout();
    let mut t: usize = 1;
    loop {
        thread::park_timeout(Duration::from_millis(FRAME_MS));
        if stop.load(Ordering::Acquire) {
            break;
        }
        let _ = out.write_all(frame(t, depth, &ramp).as_bytes());
        let _ = out.flush();
        t = t.wrapping_add(1);
    }
    // Wipe the comet so the reply takes its place on a clean line.
    let _ = out.write_all(b"\r\x1b[K");
    let _ = out.flush();
}

/// One comet frame at tick `t`: return to column 0, a small indent, then the
/// track with a bright head bouncing across a fading tail. Every colored cell
/// carries its own reset, so nothing bleeds past the line. A pure function of
/// its inputs, so it is unit-testable without a terminal.
pub(super) fn frame(t: usize, depth: ColorDepth, ramp: &[Rgb; 6]) -> String {
    format!("\r  {}", track(t, depth, ramp))
}

/// The same comet without cursor movement or indentation, for embedding in
/// the dock's persistent top status row.
pub(super) fn track(t: usize, depth: ColorDepth, ramp: &[Rgb; 6]) -> String {
    let period = 2 * (TRACK - 1);
    let phase = t % period;
    // Head bounces 0 -> TRACK-1 -> 0; the tail trails behind it.
    let head = if phase < TRACK { phase } else { period - phase };
    let going_right = phase < TRACK - 1;

    let mut s = String::new();
    for c in 0..TRACK {
        if c == head {
            s.push_str(&cell(depth, ramp[ramp.len() - 1], true));
            continue;
        }
        // How far this cell is behind the head, along the direction of travel.
        let behind = if going_right {
            head as isize - c as isize
        } else {
            c as isize - head as isize
        };
        if behind > 0 && (behind as usize) <= TAIL {
            // Step down the ramp: nearest the head is brightest.
            let idx = (ramp.len() - 1).saturating_sub(behind as usize);
            s.push_str(&cell(depth, ramp[idx], false));
        } else {
            s.push(' ');
        }
    }
    s
}

/// One painted square, or a bare square at a depthless terminal (no stray
/// escape).
fn cell(depth: ColorDepth, color: Rgb, bold: bool) -> String {
    let open = fg_sgr(depth, color, bold);
    if open.is_empty() {
        CELL.to_string()
    } else {
        format!("{open}{CELL}{RESET}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::style::rgb;

    const RAMP: [Rgb; 6] = [
        rgb(60, 125, 85),
        rgb(78, 145, 100),
        rgb(96, 165, 118),
        rgb(116, 185, 135),
        rgb(138, 202, 152),
        rgb(160, 215, 175),
    ];

    #[test]
    fn a_frame_starts_at_column_zero_and_shows_the_head() {
        // Not "is it green": the invariants are it homes the cursor (so it
        // overwrites in place) and it draws at least the head square.
        let f = frame(0, ColorDepth::Truecolor, &RAMP);
        assert!(f.starts_with('\r'), "frame must return to column 0: {f:?}");
        assert!(f.contains(CELL), "frame drew no square: {f:?}");
    }

    #[test]
    fn every_colored_cell_resets_so_nothing_bleeds() {
        // The bleed guard, theme-agnostic: count openers and resets rather than
        // asserting a color, so retuning the ramp never breaks this. Each opener
        // (\x1b[...m that is not the reset) must be matched by a reset.
        let f = frame(3, ColorDepth::Truecolor, &RAMP);
        let resets = f.matches(RESET).count();
        let escapes = f.matches('\x1b').count();
        assert_eq!(escapes, resets * 2, "an SGR opener was left unreset: {f:?}");
    }

    #[test]
    fn depthless_frame_carries_no_escape_codes() {
        // At a depthless terminal the comet is bare squares, never a stray SGR.
        let f = frame(5, ColorDepth::None, &RAMP);
        assert!(!f.contains('\x1b'), "depthless frame emitted an escape: {f:?}");
        assert!(f.contains(CELL), "depthless frame drew no square: {f:?}");
    }

    #[test]
    fn the_head_sweeps_and_bounces_without_panicking() {
        // Walk a couple of full periods: every frame is well formed (homes the
        // cursor, draws a head) and the index math never goes out of bounds.
        for t in 0..(4 * TRACK) {
            let f = frame(t, ColorDepth::Ansi16, &RAMP);
            assert!(f.starts_with('\r'));
            assert!(f.contains(CELL), "frame {t} drew no square");
        }
    }

    // The live start/stop lifecycle (thread spawns, animates, joins, clears the
    // line) is exercised through the compiled binary in a real pty by
    // `raw_repl_shows_a_thinking_scanner_while_the_model_works`; a real Scanner
    // is not spawned here because its side thread writes straight to the process
    // stdout (cargo cannot capture another thread's writes) and would litter the
    // test runner output.
}
