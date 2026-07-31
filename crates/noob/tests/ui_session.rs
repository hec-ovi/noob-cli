//! Sessions through the UI: create, persist, resume (and its flag aliases),
//! replay on screen, list. Resume is display-only until the next turn, and a
//! resumed transcript extends append-only (the mock's prefix assertion sees
//! every break).

mod ui;

use serde_json::Value;

use ui::*;

/// The REPL persists its session and `--session <id>` resumes it: a second run
/// against the same id byte-extends the first run's transcript.
#[test]
fn repl_session_resume_extends_the_transcript() {
    let rig = rig();
    rig.server.enqueue_stream_completion("noted");
    let out1 = run_repl(&rig, &["--session", "reptest"], b"remember alpha\n/quit\n");
    assert!(out1.status.success(), "run 1 failed: {out1:?}");

    rig.server.enqueue_stream_completion("recalled");
    let out2 = run_repl(&rig, &["--restore", "reptest"], b"what did i say\n/quit\n");
    assert!(out2.status.success(), "run 2 failed: {out2:?}");

    // Run 2's request replays run 1's user message: the transcript resumed and
    // extended append-only (the mock's prefix assertion also saw no break).
    let reqs = rig.api_requests();
    let last = reqs.last().unwrap();
    let msgs = last["messages"].as_array().unwrap();
    assert!(
        msgs.iter()
            .any(|m| m["role"] == "user" && m["content"] == "remember alpha"),
        "resumed transcript missing the first turn: {msgs:?}"
    );
    rig.server.assert_clean();
}

#[test]
fn clear_plan_redacts_plan_payloads_from_resumed_context() {
    let rig = rig();
    let plan = r#"{"todos":[{"content":"LARGE-PLAN-PAYLOAD","status":"completed"}]}"#;
    rig.server
        .enqueue_stream_toolcalls(&[("p1", "plan", plan)], None);
    rig.server.enqueue_stream_completion("finished");
    let first = run_repl(&rig, &[], b"do it\n/clear-plan\n/quit\n");
    assert!(
        first.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&first.stderr)
    );

    let session_path = std::fs::read_dir(rig.config.path().join("sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let id = session_path
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let last_reset = std::fs::read_to_string(&session_path)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .rfind(|line| line["t"] == "reset")
        .expect("/clear-plan must persist a reset record");
    let reset_text = last_reset.to_string();
    assert!(!reset_text.contains("LARGE-PLAN-PAYLOAD"), "{reset_text}");
    assert!(
        reset_text.contains("plan cleared from context"),
        "{reset_text}"
    );

    rig.server.expect_prefix_break();
    rig.server.enqueue_stream_completion("no payload");
    let second = run_repl(&rig, &["--resume", &id], b"what remains\n/quit\n");
    assert!(second.status.success());
    let resumed = rig.api_requests().last().unwrap().to_string();
    assert!(!resumed.contains("LARGE-PLAN-PAYLOAD"), "{resumed}");
    assert!(resumed.contains("plan cleared from context"), "{resumed}");
    rig.server.assert_clean();
}

/// Resuming a saved session redraws the prior conversation on screen: the
/// earlier human line and the model's reply both appear (as plain, strip-ANSI
/// tokens) before the first new prompt, while a synthetic `[skills updated]`
/// item is filtered out. Display-only: no model request is made on resume.
#[test]
fn resume_redisplays_the_prior_conversation() {
    let rig = rig();
    write_session(
        rig.config.path(),
        "replayme",
        &[
            serde_json::json!({"role": "user", "text": "PRIORUSERLINE remember this"}),
            serde_json::json!({"role": "assistant", "text": "PRIORASSISTANTLINE understood.", "calls": [], "raw": []}),
            // Synthetic plumbing that must NOT be redisplayed.
            serde_json::json!({"role": "user", "text": "[skills updated] now available: HIDDENSKILL: nope."}),
            serde_json::json!({"role": "user", "text": "SECONDUSERLINE and this"}),
            serde_json::json!({"role": "assistant", "text": "SECONDASSISTANTLINE noted.", "calls": [], "raw": []}),
        ],
    );

    // Classic per-prompt editor so the replay lands before a plain RAW_READY.
    let mut pty = spawn_pty_sized(&rig, &[("NOOB_DOCK", "0")], None, &["--resume", "replayme"]);
    pty.wait_for("type a task");
    // The replay renders before the first prompt; wait for the last replayed
    // assistant line to be sure the whole transcript was drawn.
    pty.wait_for("SECONDASSISTANTLINE");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]); // Ctrl-D at the fresh prompt exits
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let plain = strip_ansi(&pty.seen());
    assert!(
        plain.contains("PRIORUSERLINE"),
        "prior user line not replayed:\n{plain}"
    );
    assert!(
        plain.contains("PRIORASSISTANTLINE"),
        "prior assistant line not replayed:\n{plain}"
    );
    assert!(
        plain.contains("SECONDUSERLINE"),
        "later user line not replayed:\n{plain}"
    );
    assert!(
        !plain.contains("HIDDENSKILL"),
        "a synthetic [skills updated] item leaked into the replay:\n{plain}"
    );
    // Replay is display-only: resuming fires no model request.
    assert!(
        rig.api_requests().is_empty(),
        "replay must not make a model request"
    );
    rig.server.assert_clean();
}

/// `--resume <bogus>` with no matching saved session prints a not-found notice
/// and still reaches a working prompt (it starts a fresh session).
#[test]
fn resume_of_a_missing_session_notes_and_starts_fresh() {
    let rig = rig();
    let mut pty = spawn_pty_sized(&rig, &[("NOOB_DOCK", "0")], None, &["--resume", "nosuchid"]);
    pty.wait_for("type a task");
    pty.wait_for("no saved session"); // the not-found notice
    pty.wait_for(RAW_READY); // still reaches a working prompt
    pty.send(&[0x04]); // Ctrl-D exits
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    assert!(
        pty.seen().contains("no saved session"),
        "the not-found notice never printed:\n{}",
        pty.seen()
    );
    rig.server.assert_clean();
}

/// `--resume` is a canonical alias for `--session`/`--restore`: a session
/// created with `--session` resumes and extends under `--resume`.
#[test]
fn resume_alias_extends_a_session_created_with_session() {
    let rig = rig();
    rig.server.enqueue_stream_completion("noted");
    let out1 = run_repl(
        &rig,
        &["--session", "aliastest"],
        b"remember gamma\n/quit\n",
    );
    assert!(out1.status.success(), "run 1 failed: {out1:?}");

    rig.server.enqueue_stream_completion("recalled");
    let out2 = run_repl(&rig, &["--resume", "aliastest"], b"what did i say\n/quit\n");
    assert!(out2.status.success(), "run 2 failed: {out2:?}");

    // Run 2 (under --resume) replayed run 1's user message into the request:
    // the alias resumed the same transcript --session created.
    let reqs = rig.api_requests();
    let last = reqs.last().unwrap();
    let msgs = last["messages"].as_array().unwrap();
    assert!(
        msgs.iter()
            .any(|m| m["role"] == "user" && m["content"] == "remember gamma"),
        "--resume did not resume the --session transcript: {msgs:?}"
    );
    rig.server.assert_clean();
}

#[test]
fn sessions_command_lists_newest_and_resume_latest_replays_it() {
    let rig = rig();
    write_session(
        rig.config.path(),
        "older-session",
        &[serde_json::json!({"role":"user","text":"OLDER-MARKER"})],
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    write_session(
        rig.config.path(),
        "newer-session",
        &[serde_json::json!({"role":"user","text":"NEWER-MARKER"})],
    );

    let listed = noob(rig.config.path(), rig.work.path())
        .arg("sessions")
        .output()
        .unwrap();
    assert!(listed.status.success());
    let stdout = String::from_utf8_lossy(&listed.stdout);
    let mut lines = stdout.lines();
    assert!(
        lines.next().unwrap().starts_with("newer-session (latest)"),
        "{stdout}"
    );
    assert!(
        lines.next().unwrap().starts_with("older-session"),
        "{stdout}"
    );

    rig.server.enqueue_stream_completion("LATEST-RESUMED");
    let resumed = run_repl(&rig, &["--resume", "latest"], b"continue latest\n/quit\n");
    assert!(
        resumed.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let messages = rig.api_requests().last().unwrap()["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert!(
        messages
            .iter()
            .any(|message| message["content"] == "NEWER-MARKER")
    );
    assert!(
        !messages
            .iter()
            .any(|message| message["content"] == "OLDER-MARKER")
    );
    rig.server.assert_clean();
}

#[test]
fn sessions_command_lists_more_than_twenty_sessions() {
    let rig = rig();
    for index in 0..25 {
        write_session(
            rig.config.path(),
            &format!("session-{index:02}"),
            &[serde_json::json!({"role":"user","text":format!("marker-{index}")})],
        );
    }

    let listed = noob(rig.config.path(), rig.work.path())
        .arg("sessions")
        .output()
        .unwrap();
    assert!(listed.status.success());
    let stdout = String::from_utf8_lossy(&listed.stdout);
    assert_eq!(stdout.lines().count(), 25, "{stdout}");
    for index in 0..25 {
        assert!(stdout.contains(&format!("session-{index:02}")), "{stdout}");
    }
}
