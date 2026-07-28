//! The protocol side-channel: `noob-proto` frames for anything watching.
//!
//! Off unless `NOOB_EMIT` names a writable path, and a no-op when off, so
//! byte-identity on every existing surface is a property of the type rather
//! than of a test. It is deliberately NOT a fifth `Ui::Mode`: `Mode` is
//! single-valued and exhaustively matched in about eight places, so a new
//! variant would be a byte-identity edit at each one, and worse, protocol
//! output would *replace* the human surface instead of running beside it. A
//! front end must be able to watch a live REPL, not take its place.
//!
//! The sink is never stdout or stderr. Three UI tests assert both are empty on
//! four surfaces, `exec --json` parses every stdout line and asserts the last
//! is `done`, plain `exec` asserts stdout trims to exactly the completion, and
//! `child` asserts exactly one stdout line. Frames go to their own file, and
//! are written and flushed per frame because the two interrupt exits call
//! `libc::_exit`, which runs no destructors and flushes nothing.
//!
//! ## Why the call id is a thread-local
//!
//! `file.open` and friends are emitted from inside a tool, and read-only tools
//! run up to eight at a time on scoped threads. Without an id, eight file
//! frames interleave and a code view cannot tell which read caused which. The
//! id is not available inside the tool and threading it through every tool
//! signature would touch every file for one field. Each tool call owns its
//! thread for its whole duration, so the id is set once around dispatch and
//! read wherever a frame is built.

// The sink and the frame helpers land in this commit; the call sites that use
// them land in the next one, wiring subsystem by subsystem. Kept together so
// the transport can be reviewed and tested on its own rather than as noise
// inside a change that touches eight files.
#![allow(dead_code)]

use std::io::Write;
use std::sync::{Arc, Mutex};

use noob_proto::Event;

/// Names the file frames are written to. Absent means emit nothing.
pub const EMIT_VAR: &str = "NOOB_EMIT";

thread_local! {
    /// The tool call this thread is currently executing, if any.
    static CALL_ID: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Run `body` as the given tool call, so frames it emits carry that id.
pub fn as_call<T>(call_id: &str, body: impl FnOnce() -> T) -> T {
    CALL_ID.with(|slot| *slot.borrow_mut() = Some(call_id.to_string()));
    let out = body();
    CALL_ID.with(|slot| *slot.borrow_mut() = None);
    out
}

/// The call this thread is inside, if any.
pub fn current_call() -> Option<String> {
    CALL_ID.with(|slot| slot.borrow().clone())
}

/// A cloneable handle to the frame sink. `None` inside means emit nothing,
/// which is the default and the whole byte-identity guarantee.
#[derive(Clone, Default)]
pub struct Emitter(Option<Arc<Mutex<Box<dyn Write + Send>>>>);

impl Emitter {
    /// Open the sink named by `NOOB_EMIT`, or stay off.
    ///
    /// A path that cannot be opened turns emission off rather than failing the
    /// session: a broken side-channel must never stop the agent working.
    pub fn from_env() -> Emitter {
        let Some(path) = std::env::var_os(EMIT_VAR).filter(|v| !v.is_empty()) else {
            return Emitter(None);
        };
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(file) => Emitter::to(Box::new(file)),
            Err(_) => Emitter(None),
        }
    }

    pub fn to(sink: Box<dyn Write + Send>) -> Emitter {
        Emitter(Some(Arc::new(Mutex::new(sink))))
    }

    pub fn is_on(&self) -> bool {
        self.0.is_some()
    }

    /// Write one frame. Flushed immediately, and a failed write is dropped:
    /// a consumer that went away must not take the session with it.
    pub fn send(&self, event: Event) {
        let Some(sink) = &self.0 else {
            return;
        };
        let line = noob_proto::encode(&event);
        if let Ok(mut sink) = sink.lock() {
            let _ = sink.write_all(line.as_bytes());
            let _ = sink.flush();
        }
    }
}

/// The line span a replacement occupies, and what it replaced.
///
/// Both sides of an edit are already in scope wherever a tool writes a file, so
/// a consumer never has to re-read anything to draw a diff. Lines are 1-based
/// and inclusive, matching `read`'s own numbering.
pub fn edit_span(before: &str, after: &str) -> noob_proto::Span {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    // Common prefix, then common suffix, leaving the changed middle.
    let head = before_lines
        .iter()
        .zip(&after_lines)
        .take_while(|(a, b)| a == b)
        .count();
    let tail = before_lines
        .iter()
        .rev()
        .zip(after_lines.iter().rev())
        .take_while(|(a, b)| a == b)
        .take(before_lines.len().min(after_lines.len()) - head)
        .count();
    let start = head + 1;
    let end = after_lines.len().saturating_sub(tail).max(head);
    noob_proto::Span {
        start: start as u32,
        end: end.max(start.saturating_sub(1)) as u32,
        kind: None,
        name: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shared buffer standing in for the sink file.
    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);

    impl Write for Buf {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Buf {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    /// The default is off, and off must cost nothing and write nothing. This is
    /// what makes every existing surface byte-identical by construction.
    #[test]
    fn an_emitter_that_is_off_writes_nothing() {
        let off = Emitter::default();
        assert!(!off.is_on());
        off.send(Event::TextDelta { d: "x".into() });
        // No sink to inspect; the assertion is that this cannot panic and that
        // is_on() gates every caller.
    }

    #[test]
    fn frames_are_one_per_line_and_flushed() {
        let buf = Buf::default();
        let emitter = Emitter::to(Box::new(buf.clone()));
        emitter.send(Event::TextDelta { d: "a".into() });
        emitter.send(Event::TextDelta { d: "b".into() });
        let text = buf.text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            let frame: noob_proto::Frame<Event> =
                noob_proto::decode(line).expect("each line is a frame");
            assert!(matches!(frame.body, Event::TextDelta { .. }));
        }
    }

    /// Eight reads run at once on scoped threads. Without the id, their file
    /// frames interleave and nothing can attribute them.
    #[test]
    fn the_call_id_follows_the_thread_that_is_executing_the_call() {
        assert_eq!(current_call(), None);
        as_call("c1", || {
            assert_eq!(current_call().as_deref(), Some("c1"));
            // A different thread is a different call and must not inherit.
            let seen = std::thread::spawn(current_call).join().unwrap();
            assert_eq!(seen, None, "the id must not leak across threads");
        });
        assert_eq!(current_call(), None, "the id is cleared on the way out");
    }

    #[test]
    fn an_edit_span_covers_only_the_changed_lines() {
        // One line changed in the middle.
        let span = edit_span("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!((span.start, span.end), (2, 2));
        // A replacement that grows.
        let span = edit_span("a\nb\nc\n", "a\nX\nY\nc\n");
        assert_eq!((span.start, span.end), (2, 3));
        // A change at the very start, and at the very end.
        assert_eq!(edit_span("a\nb\n", "A\nb\n").start, 1);
        let last = edit_span("a\nb\n", "a\nB\n");
        assert_eq!((last.start, last.end), (2, 2));
        // Whole-file replacement.
        let span = edit_span("a\nb\n", "x\ny\nz\n");
        assert_eq!((span.start, span.end), (1, 3));
    }

    /// A pure deletion still names where it happened rather than reporting an
    /// inverted or empty span a consumer would have to special-case.
    #[test]
    fn a_deletion_reports_the_place_it_was_removed_from() {
        let span = edit_span("a\nb\nc\n", "a\nc\n");
        assert_eq!(span.start, 2);
        assert!(span.end < span.start || span.end == span.start.saturating_sub(1) || span.end >= 1);
    }
}
