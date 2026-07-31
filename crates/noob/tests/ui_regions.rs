//! The dock's pinned regions on the screen model: the plan block updates in
//! place, retires into the transcript when finished, survives interrupts and
//! turn boundaries as a single copy, and every region is capped so the live
//! frame never outgrows the terminal.

mod ui;

use noob_testkit::RequestMatch;

use ui::*;

/// An Esc Esc cancel does not throw the plan away: it stays pinned above the
/// idle input in its actual state (the in-progress step still marked), so the
/// human can resume where the canceled turn left off. Nothing claims the plan
/// itself was "canceled"; only the turn was. The in-progress glyph keeps
/// SPINNING at the idle prompt (the live stage-3 freeze: a frozen [~]
/// between turns read as the work having stalled).
#[test]
fn dock_keeps_the_pinned_plan_after_an_interrupted_turn() {
    const ROWS: u16 = 16;
    const COLS: u16 = 64;

    let rig = rig();
    let plan = r#"{"todos":[{"content":"finished","status":"completed"},{"content":"still working","status":"in_progress"},{"content":"later","status":"pending"}]}"#;
    rig.server
        .enqueue_stream_toolcalls(&[("p1", "plan", plan)], None);
    rig.server
        .enqueue_raw(stalled_stream("WAITING END-NEVER", 2, 8000, false));

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"run the plan\r");
    pty.wait_for("plan (1/3 done):");
    pty.send(&[0x1b]);
    pty.wait_for("press ESC again to cancel");
    pty.send(&[0x1b]);
    pty.wait_for("[interrupted]");
    settle();
    pty.drain(std::time::Duration::from_millis(400));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();

    // At the idle prompt (no hub, no turn) the pinned step must keep ticking:
    // over ~700ms of pure idleness the spinner passes through several frames.
    let mark = pty.raw().len();
    pty.drain(std::time::Duration::from_millis(700));
    let idle_bytes = &pty.raw()[mark..];
    let distinct_frames = ["[|]", "[/]", "[-]", "[\\]"]
        .iter()
        .filter(|frame| {
            let needle = format!("{frame} still working");
            idle_bytes
                .windows(needle.len())
                .any(|w| w == needle.as_bytes())
        })
        .count();

    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let marker = rows
        .iter()
        .rposition(|r| r.contains(MARKER))
        .unwrap_or_else(|| panic!("idle input box missing:\n{}", screen.dump("idle")));
    let step = rows
        .iter()
        .position(|r| {
            SPINNER_FRAMES
                .iter()
                .any(|frame| r.contains(&format!("{frame} still working")))
        })
        .unwrap_or_else(|| {
            panic!(
                "the plan must stay pinned after a cancel:\n{}",
                screen.dump("idle")
            )
        });
    assert!(
        step < marker,
        "the pinned plan sits above the idle input:\n{}",
        screen.dump("idle")
    );
    assert!(
        distinct_frames >= 2,
        "the pinned in-progress step must keep spinning at the idle prompt \
         ({distinct_frames} distinct frames seen):\n{}",
        pty.seen()
    );
    assert!(!pty.seen().contains("plan canceled"), "{}", pty.seen());
    assert!(!pty.seen().contains("END-NEVER"));
    rig.server.assert_clean();
}

/// The plan is a single pinned region that updates in place, never a fresh block
/// stacked on every `plan` call (the reported console redundancy). Two `plan`
/// calls advance the same plan; mid-turn the live screen shows the LATEST state
/// exactly once, the superseded state is gone (overwritten in place, not scrolled
/// into history), and the plan sits inside the dock between the "Working" status
/// and the input row. Asserted on the screen, not the raw byte log: the old
/// state's bytes were emitted and then erased, so only a screen model can prove
/// it is no longer visible.
#[test]
fn dock_pins_the_plan_as_one_in_place_region() {
    const ROWS: u16 = 14;
    const COLS: u16 = 64;

    let rig = rig();
    let a = r#"{"todos":[{"content":"alpha","status":"pending"},{"content":"beta","status":"pending"}]}"#;
    let b = r#"{"todos":[{"content":"alpha","status":"completed"},{"content":"beta","status":"pending"}]}"#;
    rig.server
        .enqueue_stream_toolcalls(&[("p1", "plan", a)], None);
    rig.server
        .enqueue_stream_toolcalls(&[("p2", "plan", b)], None);
    // A stalled final turn so the screen can be snapped while the frame is live
    // (turn end tears the frame, regions and all, down).
    rig.server
        .enqueue_raw(stalled_stream("all planned ZZEND", 1, 3000, true));

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"plan it\r");
    pty.wait_for("Working");
    pty.wait_for("plan (1/2 done):"); // the second todo call pinned the new state

    pty.drain(std::time::Duration::from_millis(500));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();
    println!("\n{}", screen.dump("PLAN PINNED (mid-turn)"));

    // Release the stall and finish so the child exits cleanly.
    pty.wait_for("ZZEND");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();

    // Exactly one plan block on the live screen: the pinned region, latest state.
    // The scrolled tool summary is "plan: N/2 done" (no paren), so "plan (" keys
    // on the block header alone.
    let headers = rows.iter().filter(|r| r.contains("plan (")).count();
    assert_eq!(
        headers,
        1,
        "the plan must be one pinned block, not stacked:\n{}",
        screen.dump("plan")
    );
    let joined = rows.join("\n");
    assert!(
        joined.contains("[x] alpha"),
        "the advanced item is not shown:\n{}",
        screen.dump("plan")
    );
    assert!(
        joined.contains("[ ] beta"),
        "the pending item is not shown:\n{}",
        screen.dump("plan")
    );
    // The superseded state was overwritten in place, not left in the transcript.
    assert!(
        !joined.contains("[ ] alpha"),
        "the old plan state was stacked, not replaced in place:\n{}",
        screen.dump("plan")
    );
    assert!(
        !joined.contains("plan (0/2 done):"),
        "the old plan header was stacked, not replaced in place:\n{}",
        screen.dump("plan")
    );

    // The region sits inside the dock: below "Working", above the input row.
    let working = rows
        .iter()
        .rposition(|r| r.contains("Working"))
        .expect("Working status row");
    let header = rows
        .iter()
        .position(|r| r.contains("plan (1/2 done):"))
        .expect("plan header row");
    let input = rows
        .iter()
        .rposition(|r| r.contains(MARKER))
        .expect("input row");
    assert!(
        working < header && header < input,
        "plan not pinned between status and input (working {working}, header {header}, input {input}):\n{}",
        screen.dump("plan")
    );
}

/// A plan whose every step completes is RETIRED at turn end: one timed
/// "plan completed" summary goes into the transcript (exactly once) and the
/// pinned copy is dropped, so the finished plan scrolls with history instead
/// of sticking to the input forever. A later turn proves it: the summary
/// stays above that turn's output rather than re-pinning below it.
#[test]
fn dock_retires_the_finished_plan_into_the_transcript() {
    const ROWS: u16 = 18;
    const COLS: u16 = 64;

    let rig = rig();
    let a = r#"{"todos":[{"content":"alpha","status":"pending"},{"content":"beta","status":"pending"}]}"#;
    let b = r#"{"todos":[{"content":"alpha","status":"completed"},{"content":"beta","status":"completed"}]}"#;
    rig.server
        .enqueue_stream_toolcalls(&[("p1", "plan", a)], None);
    rig.server
        .enqueue_stream_toolcalls(&[("p2", "plan", b)], None);
    rig.server.enqueue_stream_completion("PLAN-COMPLETE-ZZ");
    rig.server.enqueue_stream_completion("SECOND-TURN-ZZ");

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"do the plan\r");
    pty.wait_for("PLAN-COMPLETE-ZZ"); // the turn's final text landed
    settle();
    pty.send(b"next task\r");
    pty.wait_for("SECOND-TURN-ZZ");
    settle();
    pty.drain(std::time::Duration::from_millis(400));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();
    println!("\n{}", screen.dump("FINISHED PLAN RETIRED INTO TRANSCRIPT"));

    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();

    assert!(
        dock_rows(&rows).is_none(),
        "the live turn frame must be gone at idle"
    );
    // Exactly one summary, recorded when the plan finished, not per turn.
    let summaries: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| row.contains("plan completed · 2/2 ·").then_some(index))
        .collect();
    assert_eq!(
        summaries.len(),
        1,
        "the finished plan is recorded exactly once:\n{}",
        screen.dump("end")
    );
    // It scrolled with history: the whole second turn sits BELOW it. A plan
    // still pinned would instead hug the input, below SECOND-TURN-ZZ.
    let second = rows
        .iter()
        .position(|r| r.contains("SECOND-TURN-ZZ"))
        .expect("second turn output");
    assert!(
        summaries[0] < second,
        "the summary must scroll with history, not stay pinned (summary {}, second turn {second}):\n{}",
        summaries[0],
        screen.dump("end")
    );
    let joined = rows.join("\n");
    assert!(
        !joined.contains("[x] alpha") && !joined.contains("[x] beta"),
        "completed items must collapse into the summary:\n{}",
        screen.dump("end")
    );
}

/// The plan is pinned once, permanently: it stays above the input across
/// turn boundaries (a later turn that updates it repaints the same pinned
/// region), and no copy is ever re-recorded into the scrolling transcript at
/// turn end. Two turns each touch the plan; afterward every step appears
/// exactly once on screen, in its latest state, above the idle input.
#[test]
fn dock_pins_the_plan_across_turns_with_a_single_copy_on_screen() {
    const ROWS: u16 = 18;
    const COLS: u16 = 64;

    let rig = rig();
    let first = r#"{"todos":[{"content":"alpha","status":"in_progress"},{"content":"beta","status":"pending"}]}"#;
    let second = r#"{"todos":[{"content":"alpha","status":"completed"},{"content":"beta","status":"in_progress"}]}"#;
    rig.server
        .enqueue_stream_toolcalls(&[("p1", "plan", first)], None);
    rig.server.enqueue_stream_completion("TURN-ONE-END");
    rig.server
        .enqueue_stream_toolcalls(&[("p2", "plan", second)], None);
    rig.server.enqueue_stream_completion("TURN-TWO-END");

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"make a plan\r");
    pty.wait_for("TURN-ONE-END");
    settle();
    pty.send(b"advance the plan\r");
    pty.wait_for("TURN-TWO-END");
    settle();
    pty.drain(std::time::Duration::from_millis(400));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();

    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();

    let marker = rows
        .iter()
        .rposition(|r| r.contains(MARKER))
        .unwrap_or_else(|| panic!("idle input box missing:\n{}", screen.dump("idle")));
    for (step, glyphs) in [("alpha", &["[x]"][..]), ("beta", &SPINNER_FRAMES[..])] {
        let hits: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| row.contains(step).then_some(index))
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "step {step:?} must appear exactly once (pinned), not per turn:\n{}",
            screen.dump("idle")
        );
        assert!(
            hits[0] < marker,
            "the pinned {step:?} row sits above the idle input:\n{}",
            screen.dump("idle")
        );
        assert!(
            glyphs.iter().any(|glyph| rows[hits[0]].contains(glyph)),
            "the pinned {step:?} row shows its LATEST state {glyphs:?}: {:?}\n{}",
            rows[hits[0]],
            screen.dump("idle")
        );
    }
}

/// A pinned region row longer than the terminal is clamped to exactly one
/// physical row ending in an ellipsis. The in-place refresh (comet cadence,
/// keystrokes) must not erase that trailing glyph: a full-width row parks the
/// terminal's deferred-wrap latch in the last column, so a clear-to-end there
/// would blank the ellipsis. Snap the screen after several refresh ticks and
/// confirm the ellipsis is still on the row.
#[test]
fn dock_region_row_keeps_its_ellipsis_across_an_in_place_refresh() {
    const ROWS: u16 = 12;
    const COLS: u16 = 40;

    let rig = rig();
    let long = "this is a very long plan item that certainly exceeds the terminal width";
    let todo = format!(r#"{{"todos":[{{"content":"{long}","status":"pending"}}]}}"#);
    rig.server
        .enqueue_stream_toolcalls(&[("p1", "plan", todo.as_str())], None);
    rig.server
        .enqueue_raw(stalled_stream("done ZZEND", 1, 3000, true));

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.wait_for("plan (0/1 done):");
    // Span several 120ms comet refreshes: the in-place repaint is where a
    // full-width region row could lose its trailing ellipsis.
    pty.drain(std::time::Duration::from_millis(500));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();
    println!("\n{}", screen.dump("FULL-WIDTH REGION ROW"));

    pty.wait_for("ZZEND");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();

    let item = rows
        .iter()
        .find(|r| r.contains("this is a very"))
        .expect("clamped plan item row");
    assert!(
        item.ends_with('…'),
        "the clamped region row lost its ellipsis on an in-place refresh: {item:?}\n{}",
        screen.dump("row")
    );
}

/// The pinned regions are bounded by the screen height, so a long plan can never
/// grow the live frame past the terminal (where the relative cursor moves would
/// clamp at the top edge and desync). On a short screen the overflow collapses
/// into one summary row and the frame stays intact and in order.
#[test]
fn dock_caps_pinned_regions_to_the_screen_height() {
    const ROWS: u16 = 10;
    const COLS: u16 = 50;

    let rig = rig();
    // Twelve items plus the header would be 13 region rows; the cap on a 10-row
    // screen is term_height - 4 = 6, so most collapse into a counted row.
    let mut items = String::new();
    for i in 1..=12 {
        if i > 1 {
            items.push(',');
        }
        items.push_str(&format!(
            r#"{{"content":"item number {i:02}","status":"pending"}}"#
        ));
    }
    let todo = format!(r#"{{"todos":[{items}]}}"#);
    rig.server
        .enqueue_stream_toolcalls(&[("p1", "plan", todo.as_str())], None);
    rig.server
        .enqueue_raw(stalled_stream("done ZZEND", 1, 3000, true));

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.wait_for("plan (0/12 done):");
    pty.drain(std::time::Duration::from_millis(500));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();
    println!("\n{}", screen.dump("CAPPED REGION (short screen)"));

    pty.wait_for("ZZEND");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();

    // The frame is intact and on-screen: status, input row, and bottom rule all
    // present and in order within the ten rows (no top-edge clamp corruption).
    let working = rows
        .iter()
        .rposition(|r| r.contains("Working"))
        .expect("Working row on screen");
    let input = rows
        .iter()
        .rposition(|r| r.contains(MARKER))
        .expect("input row on screen");
    let bottom = rows
        .iter()
        .rposition(|r| r.contains("Esc Esc to cancel"))
        .expect("bottom rule on screen");
    assert!(
        working < input && input < bottom,
        "frame rows out of order (working {working}, input {input}, bottom {bottom}):\n{}",
        screen.dump("cap")
    );
    // The overflow collapsed into a single summary row rather than overrunning.
    assert!(
        rows.iter()
            .any(|r| r.contains("12 pending") && r.contains("hidden")),
        "no overflow summary row; the region was not capped to the screen:\n{}",
        screen.dump("cap")
    );
    let header = rows
        .iter()
        .position(|r| r.contains("plan (0/12 done):"))
        .expect("plan header");
    assert!(
        working < header && header < input,
        "plan not pinned inside the frame:\n{}",
        screen.dump("cap")
    );
}

/// A cap must reserve independent rows for the active plan step and the compact
/// detached-agent indicator. Source-order truncation used to hide one or both
/// when the active plan item appeared late in a long checklist.
#[test]
fn dock_cap_keeps_active_plan_step_and_agent_summary() {
    const ROWS: u16 = 10;
    const COLS: u16 = 64;

    let rig = rig();
    rig.server.allow_interleaving();
    let mut items = String::new();
    for i in 1..=12 {
        if i > 1 {
            items.push(',');
        }
        let status = if i == 12 { "in_progress" } else { "pending" };
        let content = if i == 12 {
            "late active step"
        } else {
            "early pending step"
        };
        items.push_str(&format!(
            r#"{{"content":"{content} {i:02}","status":"{status}"}}"#
        ));
    }
    let plan = format!(r#"{{"todos":[{items}]}}"#);
    let parent = || RequestMatch::HasTool("subagent".to_string());
    let child = || RequestMatch::LacksTool("subagent".to_string());
    rig.server.enqueue_stream_toolcalls_for(
        parent(),
        &[
            ("cap-plan", "plan", plan.as_str()),
            ("cap-agent", "subagent", r#"{"prompt":"slow cap child"}"#),
        ],
        None,
    );
    rig.server
        .enqueue_raw_for(parent(), stalled_stream("PARENT-CAP-END", 1, 1600, true));
    rig.server
        .enqueue_raw_for(child(), stalled_stream("CAP-CHILD-DONE", 1, 2400, true));
    rig.server
        .enqueue_stream_completion_for(parent(), "CAP-COLLECTED-END");

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"run capped plan and helper\r");
    pty.wait_for("plan (0/12 done):");
    pty.wait_for("[1] agents running (Tab to view)");
    pty.drain(std::time::Duration::from_millis(450));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();
    let visible = rows.join("\n");

    pty.wait_for("PARENT-CAP-END");
    pty.wait_for("agent-1 ok");
    pty.wait_for("CAP-COLLECTED-END");
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();

    assert!(
        visible.contains("late active step 12"),
        "the active plan step was hidden by the cap:\n{}",
        screen.dump("combined cap")
    );
    assert!(
        visible.contains("agents running (Tab to view)"),
        "the agent summary was hidden by the long plan:\n{}",
        screen.dump("combined cap")
    );
    assert!(
        visible.contains("hidden"),
        "the remaining capped rows were not summarized:\n{}",
        screen.dump("combined cap")
    );
}

/// The plan's own cap, independent of the screen: even on a tall terminal a
/// long checklist pins the header, at most six step rows windowed on the
/// active step, and one "… +N more" row with done/queued counts, instead of
/// the whole list.
#[test]
fn dock_plan_region_caps_at_six_steps_with_a_more_row() {
    const ROWS: u16 = 40;
    const COLS: u16 = 72;

    let rig = rig();
    let mut items = String::new();
    for i in 1..=12 {
        if i > 1 {
            items.push(',');
        }
        let status = match i {
            1..=3 => "completed",
            4 => "in_progress",
            _ => "pending",
        };
        items.push_str(&format!(
            r#"{{"content":"step {i:02}","status":"{status}"}}"#
        ));
    }
    let plan = format!(r#"{{"todos":[{items}]}}"#);
    rig.server
        .enqueue_stream_toolcalls(&[("p1", "plan", plan.as_str())], None);
    rig.server
        .enqueue_raw(stalled_stream("PLANCAP-END", 1, 2500, true));

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("plan (3/12 done):");
    pty.drain(std::time::Duration::from_millis(500));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();
    let visible = rows.join("\n");

    pty.wait_for("PLANCAP-END");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();

    // The window leads with the active step and runs into the queue; the
    // three completed steps and the overflow tail collapse into counts.
    for shown in ["step 04", "step 05", "step 09"] {
        assert!(
            visible.contains(shown),
            "{shown} missing from the plan window:\n{}",
            screen.dump("plan cap")
        );
    }
    for hidden in ["step 01", "step 03", "step 10", "step 12"] {
        assert!(
            !visible.contains(hidden),
            "{hidden} should be behind the cap:\n{}",
            screen.dump("plan cap")
        );
    }
    assert!(
        visible.contains("… +6 more steps · 3 done · 3 queued"),
        "the more-row with counts is missing:\n{}",
        screen.dump("plan cap")
    );
}
