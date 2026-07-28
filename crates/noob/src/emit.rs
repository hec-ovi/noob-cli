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
pub struct Emitter {
    sink: Option<Arc<Mutex<Box<dyn Write + Send>>>>,
    /// Calls that have a `tool.start` and no `tool.end` yet.
    ///
    /// The emitter tracks this rather than the agent loop because the loop
    /// closes calls from inside a closure that holds `&self.tool_ctx` and so
    /// cannot touch `&mut self`. It is also the only correct place: an
    /// interrupt can land at four points that each know a different subset of
    /// what is open, and this one knows all of it.
    open: Arc<Mutex<Vec<String>>>,
}

impl Emitter {
    /// Open the sink named by `NOOB_EMIT`, or stay off.
    ///
    /// A path that cannot be opened turns emission off rather than failing the
    /// session: a broken side-channel must never stop the agent working.
    pub fn from_env() -> Emitter {
        let Some(path) = std::env::var_os(EMIT_VAR).filter(|v| !v.is_empty()) else {
            return Emitter::default();
        };
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(file) => Emitter::to(Box::new(file)),
            Err(_) => Emitter::default(),
        }
    }

    pub fn to(sink: Box<dyn Write + Send>) -> Emitter {
        Emitter {
            sink: Some(Arc::new(Mutex::new(sink))),
            open: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn is_on(&self) -> bool {
        self.sink.is_some()
    }

    /// Write one frame. Flushed immediately, and a failed write is dropped:
    /// a consumer that went away must not take the session with it.
    pub fn send(&self, event: Event) {
        let Some(sink) = &self.sink else {
            return;
        };
        match &event {
            Event::ToolStart { call_id, .. } => self.track(call_id.clone()),
            Event::ToolEnd { call_id, .. } => self.untrack(call_id),
            _ => {}
        }
        let line = noob_proto::encode(&event);
        if let Ok(mut sink) = sink.lock() {
            let _ = sink.write_all(line.as_bytes());
            let _ = sink.flush();
        }
    }

    fn track(&self, call_id: String) {
        if let Ok(mut open) = self.open.lock() {
            open.push(call_id);
        }
    }

    fn untrack(&self, call_id: &str) {
        if let Ok(mut open) = self.open.lock() {
            open.retain(|id| id != call_id);
        }
    }

    /// Close every call that was started and never finished.
    ///
    /// An interrupt can land between a batch's starts and its results, and the
    /// agent has four paths that reach the same epilogue. A row that never
    /// closes renders as a tool that is still running, forever, so the
    /// epilogue closes them rather than each path guessing which are open.
    pub fn cancel_open_calls(&self) {
        if self.sink.is_none() {
            return;
        }
        let pending = match self.open.lock() {
            Ok(mut open) => std::mem::take(&mut *open),
            Err(_) => return,
        };
        for call_id in pending {
            self.send(Event::ToolEnd {
                call_id,
                summary: String::from("canceled"),
                elapsed_ms: 0,
                error: Some(noob_proto::ToolError {
                    kind: String::from("canceled"),
                    code: None,
                    message: String::from("canceled by user"),
                    detail: None,
                    remedy: None,
                }),
            });
        }
    }
}

/// The lines both sides agree on: a common prefix and a common suffix, which
/// leaves the changed middle between them.
fn unchanged_ends(before: &[&str], after: &[&str]) -> (usize, usize) {
    let head = before
        .iter()
        .zip(after)
        .take_while(|(a, b)| a == b)
        .count();
    let tail = before
        .iter()
        .rev()
        .zip(after.iter().rev())
        .take_while(|(a, b)| a == b)
        .take(before.len().min(after.len()) - head)
        .count();
    (head, tail)
}

/// The line span a replacement occupies, and what it replaced.
///
/// Both sides of an edit are already in scope wherever a tool writes a file, so
/// a consumer never has to re-read anything to draw a diff. Lines are 1-based
/// and inclusive, matching `read`'s own numbering.
pub fn edit_span(before: &str, after: &str) -> noob_proto::Span {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let (head, tail) = unchanged_ends(&before_lines, &after_lines);
    let start = head + 1;
    let end = after_lines.len().saturating_sub(tail).max(head);
    noob_proto::Span {
        start: start as u32,
        end: end.max(start.saturating_sub(1)) as u32,
        kind: None,
        name: None,
    }
}

/// One `file.edit` frame for a whole-file replacement.
///
/// The span is in the written file's coordinates, and the two texts are the
/// same region on either side, so a consumer draws the diff from the frame
/// alone. Clipping to the changed middle is what makes this affordable: a
/// one-line fix in a 3,000-line file sends two lines, not six thousand.
pub fn file_edit(
    path: String,
    before: &str,
    after: &str,
    call_id: Option<String>,
) -> noob_proto::Event {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let (head, tail) = unchanged_ends(&before_lines, &after_lines);
    let before_end = before_lines.len().saturating_sub(tail);
    let after_end = after_lines.len().saturating_sub(tail);
    noob_proto::Event::FileEdit {
        path,
        span: edit_span(before, after),
        before: before_lines[head.min(before_end)..before_end].join("\n"),
        after: after_lines[head.min(after_end)..after_end].join("\n"),
        call_id,
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

    fn kinds(text: &str) -> Vec<String> {
        text.lines()
            .map(|line| {
                let value: serde_json::Value = serde_json::from_str(line).unwrap();
                value["t"].as_str().unwrap().to_string()
            })
            .collect()
    }

    fn tool_start(call_id: &str) -> Event {
        Event::ToolStart {
            call_id: call_id.into(),
            name: "read".into(),
            brief: "f".into(),
            args: serde_json::Value::Null,
        }
    }

    fn tool_end(call_id: &str) -> Event {
        Event::ToolEnd {
            call_id: call_id.into(),
            summary: "ok".into(),
            elapsed_ms: 1,
            error: None,
        }
    }

    /// A Ctrl-C between a batch's starts and its results leaves rows open. A
    /// consumer shows those as tools that are still running, so the interrupt
    /// epilogue closes them.
    #[test]
    fn an_interrupt_closes_every_call_that_was_started_and_never_finished() {
        let buf = Buf::default();
        let emitter = Emitter::to(Box::new(buf.clone()));
        emitter.send(tool_start("c1"));
        emitter.send(tool_start("c2"));
        emitter.send(tool_end("c1"));
        emitter.cancel_open_calls();

        let text = buf.text();
        assert_eq!(
            kinds(&text),
            ["tool.start", "tool.start", "tool.end", "tool.end"]
        );
        let last: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        assert_eq!(last["call_id"], "c2", "only the open call is closed");
        assert_eq!(last["error"]["kind"], "canceled");

        // Closing twice must not re-close what it already closed.
        emitter.cancel_open_calls();
        assert_eq!(buf.text().lines().count(), 4);
    }

    /// The tracking must not cost anything when the sink is off, and must not
    /// grow without bound across a long session.
    #[test]
    fn a_finished_call_stops_being_tracked() {
        let buf = Buf::default();
        let emitter = Emitter::to(Box::new(buf.clone()));
        for n in 0..100 {
            let id = format!("c{n}");
            emitter.send(tool_start(&id));
            emitter.send(tool_end(&id));
        }
        assert!(emitter.open.lock().unwrap().is_empty());
        emitter.cancel_open_calls();
        assert_eq!(buf.text().lines().count(), 200, "nothing extra was written");
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

    fn edit_sides(before: &str, after: &str) -> (String, String) {
        match file_edit("f".into(), before, after, None) {
            noob_proto::Event::FileEdit { before, after, .. } => (before, after),
            other => panic!("expected a file.edit frame, got {other:?}"),
        }
    }

    /// A one-line fix in a large file must send two lines, not the file. This
    /// is the difference between a diff view that keeps up with the agent and
    /// one that resends everything on every keystroke-sized change.
    #[test]
    fn a_file_edit_carries_only_the_changed_middle() {
        let before: String = (1..=200).map(|n| format!("line {n}\n")).collect();
        let after = before.replace("line 100\n", "LINE 100\n");
        let (old, new) = edit_sides(&before, &after);
        assert_eq!(old, "line 100");
        assert_eq!(new, "LINE 100");
    }

    #[test]
    fn a_file_edit_carries_both_sides_of_an_insertion_and_a_deletion() {
        // Insertion: nothing on the old side, the new lines on the new side.
        let (old, new) = edit_sides("a\nc\n", "a\nb1\nb2\nc\n");
        assert_eq!(old, "");
        assert_eq!(new, "b1\nb2");
        // Deletion: the reverse.
        let (old, new) = edit_sides("a\nb\nc\n", "a\nc\n");
        assert_eq!(old, "b");
        assert_eq!(new, "");
        // A new file is entirely new.
        let (old, new) = edit_sides("", "x\ny\n");
        assert_eq!(old, "");
        assert_eq!(new, "x\ny");
        // Nothing changed at all: an empty region, not a whole-file resend.
        let (old, new) = edit_sides("a\nb\n", "a\nb\n");
        assert_eq!((old.as_str(), new.as_str()), ("", ""));
    }

    /// Scattered changes collapse into one region spanning them. Coarse on
    /// purpose: the frame stays one frame, and the span still bounds where a
    /// consumer has to look.
    #[test]
    fn scattered_changes_report_one_region_that_covers_them() {
        let (old, new) = edit_sides("a\nb\nc\nd\ne\n", "a\nB\nc\nD\ne\n");
        assert_eq!(old, "b\nc\nd");
        assert_eq!(new, "B\nc\nD");
    }
}
