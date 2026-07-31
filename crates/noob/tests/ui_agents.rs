//! Detached sub-agents through the UI: spawn rounds end the input, the fleet
//! stays visible (pinned counter, Tab detail view), children report exactly
//! once, cancellation flows through every path (Esc Esc, /agents cancel, the
//! model's own cancel call), and nothing a child does steals or interrupts
//! the human's prompt.

mod ui;

use noob_testkit::RequestMatch;
use serde_json::Value;

use ui::*;

/// The live-caught combination: a detached child is running AND the parent
/// turn is inside a real bash when the user types a follow-up. The message
/// queues; the bash, its turn, and the child are all untouched; the queued
/// message is answered as the next turn; and the child still delivers its
/// report afterward. No [interrupted] marker exists anywhere on the wire.
#[test]
fn dock_queue_during_bash_with_a_running_agent_answers_after_the_turn() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());

    // One MIXED batch: the spawn is paired with a finite REAL command (a
    // spawn-only round would end the input under the detached contract, and
    // a leading sleep would be refused by the agent-wait block; queueing
    // must work when genuine work is in flight after a spawn).
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[
            (
                "bg-call",
                "subagent",
                r#"{"prompt":"slow standalone research"}"#,
            ),
            ("wait-call", "bash", r#"{"cmd":"sleep 2"}"#),
        ],
        None,
    );
    rig.server.enqueue_raw_for(
        child(),
        stalled_stream("CHILD-RESULT-UNIQUE", 1, 2500, true),
    );
    rig.server
        .enqueue_stream_completion_for(parent(), "PARENT-TURN-END");
    rig.server
        .enqueue_stream_completion_for(parent(), "QUEUED-ANSWER-END");
    rig.server
        .enqueue_stream_completion_for(parent(), "AGENT-COLLECTED-END");

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start research\r");
    pty.wait_for("[1] agents running (Tab to view)");
    // Queue off the spinner's live "· bash" label, not the scrollback start
    // line: the dock emits that line before repainting the pinned agents row,
    // so a wait on the row would already have consumed past it.
    pty.wait_for("· bash");
    pty.send(b"queue now\r");
    pty.wait_for("[queued]");
    // The parent's bash and turn complete untouched, the queued message is
    // answered next, and the child's report still arrives on its own.
    pty.wait_for("PARENT-TURN-END");
    pty.wait_for("QUEUED-ANSWER-END");
    pty.wait_for("AGENT-COLLECTED-END");
    quit_at_idle(&mut pty);
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    for noise in ["[steering]", "turn stopped", "[interrupted]", "canceling"] {
        assert!(
            !pty.seen().contains(noise),
            "a queued message must not surface {noise:?}:\n{}",
            pty.seen()
        );
    }
    let requests = rig.api_requests();
    let queued = requests
        .iter()
        .find(|request| last_user(request) == "queue now")
        .expect("queued turn request");
    let interrupts = queued["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "user" && message["content"] == "[interrupted]")
        .count();
    assert_eq!(interrupts, 0, "queueing interrupts nothing, ever: {queued}");
    rig.server.assert_clean();
}

/// The live scenario that motivated the agent-wait block, end to end: the
/// model spawns a detached researcher, tries to sleep-wait for it, gets the
/// structural refusal, and ends its turn. The user's next message is then a
/// PLAIN prompt turn: no steering, no cancellation, no interrupt anywhere on
/// screen, and the child's report still arrives on its own afterward.
#[test]
fn sleep_wait_is_refused_and_the_prompt_frees_without_steering() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());

    // A spawn-only round now ends the input on its own, so the sleep idiom
    // is only reachable from a turn that spawned AND kept real work: pair
    // the spawn with a quick command in one mixed batch.
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[
            ("bg-call", "subagent", r#"{"prompt":"deep research"}"#),
            ("prime-call", "bash", r#"{"cmd":"echo prime-ok"}"#),
        ],
        None,
    );
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[("wait-call", "bash", r#"{"cmd":"sleep 30 && echo waited"}"#)],
        None,
    );
    rig.server
        .enqueue_stream_completion_for(parent(), "SPAWNED-END");
    rig.server.enqueue_raw_for(
        child(),
        stalled_stream("CHILD-RESULT-UNIQUE", 1, 2500, true),
    );
    rig.server
        .enqueue_stream_completion_for(parent(), "CHAT-ANSWER-END");
    rig.server
        .enqueue_stream_completion_for(parent(), "COLLECTED-END");

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"research it\r");
    // The refusal ends the wait instantly; the turn completes on its own.
    pty.wait_for("SPAWNED-END");
    // The prompt is free while the child still runs: a normal message, not
    // steering.
    pty.send(b"can we keep talking?\r");
    pty.wait_for("CHAT-ANSWER-END");
    pty.wait_for("agent-1 ok");
    pty.wait_for("COLLECTED-END");
    quit_at_idle(&mut pty);
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());

    // The whole exchange shows zero interruption vocabulary.
    for noise in ["[steering]", "canceled", "[interrupted]"] {
        assert!(
            !pty.seen().contains(noise),
            "{noise:?} appeared in a flow with nothing to interrupt:\n{}",
            pty.seen()
        );
    }

    let requests = rig.api_requests();
    // The skip reached the model as the sleep call's result.
    assert!(
        requests.iter().any(|request| {
            request["messages"].as_array().unwrap().iter().any(|m| {
                m["role"] == "tool"
                    && m["tool_call_id"] == "wait-call"
                    && m["content"]
                        .as_str()
                        .is_some_and(|c| c.contains("sleep skipped"))
            })
        }),
        "the sleep skip never reached the model"
    );
    // A skip is not an error: the screen must not show a red error line
    // for it (the red-wall regression).
    assert!(
        !pty.seen().contains("error: sleep"),
        "the sleep skip rendered as an error:\n{}",
        pty.seen()
    );
    // And the user's message dispatched as an ordinary turn.
    assert!(
        requests
            .iter()
            .any(|request| last_user(request) == "can we keep talking?"),
        "the free-prompt message never dispatched"
    );
    rig.server.assert_clean();
}

/// The v0.3.5 live screenshot bug: after spawning a detached researcher the
/// model polled subagent {"status":true} every round, painting two scrollback
/// lines per poll for 1m41s while the human's follow-up turned into a
/// steering interrupt. The cap answers the first snapshot (so a "how is it
/// going?" question stays answerable), cans the second, and ends the input:
/// the follow-up is a plain turn and the report still arrives on its own.
#[test]
fn status_poll_loop_is_capped_and_the_prompt_frees() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());

    // A spawn-only round would end the input immediately; pair the spawn
    // with a quick command so the turn survives into the poll rounds this
    // test exists to cap.
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[
            ("bg-call", "subagent", r#"{"prompt":"deep research"}"#),
            ("prime-call", "bash", r#"{"cmd":"echo prime-ok"}"#),
        ],
        None,
    );
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[("poll-1", "subagent", r#"{"status":true}"#)],
        None,
    );
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[("poll-2", "subagent", r#"{"status":true}"#)],
        None,
    );
    rig.server.enqueue_raw_for(
        child(),
        stalled_stream("CHILD-RESULT-UNIQUE", 1, 2500, true),
    );
    rig.server
        .enqueue_stream_completion_for(parent(), "STILL-HERE-END");
    rig.server
        .enqueue_stream_completion_for(parent(), "COLLECTED-END");

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"research it\r");
    // Poll 1 gets a real snapshot (the first word is styled separately, so
    // match the tail on its own). The pinned agents row paints later, so it
    // is asserted on the final screen instead of waited on here.
    pty.wait_for("active · 0 ready");
    // Poll 2 is the cap: a calm canned skip, and the input ends right after.
    pty.wait_for("prompt freed");
    // The prompt is free while the child still runs: a plain message.
    pty.wait_for("type a message");
    pty.send(b"still there?\r");
    pty.wait_for("STILL-HERE-END");
    pty.wait_for("agent-1 ok");
    pty.wait_for("COLLECTED-END");
    quit_at_idle(&mut pty);
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());

    for noise in ["[steering]", "[interrupted]", "canceled", "round cap"] {
        assert!(
            !pty.seen().contains(noise),
            "{noise:?} appeared in a capped-poll flow:\n{}",
            pty.seen()
        );
    }
    // The snapshot digest painted exactly once: no per-poll scrollback spam.
    assert_eq!(
        pty.seen().matches("active · 0 ready").count(),
        1,
        "the status digest must appear exactly once:\n{}",
        pty.seen()
    );
    assert!(
        pty.seen().contains("[1] agents running (Tab to view)"),
        "the compact fleet row never rendered:\n{}",
        pty.seen()
    );
    let requests = rig.api_requests();
    // The canned cap reached the model as poll-2's tool result.
    assert!(
        requests.iter().any(|request| {
            request["messages"].as_array().unwrap().iter().any(|m| {
                m["role"] == "tool"
                    && m["tool_call_id"] == "poll-2"
                    && m["content"]
                        .as_str()
                        .is_some_and(|c| c.contains("polling stopped"))
            })
        }),
        "the poll cap never reached the model"
    );
    // And the follow-up dispatched as an ordinary turn.
    assert!(
        requests
            .iter()
            .any(|request| last_user(request) == "still there?"),
        "the free-prompt message never dispatched"
    );
    // Input one spent exactly three rounds (spawn, answered poll, capped
    // poll); the follow-up and the report collection add one each. A poll
    // loop grinding to the 50-round cap would blow this count.
    let parent_requests = requests
        .iter()
        .filter(|request| {
            request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["function"]["name"] == "subagent")
        })
        .count();
    assert_eq!(parent_requests, 5, "the poll loop burned extra rounds");
    rig.server.assert_clean();
}

/// Double-ESC is the stop-everything gesture. At the idle prompt, with the
/// fleet running and no turn in flight, the first ESC arms a visible hint and
/// the second cancels every detached child; each still delivers exactly one
/// canceled terminal packet, and the canceled batch spends no parent
/// inference.
#[test]
fn double_esc_at_idle_stops_all_detached_agents() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());

    // The two-spawn round ends the input by itself under the detached
    // contract; the idle prompt follows immediately.
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[
            ("bg-1", "subagent", r#"{"prompt":"alpha research"}"#),
            ("bg-2", "subagent", r#"{"prompt":"beta research"}"#),
        ],
        None,
    );
    rig.server
        .enqueue_raw_for(child(), stalled_stream("NEVER-A", 1, 30_000, true));
    rig.server
        .enqueue_raw_for(child(), stalled_stream("NEVER-B", 1, 30_000, true));

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start two researchers\r");
    // Only press ESC once the idle prompt is provably up, or the key would be
    // consumed as an in-turn cancel arm during teardown.
    pty.wait_for("type a message");
    pty.send(&[0x1b]);
    pty.wait_for("press ESC again to stop all agents");
    pty.send(&[0x1b]);
    // Both canceled packets surface at the idle prompt on their own.
    pty.wait_for("agent-1 canceled");
    pty.wait_for("agent-2 canceled");
    quit_at_idle(&mut pty);
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());

    assert!(
        !pty.seen().contains("NEVER-A") && !pty.seen().contains("NEVER-B"),
        "a canceled child's output leaked:\n{}",
        pty.seen()
    );
    // The canceled batch keeps the prompt idle: exactly the spawn round, no
    // continuation inference (the other recorded requests belong to the
    // killed children themselves).
    let parent_requests = rig
        .api_requests()
        .iter()
        .filter(|request| {
            request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["function"]["name"] == "subagent")
        })
        .count();
    assert_eq!(
        parent_requests, 1,
        "a canceled batch must not trigger parent inference"
    );
    let session_path = std::fs::read_dir(rig.config.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let saved = std::fs::read_to_string(session_path).unwrap();
    for id in ["agent-1", "agent-2"] {
        assert_eq!(
            saved
                .matches(&format!("[background sub-agent result {id}]"))
                .count(),
            1,
            "{id} terminal packet missing or duplicated: {saved}"
        );
    }
    assert!(saved.contains(r#"\"status\":\"canceled\""#), "{saved}");
    rig.server.assert_clean();
}

/// Double-ESC during a running turn now stops the fleet along with the turn,
/// and the interrupt note stops claiming the children "keep running". The
/// canceled child still delivers its one terminal packet.
#[test]
fn double_esc_during_a_turn_stops_the_fleet_too() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());

    // One MIXED batch keeps the turn alive after the spawn (a spawn-only
    // round would end the input before the ESC presses land).
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[
            ("bg-call", "subagent", r#"{"prompt":"doomed research"}"#),
            ("wait-call", "bash", r#"{"cmd":"tail -f /dev/null"}"#),
        ],
        None,
    );
    rig.server
        .enqueue_raw_for(child(), stalled_stream("NEVER-DELIVERED", 1, 30_000, true));

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start then get stuck\r");
    pty.wait_for("[1] agents running (Tab to view)");
    pty.wait_for("· bash");
    pty.send(&[0x1b]);
    pty.wait_for("press ESC again to cancel");
    pty.send(&[0x1b]);
    pty.wait_for("[interrupted]");
    pty.wait_for("agent-1 canceled");
    quit_at_idle(&mut pty);
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());

    assert!(
        !pty.seen().contains("keeps running") && !pty.seen().contains("keep running"),
        "the interrupt note lied about a stopped fleet:\n{}",
        pty.seen()
    );
    assert!(
        !pty.seen().contains("NEVER-DELIVERED"),
        "the canceled child's output leaked:\n{}",
        pty.seen()
    );
    let parent_requests = rig
        .api_requests()
        .iter()
        .filter(|request| {
            request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["function"]["name"] == "subagent")
        })
        .count();
    assert_eq!(
        parent_requests, 1,
        "the canceled turn must not spend further inference"
    );
    let session_path = std::fs::read_dir(rig.config.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let saved = std::fs::read_to_string(session_path).unwrap();
    assert_eq!(
        saved
            .matches("[background sub-agent result agent-1]")
            .count(),
        1,
        "canceled terminal packet missing or duplicated: {saved}"
    );
    rig.server.assert_clean();
}

/// The model manages its own fleet: subagent {"cancel":"agent-N"} stops a
/// detached child through the same path as /agents cancel, the ack names the
/// canceling job, and the child's terminal packet still closes the loop. A
/// cancellation does not spend another parent inference merely to report it.
#[test]
fn model_cancels_its_own_subagent() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());

    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[("bg-call", "subagent", r#"{"prompt":"doomed research"}"#)],
        None,
    );
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[("stop-call", "subagent", r#"{"cancel":"agent-1"}"#)],
        None,
    );
    rig.server
        .enqueue_stream_completion_for(parent(), "STOPPED-END");
    // The child would stall a long time; the cancel must beat it.
    rig.server
        .enqueue_raw_for(child(), stalled_stream("NEVER-DELIVERED", 1, 30_000, true));

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    // The spawn round ends the input on its own; the cancel is what the
    // model does with the user's NEXT instruction.
    pty.send(b"start the research\r");
    pty.wait_for("[1] agents running (Tab to view)");
    pty.wait_for("type a message");
    pty.send(b"actually stop it\r");
    // The done-line renderer styles the summary's first word separately, so
    // the two halves are matched on their own. Detached subagent digests
    // carry no per-call elapsed (a hub control returns in microseconds).
    pty.wait_for("canceling");
    pty.wait_for("agent-1");
    pty.wait_for("STOPPED-END");
    quit_at_idle(&mut pty);
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());

    let requests = rig.api_requests();
    let parent_requests = requests
        .iter()
        .filter(|request| {
            request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["function"]["name"] == "subagent")
        })
        .count();
    assert_eq!(
        parent_requests, 3,
        "cancellation triggered an extra parent model turn"
    );
    // The cancel ack reached the model on the stop call...
    assert!(
        requests.iter().any(|request| {
            request["messages"].as_array().unwrap().iter().any(|m| {
                m["role"] == "tool"
                    && m["tool_call_id"] == "stop-call"
                    && m["content"]
                        .as_str()
                        .is_some_and(|c| c.contains("\"canceling\""))
            })
        }),
        "cancel acknowledgment missing"
    );
    // ...and the canceled child still persisted exactly one terminal packet,
    // without spending a provider request merely to echo that cancellation.
    let session_path = std::fs::read_dir(rig.config.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let saved = std::fs::read_to_string(session_path).unwrap();
    assert_eq!(
        saved
            .matches("[background sub-agent result agent-1]")
            .count(),
        1,
        "canceled terminal packet was missing or duplicated: {saved}"
    );
    assert!(saved.contains(r#"\"status\":\"canceled\""#), "{saved}");
    assert!(
        !pty.seen().contains("NEVER-DELIVERED"),
        "the canceled child's output leaked into the session"
    );
    rig.server.assert_clean();
}

/// Cancel and replacement calls in one model batch are ordered controls. The
/// accepted cancel closes admission immediately, before its terminal packet is
/// drained, so the second call cannot start an autonomous replacement.
#[test]
fn cancel_then_spawn_in_one_batch_blocks_the_replacement() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());

    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[("start", "subagent", r#"{"prompt":"original slow child"}"#)],
        None,
    );
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[
            ("cancel", "subagent", r#"{"cancel":"agent-1"}"#),
            (
                "replace",
                "subagent",
                r#"{"prompt":"unrequested replacement"}"#,
            ),
        ],
        None,
    );
    rig.server
        .enqueue_stream_completion_for(parent(), "REPLACEMENT-BLOCKED-END");
    rig.server
        .enqueue_raw_for(child(), stalled_stream("MUST-NOT-FINISH", 1, 30_000, true));

    let mut pty = spawn_pty_with(&rig, DOCK);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    // The spawn round ends its input; the [cancel, replace] control batch
    // rides the user's next instruction (a control batch is not a spawn-only
    // round, so that turn survives to REPLACEMENT-BLOCKED-END).
    pty.send(b"start the slow child\r");
    pty.wait_for("[1] agents running (Tab to view)");
    pty.wait_for("type a message");
    pty.send(b"cancel it and wrongly replace\r");
    pty.wait_for("canceling");
    pty.wait_for("do not spawn a replacement until the human gives a new instruction");
    pty.wait_for("REPLACEMENT-BLOCKED-END");
    quit_at_idle(&mut pty);
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());

    let requests = rig.api_requests();
    assert!(
        requests
            .iter()
            .all(|request| last_user(request) != "unrequested replacement"),
        "the replacement reached the provider: {requests:?}"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| { tool["function"]["name"] == "subagent" }))
            .count(),
        3
    );
    assert!(!pty.seen().contains("MUST-NOT-FINISH"), "{}", pty.seen());
    rig.server.assert_clean();
}

/// Dock fan-out is detached. The compact row opens into three live, distinct
/// snapshot rows on Tab; shared prompt prefixes must not collapse their tails.
#[test]
fn dock_renders_a_detached_multi_agent_detail_view() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::UserPrompt("fan out".to_string());
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[
            (
                "f1",
                "subagent",
                r#"{"prompt":"Read the article at http://x/ALPHATAIL","tools":"all"}"#,
            ),
            (
                "f2",
                "subagent",
                r#"{"prompt":"Read the article at http://x/BETATAIL","tools":"all"}"#,
            ),
            (
                "f3",
                "subagent",
                r#"{"prompt":"Read the article at http://x/GAMMATAIL","tools":"all"}"#,
            ),
        ],
        None,
    );
    for (tail, result, delay) in [
        ("ALPHATAIL", "ALPHA-RESULT one", 800),
        ("BETATAIL", "BETA-RESULT two", 1800),
        ("GAMMATAIL", "GAMMA-RESULT three", 2800),
    ] {
        rig.server.enqueue_raw_for(
            RequestMatch::UserPrompt(format!("Read the article at http://x/{tail}")),
            stalled_stream(result, 1, delay, true),
        );
    }
    rig.server
        .enqueue_stream_completion_for(parent(), "COLLECTED-ONE");
    rig.server
        .enqueue_stream_completion_for(parent(), "COLLECTED-TWO");
    rig.server
        .enqueue_stream_completion_for(parent(), "COLLECTED-END");

    // Force the cap so the header text is deterministic and all three overlap.
    let mut pty = spawn_pty_with(&rig, &[("NOOB_TASK_CONCURRENCY", "4")]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"fan out\r");
    // The three-spawn round ends the input by itself; the pinned row is the
    // signal that all three admissions landed.
    pty.wait_for("[3] agents running (Tab to view)");
    pty.send(b"\t");
    pty.wait_for("agents (3 active, 0 ready):");
    for tail in ["ALPHATAIL", "BETATAIL", "GAMMATAIL"] {
        pty.wait_for(tail);
    }
    pty.wait_for("COLLECTED-END");
    pty.wait_for("type a message");
    settle();
    pty.send(b"/quit\r");
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let seen = &pty.seen();
    for tail in ["ALPHATAIL", "BETATAIL", "GAMMATAIL"] {
        assert!(
            seen.contains(tail),
            "distinct row for {tail} missing:\n{seen}"
        );
    }
    assert!(
        seen.contains("agents (3 active, 0 ready):"),
        "the detached detail view never opened:\n{seen}"
    );
    rig.server.assert_clean();
}

/// Detached read-only sub-agents acknowledge their original tool call, then
/// leave the dock free to dispatch a human follow-up before the child finishes.
/// Tab opens a persistent detail region that survives the parent turn ending
/// while the ordinary prompt remains editable. The final child output returns
/// once as a synthetic user item and triggers one automatic continuation.
#[test]
fn background_agent_view_stays_pinned_while_the_prompt_remains_usable() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());

    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[(
            "bg-call",
            "subagent",
            r#"{"prompt":"slow standalone research"}"#,
        )],
        None,
    );
    rig.server.enqueue_raw_for(
        child(),
        stalled_stream("CHILD-RESULT-UNIQUE", 1, 2500, true),
    );
    rig.server
        .enqueue_stream_completion_for(parent(), "STEERED-END");
    rig.server
        .enqueue_stream_completion_for(parent(), "AGENT-COLLECTED-END");

    let mut pty = spawn_pty_with(&rig, &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start research\r");
    pty.wait_for("[1] agents running (Tab to view)");
    // The spawn-only round has ended the input; the Tab view opens over the
    // idle prompt and must stay pinned while the human types.
    pty.wait_for("type a message");
    pty.send(b"\t");
    pty.wait_for("agents (1 active, 0 ready):");
    pty.wait_for("slow standalone research");
    pty.send(b"answer me while it runs");
    pty.drain(std::time::Duration::from_millis(300));
    let open_view = pty.screen(18, 90);
    let open_rows = open_view.render();
    let visible = open_rows.join("\n");
    assert!(
        visible.contains("slow standalone research"),
        "agent detail did not remain pinned after the parent turn:\n{}",
        open_view.dump("persistent agents")
    );
    assert!(
        open_rows
            .iter()
            .any(|row| row.contains(MARKER) && row.contains("answer me while it runs")),
        "the editor is not usable under the persistent agents region:\n{}",
        open_view.dump("persistent agents")
    );
    pty.send(b"\r");
    pty.wait_for("STEERED-END");
    pty.wait_for("agent-1 ok");
    pty.wait_for("AGENT-COLLECTED-END");
    quit_at_idle(&mut pty);
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let requests = rig.api_requests();
    let child_request = requests
        .iter()
        .find(|request| last_user(request) == "slow standalone research")
        .expect("child request");
    assert_eq!(child_request["messages"].as_array().unwrap().len(), 2);

    let final_parent = requests
        .iter()
        .rev()
        .find(|request| {
            request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["function"]["name"] == "subagent")
        })
        .expect("final parent request");
    let messages = final_parent["messages"].as_array().unwrap();
    let acks: Vec<&Value> = messages
        .iter()
        .filter(|message| message["role"] == "tool" && message["tool_call_id"] == "bg-call")
        .collect();
    assert_eq!(
        acks.len(),
        1,
        "one immediate result per original call: {messages:?}"
    );
    let ack: Value = serde_json::from_str(acks[0]["content"].as_str().unwrap()).unwrap();
    assert_eq!(ack["job_id"], "agent-1");
    assert_eq!(ack["status"], "running");
    // The acknowledgment carries the lifecycle contract for the orchestrator.
    assert!(
        ack["contract"]
            .as_str()
            .unwrap()
            .contains("listing files cannot fetch it"),
        "{ack}"
    );
    let packets: Vec<&str> = messages
        .iter()
        .filter(|message| message["role"] == "user")
        .filter_map(|message| message["content"].as_str())
        .filter(|content| content.starts_with("[background sub-agent result agent-1]"))
        .collect();
    assert_eq!(
        packets.len(),
        1,
        "completion packet duplicated: {messages:?}"
    );
    assert!(packets[0].contains("CHILD-RESULT-UNIQUE"));
    assert!(
        !acks[0]["content"]
            .as_str()
            .unwrap()
            .contains("CHILD-RESULT-UNIQUE")
    );

    let recorded = rig.server.recorded();
    let steered = recorded
        .iter()
        .find(|record| {
            record.json().is_some_and(|request| {
                request["messages"].as_array().is_some_and(|messages| {
                    messages.iter().any(|message| {
                        message["role"] == "user" && message["content"] == "answer me while it runs"
                    })
                }) && !request["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|message| {
                        message["role"] == "user"
                            && message["content"].as_str().is_some_and(|content| {
                                content.starts_with("[background sub-agent result agent-1]")
                            })
                    })
            })
        })
        .expect("steered parent request");
    let collected = recorded
        .iter()
        .find(|record| {
            record.json().is_some_and(|request| {
                request["messages"].as_array().is_some_and(|messages| {
                    messages.iter().any(|message| {
                        message["role"] == "user"
                            && message["content"].as_str().is_some_and(|content| {
                                content.starts_with("[background sub-agent result agent-1]")
                            })
                    })
                })
            })
        })
        .expect("result continuation request");
    assert!(
        steered.arrived < collected.arrived,
        "the human turn was blocked by the child"
    );
    rig.server.assert_clean();
}

/// Three real child processes remain in flight while the idle parent handles
/// an ordinary human turn. The follow-up reaches the provider before any child
/// report, then every child result is integrated exactly once without plan or
/// cancellation traffic.
#[test]
fn main_turn_runs_while_three_background_children_remain_in_flight() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());

    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[
            ("three-a", "subagent", r#"{"prompt":"THREE-CHILD-A"}"#),
            ("three-b", "subagent", r#"{"prompt":"THREE-CHILD-B"}"#),
            ("three-c", "subagent", r#"{"prompt":"THREE-CHILD-C"}"#),
        ],
        None,
    );
    rig.server
        .enqueue_stream_completion_for(parent(), "MAIN-WHILE-THREE");
    for (prompt, result, delay) in [
        ("THREE-CHILD-A", "THREE-RESULT-A", 3500),
        ("THREE-CHILD-B", "THREE-RESULT-B", 5500),
        ("THREE-CHILD-C", "THREE-RESULT-C", 7500),
    ] {
        rig.server.enqueue_raw_for(
            RequestMatch::UserPrompt(prompt.to_string()),
            stalled_stream(result, 1, delay, true),
        );
    }
    rig.server
        .enqueue_stream_completion_for(parent(), "COLLECT-THREE-A");
    rig.server
        .enqueue_stream_completion_for(parent(), "COLLECT-THREE-B");
    rig.server
        .enqueue_stream_completion_for(parent(), "COLLECT-THREE-END");

    let mut pty = spawn_pty_with(&rig, &[("NOOB_TASK_CONCURRENCY", "4")]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start three background children\r");
    // The three-spawn round ends the input on its own.
    pty.wait_for("[3] agents running (Tab to view)");
    pty.wait_for("type a message");
    pty.send(b"human main work\r");
    pty.wait_for("MAIN-WHILE-THREE");
    pty.wait_for("agent-1 ok");
    pty.wait_for("agent-2 ok");
    pty.wait_for("agent-3 ok");
    pty.wait_for("COLLECT-THREE-END");
    quit_at_idle(&mut pty);
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    assert!(!pty.seen().contains("[steering]"), "{}", pty.seen());
    assert!(!pty.seen().contains("[interrupted]"), "{}", pty.seen());
    assert!(!pty.seen().contains(" canceled"), "{}", pty.seen());
    assert!(
        pty.seen().find("MAIN-WHILE-THREE") < pty.seen().find("agent-1 ok"),
        "the human response did not finish before the children: {}",
        pty.seen()
    );

    let recorded = rig.server.recorded();
    let human = recorded
        .iter()
        .find(|record| {
            record.json().is_some_and(|request| {
                request["messages"].as_array().is_some_and(|messages| {
                    messages.iter().any(|message| {
                        message["role"] == "user" && message["content"] == "human main work"
                    })
                })
            })
        })
        .expect("human parent request");
    let first_report = recorded
        .iter()
        .find(|record| {
            record.json().is_some_and(|request| {
                request["messages"].as_array().is_some_and(|messages| {
                    messages.iter().any(|message| {
                        message["role"] == "user"
                            && message["content"].as_str().is_some_and(|content| {
                                content.starts_with("[background sub-agent result agent-")
                            })
                    })
                })
            })
        })
        .expect("first report continuation");
    assert!(
        human.arrived < first_report.arrived,
        "the human request waited behind a child"
    );

    let requests = rig.api_requests();
    for prompt in ["THREE-CHILD-A", "THREE-CHILD-B", "THREE-CHILD-C"] {
        let child = requests
            .iter()
            .find(|request| last_user(request) == prompt)
            .unwrap_or_else(|| panic!("missing request for {prompt}"));
        assert!(
            child["tools"]
                .as_array()
                .unwrap()
                .iter()
                .all(|tool| tool["function"]["name"] != "subagent"),
            "detached child {prompt} retained nested delegation"
        );
    }
    let final_parent = requests
        .iter()
        .rev()
        .find(|request| {
            request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["function"]["name"] == "subagent")
        })
        .expect("final parent request");
    let messages = final_parent["messages"].as_array().unwrap();
    for (id, result) in [
        ("agent-1", "THREE-RESULT-A"),
        ("agent-2", "THREE-RESULT-B"),
        ("agent-3", "THREE-RESULT-C"),
    ] {
        let packets = messages
            .iter()
            .filter(|message| message["role"] == "user")
            .filter_map(|message| message["content"].as_str())
            .filter(|content| content.starts_with(&format!("[background sub-agent result {id}]")))
            .collect::<Vec<_>>();
        assert_eq!(packets.len(), 1, "duplicate or missing {id}: {messages:?}");
        assert!(
            packets[0].contains(result),
            "wrong {id} report: {}",
            packets[0]
        );
    }
    assert!(messages.iter().all(|message| {
        message["tool_calls"]
            .as_array()
            .is_none_or(|calls| calls.iter().all(|call| call["function"]["name"] != "plan"))
    }));
    rig.server.assert_clean();
}

/// A completed child must not steal an idle prompt that the human is already
/// composing. The ordinary submitted turn integrates the ready packet first,
/// then the complete human message, without relabeling Enter as steering.
#[test]
fn typed_idle_followup_wins_the_race_with_a_ready_child() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());

    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[(
            "race-agent",
            "subagent",
            r#"{"prompt":"finish during typing"}"#,
        )],
        None,
    );
    rig.server
        .enqueue_raw_for(child(), stalled_stream("RACE-CHILD-DONE", 1, 1200, true));
    rig.server
        .enqueue_stream_completion_for(parent(), "FOLLOWUP-WITH-RESULT-END");

    let mut pty = spawn_pty_with(&rig, &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start race child\r");
    // The spawn-only round ends the input on its own.
    pty.wait_for("type a message");
    pty.send(b"human follow");
    pty.wait_for("human follow");

    // The child settles while the draft is nonempty. Its row may update, but
    // no automatic provider continuation may start before the human presses
    // Enter.
    pty.wait_for("[1] agents ready (Tab to view)");
    let before_enter = rig.api_requests();
    let parent_before_enter = before_enter
        .iter()
        .filter(|request| {
            request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["function"]["name"] == "subagent")
        })
        .count();
    assert_eq!(
        parent_before_enter, 1,
        "a background continuation stole the typed prompt: {before_enter:?}"
    );

    pty.send(b"up\r");
    pty.wait_for("agent-1 ok");
    pty.wait_for("FOLLOWUP-WITH-RESULT-END");
    quit_at_idle(&mut pty);
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    assert!(!pty.seen().contains("[steering]"), "{}", pty.seen());
    assert!(!pty.seen().contains("[interrupted]"), "{}", pty.seen());

    let requests = rig.api_requests();
    let combined = requests
        .iter()
        .find(|request| last_user(request) == "human followup")
        .expect("combined human/result request");
    let messages = combined["messages"].as_array().unwrap();
    assert!(messages.iter().any(|message| {
        message["role"] == "user"
            && message["content"].as_str().is_some_and(|content| {
                content.starts_with("[background sub-agent result agent-1]")
                    && content.contains("RACE-CHILD-DONE")
            })
    }));
    rig.server.assert_clean();
}

/// A terminal child error is visible and durable, but it must not start an
/// unrequested parent inference that can spawn replacement children. The next
/// provider request belongs to the next human turn.
#[test]
fn failed_background_child_leaves_the_idle_prompt_free_without_auto_retry() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());

    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[("fail-agent", "subagent", r#"{"prompt":"hit the cap"}"#)],
        None,
    );
    // With one child round, this tool call executes and then the child reaches
    // its configured inference cap before it can produce a final response.
    rig.server
        .enqueue_stream_toolcalls_for(child(), &[("child-ls", "ls", r#"{}"#)], None);

    let mut pty = spawn_pty_with(&rig, &[("NOOB_TASK_MAX_TURNS", "1")]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start failing child\r");
    pty.wait_for("agent-1 error");
    pty.wait_for("type a message");
    settle();

    let parent_requests = rig
        .api_requests()
        .iter()
        .filter(|request| {
            request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["function"]["name"] == "subagent")
        })
        .count();
    assert_eq!(
        parent_requests, 1,
        "the failed child triggered an unrequested parent retry"
    );

    quit_at_idle(&mut pty);
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();
}

/// Coalescing must not weaken the failure rule. If one ready child succeeded
/// while another failed, both reports become visible and durable, but the
/// success cannot reopen parent inference and give the model an opportunity
/// to hammer replacement spawns rejected by the same-turn failure gate.
#[test]
fn mixed_success_and_failure_leave_the_idle_prompt_without_auto_retry() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());

    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[
            (
                "mixed-ok",
                "subagent",
                r#"{"prompt":"mixed successful child"}"#,
            ),
            (
                "mixed-fail",
                "subagent",
                r#"{"prompt":"mixed failing child"}"#,
            ),
        ],
        None,
    );
    rig.server.enqueue_raw_for(
        RequestMatch::UserPrompt("mixed successful child".to_string()),
        stalled_stream("MIXED-CHILD-OK", 1, 1200, true),
    );

    let datas =
        noob_testkit::chat_stream_toolcalls_datas(&[("mixed-child-ls", "ls", r#"{}"#)], None);
    let delayed_tool_call = noob_testkit::sse_headers();
    let mut tool_call_tail = sse_frames(&datas);
    tool_call_tail.extend_from_slice(b"0\r\n\r\n");
    rig.server.enqueue_raw_for(
        RequestMatch::UserPrompt("mixed failing child".to_string()),
        vec![
            noob_testkit::RawStep::Bytes(delayed_tool_call),
            noob_testkit::RawStep::SleepMs(1200),
            noob_testkit::RawStep::Bytes(tool_call_tail),
        ],
    );

    let mut pty = spawn_pty_with(&rig, &[("NOOB_TASK_MAX_TURNS", "1")]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start mixed children\r");
    // The two-spawn round ends the input on its own.
    pty.wait_for("[2] agents running (Tab to view)");
    pty.wait_for("type a message");
    pty.send(b"hold completion");
    pty.wait_for("hold completion");
    pty.wait_for("[2] agents ready (Tab to view)");

    // Emptying the draft lets the owner drain both terminal results together.
    // The mixed batch must return directly to idle without another request.
    pty.send(&[0x15]);
    pty.wait_for("agent-1 ok");
    pty.wait_for("agent-2 error");
    pty.wait_for("type a message");
    settle();

    let parent_requests = rig
        .api_requests()
        .iter()
        .filter(|request| {
            request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["function"]["name"] == "subagent")
        })
        .count();
    assert_eq!(
        parent_requests, 1,
        "a mixed terminal batch triggered parent inference"
    );

    quit_at_idle(&mut pty);
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();
}

/// The 11-13 screenshot: at the idle prompt with the `[1] agents running`
/// counter pinned, the counter must sit INSIDE the frame (below the top rule,
/// above the input row), exactly like the active frame lays it out, and
/// typing a draft must keep the whole frame exact. The live complaint was the
/// idle counter floating ABOVE the box, reading as loose transcript text.
#[test]
fn idle_box_stays_exact_while_typing_with_a_pinned_agents_row() {
    const ROWS: u16 = 16;
    const COLS: u16 = 90;

    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());

    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[("bg-a", "subagent", r#"{"prompt":"slow typing child"}"#)],
        None,
    );
    rig.server
        .enqueue_raw_for(child(), stalled_stream("TYPING-CHILD-DONE", 1, 8000, true));
    rig.server
        .enqueue_stream_completion_for(parent(), "TYPING-COLLECTED-END");

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start one child\r");
    // The spawn-only round ends the turn; the idle box pins the counter.
    pty.wait_for("[1] agents running (Tab to view)");
    pty.drain(std::time::Duration::from_millis(500));

    // Type a draft at the idle prompt, spread over the idle tick cadence so
    // periodic repaints interleave with the keystrokes, then let it sit.
    for chunk in [&b"okey did"[..], &b" it"[..], &b" finished?"[..]] {
        pty.send(chunk);
        pty.drain(std::time::Duration::from_millis(250));
    }
    pty.drain(std::time::Duration::from_millis(600));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();

    let draft_row = rows
        .iter()
        .rposition(|r| r.contains("okey did it finished?"))
        .unwrap_or_else(|| panic!("typed draft missing:\n{}", screen.dump("typing idle")));
    assert!(
        rows[draft_row].contains(MARKER),
        "the draft renders on the input row, after the marker: {:?}\n{}",
        rows[draft_row],
        screen.dump("typing idle")
    );
    let rules = rule_row_indices(&rows);
    assert_eq!(
        rules.len(),
        2,
        "exactly the frame's two rules while typing:\n{}",
        screen.dump("typing idle")
    );
    assert_eq!(
        (rules[0], rules[1]),
        (draft_row - 2, draft_row + 1),
        "frame order: top rule, agents row, draft, bottom rule:\n{}",
        screen.dump("typing idle")
    );
    let counter = rows
        .iter()
        .position(|r| r.contains("[1] agents running (Tab to view)"))
        .unwrap_or_else(|| panic!("agents counter missing:\n{}", screen.dump("typing idle")));
    assert_eq!(
        counter,
        draft_row - 1,
        "the agents row sits INSIDE the frame, above the input (counter {counter}, draft {draft_row}):\n{}",
        screen.dump("typing idle")
    );

    // Ctrl-C clears the draft so the settled child's report can be collected,
    // then the session ends cleanly.
    pty.send(&[0x03]);
    pty.wait_for("TYPING-COLLECTED-END");
    quit_at_idle(&mut pty);
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();
}

/// The running-agents counter must survive the idle prompt: while a detached
/// child still runs, the collapsed `[N] agents running` row stays pinned above
/// the idle box, Tab expands the panel, and Tab again falls back to the live
/// counter, never to nothing (the live-work-goes-invisible regression). The
/// counter must be LIVE: every static end-of-turn record froze at "[2] agents
/// running", so a row reading "[1]" can only come from the live snapshot.
#[test]
fn idle_prompt_keeps_the_running_agents_counter_after_closing_the_tab_view() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());

    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[
            ("bg-a", "subagent", r#"{"prompt":"fast idle child"}"#),
            ("bg-b", "subagent", r#"{"prompt":"slow idle child"}"#),
        ],
        None,
    );
    rig.server
        .enqueue_raw_for(child(), stalled_stream("FAST-CHILD-DONE", 1, 1200, true));
    rig.server
        .enqueue_raw_for(child(), stalled_stream("SLOW-CHILD-DONE", 1, 8000, true));
    rig.server
        .enqueue_stream_completion_for(parent(), "FIRST-COLLECTED-END");
    rig.server
        .enqueue_stream_completion_for(parent(), "ALL-COLLECTED-END");

    let mut pty = spawn_pty_with(&rig, &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start two idle children\r");
    // The two-spawn round ends the input on its own.
    pty.wait_for("[2] agents running (Tab to view)");

    // The fast child settles and is collected; the slow child keeps running,
    // so the next idle prompt has exactly one live background agent. Wait for
    // the pinned row itself, then DRAIN (not just sleep): the counter is only
    // pinned now (turn end records nothing), and a sleep without reading can
    // snapshot the byte stream between an in-place repaint's erase and its
    // redraw.
    pty.wait_for("FIRST-COLLECTED-END");
    pty.wait_for("[1] agents running (Tab to view)");
    pty.drain(std::time::Duration::from_millis(400));

    // Idle, view closed: the pinned counter reads the LIVE count of 1 (all
    // frozen records above say "[2]"; pre-fix there was no idle row at all).
    let idle = pty.screen(16, 90);
    assert!(
        idle.render()
            .join("\n")
            .contains("[1] agents running (Tab to view)"),
        "no live running counter at the idle prompt:\n{}",
        idle.dump("idle counter")
    );

    // Tab expands to the detail panel.
    pty.send(b"\t");
    pty.wait_for("agents (1 active, 0 ready):");

    // Tab again closes it: the live counter must come back, not vanish.
    pty.send(b"\t");
    pty.drain(std::time::Duration::from_millis(400));
    let closed = pty.screen(16, 90);
    let visible = closed.render().join("\n");
    assert!(
        visible.contains("[1] agents running (Tab to view)"),
        "no live counter after closing the agents view:\n{}",
        closed.dump("counter after close")
    );
    assert!(
        !visible.contains("agents (1 active"),
        "the detail panel did not close:\n{}",
        closed.dump("counter after close")
    );

    // The slow child settles, its result is collected, and the exit is clean.
    pty.wait_for("ALL-COLLECTED-END");
    quit_at_idle(&mut pty);
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();
}

/// A child that finishes while the parent turn is still running is delivered
/// at the next round INSIDE that turn, not held for the turn's end: the round
/// after the child settles must already carry the result packet, and no
/// separate background continuation happens afterwards. This is the
/// deterministic close of the sub-agent loop (a model that "waits" for a
/// report receives it at its very next step).
#[test]
fn ready_child_result_is_delivered_mid_turn_at_the_next_round() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());

    // Round 1: spawn the child. The child answers immediately.
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[("bg-mid", "subagent", r#"{"prompt":"fast goal"}"#)],
        None,
    );
    rig.server
        .enqueue_stream_completion_for(child(), "FAST-GOAL-DONE");

    // Round 2: the parent keeps its turn alive (a stalled response emitting
    // one more tool call), long enough for the child to settle meanwhile.
    let datas = noob_testkit::chat_stream_toolcalls_datas(
        &[("p2", "bash", r#"{"cmd":"echo still-here"}"#)],
        None,
    );
    let mut tail = sse_frames(&datas);
    tail.extend_from_slice(b"0\r\n\r\n");
    rig.server.enqueue_raw_for(
        parent(),
        vec![
            noob_testkit::RawStep::Bytes(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n"
                    .to_vec(),
            ),
            noob_testkit::RawStep::SleepMs(3000),
            noob_testkit::RawStep::Bytes(tail),
        ],
    );

    // Round 3: sees the injected packet and finishes the turn.
    rig.server
        .enqueue_stream_completion_for(parent(), "SAW-THE-REPORT-END");

    let mut pty = spawn_pty_with(&rig, &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"spawn and keep working\r");
    pty.wait_for("agent-1 ok");
    pty.wait_for("SAW-THE-REPORT-END");
    quit_at_idle(&mut pty);
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());

    let requests = rig.api_requests();
    let parent_requests: Vec<&Value> = requests
        .iter()
        .filter(|request| {
            request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["function"]["name"] == "subagent")
        })
        .collect();
    // Exactly three parent rounds, all within ONE turn: no post-turn
    // background continuation was needed.
    assert_eq!(parent_requests.len(), 3, "{requests:?}");
    let final_messages = parent_requests[2]["messages"].as_array().unwrap();
    assert!(
        final_messages.iter().any(|message| {
            message["role"] == "user"
                && message["content"].as_str().is_some_and(|content| {
                    content.starts_with("[background sub-agent result agent-1]")
                        && content.contains("FAST-GOAL-DONE")
                })
        }),
        "the packet must ride the SAME turn's next round: {final_messages:?}"
    );
    rig.server.assert_clean();
}

/// A full-tool dock child detaches just like a read-only child. It receives the
/// complete coding/MCP-capable schema set, may mutate the workspace under the
/// cross-process lease, and reports exactly once after the parent has already
/// returned to the prompt.
#[test]
fn detached_all_tools_child_writes_a_file_and_reports_once() {
    let rig = rig();
    rig.server.allow_interleaving();
    std::fs::write(
        rig.config.path().join("mcp.json"),
        r#"{"servers":{"example":{"url":"http://127.0.0.1:9"}}}"#,
    )
    .unwrap();

    let parent = || RequestMatch::UserPrompt("delegate single file".to_string());
    let child = || RequestMatch::UserPrompt("write the delegated file".to_string());
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[(
            "all-call",
            "subagent",
            r#"{"prompt":"write the delegated file","tools":"all"}"#,
        )],
        None,
    );

    // Hold the child's first model response so the parent must finish before
    // the write can happen. Then the child calls the real write entry point.
    let write_args = r#"{"path":"delegated.txt","content":"written by detached child\n"}"#;
    let datas =
        noob_testkit::chat_stream_toolcalls_datas(&[("child-write", "write", write_args)], None);
    let mut tail = sse_frames(&datas);
    tail.extend_from_slice(b"0\r\n\r\n");
    rig.server.enqueue_raw_for(
        child(),
        vec![
            noob_testkit::RawStep::Bytes(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n"
                    .to_vec(),
            ),
            noob_testkit::RawStep::SleepMs(1200),
            noob_testkit::RawStep::Bytes(tail),
        ],
    );
    rig.server
        .enqueue_stream_completion_for(child(), "CHILD-WRITE-DONE");
    rig.server
        .enqueue_stream_completion_for(parent(), "ALL-TOOLS-COLLECTED-END");

    let output = rig.work.path().join("delegated.txt");
    let mut pty = spawn_pty_with(&rig, &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"delegate single file\r");
    pty.wait_for("[1] agents running (Tab to view)");
    assert!(
        !output.exists(),
        "the delayed child mutated the workspace before the parent turn ended"
    );
    pty.wait_for("agent-1 ok");
    pty.wait_for("ALL-TOOLS-COLLECTED-END");
    pty.wait_for("type a message");
    settle();
    pty.send(b"/quit\r");
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    assert_eq!(
        std::fs::read_to_string(&output).unwrap(),
        "written by detached child\n"
    );

    let requests = rig.api_requests();
    let child_request = requests
        .iter()
        .find(|request| last_user(request) == "write the delegated file")
        .expect("child request");
    let schemas = child_request["tools"]
        .as_array()
        .expect("child tool schemas");
    let has_schema = |name: &str| {
        schemas
            .iter()
            .any(|schema| schema["function"]["name"] == name || schema["name"] == name)
    };
    for name in ["write", "edit", "bash", "mcp_connect", "mcp_call"] {
        assert!(
            has_schema(name),
            "full-tool child lacks {name}: {schemas:?}"
        );
    }
    // The child's system prompt carries the lifecycle contract: one goal,
    // one final report, the instance closes.
    assert!(
        child_request["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("# Sub-agent contract"),
        "the child system prompt must carry the sub-agent contract"
    );

    let final_parent = requests
        .iter()
        .rev()
        .find(|request| {
            request["messages"].as_array().is_some_and(|messages| {
                messages.iter().any(|message| {
                    message["role"] == "user"
                        && message["content"].as_str().is_some_and(|content| {
                            content.starts_with("[background sub-agent result agent-1]")
                        })
                })
            })
        })
        .expect("final parent request");
    let messages = final_parent["messages"].as_array().unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|message| {
                message["role"] == "tool" && message["tool_call_id"] == "all-call"
            })
            .count(),
        1,
        "the original tool call must receive exactly one running ack: {messages:?}"
    );
    let packets: Vec<&str> = messages
        .iter()
        .filter(|message| message["role"] == "user")
        .filter_map(|message| message["content"].as_str())
        .filter(|content| content.starts_with("[background sub-agent result agent-1]"))
        .collect();
    assert_eq!(packets.len(), 1, "result packet duplicated: {messages:?}");
    assert!(packets[0].contains("CHILD-WRITE-DONE"));
    rig.server.assert_clean();
}

#[test]
fn responses_background_result_preserves_one_call_output_and_one_report() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::UserPrompt("start responses helper".to_string());
    let child = || RequestMatch::UserPrompt("responses child task".to_string());
    rig.server.enqueue_raw_for(
        parent(),
        responses_toolcall_stream(
            "responses-bg-call",
            "subagent",
            r#"{"prompt":"responses child task"}"#,
        ),
    );
    rig.server.enqueue_raw_for(
        child(),
        responses_completion_stream("RESPONSES-CHILD-RESULT", 600),
    );
    rig.server.enqueue_raw_for(
        parent(),
        responses_completion_stream("RESPONSES-COLLECTED", 0),
    );

    let mut pty = spawn_pty_with(&rig, &[("NOOB_API_STYLE", "responses")]);
    pty.wait_for(RAW_READY);
    pty.send(b"start responses helper\r");
    // The spawn-only round ends the input under the Responses adapter too.
    pty.wait_for("[1] agents running (Tab to view)");
    pty.wait_for("agent-1 ok");
    pty.wait_for("RESPONSES-COLLECTED");
    quit_at_idle(&mut pty);
    assert!(pty.finish().success());

    let requests = rig.responses_requests();
    let final_parent = requests
        .iter()
        .rev()
        .find(|request| {
            request["input"].as_array().is_some_and(|input| {
                input.iter().any(|item| {
                    item["type"] == "message"
                        && item["role"] == "user"
                        && item["content"].as_str().is_some_and(|content| {
                            content.starts_with("[background sub-agent result agent-1]")
                        })
                })
            })
        })
        .expect("automatic result continuation");
    let input = final_parent["input"].as_array().unwrap();
    let outputs: Vec<&Value> = input
        .iter()
        .filter(|item| {
            item["type"] == "function_call_output" && item["call_id"] == "responses-bg-call"
        })
        .collect();
    assert_eq!(outputs.len(), 1, "{input:?}");
    let ack = serde_json::from_str::<Value>(outputs[0]["output"].as_str().unwrap()).unwrap();
    assert_eq!(ack["job_id"], "agent-1");
    assert_eq!(ack["status"], "running");
    let reports = input
        .iter()
        .filter(|item| {
            item["type"] == "message"
                && item["role"] == "user"
                && item["content"].as_str().is_some_and(|content| {
                    content.starts_with("[background sub-agent result agent-1]")
                })
        })
        .count();
    assert_eq!(reports, 1, "{input:?}");
    rig.server.assert_clean();
}

#[test]
fn resume_repairs_a_persisted_background_ack_after_hard_exit_once() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::UserPrompt("launch orphan".to_string());
    let child = || RequestMatch::UserPrompt("slow orphan work".to_string());
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[("bg-orphan", "subagent", r#"{"prompt":"slow orphan work"}"#)],
        None,
    );
    rig.server
        .enqueue_raw_for(child(), stalled_stream("NEVER-COLLECTED", 1, 10_000, true));

    let mut first = spawn_pty_with(&rig, &[]);
    first.wait_for(RAW_READY);
    first.send(b"launch orphan\r");
    first.wait_for("[1] agents running (Tab to view)");
    let pid = first.child_id() as libc::pid_t;
    unsafe { libc::kill(pid, libc::SIGKILL) };
    let status = first.finish();
    assert!(!status.success(), "hard exit unexpectedly succeeded");

    let session_path = std::fs::read_dir(rig.config.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut resumed = spawn_pty_sized(&rig, &[], None, &["--resume", "latest"]);
    resumed.wait_for("recovered 1 unfinished background sub-agent(s) as canceled");
    resumed.wait_for(RAW_READY);
    resumed.send(&[0x04]);
    assert!(resumed.finish().success());

    let saved = std::fs::read_to_string(session_path).unwrap();
    assert_eq!(
        saved
            .matches("[background sub-agent result agent-1]")
            .count(),
        1,
        "orphan repair must be durable and exact once: {saved}"
    );
    rig.server.assert_clean();
}

#[test]
fn agents_cancel_kills_a_detached_child_and_keeps_the_prompt_usable() {
    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[("cancel-call", "subagent", r#"{"prompt":"wait forever"}"#)],
        None,
    );
    rig.server.enqueue_raw_for(
        child(),
        stalled_stream("NEVER-SHOULD-FINISH", 1, 20_000, true),
    );

    let started = std::time::Instant::now();
    let mut pty = spawn_pty_with(&rig, &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start doomed helper\r");
    pty.wait_for("[1] agents running (Tab to view)");
    // The spawn-only round has ended the input, so the command lands at the
    // idle prompt as an ordinary slash command, not a steering interrupt.
    pty.wait_for("type a message");
    pty.send(b"/agents cancel agent-1\r");
    pty.wait_for("canceling agent-1");
    pty.wait_for("agent-1 canceled");
    pty.wait_for("type a message");
    settle();
    pty.send(b"/quit\r");
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    rig.server.assert_clean();
}

/// The collapsed `[N] agents running` counter lives in exactly one place: the
/// pinned row above the input. A finished spawn turn must not also record a
/// static copy into the transcript (the historical duplicate stacked after
/// every turn).
#[test]
fn dock_shows_the_agents_row_exactly_once_at_idle() {
    const ROWS: u16 = 14;
    const COLS: u16 = 72;

    let rig = rig();
    rig.server.allow_interleaving();
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());

    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[(
            "bg-call",
            "subagent",
            r#"{"prompt":"slow standalone research"}"#,
        )],
        None,
    );
    rig.server.enqueue_raw_for(
        child(),
        stalled_stream("CHILD-RESULT-UNIQUE", 1, 3000, true),
    );
    rig.server
        .enqueue_stream_completion_for(parent(), "AGENT-COLLECTED-END");

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"start research\r");
    pty.wait_for("[1] agents running (Tab to view)");
    settle();
    pty.drain(std::time::Duration::from_millis(500));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();

    pty.wait_for("AGENT-COLLECTED-END");
    quit_at_idle(&mut pty);
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();

    let hits = rows
        .iter()
        .filter(|row| row.contains("agents running (Tab to view)"))
        .count();
    assert_eq!(
        hits,
        1,
        "the agents counter appears exactly once (the pinned row):\n{}",
        screen.dump("idle with agent")
    );
}
