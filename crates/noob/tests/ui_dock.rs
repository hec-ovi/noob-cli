//! The dock driver (the product default; NOOB_DOCK=0 is the classic opt-out):
//! the persistent-input REPL where the input frame stays live during a turn.
//! These prove the driver against the same bar as the classic editor: what
//! reaches the agent, never how it looks. Mid-turn typing lands in the draft,
//! confirmations travel the dock's modal, Esc Esc / Ctrl-C cancel exactly
//! what they claim, and Enter on a running turn queues instead of steering.

mod ui;

use ui::*;

#[test]
fn dock_is_default_and_liveness_survives_first_output() {
    let rig = rig();
    rig.server.enqueue_stream_completion("default dock reply");

    // No NOOB_DOCK variable: the persistent driver is the default.
    let mut pty = spawn_pty_with(&rig, &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.wait_for("default dock reply");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let reply = pty.seen().find("default dock reply").unwrap();
    let last_working = pty.seen().rfind("Working").unwrap();
    assert!(
        last_working > reply,
        "whole-turn liveness disappeared after the first output:\n{}",
        pty.seen()
    );
    rig.server.assert_clean();
}

#[test]
fn interactive_model_markdown_renders_headings_code_json_and_tables() {
    let rig = rig();
    rig.server.enqueue_stream_completion(
        "### Status\n**ready** with `inline`\n```json\n{\"ok\": true, \"n\": 2}\n```\n\
         | name | state |\n| :--- | ---: |\n| noob | ready |\nRENDER-END",
    );

    let mut pty = spawn_pty_with(&rig, &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"show formatting\r");
    pty.wait_for("RENDER-END");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    assert!(
        !pty.seen().contains("### Status"),
        "heading markdown leaked as source"
    );
    assert!(
        !pty.seen().contains("**ready**"),
        "bold markdown leaked as source"
    );
    assert!(
        !pty.seen().contains("```json"),
        "fence markdown leaked as source"
    );
    assert!(
        pty.seen().contains("┌─ ") && pty.seen().contains("json"),
        "JSON fence lost its labelled gutter"
    );
    assert!(
        pty.seen().contains('┬'),
        "the table was not laid out as a grid"
    );
    rig.server.assert_clean();
}

/// Dock parity with the classic editor: editing keys shape the line, only
/// the edited line reaches the agent, Ctrl-D exits with the session hint.
#[test]
fn dock_edits_and_submits_like_the_classic_editor() {
    let rig = rig();
    rig.server.enqueue_stream_completion("docked reply");

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY); // the session guard went raw (once per session)
    pty.send(b"garbage draft");
    pty.send(&[0x15]); // Ctrl-U kills the line
    pty.send(b"say hi\r");
    // Streamed words arrive as separate deltas with dock repaints between
    // them, so multi-word markers would never match contiguously.
    pty.wait_for("docked");
    pty.wait_for("reply");
    settle(); // the next prompt has no raw-toggle marker: raw spans the session
    pty.send(&[0x04]); // Ctrl-D at the empty prompt exits
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1, "only the edited line should have run");
    assert_eq!(
        last_user(&reqs[0]),
        "say hi",
        "the killed draft leaked into the message"
    );
    rig.server.assert_clean();
}

/// The root corruption the dock exists to fix: keystrokes during a streaming
/// turn are captured into the live draft (nothing echoes into the model's
/// output), survive the turn, and submit as the NEXT message. The reply text
/// itself arrives intact around the stall.
#[test]
fn dock_captures_typing_during_a_slow_stream() {
    let rig = rig();
    let datas = noob_testkit::chat_stream_datas("Alpha waits then finishes cleanly.");
    // Stall the stream after the first content word, long enough to type.
    let mut steps = vec![noob_testkit::RawStep::Bytes({
        let mut b = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n".to_vec();
        b.extend_from_slice(&sse_frames(&datas[..2])); // role delta + "Alpha "
        b
    })];
    steps.push(noob_testkit::RawStep::SleepMs(900));
    steps.push(noob_testkit::RawStep::Bytes({
        let mut b = sse_frames(&datas[2..]);
        b.extend_from_slice(b"0\r\n\r\n");
        b
    }));
    rig.server.enqueue_raw(steps);
    rig.server.enqueue_stream_completion("second turn ran");

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start\r");
    pty.wait_for("Alpha"); // the stream is up, now inside the 900 ms stall
    pty.send(b"queued while busy"); // typed mid-turn: must land in the draft
    pty.wait_for("finishes");
    pty.wait_for("cleanly.");
    settle(); // back at the prompt, the draft already in the input row
    pty.send(b"\r"); // submit the captured draft as the next message
    pty.wait_for("second");
    pty.wait_for("ran");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let reqs = rig.api_requests();
    assert_eq!(
        reqs.len(),
        2,
        "the mid-turn typing must not fire its own request"
    );
    assert_eq!(last_user(&reqs[0]), "start");
    assert_eq!(
        last_user(&reqs[1]),
        "queued while busy",
        "the mid-turn draft must submit whole as the next message"
    );
    rig.server.assert_clean();
}

/// A confirmation raised by agent code mid-turn (the skills-dir write gate)
/// is answered from the keyboard through the dock's modal: the reader thread
/// owns stdin, so the ask must travel the event channel and back.
#[test]
fn dock_answers_a_mid_turn_confirmation() {
    let rig = rig();
    rig.server.enqueue_stream_toolcalls(
        &[(
            "call_1",
            "write",
            r#"{"path": ".claude/skills/made/SKILL.md", "content": "---\nname: made\ndescription: test\n---\nbody\n"}"#,
        )],
        None,
    );
    rig.server.enqueue_stream_completion("skill written");

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"make a skill\r");
    pty.wait_for("[y/N]"); // the gate's question, rendered by the dock modal
    pty.send(b"y\r");
    pty.wait_for("skill");
    pty.wait_for("written");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let written = rig.work.path().join(".claude/skills/made/SKILL.md");
    assert!(written.is_file(), "the granted write must have executed");
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2, "toolcall turn + result turn");
    rig.server.assert_clean();
}

#[test]
fn dock_double_esc_cancels_an_open_confirmation_and_the_tool_batch() {
    let rig = rig();
    rig.server.enqueue_stream_toolcalls(
        &[(
            "call_1",
            "write",
            r#"{"path": ".claude/skills/nope/SKILL.md", "content": "never"}"#,
        )],
        None,
    );

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"try the write\r");
    pty.wait_for("[y/N]");
    pty.send(b"\x1b\x1b");
    pty.wait_for("[interrupted]");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    assert!(
        !rig.work
            .path()
            .join(".claude/skills/nope/SKILL.md")
            .exists()
    );
    assert_eq!(
        rig.api_requests().len(),
        1,
        "the canceled batch must not continue"
    );
    rig.server.assert_clean();
}

#[test]
fn dock_typeahead_before_an_ask_cannot_confirm_it() {
    let rig = rig();
    let datas = noob_testkit::chat_stream_toolcalls_datas(
        &[(
            "call_1",
            "write",
            r#"{"path": ".claude/skills/nope/SKILL.md", "content": "never"}"#,
        )],
        None,
    );
    let mut tail = sse_frames(&datas);
    tail.extend_from_slice(b"0\r\n\r\n");
    rig.server.enqueue_raw(vec![
        noob_testkit::RawStep::Bytes(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n"
                .to_vec(),
        ),
        noob_testkit::RawStep::SleepMs(500),
        noob_testkit::RawStep::Bytes(tail),
    ]);

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"try the write\r");
    pty.wait_for("Working");
    pty.send(b"y"); // type-ahead before the question exists, never consent
    pty.wait_for("[y/N]"); // still waiting for a fresh answer
    pty.send(b"\x1b\x1b");
    pty.wait_for("[interrupted]");
    pty.wait_for("y"); // canceled queue returned to the editable draft
    pty.send(&[0x15]);
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    assert!(
        !rig.work
            .path()
            .join(".claude/skills/nope/SKILL.md")
            .exists()
    );
    assert_eq!(rig.api_requests().len(), 1);
    rig.server.assert_clean();
}

/// Review fix (high): in dock mode /compact runs its summarizer request
/// through the render loop, so a keyboard Ctrl-C (a raw byte, not SIGINT)
/// still cancels it. Without the fix the byte is captured by the reader and
/// never sets INTERRUPTED, so the request is uninterruptible for up to 300s.
/// Here the summarizer stalls; Ctrl-C must cancel within ~1 watchdog tick.
#[test]
fn dock_compact_is_cancelable_with_ctrl_c() {
    let rig = rig();
    // One bulky text reply (no tool result, so pruning saves nothing) gives
    // compaction a middle and forces the summarizer LLM call. The END marker
    // lets the test wait for the whole reply to stream so it is back at an idle
    // prompt before /compact; the mock reports tiny usage, so auto-compaction
    // never fires on its own.
    // The reply must exceed the tail budget (NOOB_CTX/4 = 1024 tokens ≈ 4 KiB)
    // on its own so it does not all fit in the retained tail, leaving a middle
    // of >= 2 items for the summarizer.
    rig.server
        .enqueue_stream_completion(&format!("reply {} END-ONE", "x".repeat(6000)));
    // The summarizer request: 200 headers, then a long silence. The watchdog
    // first-byte budget is 300s, so only INTERRUPTED can end this early.
    rig.server.enqueue_raw(vec![
        noob_testkit::RawStep::Bytes(noob_testkit::sse_headers()),
        noob_testkit::RawStep::SleepMs(8000),
    ]);
    // The summarizer request is the sanctioned compaction prefix break.
    rig.server.expect_prefix_break();

    // NOOB_CTX floors at 4096; a smaller value silently reverts to the default.
    let mut pty = spawn_pty_with(&rig, &[("NOOB_DOCK", "1"), ("NOOB_CTX", "4096")]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start\r");
    pty.wait_for("END-ONE"); // the whole reply has streamed; the turn is ending
    settle(); // back at the idle prompt (a mid-turn Enter is inert pre-queue)
    pty.send(b"/compact\r");
    pty.wait_for("compacting"); // the summarizer request is now in flight, stalled
    pty.send(b"keep this draft");
    pty.wait_for("keep this draft");
    pty.send(&[0x03]); // Ctrl-C: a raw byte in dock mode, must still cancel
    pty.wait_for("compaction canceled"); // the watchdog tripped via INTERRUPTED
    pty.wait_for("keep this draft"); // canceled auxiliary turns restore queued input
    pty.send(&[0x15]); // clear the restored draft
    pty.send(&[0x04]); // Ctrl-D exits
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2, "the driving turn + the canceled summarizer");
    // The 2nd request is the summarizer (compact.md system prompt), proving
    // the cancel hit the compaction request, not a normal turn.
    let sys = reqs[1]["messages"][0]["content"].as_str().unwrap_or("");
    assert!(
        sys.contains("summarize an agent session"),
        "2nd req not the summarizer: {sys}"
    );
    rig.server.assert_clean();
}

#[test]
fn dock_second_ctrl_c_hard_exits_with_terminal_restore() {
    let rig = rig();
    rig.server
        .enqueue_raw(stalled_stream("Working END-NEVER", 2, 8000, false));

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.send(&[0x03, 0x03]);
    pty.wait_for("\x1b[?2004l");
    let status = pty.finish();

    assert_eq!(
        status.code(),
        Some(130),
        "hard cancel: {status:?};\n{}",
        pty.seen()
    );
    assert!(
        pty.seen().contains("\x1b[?2004l"),
        "hard exit did not restore terminal modes:\n{}",
        pty.seen()
    );
}

/// Review fix (medium): Ctrl-D at a mid-turn y/N confirmation denies (the
/// contract: anything but an explicit yes is No) instead of being swallowed.
/// The same Key::Eof path also unblocks the worker if the reader dies while a
/// modal is open, which would otherwise hang the render loop forever.
#[test]
fn dock_ctrl_d_at_a_confirmation_denies_and_continues() {
    let rig = rig();
    rig.server.enqueue_stream_toolcalls(
        &[(
            "call_1",
            "write",
            r#"{"path": ".claude/skills/nope/SKILL.md", "content": "---\nname: nope\ndescription: t\n---\nb\n"}"#,
        )],
        None,
    );
    // After the denial the tool result is a refusal; the agent continues and
    // the mock answers the follow-up turn.
    rig.server.enqueue_stream_completion("left it alone");

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"make a skill\r");
    pty.wait_for("[y/N]");
    pty.send(&[0x04]); // Ctrl-D at the confirmation: deny
    pty.wait_for("left");
    pty.wait_for("alone");
    settle();
    pty.send(&[0x04]); // Ctrl-D at the empty prompt: exit
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let denied = rig.work.path().join(".claude/skills/nope/SKILL.md");
    assert!(
        !denied.is_file(),
        "the write must have been denied, not executed"
    );
    rig.server.assert_clean();
}

/// M5 (double-ESC cancel): a first ESC during a turn arms a red hint; a second
/// ESC inside the window commits, setting INTERRUPTED so the watchdog trips the
/// in-flight read and the agent finalizes the turn with `[interrupted]`.
#[test]
fn dock_double_esc_cancels_a_running_turn() {
    let rig = rig();
    // Stream one word then stall indefinitely; only a cancel ends it.
    rig.server
        .enqueue_raw(stalled_stream("Working END-NEVER", 2, 8000, false));

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working"); // the turn is streaming, now stalled
    pty.send(&[0x1b]); // first ESC: arm
    pty.wait_for("press ESC again to cancel"); // the red hint appears
    pty.send(&[0x1b]); // second ESC: commit the cancel
    pty.wait_for("[interrupted]"); // the agent finalized the canceled turn
    settle();
    pty.send(&[0x04]); // Ctrl-D exits
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    assert!(
        !pty.seen().contains("END-NEVER"),
        "the stalled tail must never have streamed"
    );
    rig.server.assert_clean();
}

/// M5: a single ESC only arms; if no second ESC lands the turn runs to
/// completion. Here the stream resumes after the arm and the reply finishes
/// normally, with no interrupt.
#[test]
fn dock_single_esc_does_not_cancel() {
    let rig = rig();
    // One word, a short stall, then the rest of the reply and a clean close.
    rig.server
        .enqueue_raw(stalled_stream("Working through it END-OK", 2, 1500, true));

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.send(&[0x1b]); // a lone ESC: arms only
    pty.wait_for("press ESC again to cancel");
    // No second ESC. The stall lapses, the rest streams, the turn completes.
    pty.wait_for("END-OK");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    assert!(
        !pty.seen().contains("[interrupted]"),
        "one ESC must not cancel the turn"
    );
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1, "exactly the one turn ran to completion");
    rig.server.assert_clean();
}

/// Explicit cancellation keeps unsubmitted type-ahead as an editable draft and
/// does not dispatch it. This remains distinct from Enter steering above.
#[test]
fn dock_interrupt_preserves_the_unsubmitted_draft() {
    let rig = rig();
    rig.server
        .enqueue_raw(stalled_stream("Working END-NEVER", 2, 8000, false));

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.send(b"hold me");
    pty.wait_for("hold me");
    pty.send(b"\x1b\x1b"); // both taps in one kernel read must still cancel
    pty.wait_for("[interrupted]");
    // The unsubmitted draft remains editable and was not dispatched.
    pty.wait_for("hold me");
    pty.send(&[0x15]); // Ctrl-U clears the restored draft
    pty.send(&[0x04]); // Ctrl-D on the now-empty line exits
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let reqs = rig.api_requests();
    assert_eq!(
        reqs.len(),
        1,
        "the draft must not dispatch after explicit cancel"
    );
    rig.server.assert_clean();
}

/// Enter on a non-empty running-turn draft QUEUES the message and touches
/// nothing: the running turn (its provider request and its tools) completes
/// on its own, and the queued message dispatches on the next REPL iteration.
/// While it waits, the message is a pinned `› message [queued]` row above the
/// input; dispatch replaces it with the plain `› message` transcript record,
/// so no [queued] marker survives once the message is answered. The screen
/// never says steering, turn stopped, or interrupted; the wire carries no
/// [interrupted] marker at all. Esc Esc remains the only way to stop a turn.
#[test]
fn dock_enter_queues_and_dispatches_after_the_turn_completes() {
    const ROWS: u16 = 20;
    const COLS: u16 = 80;

    let rig = rig();
    // Turn 1 runs a real (finite) tool. Enter mid-tool must not stop it; the
    // queued message becomes turn 2 only after turn 1 finishes naturally.
    rig.server
        .enqueue_stream_toolcalls(&[("slow-tool", "bash", r#"{"cmd":"sleep 2"}"#)], None);
    rig.server.enqueue_stream_completion("first turn done");
    rig.server.enqueue_stream_completion("queued turn done");

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("sleep 2");
    pty.send(b"queue now\r");
    pty.wait_for("[queued]"); // the pinned acceptance row while it waits
    // The tool and its turn complete untouched, then the queued turn runs.
    pty.wait_for("first turn done");
    pty.wait_for("queued turn done");
    settle();
    pty.drain(std::time::Duration::from_millis(400));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();

    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    // Once answered, the message is a plain transcript record above its
    // answer; the [queued] marker died with the pinned row.
    let joined = rows.join("\n");
    assert!(
        !joined.contains("[queued]"),
        "the [queued] marker must vanish once the message is answered:\n{}",
        screen.dump("after the queued turn")
    );
    let record = rows
        .iter()
        .position(|r| r.contains(&format!("{MARKER} queue now")))
        .unwrap_or_else(|| {
            panic!(
                "the dispatched message must be a plain record:\n{}",
                screen.dump("after the queued turn")
            )
        });
    let answer = rows
        .iter()
        .position(|r| r.contains("queued turn done"))
        .expect("the queued answer is on screen");
    assert!(
        record < answer,
        "the record precedes its answer (record {record}, answer {answer}):\n{}",
        screen.dump("after the queued turn")
    );
    for noise in ["[steering]", "turn stopped", "[interrupted]", "canceling"] {
        assert!(
            !pty.seen().contains(noise),
            "a queued message must not surface {noise:?}:\n{}",
            pty.seen()
        );
    }
    let reqs = rig.api_requests();
    assert_eq!(
        reqs.len(),
        3,
        "the tool round, its completing follow-up, then the queued turn"
    );
    assert_eq!(last_user(&reqs[0]), "go");
    assert_eq!(last_user(&reqs[2]), "queue now");
    let messages = reqs[2]["messages"].as_array().unwrap();
    assert!(
        messages
            .iter()
            .all(|message| message["content"] != "[interrupted]"),
        "queueing must not interrupt anything in-band: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| { message["role"] == "tool" && message["tool_call_id"] == "slow-tool" }),
        "the tool call completed and its result reached the wire: {messages:?}"
    );
    rig.server.assert_clean();
}

/// Several messages queued during one turn dispatch in order, one turn each;
/// the bottom rule counts them while they wait, each waits as its own pinned
/// [queued] row, and after both answers the records read as plain `› message`
/// lines with no [queued] marker left anywhere on screen.
#[test]
fn dock_queues_multiple_messages_fifo() {
    const ROWS: u16 = 20;
    const COLS: u16 = 80;

    let rig = rig();
    rig.server
        .enqueue_stream_toolcalls(&[("slow-tool", "bash", r#"{"cmd":"sleep 2"}"#)], None);
    rig.server.enqueue_stream_completion("turn one done");
    rig.server.enqueue_stream_completion("answer alpha");
    rig.server.enqueue_stream_completion("answer beta");

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("sleep 2");
    pty.send(b"first question\r");
    pty.wait_for("[queued]");
    pty.send(b"second question\r");
    pty.wait_for("2 queued");
    pty.wait_for("turn one done");
    pty.wait_for("answer alpha");
    pty.wait_for("answer beta");
    settle();
    pty.drain(std::time::Duration::from_millis(400));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();

    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let joined = rows.join("\n");
    assert!(
        !joined.contains("[queued]"),
        "no [queued] marker may survive the answers:\n{}",
        screen.dump("after both queued turns")
    );
    for (question, answer) in [
        ("first question", "answer alpha"),
        ("second question", "answer beta"),
    ] {
        let record = rows
            .iter()
            .position(|r| r.contains(&format!("{MARKER} {question}")))
            .unwrap_or_else(|| {
                panic!(
                    "{question:?} must be a plain record:\n{}",
                    screen.dump("after both queued turns")
                )
            });
        let answered = rows
            .iter()
            .position(|r| r.contains(answer))
            .expect("answer on screen");
        assert!(
            record < answered,
            "{question:?} precedes {answer:?}:\n{}",
            screen.dump("after both queued turns")
        );
    }
    let reqs = rig.api_requests();
    assert_eq!(
        reqs.len(),
        4,
        "tool round, its follow-up, then both queued turns"
    );
    assert_eq!(last_user(&reqs[2]), "first question");
    assert_eq!(last_user(&reqs[3]), "second question");
    rig.server.assert_clean();
}
