//! A pseudo-terminal driver for e2e suites: spawn a compiled binary with all
//! three stdio streams on a fresh pty slave, so `is_terminal()` is true and
//! its raw-mode interactive path engages, then drive it byte-for-byte the way
//! a keyboard would and read back exactly what a terminal would receive.
//! Unix only: the whole module rides `openpty`.

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::vt::Vt;

pub struct Pty {
    master: std::fs::File,
    child: Option<Child>,
    done: Arc<AtomicBool>,
    watchdog: Option<JoinHandle<()>>,
    seen: String,
    /// The exact bytes read from the master, undecoded. `seen` is a lossy
    /// UTF-8 view for substring waits; the screen emulator needs the real
    /// bytes (a box-drawing glyph split across a read boundary would otherwise
    /// become a replacement char).
    raw: Vec<u8>,
    /// How far `wait_for` has consumed, so successive calls match successive
    /// occurrences (each prompt re-emits the same markers).
    cursor: usize,
}

impl Pty {
    /// Spawn `cmd` on a fresh pty and return the driver. The child's
    /// stdin/stdout/stderr are the slave, so `is_terminal()` is true and the
    /// raw editor engages. `size = Some((rows, cols))` sets the pty winsize so
    /// behavior on a small screen is reproducible; a program that reads only
    /// the width (TIOCGWINSZ) never sees the row count, which matters only to
    /// the emulator that replays the captured bytes. A watchdog SIGKILLs the
    /// child after 20 s so a wedged binary fails its test instead of hanging
    /// the suite.
    pub fn spawn(mut cmd: Command, size: Option<(u16, u16)>) -> Pty {
        let (master, slave) = unsafe {
            let mut m: libc::c_int = 0;
            let mut s: libc::c_int = 0;
            let ws = size.map(|(rows, cols)| libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            });
            let mut ws = ws;
            let ws_ptr = ws
                .as_mut()
                .map(|w| w as *mut libc::winsize)
                .unwrap_or(std::ptr::null_mut());
            assert_eq!(
                libc::openpty(
                    &mut m,
                    &mut s,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    ws_ptr
                ),
                0,
                "openpty failed"
            );
            (std::fs::File::from_raw_fd(m), s)
        };
        let stdio = |fd: i32| unsafe { Stdio::from_raw_fd(libc::dup(fd)) };
        let child = cmd
            .stdin(stdio(slave))
            .stdout(stdio(slave))
            .stderr(stdio(slave))
            .spawn()
            .unwrap();
        unsafe { libc::close(slave) };

        let child_pid = child.id() as i32;
        let done = Arc::new(AtomicBool::new(false));
        let wd_done = done.clone();
        let watchdog = std::thread::spawn(move || {
            for _ in 0..200 {
                if wd_done.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            unsafe { libc::kill(child_pid, libc::SIGKILL) };
        });
        Pty {
            master,
            child: Some(child),
            done,
            watchdog: Some(watchdog),
            seen: String::new(),
            raw: Vec::new(),
            cursor: 0,
        }
    }

    /// Type bytes at the child, exactly as a keyboard would deliver them.
    pub fn send(&mut self, bytes: &[u8]) {
        self.master.write_all(bytes).unwrap();
    }

    /// Read the master until `marker` appears at or after the last match, then
    /// advance past it. Consuming, so it syncs to one prompt at a time.
    pub fn wait_for(&mut self, marker: &str) {
        let mut buf = [0u8; 4096];
        loop {
            if let Some(pos) = self.seen[self.cursor..].find(marker) {
                self.cursor += pos + marker.len();
                return;
            }
            match self.master.read(&mut buf) {
                Ok(0) => panic!("pty closed before {marker:?}; saw:\n{}", self.seen),
                Ok(n) => {
                    self.raw.extend_from_slice(&buf[..n]);
                    self.seen.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
                Err(e) => panic!(
                    "pty read error: {e} while waiting for {marker:?}; saw:\n{}",
                    self.seen
                ),
            }
        }
    }

    /// Pull whatever the child emits over `budget`, into `raw`/`seen`, without
    /// blocking on a marker. Used to capture trailing repaints (an animation
    /// cadence keeps redrawing after the last output) before snapshotting the
    /// screen.
    pub fn drain(&mut self, budget: Duration) {
        let fd = self.master.as_raw_fd();
        let deadline = Instant::now() + budget;
        let mut buf = [0u8; 4096];
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let ms = (remaining.as_millis() as i32).min(40);
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut pfd, 1, ms) };
            if ready <= 0 {
                continue; // timeout or EINTR: keep polling until the budget ends
            }
            match self.master.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    self.raw.extend_from_slice(&buf[..n]);
                    self.seen.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
                Err(_) => break,
            }
        }
    }

    /// Everything read so far as lossy UTF-8, for substring assertions.
    pub fn seen(&self) -> &str {
        &self.seen
    }

    /// The exact bytes read so far, undecoded, for the screen emulator.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The child's pid, for tests that signal it directly.
    pub fn child_id(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    /// Replay everything captured so far into a fresh rows x cols screen.
    pub fn screen(&self, rows: u16, cols: u16) -> Vt {
        let mut vt = Vt::new(rows as usize, cols as usize);
        vt.feed(&self.raw);
        vt
    }

    /// Resize the pty (TIOCSWINSZ updates the winsize the child reads) and raise
    /// SIGWINCH in the child. The child here is not a controlling-tty session
    /// leader, so TIOCSWINSZ alone does not auto-deliver the signal the way a
    /// real terminal does; sending it explicitly exercises the child's reflow
    /// path against the freshly updated width.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let ws = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &ws);
            if let Some(child) = &self.child {
                libc::kill(child.id() as i32, libc::SIGWINCH);
            }
        }
    }

    /// Wait for the child to exit and return its status, stopping the watchdog.
    pub fn finish(&mut self) -> ExitStatus {
        let status = self.child.take().unwrap().wait().unwrap();
        self.done.store(true, Ordering::SeqCst);
        self.watchdog.take().unwrap().join().ok();
        status
    }
}
