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

mod diff;
mod progress;

pub use diff::file_edit;
pub use progress::Progress;

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Instant;

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
#[derive(Clone)]
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
    /// When this stream began, so `metrics.at_ms` is a time axis.
    ///
    /// The epoch belongs to the stream rather than to the agent: a consumer
    /// assembles successive frames into a series without a clock of its own,
    /// and the two sides never have to agree on a wall time. Copied rather
    /// than shared on clone, which is the same value either way because every
    /// clone descends from the one the session opened with.
    started: Instant,
}

impl Default for Emitter {
    fn default() -> Emitter {
        Emitter {
            sink: None,
            open: Arc::new(Mutex::new(Vec::new())),
            started: Instant::now(),
        }
    }
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
            ..Emitter::default()
        }
    }

    pub fn is_on(&self) -> bool {
        self.sink.is_some()
    }

    /// Milliseconds since this stream opened.
    pub fn at_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// One batch of measurements, stamped on the stream's own clock.
    ///
    /// A helper rather than a plain `send` because `at_ms` is the field every
    /// caller would otherwise have to source, and there is exactly one right
    /// answer for it.
    pub fn metrics(&self, group: &str, samples: Vec<noob_proto::Sample>) {
        if self.sink.is_none() {
            return;
        }
        self.send(Event::Metrics {
            group: group.to_string(),
            at_ms: self.at_ms(),
            samples,
        });
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

/// A shared buffer standing in for the sink file. Module-level so any test in
/// the crate can watch what its subject emitted.
#[cfg(test)]
#[derive(Clone, Default)]
pub struct Buf(Arc<Mutex<Vec<u8>>>);

#[cfg(test)]
impl Write for Buf {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
impl Buf {
    pub fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }

    /// Every frame written so far, decoded.
    pub fn frames(&self) -> Vec<serde_json::Value> {
        self.text()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

/// An emitter writing to a buffer the caller can read back.
#[cfg(test)]
pub fn watched() -> (Emitter, Buf) {
    let buf = Buf::default();
    (Emitter::to(Box::new(buf.clone())), buf)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// `at_ms` is the stream's own clock, which is what lets a consumer build
    /// a series without either side agreeing on a wall time.
    #[test]
    fn measurements_are_stamped_on_the_streams_own_clock() {
        let buf = Buf::default();
        let emitter = Emitter::to(Box::new(buf.clone()));
        emitter.metrics(
            "context",
            vec![noob_proto::Sample {
                key: "used".into(),
                label: "context used".into(),
                value: 12.0,
                max: Some(100.0),
                unit: Some("tokens".into()),
            }],
        );
        let frame: serde_json::Value = serde_json::from_str(buf.text().trim()).unwrap();
        assert_eq!(frame["t"], "metrics");
        assert_eq!(frame["group"], "context");
        assert!(frame["at_ms"].is_u64(), "{frame}");
        assert_eq!(frame["samples"][0]["value"], 12.0);
        // Every clone reports the same epoch, or successive frames would walk
        // backwards depending on which handle wrote them.
        assert_eq!(emitter.clone().at_ms(), emitter.at_ms());
    }
}
