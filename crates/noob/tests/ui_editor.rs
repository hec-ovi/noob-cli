//! The classic per-prompt raw-mode editor (NOOB_DOCK=0) through the compiled
//! binary, plus the piped cooked path and slash-command Tab completion. A real
//! pty makes the REPL see a terminal, so the termios editor engages; these
//! drive it byte-for-byte the way a keyboard would and assert on the EDITED
//! result that reaches the agent (the recorded request), never on how it
//! looks. A piped run must take the cooked path with no box and no
//! bracketed-paste toggles, byte-identical to before the editor existed.
//! Completion: Tab completes a unique `/` command, an ambiguous prefix shows
//! a candidate hint and stops at the common stem, and a non-slash line (or
//! the argument region) is never touched. Colors are never asserted.

mod ui;

use std::io::Write;

use ui::*;

/// The editor's line editing reaches the agent: text typed, then killed with
/// Ctrl-U, then the real line typed and submitted with a carriage return. The
/// agent must receive only the edited line. Ctrl-D on the next empty prompt
/// exits cleanly (distinct from a reprompt).
#[test]
fn raw_editor_edits_then_submits_the_clean_line() {
    let rig = rig();
    rig.server.enqueue_stream_completion("done one");

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY); // prompt 1 is now in raw mode
    pty.send(b"garbage draft");
    pty.send(&[0x15]); // Ctrl-U: kill the whole line
    pty.send(b"say hi\r"); // the real line, submitted with CR
    pty.wait_for("done one");
    pty.wait_for(RAW_READY); // prompt 2 is now in raw mode
    pty.send(&[0x04]); // Ctrl-D at the empty prompt: exit
    pty.wait_for("resume with"); // the exit hint tells you how to reopen
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

/// The idle prompt is a bare marker; the first keystroke expands it into a
/// framed box, so a horizontal rule (the frame's top/bottom line) only appears
/// once the human starts typing. The assertion is behavioral, not cosmetic: the
/// rule glyph is present after typing (the frame drew) and the edited line still
/// reaches the agent. Colors are never asserted.
#[test]
fn raw_editor_expands_a_framed_box_when_typing_starts() {
    let rig = rig();
    rig.server.enqueue_stream_completion("framed reply");

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY); // raw mode: the bare marker is drawn, no frame yet
    let before_typing = pty.seen().len();
    pty.send(b"hello frame");
    // Typing expands the box, so the frame's rule (a run of the horizontal line
    // glyph) is emitted; the banner's own rule is already behind the cursor.
    pty.wait_for("──");
    // The rule must appear after the point where typing began (the banner's own
    // rule is earlier, already behind the cursor when raw mode started).
    assert!(
        pty.seen()[before_typing..].contains("──"),
        "the frame rule must be drawn only after typing:\n{}",
        pty.seen()
    );
    pty.send(b"\r"); // submit
    pty.wait_for("framed reply");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]); // Ctrl-D exits
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1, "only the submitted line should have run");
    assert_eq!(
        last_user(&reqs[0]),
        "hello frame",
        "the framed line must reach the agent intact"
    );
    rig.server.assert_clean();
}

/// Ctrl-C at the prompt cancels the current line and reprompts; it never
/// submits. The line typed before it must not reach the agent, and the line
/// typed after it must.
#[test]
fn raw_ctrl_c_cancels_the_line_without_submitting() {
    let rig = rig();
    rig.server.enqueue_stream_completion("answered");

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY); // in raw mode: Ctrl-C is a byte, not VINTR
    pty.send(b"abandon this");
    pty.send(&[0x03]); // Ctrl-C: cancel, reprompt
    pty.wait_for("interrupted");
    pty.wait_for(RAW_READY); // the reprompt is in raw mode
    pty.send(b"real one\r");
    pty.wait_for("answered");
    pty.wait_for(RAW_READY); // the next prompt is in raw mode
    pty.send(b"/quit\r");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1, "the canceled draft must not have run");
    assert_eq!(last_user(&reqs[0]), "real one");
    rig.server.assert_clean();
}

/// A multi-line submission delivered in one raw read (as a terminal that
/// ignores bracketed paste would deliver a multi-line paste) runs one turn per
/// line: the tail after the first Enter is carried to the next prompt instead
/// of being dropped.
#[test]
fn raw_multiline_input_runs_one_turn_per_line() {
    let rig = rig();
    rig.server.enqueue_stream_completion("first done");
    rig.server.enqueue_stream_completion("second done");

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY); // raw: the tty does not canonicalize the newlines
    pty.send(b"line one\nline two\n"); // two lines in a single write
    pty.wait_for("first done");
    pty.wait_for("second done");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]); // Ctrl-D exits
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2, "each line should be its own turn");
    assert_eq!(last_user(&reqs[0]), "line one");
    assert_eq!(last_user(&reqs[1]), "line two");
    rig.server.assert_clean();
}

/// The thinking scanner sweeps during the request-to-first-token gap: after a
/// prompt is submitted, at least one comet frame reaches the terminal before the
/// reply arrives. The assertion is that it rendered at all (a lifecycle fact),
/// not how it looks; the piped test below is the byte-identity counterpart that
/// proves a non-tty surface shows none of it.
#[test]
fn raw_repl_shows_a_thinking_scanner_while_the_model_works() {
    let rig = rig();
    rig.server.enqueue_stream_completion("scanned reply");

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"work on it\r");
    pty.wait_for("scanned reply");
    pty.wait_for(RAW_READY); // back at a fresh prompt
    pty.send(&[0x04]); // Ctrl-D exits
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    // The comet glyph appears before the reply and is then cleared; its bytes
    // remain in the stream even though the line was wiped.
    let last_comet = pty.seen().rfind('▪').unwrap_or_else(|| {
        panic!(
            "the thinking scanner never rendered a comet frame:\n{}",
            pty.seen()
        )
    });
    // ...and it is torn down before the reply: no frame lands after the reply
    // text begins, so the model's words never interleave with the animation
    // (the first output byte joins the animation thread before it is written).
    let reply_at = pty.seen().find("scanned reply").expect("reply never arrived");
    assert!(
        last_comet < reply_at,
        "a comet frame rendered after the reply began (scanner not torn down):\n{}",
        pty.seen()
    );
    rig.server.assert_clean();
}

/// A piped REPL (stdin not a terminal) takes the cooked reader: the plain `> `
/// marker prints, and neither the box frame, the bracketed-paste toggles, nor
/// the thinking scanner ever reach the output. This is the byte-identity guard
/// for the non-tty surface.
#[test]
fn piped_repl_uses_cooked_reader_with_no_box() {
    let rig = rig();
    rig.server.enqueue_stream_completion("piped answer");

    let mut child = noob(rig.config.path(), rig.work.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"hello there\n/quit\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("piped answer"),
        "turn did not run: {stdout}"
    );
    assert!(
        stdout.contains("> "),
        "cooked prompt marker missing: {stdout}"
    );
    assert!(
        !stdout.contains('›'),
        "the box marker leaked into a piped repl: {stdout}"
    );
    assert!(
        !stdout.contains("\x1b[?2004h") && !stdout.contains("\x1b[?2004l"),
        "bracketed paste toggled on a piped repl: {stdout}"
    );
    assert!(
        !stdout.contains('▪'),
        "the thinking scanner leaked into a piped repl: {stdout}"
    );
    rig.server.assert_clean();
}

/// Tab on a unique slash-command prefix completes it: `/pl` + Tab submits as
/// `/plan`, which dispatches (the plan-mode note prints). Without completion the
/// line would submit as `/pl` and be rejected as an unknown command. The classic
/// per-prompt editor gives a RAW_READY sync point and exercises the read_raw Tab
/// path.
#[test]
fn tab_completes_a_unique_slash_command_prefix() {
    let rig = rig();

    let mut pty = spawn_pty(&rig); // NOOB_DOCK=0: the read_raw path
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"/pl"); // an unambiguous prefix of exactly one command
    pty.send(&[0x09]); // Tab: complete the token to /plan
    pty.send(b"\r"); // submit the completed command
    pty.wait_for("cache prefix reset"); // enter_plan's note: /plan actually ran
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]); // Ctrl-D exits
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    // The tell-tale of a missed completion: `/pl` would dispatch as unknown.
    assert!(
        !pty.seen().contains("unknown command"),
        "the prefix did not complete; it dispatched as an unknown command:\n{}",
        pty.seen()
    );
    assert!(
        rig.api_requests().is_empty(),
        "/plan makes no model request"
    );
    rig.server.assert_clean();
}

/// An ambiguous prefix shows a dim candidate hint on the input row (both
/// commands listed), and Tab advances only to the common stem: it must never
/// pick one of them. `/s` matches `/status` and `/skills`, whose common stem is
/// `s` (already typed), so the hint stays and the token stays `/s`. Uses the
/// default dock driver and the screen emulator (colors stripped for the
/// assertion).
#[test]
fn ambiguous_prefix_shows_a_candidate_hint_and_tab_never_guesses() {
    const ROWS: u16 = 12;
    const COLS: u16 = 64;

    let rig = rig();
    let mut pty = spawn_pty_sized(&rig, &[], Some((ROWS, COLS)), &[]); // default dock
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);

    // Typing the ambiguous prefix: the input row lists both candidates.
    pty.send(b"/s");
    pty.drain(std::time::Duration::from_millis(400));
    let typed = pty.screen(ROWS, COLS);
    let typed_rows = typed.render();
    let row = input_row(&typed_rows)
        .unwrap_or_else(|| panic!("no input row after typing:\n{}", typed.dump("typed /s")));
    let plain = strip_ansi(row);
    assert!(
        plain.contains("/skills") && plain.contains("/status"),
        "the candidate hint did not list both commands: {plain:?}\n{}",
        typed.dump("typed /s")
    );

    // Tab advances only to the common stem `s` (already typed), so it neither
    // collapses to one command nor loses the hint.
    pty.send(&[0x09]);
    pty.drain(std::time::Duration::from_millis(400));
    let after = pty.screen(ROWS, COLS);
    let after_rows = after.render();
    let row = input_row(&after_rows)
        .unwrap_or_else(|| panic!("no input row after Tab:\n{}", after.dump("after tab")));
    let plain = strip_ansi(row);
    assert!(
        plain.contains("/skills") && plain.contains("/status"),
        "Tab wrongly collapsed the ambiguous prefix to one command: {plain:?}\n{}",
        after.dump("after tab")
    );

    pty.send(&[0x15]); // Ctrl-U clears the `/s` draft so Ctrl-D can exit
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    rig.server.assert_clean();
}

/// Regression guard: Tab on a non-slash line is inert. It inserts no literal
/// tab and completes nothing, so the exact typed line reaches the agent.
#[test]
fn tab_on_a_non_slash_line_is_inert() {
    let rig = rig();
    rig.server.enqueue_stream_completion("answered");

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"say");
    pty.send(&[0x09]); // Tab mid-line: must not insert a tab or complete
    pty.send(b" hi\r");
    pty.wait_for("answered");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(
        last_user(&reqs[0]),
        "say hi",
        "Tab altered a non-slash line"
    );
    assert!(
        !last_user(&reqs[0]).contains('\t'),
        "a literal tab leaked into the line"
    );
    rig.server.assert_clean();
}

/// Completion applies only to the command token, never its arguments. Once a
/// space is present, Tab is inert: `/skills st` + Tab submits verbatim (the
/// `/skills` subcommand handler then rejects `st`), rather than completing `st`
/// to `/status`.
#[test]
fn tab_does_not_complete_in_the_argument_region() {
    let rig = rig();

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"/skills st"); // a space has started the arguments
    pty.send(&[0x09]); // Tab in the argument region: inert
    pty.send(b"\r");
    // The line submitted as `/skills st`: the subcommand handler rejects `st`.
    // Had Tab completed the argument to `/status`, this notice would be absent.
    pty.wait_for("unknown /skills subcommand");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    assert!(
        rig.api_requests().is_empty(),
        "no command here makes a model request"
    );
    rig.server.assert_clean();
}
