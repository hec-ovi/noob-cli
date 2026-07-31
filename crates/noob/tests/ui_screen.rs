//! Screen-level dock reproduction. Byte-only PTY assertions cannot see a
//! scroll-at-bottom cursor-math desync: they have no screen model. These
//! replay noob's exact captured bytes into a small rows x cols emulator
//! (noob-testkit's Vt) and inspect the dock the way a human would: mid-turn
//! with the frame live, at idle with it torn down, and across SIGWINCH
//! reflows and resize storms, scrollback included.

mod ui;

use ui::*;

/// Diagnostic replay: feed a captured raw byte file (NOOB_REPLAY, with
/// NOOB_REPLAY_ROWS/COLS) through the reflow emulator and print the screen.
/// Ignored in normal runs, and a no-op without NOOB_REPLAY set: `./dev.sh
/// smoke` runs every `--ignored` test, so this must pass quietly there
/// instead of failing each smoke run on an unset diagnostic variable. Run it
/// on demand with `--nocapture` to see the dump.
#[test]
#[ignore]
fn replay_captured_bytes() {
    let Ok(path) = std::env::var("NOOB_REPLAY") else {
        eprintln!("replay: set NOOB_REPLAY to a raw capture file to use this diagnostic");
        return;
    };
    let rows: usize = std::env::var("NOOB_REPLAY_ROWS")
        .unwrap_or_else(|_| "24".into())
        .parse()
        .unwrap();
    let cols: usize = std::env::var("NOOB_REPLAY_COLS")
        .unwrap_or_else(|_| "100".into())
        .parse()
        .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut vt = Vt::new(rows, cols);
    vt.feed(&bytes);
    println!("{}", vt.dump(&path));
}

#[test]
fn dock_input_row_survives_a_scrolling_stream_at_the_screen_level() {
    // A small screen so the stream scrolls it several times over, and a width
    // wide enough that no single short line wraps.
    const ROWS: u16 = 12;
    const COLS: u16 = 64;

    let rig = rig();
    // Twenty-four short, unique lines (one per stream delta, since
    // `chat_stream_datas` cuts on whitespace and each line ends in `\n`), then a
    // final ZZEND marker. Stream the first fourteen, stall long enough to snap a
    // mid-turn screen, then stream the rest and finish.
    let mut text = String::new();
    for i in 1..=24 {
        text.push_str(&format!("row-{i:02}-xyz\n"));
    }
    text.push_str("ZZEND");
    // datas: [role, row-01..row-24, ZZEND, finish, usage, DONE]. Head = role +
    // rows 1..14 => 15 deltas.
    rig.server
        .enqueue_raw(stalled_stream(&text, 15, 1200, true));

    let mut pty = spawn_pty_sized(&rig, &[], Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working"); // the dock is up and the stream is flowing
    pty.wait_for("row-14-xyz"); // the last line before the stall has landed

    // MID-TURN: drain the trailing frame repaints during the stall, then snap.
    pty.drain(std::time::Duration::from_millis(500));
    let mid = pty.screen(ROWS, COLS);
    let mid_rows = mid.render();
    println!("\n{}", mid.dump("MID-TURN (frame live, mid-stall)"));

    // Let the stall lapse, the rest stream, and the turn finish.
    pty.wait_for("ZZEND");
    settle();
    pty.drain(std::time::Duration::from_millis(300));
    let end = pty.screen(ROWS, COLS);
    println!("\n{}", end.dump("END-OF-TURN (idle prompt)"));

    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();

    // ---- MID-TURN assertions: the dock must be intact and live. ----
    let (top, bottom) = dock_rows(&mid_rows)
        .unwrap_or_else(|| panic!("mid-turn dock rules missing entirely:\n{}", mid.dump("mid")));
    assert_eq!(
        bottom,
        top + 2,
        "the dock is not three contiguous rows (top {top}, bottom {bottom}):\n{}",
        mid.dump("mid")
    );
    let input = &mid_rows[top + 1];
    assert!(
        input.contains(MARKER),
        "MID-TURN the input row lost its `{MARKER}` marker (input disappeared during \
         activity); input row = {input:?}\n{}",
        mid.dump("mid")
    );
    // The input row must be the dock's own row, not a line of streamed output
    // that scrolled into the marker's position.
    assert!(
        !input.contains("row-") && !input.contains("ZZEND"),
        "MID-TURN streamed output bled into the input row: {input:?}\n{}",
        mid.dump("mid")
    );

    // ---- END-OF-TURN assertions: the live turn frame (Working/cancel) is gone,
    //      replaced by the persistent idle input box so the input never collapses
    //      to a lone marker between turns. ----
    let end_rows = end.render();
    assert!(
        dock_rows(&end_rows).is_none(),
        "END-OF-TURN the live turn frame (Working/cancel) was not torn down:\n{}",
        end.dump("end")
    );
    let marker = end_rows
        .iter()
        .rposition(|r| r.contains(MARKER))
        .unwrap_or_else(|| panic!("END-OF-TURN no idle input box:\n{}", end.dump("end")));
    // The empty idle box reads as a live input (dim hint), never a bare marker,
    // and no streamed output bled into the input row.
    assert!(
        end_rows[marker].contains("type a message"),
        "END-OF-TURN the idle input lost its hint (collapsed to a bare marker): {:?}\n{}",
        end_rows[marker],
        end.dump("end")
    );
    assert!(
        !end_rows[marker].contains("row-") && !end_rows[marker].contains("ZZEND"),
        "END-OF-TURN streamed output bled into the idle input row: {:?}\n{}",
        end_rows[marker],
        end.dump("end")
    );
    // The box is framed: a rule directly below the input, and nothing past it.
    assert!(
        end_rows.get(marker + 1).is_some_and(|r| r.contains("──")),
        "END-OF-TURN the idle box has no bottom rule under the input:\n{}",
        end.dump("end")
    );
    for (i, r) in end_rows.iter().enumerate().skip(marker + 2) {
        assert!(
            r.is_empty(),
            "END-OF-TURN row {i} below the idle box is not blank: {r:?}\n{}",
            end.dump("end")
        );
    }
}

/// The input row is a visible affordance during a turn: while the draft is
/// empty the dock shows a dim "type a message; Enter queues it" placeholder (so the row
/// never reads as absent, the reported "input disappears during activity"), and
/// the first keystroke replaces it with the draft rather than sitting beside it.
#[test]
fn dock_input_row_shows_a_placeholder_when_empty_and_replaces_it_on_typing() {
    const ROWS: u16 = 12;
    const COLS: u16 = 64;

    let rig = rig();
    // Newline-terminated lines so each flushes mid-stream (the markdown renderer
    // holds an un-terminated line until turn end). Stream role + two lines, then
    // stall long enough to snap twice, then finish.
    let text = "aa-line\nbb-line\ncc-line\ndd-line\nZZEND";
    rig.server.enqueue_raw(stalled_stream(text, 3, 4000, true));
    rig.server.enqueue_stream_completion("second turn ran");

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.wait_for("bb-line"); // last head line flushed; inside the 4000 ms stall

    // EMPTY DRAFT: the placeholder is the visible input affordance.
    pty.drain(std::time::Duration::from_millis(500));
    let empty = pty.screen(ROWS, COLS);
    let empty_rows = empty.render();
    let (top, _bottom) = dock_rows(&empty_rows)
        .unwrap_or_else(|| panic!("dock rules missing:\n{}", empty.dump("empty")));
    assert!(
        empty_rows[top + 1].contains("type a message; Enter queues it"),
        "the empty input row shows no placeholder affordance: {:?}\n{}",
        empty_rows[top + 1],
        empty.dump("empty")
    );

    // TYPED: the placeholder is replaced by the draft, never shown alongside it.
    pty.send(b"my note");
    pty.drain(std::time::Duration::from_millis(400));
    let typed = pty.screen(ROWS, COLS);
    let typed_rows = typed.render();
    let (ttop, _) = dock_rows(&typed_rows)
        .unwrap_or_else(|| panic!("dock rules missing after typing:\n{}", typed.dump("typed")));
    let tinput = &typed_rows[ttop + 1];
    assert!(
        tinput.contains("my note") && !tinput.contains("type a message"),
        "typing did not replace the placeholder: {tinput:?}\n{}",
        typed.dump("typed")
    );

    // The typed draft carries to the next prompt and submits whole (proving it
    // is a real draft, not the display-only placeholder).
    pty.wait_for("ZZEND");
    settle();
    pty.send(b"\r");
    pty.wait_for("second turn ran");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let reqs = rig.api_requests();
    assert_eq!(last_user(reqs.last().unwrap()), "my note");
    rig.server.assert_clean();
}

/// The idle input is a persistent framed box from the very first prompt: a plain
/// rule above and below a `› type a message` line, present before any keystroke,
/// so the input never reads as a lone marker (the reported "input disappears when
/// inference finishes"). This is the dock default; the classic NOOB_DOCK=0 editor
/// keeps its bare-marker-expands behavior.
#[test]
fn dock_idle_input_is_a_persistent_framed_box() {
    const ROWS: u16 = 10;
    const COLS: u16 = 50;

    let rig = rig();
    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    // No keystroke: the framed idle box must already be on screen.
    pty.drain(std::time::Duration::from_millis(300));
    let screen = pty.screen(ROWS, COLS);
    let rows = screen.render();
    println!("\n{}", screen.dump("FRESH IDLE BOX (no keystroke)"));

    pty.send(&[0x04]); // Ctrl-D exits from the empty box
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();

    let marker = rows
        .iter()
        .rposition(|r| r.contains(MARKER))
        .unwrap_or_else(|| panic!("no idle input box before typing:\n{}", screen.dump("idle")));
    assert!(
        rows[marker].contains("type a message"),
        "the fresh idle box is missing its hint (bare marker): {:?}\n{}",
        rows[marker],
        screen.dump("idle")
    );
    assert!(
        marker >= 1 && rows[marker - 1].contains("──"),
        "no top rule above the idle input:\n{}",
        screen.dump("idle")
    );
    assert!(
        rows.get(marker + 1).is_some_and(|r| r.contains("──")),
        "no bottom rule below the idle input:\n{}",
        screen.dump("idle")
    );
}

/// A terminal resize (SIGWINCH) reflows the idle box to the new width WITHOUT a
/// keystroke. The dock reads the width once and then blocks on input, so without
/// the signal the box would keep its startup width (the "first appearance width
/// is wrong" report, seen when a Docker pty is sized a beat after noob starts)
/// until the user typed. The box rules span the full terminal width, so their
/// dash count tracks the resize.
#[test]
fn dock_idle_box_reflows_on_resize_without_a_keystroke() {
    const ROWS: u16 = 12;

    let rig = rig();
    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, 50)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.drain(std::time::Duration::from_millis(300));

    let rule_dashes = |pty: &Pty, cols: u16| -> usize {
        let rows = pty.screen(ROWS, cols).render();
        let marker = rows
            .iter()
            .rposition(|r| r.contains(MARKER))
            .expect("idle box marker");
        // The rule directly under the input row is the box bottom.
        rows.get(marker + 1)
            .map(|r| r.chars().filter(|&c| c == '─').count())
            .unwrap_or(0)
    };

    let narrow = rule_dashes(&pty, 50);
    assert_eq!(
        narrow, 50,
        "the initial idle box rule should span the 50-col terminal"
    );

    // Resize wider with NO keystroke: SIGWINCH must reflow the box.
    pty.resize(ROWS, 100);
    pty.drain(std::time::Duration::from_millis(500));
    let wide = rule_dashes(&pty, 100);
    assert_eq!(
        wide, 100,
        "the idle box did not reflow to 100 cols on resize (SIGWINCH ignored)"
    );

    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();
}

/// The same shrink guarantee mid-turn: the active frame (top status rule,
/// input row, bottom rule) is erased by its physical reflowed height and
/// repainted at the new width, leaving no fragments and no duplicated frame.
#[test]
fn dock_active_frame_shrink_resize_leaves_no_rule_fragments() {
    const ROWS: u16 = 14;

    let rig = rig();
    // A stream that stalls long enough to resize mid-turn, then finishes.
    let text = "aa-line\nbb-line\nZZEND";
    rig.server.enqueue_raw(stalled_stream(text, 2, 4000, true));

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, 100)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.wait_for("aa-line"); // inside the stall
    pty.drain(std::time::Duration::from_millis(300));
    let mark = pty.raw().len();

    pty.resize(ROWS, 60);
    pty.drain(std::time::Duration::from_millis(800));
    let post = pty.raw().len();

    pty.wait_for("ZZEND");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();

    let mut vt = Vt::new(ROWS as usize, 100);
    vt.feed(&pty.raw()[..mark]);
    vt.resize(ROWS as usize, 60);
    vt.feed(&pty.raw()[mark..post]);
    let rows = vt.render();

    let rules = rule_row_indices(&rows);
    assert_eq!(
        rules.len(),
        2,
        "exactly the active frame's two rules survive a mid-turn shrink:\n{}",
        vt.dump("after shrink")
    );
    let (top, bottom) = (rules[0], rules[1]);
    assert!(
        rows[top].contains("Working") && rows[bottom].contains("Esc Esc to cancel"),
        "the surviving rules are the live frame's status rows:\n{}",
        vt.dump("after shrink")
    );
    assert_eq!(
        bottom - top,
        2,
        "the frame is exactly top rule, input row, bottom rule:\n{}",
        vt.dump("after shrink")
    );
}

/// REPEATED resizes at the idle prompt leave scrollback untouched. The old
/// path reset the viewport per resize and VTE-family terminals archive the
/// whole cleared screen (reflowed stale frame plus blank tail) into history,
/// so a resize storm stacked one garbage screen per step: the README's
/// former known issue. The frame is now retired wrap-aware and repainted in
/// place, so after shrink-widen-shrink the scrollback holds nothing new and
/// the screen holds exactly one idle box at the final width.
#[test]
fn dock_repeated_resizes_archive_nothing_into_scrollback() {
    const ROWS: u16 = 14;

    let rig = rig();
    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, 100)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.drain(std::time::Duration::from_millis(300));

    let mut vt = Vt::new(ROWS as usize, 100);
    let mut mark = pty.raw().len();
    vt.feed(&pty.raw()[..mark]);
    for &cols in &[60u16, 100, 50] {
        pty.resize(ROWS, cols);
        pty.drain(std::time::Duration::from_millis(800));
        vt.resize(ROWS as usize, cols as usize);
        vt.feed(&pty.raw()[mark..]);
        mark = pty.raw().len();
    }

    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();

    assert_scrollback_clean(&vt, ROWS as usize, "idle resize storm");
    let rows = vt.render();
    let rules = rule_row_indices(&rows);
    assert_eq!(
        rules.len(),
        2,
        "exactly the idle box's two rules survive the storm:\n{}",
        vt.dump("after storm")
    );
    let marker = rows
        .iter()
        .rposition(|r| r.contains(MARKER))
        .unwrap_or_else(|| panic!("idle input row missing:\n{}", vt.dump("after storm")));
    assert_eq!(
        (rules[0], rules[1]),
        (marker - 1, marker + 1),
        "the surviving rules are the box around the input row:\n{}",
        vt.dump("after storm")
    );
    for index in rules {
        let dashes = rows[index].chars().filter(|&c| c == '─').count();
        assert_eq!(
            dashes,
            50,
            "each rule spans the final width exactly:\n{}",
            vt.dump("after storm")
        );
    }
}

/// The same storm mid-turn: the streamed transcript above the frame must
/// survive the resizes on screen (not archived, not duplicated, not
/// shredded), and scrollback must gain no frame garbage. This is the
/// scenario where the old viewport reset was most destructive: it archived
/// the partial transcript together with the stale frame on every step.
#[test]
fn dock_mid_turn_repeated_resizes_keep_the_transcript_clean() {
    const ROWS: u16 = 14;

    let rig = rig();
    let text = "aa-line\nbb-line\nZZEND";
    rig.server.enqueue_raw(stalled_stream(text, 2, 6000, true));

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, 100)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.wait_for("aa-line"); // inside the stall
    pty.drain(std::time::Duration::from_millis(300));

    let mut vt = Vt::new(ROWS as usize, 100);
    let mut mark = pty.raw().len();
    vt.feed(&pty.raw()[..mark]);
    for &cols in &[60u16, 100, 50] {
        pty.resize(ROWS, cols);
        pty.drain(std::time::Duration::from_millis(700));
        vt.resize(ROWS as usize, cols as usize);
        vt.feed(&pty.raw()[mark..]);
        mark = pty.raw().len();
    }

    pty.wait_for("ZZEND");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();
    vt.feed(&pty.raw()[mark..]);

    assert_scrollback_clean(&vt, ROWS as usize, "mid-turn resize storm");
    // Every streamed line survives exactly once across screen + history: the
    // old resets archived transcript copies with the garbage, and a
    // shredded stale-geometry erase could duplicate or destroy them.
    let everything: Vec<String> = vt.scrollback().iter().cloned().chain(vt.render()).collect();
    for needle in ["aa-line", "bb-line", "ZZEND"] {
        let count = everything.iter().filter(|row| row.contains(needle)).count();
        assert_eq!(
            count,
            1,
            "{needle} must appear exactly once after the storm:\n{}",
            vt.dump("after storm")
        );
    }
}

/// Shrinking with a TYPED DRAFT wider than the new width, then widening back.
/// The draft makes the input line itself rewrap, so the erase must first hop
/// up to the line's first physical row (the cursor-hop branch that an empty
/// draft never exercises), and the widen leg proves the round trip repaints
/// cleanly in both directions. The draft buffer must survive untouched.
#[test]
fn dock_shrink_with_typed_draft_then_widen_repaints_cleanly() {
    const ROWS: u16 = 14;

    let rig = rig();
    rig.server.enqueue_stream_completion("DRAFT-TURN-END");

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, 100)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    // 80 draft chars: the input row is 82 cells, one row at 100 columns but
    // two at 60, so the resize erase must hop up one physical row first.
    let draft = "d123456789".repeat(8);
    pty.send(draft.as_bytes());
    pty.drain(std::time::Duration::from_millis(400));
    let mark_shrink = pty.raw().len();

    pty.resize(ROWS, 60);
    pty.drain(std::time::Duration::from_millis(800));
    let mark_widen = pty.raw().len();

    pty.resize(ROWS, 100);
    pty.drain(std::time::Duration::from_millis(800));
    let post = pty.raw().len();

    // The surviving draft submits whole.
    pty.send(b"\r");
    pty.wait_for("DRAFT-TURN-END");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());

    // Shrink leg: the box holds, the draft tail is on the input row.
    let mut vt = Vt::new(ROWS as usize, 100);
    vt.feed(&pty.raw()[..mark_shrink]);
    vt.resize(ROWS as usize, 60);
    vt.feed(&pty.raw()[mark_shrink..mark_widen]);
    let rows = vt.render();
    let rules = rule_row_indices(&rows);
    assert_eq!(
        rules.len(),
        2,
        "no fragments after shrinking over a wide draft:\n{}",
        vt.dump("shrunk with draft")
    );
    let marker = rows
        .iter()
        .rposition(|r| r.contains(MARKER))
        .unwrap_or_else(|| panic!("input row missing:\n{}", vt.dump("shrunk with draft")));
    assert_eq!((rules[0], rules[1]), (marker - 1, marker + 1));
    assert!(
        rows[marker].contains("d123456789"),
        "the draft window survives the shrink: {:?}\n{}",
        rows[marker],
        vt.dump("shrunk with draft")
    );

    // Widen leg: clean again at the full width.
    vt.resize(ROWS as usize, 100);
    vt.feed(&pty.raw()[mark_widen..post]);
    let rows = vt.render();
    let rules = rule_row_indices(&rows);
    assert_eq!(
        rules.len(),
        2,
        "no fragments after widening back:\n{}",
        vt.dump("widened back")
    );
    for index in rules {
        assert_eq!(
            rows[index].chars().filter(|&c| c == '─').count(),
            100,
            "rules span the restored width:\n{}",
            vt.dump("widened back")
        );
    }

    // The full 80-char draft reached the agent despite two reflows.
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(last_user(&reqs[0]), draft);
    rig.server.assert_clean();
}

/// Shrinking while pinned region rows are on screen: a plan with one step
/// clamped to the full old width contributes MULTIPLE wrapped physical rows
/// to the erase walk, the arithmetic an empty frame never exercises. After
/// the shrink the plan is still pinned exactly once above an intact box.
#[test]
fn dock_shrink_with_pinned_plan_rows_leaves_no_fragments() {
    const ROWS: u16 = 14;

    let rig = rig();
    let long = "investigate the long running renderer path and verify the erase walk keeps every physical row accounted for";
    let plan = format!(
        r#"{{"todos":[{{"content":"{long}","status":"in_progress"}},{{"content":"short step","status":"pending"}}]}}"#
    );
    rig.server
        .enqueue_stream_toolcalls(&[("p1", "plan", plan.as_str())], None);
    rig.server.enqueue_stream_completion("PLAN-PINNED-END");

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, 100)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"make the plan\r");
    pty.wait_for("PLAN-PINNED-END");
    settle();
    pty.drain(std::time::Duration::from_millis(400));
    let mark = pty.raw().len();

    pty.resize(ROWS, 60);
    pty.drain(std::time::Duration::from_millis(800));
    let post = pty.raw().len();

    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();

    let mut vt = Vt::new(ROWS as usize, 100);
    vt.feed(&pty.raw()[..mark]);
    vt.resize(ROWS as usize, 60);
    vt.feed(&pty.raw()[mark..post]);
    let rows = vt.render();

    let rules = rule_row_indices(&rows);
    assert_eq!(
        rules.len(),
        2,
        "no fragments after shrinking with pinned plan rows:\n{}",
        vt.dump("shrunk with plan")
    );
    let marker = rows
        .iter()
        .rposition(|r| r.contains(MARKER))
        .unwrap_or_else(|| panic!("input row missing:\n{}", vt.dump("shrunk with plan")));
    let step_hits = rows
        .iter()
        .filter(|r| r.contains("investigate the long"))
        .count();
    assert_eq!(
        step_hits,
        1,
        "the pinned plan appears exactly once after the shrink:\n{}",
        vt.dump("shrunk with plan")
    );
    let step_row = rows
        .iter()
        .position(|r| r.contains("investigate the long"))
        .expect("pinned step");
    assert!(
        step_row < marker,
        "the plan stays above the input box:\n{}",
        vt.dump("shrunk with plan")
    );
}

/// The live-caught combination: a shrink in the MIDDLE of a turn that has a
/// plan pinned (the 08-22 screenshots: offset input and rule fragments all
/// over). After the viewport reset the screen holds exactly one clean frame:
/// the Working rule, the plan pinned once, the input row, the bottom rule,
/// and not a single stray fragment.
#[test]
fn dock_active_shrink_with_a_pinned_plan_repaints_one_clean_frame() {
    const ROWS: u16 = 14;

    let rig = rig();
    let plan = r#"{"todos":[{"content":"alpha step","status":"in_progress"},{"content":"beta step","status":"pending"}]}"#;
    rig.server
        .enqueue_stream_toolcalls(&[("p1", "plan", plan)], None);
    rig.server
        .enqueue_raw(stalled_stream("SHRINK-TURN-ZZ", 2, 4000, true));

    let mut pty = spawn_pty_sized(&rig, DOCK, Some((ROWS, 100)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("plan (0/2 done):"); // the pinned plan is on the live frame
    pty.drain(std::time::Duration::from_millis(300));
    let mark = pty.raw().len();

    pty.resize(ROWS, 60);
    pty.drain(std::time::Duration::from_millis(800));
    let post = pty.raw().len();

    pty.wait_for("SHRINK-TURN-ZZ");
    settle();
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();

    let mut vt = Vt::new(ROWS as usize, 100);
    vt.feed(&pty.raw()[..mark]);
    vt.resize(ROWS as usize, 60);
    vt.feed(&pty.raw()[mark..post]);
    let rows = vt.render();

    let rules = rule_row_indices(&rows);
    assert_eq!(
        rules.len(),
        2,
        "exactly the frame's two rules survive a mid-turn shrink over a plan:\n{}",
        vt.dump("after shrink")
    );
    assert!(
        rows[rules[0]].contains("Working"),
        "the top rule is the live status row:\n{}",
        vt.dump("after shrink")
    );
    let step_hits = rows.iter().filter(|r| r.contains("alpha step")).count();
    assert_eq!(
        step_hits,
        1,
        "the pinned plan appears exactly once after the shrink:\n{}",
        vt.dump("after shrink")
    );
    let header = rows
        .iter()
        .position(|r| r.contains("plan (0/2 done):"))
        .unwrap_or_else(|| panic!("plan header missing:\n{}", vt.dump("after shrink")));
    let marker = rows
        .iter()
        .rposition(|r| r.contains(MARKER))
        .unwrap_or_else(|| panic!("input row missing:\n{}", vt.dump("after shrink")));
    assert!(
        rules[0] < header && header < marker && marker < rules[1],
        "frame order: Working rule, plan, input, bottom rule (top {}, header {header}, input {marker}, bottom {}):\n{}",
        rules[0],
        rules[1],
        vt.dump("after shrink")
    );
}

/// The same guarantee for logical lines LONGER than the terminal width, which
/// the terminal wraps into several physical rows. noob emits the whole line and
/// relies on the terminal to wrap and scroll; its dock erase/redraw only knows
/// three frame rows, so this is where a row-agnostic desync would surface.
#[test]
fn dock_input_row_survives_wrapping_lines_at_the_screen_level() {
    const ROWS: u16 = 12;
    const COLS: u16 = 64;

    let rig = rig();
    // Twelve lines of ~150 chars each: every one wraps to three physical rows at
    // width 64. Interior spaces mean each wraps across many word deltas.
    let mut text = String::new();
    for i in 1..=12 {
        text.push_str(&format!("para-{i:02} ").repeat(17));
        text.push('\n');
    }
    text.push_str("ZZEND");
    let datas = noob_testkit::chat_stream_datas(&text);
    rig.server
        .enqueue_raw(stalled_stream(&text, datas.len() / 2, 1200, true));

    let mut pty = spawn_pty_sized(&rig, &[], Some((ROWS, COLS)), &[]);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"go\r");
    pty.wait_for("Working");
    pty.wait_for("para-05"); // several wrapped lines have scrolled past
    pty.drain(std::time::Duration::from_millis(500));
    let mid = pty.screen(ROWS, COLS);
    let mid_rows = mid.render();
    println!("\n{}", mid.dump("WRAP MID-TURN (frame live, mid-stall)"));

    pty.wait_for("ZZEND");
    settle();
    pty.drain(std::time::Duration::from_millis(300));
    let end = pty.screen(ROWS, COLS);
    println!("\n{}", end.dump("WRAP END-OF-TURN (idle prompt)"));

    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();

    // MID-TURN: the dock is three contiguous rows and the input row is present.
    let (top, bottom) = dock_rows(&mid_rows)
        .unwrap_or_else(|| panic!("mid-turn dock rules missing:\n{}", mid.dump("mid")));
    assert_eq!(
        bottom,
        top + 2,
        "the dock is not three contiguous rows:\n{}",
        mid.dump("mid")
    );
    let input = &mid_rows[top + 1];
    assert!(
        input.contains(MARKER),
        "MID-TURN the input row lost its `{MARKER}` marker: {input:?}\n{}",
        mid.dump("mid")
    );
    assert!(
        !input.contains("para-"),
        "MID-TURN wrapped output bled into the input row: {input:?}\n{}",
        mid.dump("mid")
    );

    // END-OF-TURN: the live frame is gone and a bare idle marker remains.
    let end_rows = end.render();
    assert!(
        dock_rows(&end_rows).is_none(),
        "END-OF-TURN the live frame was not torn down:\n{}",
        end.dump("end")
    );
    assert!(
        end_rows.iter().any(|r| r.trim_start().starts_with(MARKER)),
        "END-OF-TURN no idle `{MARKER}` prompt:\n{}",
        end.dump("end")
    );
}
