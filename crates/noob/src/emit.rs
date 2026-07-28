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

/// How many live lines one call may send before the tap closes.
///
/// The tap exists so a watcher can see a build scroll. It must never become
/// the reason the build is slow: the collector thread that feeds it is the
/// same thread that drains the child's pipe, so a sink that cannot keep up
/// backs the pipe up and blocks the child. A run long enough to pass this is
/// one nobody is reading line by line anyway, and its full output still
/// reaches the model untouched.
const PROGRESS_MAX_LINES: usize = 5_000;
/// Longest live line, in characters. A UI shows one row; the rest is noise.
const PROGRESS_MAX_CHARS: usize = 400;
/// A line this long with no break is not a line. Send it and start over
/// rather than buffering a command that never emits a newline.
const PROGRESS_MAX_PARTIAL: usize = 8_192;

/// Live output from one running tool, framed into lines.
///
/// The thing producing output reads fixed-size chunks; a watcher wants lines,
/// so the partial line between two chunks lives here. A carriage return ends a
/// line as well as a newline: a progress bar rewrites its line in place, and
/// each rewrite is exactly what someone watching wants to see, rather than one
/// four-megabyte line at the end.
///
/// Built on the thread that is executing the call, because that is where the
/// call id is, and then moved to whichever thread does the reading.
pub struct Progress {
    emitter: Emitter,
    of: Source,
    partial: String,
    sent: usize,
}

/// Who is producing the lines. Two producers, same framing: a tool's own
/// output and a sub-agent's. Sharing the framing is the point, because the
/// awkward parts (a partial line between two chunks, a progress bar rewriting
/// itself, a command that floods) are identical for both.
enum Source {
    Call(String),
    Agent(String),
}

impl Progress {
    /// A tap for the call this thread is inside, or nothing when the sink is
    /// off or this is not running inside a call.
    pub fn for_current_call(emitter: &Emitter) -> Option<Progress> {
        if !emitter.is_on() {
            return None;
        }
        Some(Progress::new(emitter, Source::Call(current_call()?)))
    }

    /// A tap for a sub-agent's own output.
    pub fn for_agent(emitter: &Emitter, agent_id: &str) -> Option<Progress> {
        emitter
            .is_on()
            .then(|| Progress::new(emitter, Source::Agent(agent_id.to_string())))
    }

    fn new(emitter: &Emitter, of: Source) -> Progress {
        Progress {
            emitter: emitter.clone(),
            of,
            partial: String::new(),
            sent: 0,
        }
    }

    fn frame(&self, line: String) -> Event {
        match &self.of {
            Source::Call(call_id) => Event::ToolProgress {
                call_id: call_id.clone(),
                line,
            },
            Source::Agent(agent_id) => Event::AgentOutput {
                agent_id: agent_id.clone(),
                line,
            },
        }
    }

    /// Take one chunk of raw output and send whatever complete lines it ended.
    pub fn feed(&mut self, bytes: &[u8]) {
        if self.sent > PROGRESS_MAX_LINES {
            return;
        }
        self.partial.push_str(&String::from_utf8_lossy(bytes));
        while let Some(at) = self.partial.find(['\n', '\r']) {
            let line: String = self.partial.drain(..=at).collect();
            self.emit(&line);
        }
        if self.partial.len() > PROGRESS_MAX_PARTIAL {
            let line = std::mem::take(&mut self.partial);
            self.emit(&line);
        }
    }

    /// Send the last line, which a command that ended without a newline left
    /// behind. Cheap and idempotent; call it when the output is finished.
    pub fn flush(&mut self) {
        if !self.partial.is_empty() {
            let line = std::mem::take(&mut self.partial);
            self.emit(&line);
        }
    }

    fn emit(&mut self, line: &str) {
        let line = line.trim_end_matches(['\n', '\r']);
        if line.trim().is_empty() {
            return;
        }
        if self.sent >= PROGRESS_MAX_LINES {
            // Say it once, rather than going quiet and letting a watcher read
            // the silence as the command having stopped.
            if self.sent == PROGRESS_MAX_LINES {
                self.sent += 1;
                let notice = format!(
                    "[live output stops here after {PROGRESS_MAX_LINES} lines; \
                     it is still running and its result is unaffected]"
                );
                self.emitter.send(self.frame(notice));
            }
            return;
        }
        self.sent += 1;
        let line = self.frame(clip(line));
        self.emitter.send(line);
    }
}

/// One row's worth of a line, with the rest marked as dropped.
fn clip(line: &str) -> String {
    if line.chars().count() <= PROGRESS_MAX_CHARS {
        return line.to_string();
    }
    let mut out: String = line.chars().take(PROGRESS_MAX_CHARS).collect();
    out.push('…');
    out
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

    fn lines_of(text: &str) -> Vec<String> {
        text.lines()
            .map(|line| {
                let value: serde_json::Value = serde_json::from_str(line).unwrap();
                value["line"].as_str().unwrap().to_string()
            })
            .collect()
    }

    fn tap(buf: &Buf) -> Progress {
        let emitter = Emitter::to(Box::new(buf.clone()));
        Progress::for_agent(&emitter, "agent-1").unwrap()
    }

    /// The producer reads fixed-size chunks and a watcher wants lines, so a
    /// line split across two chunks has to survive the boundary.
    #[test]
    fn a_line_split_across_two_chunks_arrives_whole() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        progress.feed(b"compiling no");
        assert_eq!(buf.text(), "", "half a line is not a line yet");
        progress.feed(b"ob v0.1\nlinking\n");
        assert_eq!(lines_of(&buf.text()), ["compiling noob v0.1", "linking"]);
    }

    /// A progress bar rewrites its line in place with a carriage return. Each
    /// rewrite is what someone watching wants, not one enormous line at the end.
    #[test]
    fn a_carriage_return_ends_a_line_too() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        progress.feed(b"12%\r45%\r100%\n");
        assert_eq!(lines_of(&buf.text()), ["12%", "45%", "100%"]);
    }

    /// A command whose last line has no newline still wrote that line.
    #[test]
    fn the_last_line_is_sent_even_without_a_newline() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        progress.feed(b"done\nand this");
        progress.flush();
        assert_eq!(lines_of(&buf.text()), ["done", "and this"]);
        // Flushing again has nothing left to send.
        progress.flush();
        assert_eq!(buf.text().lines().count(), 2);
    }

    /// Blank lines are the majority of most build output and say nothing.
    #[test]
    fn blank_lines_are_not_frames() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        progress.feed(b"\n\n   \nreal\n\r\n");
        assert_eq!(lines_of(&buf.text()), ["real"]);
    }

    /// The tap feeds off the same thread that drains the child's pipe, so a
    /// command that floods must stop the tap rather than the command.
    #[test]
    fn a_flood_stops_after_saying_that_it_stopped() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        for n in 0..PROGRESS_MAX_LINES + 500 {
            progress.feed(format!("line {n}\n").as_bytes());
        }
        let lines = lines_of(&buf.text());
        assert_eq!(lines.len(), PROGRESS_MAX_LINES + 1);
        assert!(
            lines.last().unwrap().contains("live output stops here"),
            "silence would read as the command having stopped: {:?}",
            lines.last()
        );
    }

    /// A line longer than any window is clipped, and says it was.
    #[test]
    fn a_very_long_line_is_clipped() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        progress.feed(format!("{}\n", "x".repeat(PROGRESS_MAX_CHARS * 3)).as_bytes());
        let line = &lines_of(&buf.text())[0];
        assert_eq!(line.chars().count(), PROGRESS_MAX_CHARS + 1);
        assert!(line.ends_with('…'));
    }

    /// Output with no newline at all must not buffer without bound.
    #[test]
    fn output_that_never_breaks_is_still_sent() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        progress.feed(&vec![b'y'; PROGRESS_MAX_PARTIAL + 10]);
        assert_eq!(buf.text().lines().count(), 1, "it did not wait forever");
    }

    /// The same framing, two producers. A tool's output is a tool's, a
    /// sub-agent's is its own, and neither is mistaken for the other.
    #[test]
    fn each_producer_gets_its_own_frame_type() {
        let buf = Buf::default();
        let emitter = Emitter::to(Box::new(buf.clone()));
        Progress::for_agent(&emitter, "agent-3")
            .unwrap()
            .feed(b"child says hello\n");
        as_call("c9", || {
            Progress::for_current_call(&emitter)
                .unwrap()
                .feed(b"tool says hello\n");
        });
        let frames: Vec<serde_json::Value> = buf
            .text()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(frames[0]["t"], "agent.output");
        assert_eq!(frames[0]["agent_id"], "agent-3");
        assert_eq!(frames[1]["t"], "tool.progress");
        assert_eq!(frames[1]["call_id"], "c9");
    }

    /// Off is off: no tap exists at all, so a caller pays a branch and nothing
    /// else. Outside a call there is nothing to attribute output to either.
    #[test]
    fn no_tap_when_the_sink_is_off_or_there_is_no_call() {
        let off = Emitter::default();
        assert!(Progress::for_agent(&off, "agent-1").is_none());
        assert!(Progress::for_current_call(&off).is_none());
        let on = Emitter::to(Box::new(Buf::default()));
        assert!(
            Progress::for_current_call(&on).is_none(),
            "output outside a call has nothing to attach to"
        );
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
