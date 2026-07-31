//! Live output framed into lines, without ever blocking the thing producing it.

use super::{current_call, Emitter};
use noob_proto::Event;

/// How many live lines one call may send before the tap closes.
///
/// A run long enough to pass this is one nobody is reading line by line
/// anyway, and its full output still reaches the model untouched.
const PROGRESS_MAX_LINES: usize = 5_000;
/// Longest live line, in characters. A UI shows one row; the rest is noise.
const PROGRESS_MAX_CHARS: usize = 400;
/// A line this long with no break is not a line. Send it and start over
/// rather than buffering a command that never emits a newline.
const PROGRESS_MAX_PARTIAL: usize = 8_192;
/// Lines that may be waiting to be written before new ones are dropped.
///
/// This is the number that matters. See [`Progress`] for why dropping is the
/// only acceptable answer here.
const PROGRESS_QUEUE: usize = 512;

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
///
/// ## Why the writing happens on a thread of its own
///
/// The thread feeding this is the one draining a child's pipe. Writing a frame
/// means `write_all` on the sink, and under `serve` the sink is a pipe to a
/// front end: if that front end stops reading for a moment, the write blocks,
/// the collector stops draining, the child's pipe fills, and the child stops
/// running. That is not a slow watcher, it is a changed result. Long enough
/// and `exec` reports a timeout for a command that would have finished, or
/// appends its "background processes were left behind" note to a command that
/// left nothing.
///
/// So the collector never writes. It hands complete lines to a bounded queue
/// and a thread of its own does the writing, and when that queue is full the
/// lines are dropped and the count of them is sent as soon as there is room.
/// A watcher missing a hundred lines of a build is a watcher missing a hundred
/// lines of a build; a command that fails because someone was watching it is a
/// different kind of thing entirely.
pub struct Progress {
    lines: std::sync::mpsc::SyncSender<String>,
    writer: Option<std::thread::JoinHandle<()>>,
    partial: String,
    /// The start of a character a chunk ended in the middle of. Never more
    /// than three bytes, and every producer here reads fixed-size chunks, so
    /// without it a box-drawing character in a build log lands as two or three
    /// replacement marks whenever it straddles a read.
    carry: Vec<u8>,
    sent: usize,
    /// Lines the queue had no room for, waiting to be reported as a count.
    dropped: usize,
}

/// Who is producing the lines. Two producers, same framing: a tool's own
/// output and a sub-agent's. Sharing the framing is the point, because the
/// awkward parts (a partial line between two chunks, a progress bar rewriting
/// itself, a command that floods) are identical for both.
enum Source {
    Call(String),
    Agent(String),
}

impl Source {
    fn frame(&self, line: String) -> Event {
        match self {
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
        let (lines, queue) = std::sync::mpsc::sync_channel::<String>(PROGRESS_QUEUE);
        let emitter = emitter.clone();
        let writer = std::thread::Builder::new()
            .name(String::from("noob-progress"))
            .spawn(move || {
                for line in queue {
                    emitter.send(of.frame(line));
                }
            })
            .ok();
        Progress {
            lines,
            writer,
            partial: String::new(),
            carry: Vec::new(),
            sent: 0,
            dropped: 0,
        }
    }

    /// Take one chunk of raw output and send whatever complete lines it ended.
    pub fn feed(&mut self, bytes: &[u8]) {
        if self.sent > PROGRESS_MAX_LINES {
            return;
        }
        let mut buf = std::mem::take(&mut self.carry);
        buf.extend_from_slice(bytes);
        // Split at the last character that is actually finished. Neither `\n`
        // nor `\r` can appear inside a multi-byte sequence, so holding a tail
        // back can never delay a line.
        let cut = match std::str::from_utf8(&buf) {
            Ok(_) => buf.len(),
            // Ended mid-character: the rest is in the next chunk.
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            // Genuinely not UTF-8. There is nothing to wait for.
            Err(_) => buf.len(),
        };
        self.carry.extend_from_slice(&buf[cut..]);
        self.partial.push_str(&String::from_utf8_lossy(&buf[..cut]));
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
        if !self.carry.is_empty() {
            // Whatever it was, it is all there is going to be.
            let tail = std::mem::take(&mut self.carry);
            self.partial.push_str(&String::from_utf8_lossy(&tail));
        }
        if !self.partial.is_empty() {
            let line = std::mem::take(&mut self.partial);
            self.emit(&line);
        }
    }

    /// Wait until everything queued has been written.
    ///
    /// Call this once the output is finished and after whatever the result
    /// depends on has already been decided, never before: this is the one
    /// place that blocks, and blocking anywhere else is the whole problem the
    /// writer thread exists to avoid. Ordering is why it exists at all, so a
    /// call's live lines cannot arrive after the frame that closed it.
    pub fn finish(mut self) {
        self.flush();
        let Progress { lines, writer, .. } = self;
        drop(lines);
        if let Some(writer) = writer {
            let _ = writer.join();
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
                let _ = self.lines.try_send(notice);
            }
            return;
        }
        // Counted before the send, so the budget is a property of the
        // command's output rather than of how fast anybody is reading it.
        self.sent += 1;
        // Whatever was lost is reported as soon as there is room, rather than
        // silently: a gap nobody mentions reads as the command having gone
        // quiet, which is the one thing a watcher must not be told wrongly.
        if self.dropped > 0 {
            let notice = format!(
                "[{} live lines dropped; whatever is reading this is not keeping up]",
                self.dropped
            );
            if self.lines.try_send(notice).is_ok() {
                self.dropped = 0;
            }
        }
        match self.lines.try_send(clip(line)) {
            Ok(()) => {}
            // The queue is full: the consumer is behind. Dropping is the
            // point; blocking here would stall the command being watched.
            Err(std::sync::mpsc::TrySendError::Full(_)) => self.dropped += 1,
            // The writer thread is gone, so there is nowhere to put it.
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {}
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emit::{as_call, Buf};
    use std::io::Write;
    use std::sync::Arc;

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

    /// Finish the tap and read back what it wrote. The writing happens on a
    /// thread of its own, so nothing is guaranteed to be there until it ends.
    fn drain(progress: Progress, buf: &Buf) -> Vec<String> {
        progress.finish();
        lines_of(&buf.text())
    }

    /// The producer reads fixed-size chunks and a watcher wants lines, so a
    /// line split across two chunks has to survive the boundary.
    #[test]
    fn a_line_split_across_two_chunks_arrives_whole() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        progress.feed(b"compiling no");
        progress.feed(b"ob v0.1\nlinking\n");
        assert_eq!(drain(progress, &buf), ["compiling noob v0.1", "linking"]);
    }

    /// Every producer here reads fixed-size chunks, so a multi-byte character
    /// lands split across two of them the moment output is not ASCII. Decoding
    /// each chunk on its own turns a box-drawing character in a build log into
    /// two or three replacement marks.
    #[test]
    fn a_character_split_across_two_chunks_survives() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        let text = "cargo says \u{2500} done\n".as_bytes();
        // Split inside the three bytes of the box-drawing character.
        let at = text.iter().position(|b| *b == 0xe2).unwrap() + 1;
        progress.feed(&text[..at]);
        progress.feed(&text[at..]);
        let line = drain(progress, &buf).remove(0);
        assert_eq!(line, "cargo says \u{2500} done");
        assert!(!line.contains('\u{fffd}'), "{line:?}");
    }

    /// Bytes that are not UTF-8 at all are not a character waiting to finish.
    /// Holding them back would stall the line they are on forever.
    #[test]
    fn bytes_that_are_not_a_character_do_not_hold_up_the_line() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        progress.feed(&[b'o', b'k', 0xff, 0xfe, b'\n']);
        let lines = drain(progress, &buf);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("ok"), "{:?}", lines[0]);
    }

    /// A progress bar rewrites its line in place with a carriage return. Each
    /// rewrite is what someone watching wants, not one enormous line at the end.
    #[test]
    fn a_carriage_return_ends_a_line_too() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        progress.feed(b"12%\r45%\r100%\n");
        assert_eq!(drain(progress, &buf), ["12%", "45%", "100%"]);
    }

    /// A command whose last line has no newline still wrote that line.
    #[test]
    fn the_last_line_is_sent_even_without_a_newline() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        progress.feed(b"done\nand this");
        assert_eq!(drain(progress, &buf), ["done", "and this"]);
    }

    /// Blank lines are the majority of most build output and say nothing.
    #[test]
    fn blank_lines_are_not_frames() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        progress.feed(b"\n\n   \nreal\n\r\n");
        assert_eq!(drain(progress, &buf), ["real"]);
    }

    /// A command that floods must not be able to send forever, and must say
    /// that it stopped rather than going quiet.
    #[test]
    fn a_flood_stops_after_saying_that_it_stopped() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        for n in 0..PROGRESS_MAX_LINES + 500 {
            progress.feed(format!("line {n}\n").as_bytes());
        }
        let lines = drain(progress, &buf);
        assert!(
            lines.last().unwrap().contains("live output stops here"),
            "silence would read as the command having stopped: {:?}",
            lines.last()
        );
        // The budget counts the command's lines, not the ones that landed, so
        // it holds however fast or slow the other end happened to be.
        let content = lines.iter().filter(|l| l.starts_with("line ")).count();
        assert!(content <= PROGRESS_MAX_LINES, "{content}");
    }

    /// A line longer than any window is clipped, and says it was.
    #[test]
    fn a_very_long_line_is_clipped() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        progress.feed(format!("{}\n", "x".repeat(PROGRESS_MAX_CHARS * 3)).as_bytes());
        let line = drain(progress, &buf).remove(0);
        assert_eq!(line.chars().count(), PROGRESS_MAX_CHARS + 1);
        assert!(line.ends_with('…'));
    }

    /// Output with no newline at all must not buffer without bound.
    #[test]
    fn output_that_never_breaks_is_still_sent() {
        let buf = Buf::default();
        let mut progress = tap(&buf);
        progress.feed(&vec![b'y'; PROGRESS_MAX_PARTIAL + 10]);
        assert_eq!(drain(progress, &buf).len(), 1, "it did not wait forever");
    }

    /// The thread feeding a tap is the one draining a child's pipe. It must
    /// never block on whoever is reading the frames: a command that stops
    /// running because somebody is watching it is a changed result, not a slow
    /// display. Lines are dropped instead, and the loss is reported.
    #[test]
    fn a_consumer_that_stops_reading_cannot_stall_the_producer() {
        /// A sink that never returns from a write, standing in for a pipe to a
        /// front end that stopped reading.
        struct Wedged(Arc<std::sync::atomic::AtomicBool>);
        impl Write for Wedged {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                while !self.0.load(std::sync::atomic::Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let emitter = Emitter::to(Box::new(Wedged(release.clone())));
        let mut progress = Progress::for_agent(&emitter, "agent-1").unwrap();
        // Far more than the queue holds. If feeding blocked, this would hang
        // rather than fail, which is what it did before the writer thread.
        let started = std::time::Instant::now();
        for n in 0..PROGRESS_QUEUE * 4 {
            progress.feed(format!("line {n}\n").as_bytes());
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "feeding waited on the consumer"
        );
        assert!(
            progress.dropped > 0,
            "nothing was dropped, so nothing ever filled"
        );
        release.store(true, std::sync::atomic::Ordering::SeqCst);
        progress.finish();
    }

    /// The same framing, two producers. A tool's output is a tool's, a
    /// sub-agent's is its own, and neither is mistaken for the other.
    #[test]
    fn each_producer_gets_its_own_frame_type() {
        let buf = Buf::default();
        let emitter = Emitter::to(Box::new(buf.clone()));
        let mut child = Progress::for_agent(&emitter, "agent-3").unwrap();
        child.feed(b"child says hello\n");
        child.finish();
        as_call("c9", || {
            let mut tool = Progress::for_current_call(&emitter).unwrap();
            tool.feed(b"tool says hello\n");
            tool.finish();
        });
        let frames = buf.frames();
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
}
