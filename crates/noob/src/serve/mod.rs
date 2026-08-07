//! The `serve` subcommand: noob-proto Command frames on stdin, Event frames
//! on stdout. The surface a front end attaches to.

use std::io::Write;
use std::process::ExitCode;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use noob_provider::http::INTERRUPTED;
use noob_provider::types::{Item, Overrides};

use crate::ui::Ui;
use crate::agent::RunEnd;
use crate::boot::{BootArgs, bootstrap, value_for};
use crate::emit;

/// Every frame to stdout, and to the session's record beside it once one is
/// attached. The record is what a resumed session replays, so the window
/// gets the whole picture back; a record that cannot be written costs the
/// replay, never the session.
struct Tee {
    out: std::io::Stdout,
    record: Arc<Mutex<Option<std::fs::File>>>,
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut file) = self.record.lock()
            && let Some(file) = file.as_mut()
        {
            let _ = file.write_all(buf);
        }
        self.out.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Ok(mut file) = self.record.lock()
            && let Some(file) = file.as_mut()
        {
            let _ = file.flush();
        }
        self.out.flush()
    }
}

/// Where a session's frame record lives: beside its transcript.
fn record_path(session: &std::path::Path) -> std::path::PathBuf {
    session.with_extension("frames.jsonl")
}

/// Send a recorded session's frames back down the live stream, skipping the
/// lifecycle frames the fresh boot has already said. Junk lines are skipped
/// the way every reader of this protocol skips them.
fn replay_record(emitter: &emit::Emitter, path: &std::path::Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let mut sent = false;
    for line in text.lines() {
        let Some(frame) = noob_proto::decode::<noob_proto::Event>(line) else {
            continue;
        };
        match frame.body {
            // Lifecycle frames the fresh boot already said, and questions to
            // a live human: a replayed ask has nobody waiting on its answer.
            noob_proto::Event::SessionStart { .. }
            | noob_proto::Event::SessionEnd { .. }
            | noob_proto::Event::Ask { .. } => {}
            body => {
                emitter.send(body);
                sent = true;
            }
        }
    }
    sent
}

/// The fallback for a session recorded before frames were: the transcript
/// itself, mapped to the frames it would have produced. Coarser than a
/// record (no timings, no progress, no file diffs), but every prompt,
/// answer and call comes back.
fn replay_transcript(emitter: &emit::Emitter, items: &[Item]) {
    let mut turn = 0u32;
    let mut open = false;
    for item in items {
        match item {
            Item::User(text) => {
                if open {
                    emitter.send(noob_proto::Event::TurnEnd {
                        turn,
                        interrupted: None,
                    });
                }
                turn += 1;
                open = true;
                emitter.send(noob_proto::Event::TurnStart { turn });
                emitter.send(noob_proto::Event::UserEcho { text: text.clone() });
            }
            Item::Assistant { text, tool_calls, .. } => {
                if !text.is_empty() {
                    emitter.send(noob_proto::Event::TextDelta {
                        d: format!("{text}\n"),
                    });
                }
                for call in tool_calls {
                    let args: noob_proto::Value =
                        serde_json::from_str(&call.arguments).unwrap_or(noob_proto::Value::Null);
                    emitter.send(noob_proto::Event::ToolStart {
                        call_id: call.id.clone(),
                        name: call.name.clone(),
                        brief: String::new(),
                        args,
                    });
                }
            }
            Item::ToolResult { call_id, content } => {
                emitter.send(noob_proto::Event::ToolEnd {
                    call_id: call_id.clone(),
                    summary: content.lines().next().unwrap_or("").to_string(),
                    elapsed_ms: 0,
                    error: None,
                });
            }
        }
    }
    if open {
        emitter.send(noob_proto::Event::TurnEnd {
            turn,
            interrupted: None,
        });
    }
}

/// The surface a front end drives. It is a separate subcommand rather than a
/// mode of the REPL for one reason: the REPL owns the terminal outright, in raw
/// mode, reading STDIN_FILENO through libc. There is no seam to add a second
/// input source to that does not also change what a human sees. Here there is
/// no terminal at all, so the two halves of the protocol are just stdin and
/// stdout, and nothing about the interactive surface has to move.
///
/// Frames the agent has no answer for are ignored rather than refused, which
/// is the same degradation rule the protocol applies to unknown frames: a
/// front end built against a newer agent loses a feature, not its session.
pub(crate) fn run(args: &[String]) -> ExitCode {
    const USAGE: &str = "usage: noob serve [--resume <id> | --session <id>] [--model <name>] \
                         [--base-url <url>] [--plan] [--verbose] [--yolo]";
    let mut ov = Overrides::default();
    let mut yolo = false;
    let mut plan = false;
    let mut verbose = false;
    // A front end is a long-lived conversation, so persistence is the default
    // here where it is opt-in for exec: a fresh id unless one is named.
    let mut session_id: Option<String> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let taken = match arg.as_str() {
            "--model" => value_for(arg, it.next(), USAGE).map(|v| ov.model = Some(v)),
            "--base-url" => value_for(arg, it.next(), USAGE).map(|v| ov.base_url = Some(v)),
            "--resume" | "--session" | "--restore" => {
                value_for(arg, it.next(), USAGE).map(|v| session_id = Some(v))
            }
            "--yolo" => {
                yolo = true;
                Ok(())
            }
            "--plan" => {
                plan = true;
                Ok(())
            }
            "--verbose" => {
                verbose = true;
                Ok(())
            }
            other => Err(format!("noob serve: unknown flag {other:?}; {USAGE}")),
        };
        if let Err(msg) = taken {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    }

    let mut ui = Ui::serve();
    // Line-buffered stdout would hold a frame until a newline it already has,
    // and the emitter flushes per frame anyway, so this is the whole
    // transport - teed into the session's record once one is known.
    let record = Arc::new(Mutex::new(None));
    let emitter = emit::Emitter::to(Box::new(Tee {
        out: std::io::stdout(),
        record: record.clone(),
    }));
    let mut boot = BootArgs::new(ov, yolo, plan, Some(session_id));
    boot.verbose = verbose;
    boot.emitter = Some(emitter.clone());
    let (mut agent, _) = match bootstrap(boot, &mut ui) {
        Ok(pair) => pair,
        Err(e) => {
            // Before any frame has been written, so a front end that cannot
            // start gets a readable reason rather than an empty stream.
            emitter.send(noob_proto::Event::Error {
                line: format!("noob: {e}"),
            });
            eprintln!("noob: {e}");
            return ExitCode::from(2);
        }
    };

    // A resumed session replays its whole picture before anything new: the
    // recorded frames when this session has them, the transcript mapped to
    // frames when it predates the record. Only then does the record open
    // for appending, so a replay can never read its own writes.
    if let Some(session) = agent.session.as_ref() {
        let path = record_path(session.path());
        let recorded = replay_record(&emitter, &path);
        // Attached after the read, so a replay can never read its own
        // writes; before the fallback, so a session that predates the
        // record gets one now and replays in full next time.
        if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            *record.lock().expect("no holder panics") = Some(file);
        }
        if !recorded && !agent.items.is_empty() {
            let harness = [crate::agent::PLAN_ENTER_MSG, crate::agent::BUDGET_NUDGE_MSG];
            let shown: Vec<Item> = agent
                .items
                .iter()
                .filter(|item| !matches!(item, Item::User(text) if harness.contains(&text.as_str())))
                .cloned()
                .collect();
            replay_transcript(&emitter, &shown);
        }
    }

    // A front end drives a long-lived conversation, same as the dock, so
    // sub-agents detach here too. Without this they run inline, the session
    // blocks for the length of the longest child, and the agent.* frames a
    // front end draws its fleet from never exist.
    agent.enable_background_agents(&mut ui);

    // Commands are read on their own thread so a cancel lands while a turn is
    // running rather than after it. The turn itself stays on the main thread,
    // one at a time, which is what keeps the transcript ordered.
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let (ask_tx, ask_rx) = std::sync::mpsc::channel::<(u64, bool)>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut line = String::new();
        loop {
            line.clear();
            match std::io::BufRead::read_line(&mut stdin.lock(), &mut line) {
                Ok(0) | Err(_) => break, // the front end closed
                Ok(_) => {}
            }
            let Some(frame) = noob_proto::decode::<noob_proto::Command>(&line) else {
                continue;
            };
            match frame.body {
                // Both queue: a prompt typed while the agent is working is
                // almost always the next thing to do, not a reason to throw
                // away the work in flight. Cancelling is its own command.
                noob_proto::Command::PromptSubmit { text }
                | noob_proto::Command::PromptQueue { text } => {
                    if tx.send(text).is_err() {
                        break;
                    }
                }
                noob_proto::Command::TurnCancel => {
                    INTERRUPTED.store(true, Ordering::SeqCst);
                }
                noob_proto::Command::AskAnswer { ask_id, yes } => {
                    let _ = ask_tx.send((ask_id, yes));
                }
                _ => {}
            }
        }
    });

    // Confirmations (a skill install, a write into a skills directory) become
    // Ask frames the front end answers. Asks are serial because the turn
    // blocks on each one, so an unmatched id is an answer to a question no
    // longer pending and is dropped; a canceled turn or a closed stream
    // reads as no, the same degradation as every unanswerable surface.
    let ask_emitter = emitter.clone();
    let mut next_ask = 0u64;
    ui.set_ask_hook(Box::new(move |question| {
        next_ask += 1;
        let ask_id = next_ask;
        ask_emitter.send(noob_proto::Event::Ask {
            ask_id,
            question: question.to_string(),
        });
        loop {
            match ask_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok((id, yes)) if id == ask_id => return yes,
                Ok(_) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if INTERRUPTED.load(Ordering::SeqCst) {
                        return false;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return false,
            }
        }
    }));

    loop {
        // A detached child settles on its own schedule, after the turn that
        // started it. Deliver each report as it lands rather than at whatever
        // the human types next, which may be never.
        //
        // One snapshot decides both questions. Two would let a child that
        // settles between them be counted as neither ready nor running, and
        // its report would sit undelivered until the human typed again.
        let mut fleet = agent.background_snapshot();
        while fleet.ready > 0 {
            match agent.continue_after_background(&mut ui) {
                RunEnd::Completed(_) | RunEnd::Interrupted | RunEnd::Aborted(_) => {}
            }
            fleet = agent.background_snapshot();
        }
        // Poll only while children are actually running. Idle is a blocking
        // wait, so a session with nothing in flight costs nothing at all.
        let text = if fleet.active > 0 {
            match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(text) => text,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(text) => text,
                Err(_) => break,
            }
        };
        if text.trim().is_empty() {
            continue;
        }
        // The prompt goes into the record and only the record: the front end
        // that sent it already echoed it, and a replay is the one reader
        // that needs it back.
        if let Ok(mut file) = record.lock()
            && let Some(file) = file.as_mut()
        {
            let echo = noob_proto::Event::UserEcho { text: text.clone() };
            let _ = file.write_all(noob_proto::encode(&echo).as_bytes());
        }
        // A cancel that arrived between turns has nothing to cancel; leaving
        // it set would kill the turn about to start.
        INTERRUPTED.store(false, Ordering::SeqCst);
        match agent.run_input(&text, &mut ui) {
            RunEnd::Completed(_) | RunEnd::Interrupted => {}
            // Already emitted as an error frame by the turn itself; the loop
            // keeps going because the next prompt may well work.
            RunEnd::Aborted(_) => {}
        }
    }

    agent.shutdown_background_agents(&mut ui);
    emitter.send(noob_proto::Event::SessionEnd {
        id: agent
            .session
            .as_ref()
            .map(|s| s.id().to_string())
            .unwrap_or_default(),
    });
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The transcript mapper: every prompt, answer and call comes back as
    /// the frames it would have produced, turns bracketed, calls paired by
    /// id so nothing replays as still running or as parallel.
    #[test]
    fn a_transcript_replays_as_the_frames_it_would_have_produced() {
        let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        struct Buf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl Write for Buf {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let emitter = emit::Emitter::to(Box::new(Buf(sink.clone())));
        replay_transcript(
            &emitter,
            &[
                Item::User("hello".into()),
                Item::Assistant {
                    text: "hi there".into(),
                    tool_calls: vec![noob_provider::types::ToolCall {
                        id: "c1".into(),
                        name: "read".into(),
                        arguments: r#"{"path":"a.rs"}"#.into(),
                    }],
                    raw_items: Vec::new(),
                },
                Item::ToolResult {
                    call_id: "c1".into(),
                    content: "read 3 lines\nmore".into(),
                },
                Item::User("and now?".into()),
                Item::Assistant {
                    text: "done".into(),
                    tool_calls: Vec::new(),
                    raw_items: Vec::new(),
                },
            ],
        );
        let text = String::from_utf8(sink.lock().unwrap().clone()).unwrap();
        let tags: Vec<String> = text
            .lines()
            .filter_map(noob_proto::decode::<noob_proto::Event>)
            .map(|frame| {
                use noob_proto::Body;
                frame.body.tag().to_string()
            })
            .collect();
        assert_eq!(
            tags,
            [
                "turn.start",
                "user.echo",
                "text.delta",
                "tool.start",
                "tool.end",
                "turn.end",
                "turn.start",
                "user.echo",
                "text.delta",
                "turn.end"
            ],
            "{text}"
        );
        assert!(text.contains(r#""text":"hello""#), "{text}");
        assert!(text.contains(r#""summary":"read 3 lines""#), "{text}");
    }

    /// The record lives beside the transcript it belongs to.
    #[test]
    fn the_record_stands_beside_its_session() {
        assert_eq!(
            record_path(std::path::Path::new("/tmp/s/abc.jsonl")),
            std::path::Path::new("/tmp/s/abc.frames.jsonl")
        );
    }
}
