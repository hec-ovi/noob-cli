//! Slash commands through the compiled binary: /config, /context, and the
//! mid-session /skills and /mcp surfaces, including what each announces to
//! the model in-band and which commands must never cost a model round-trip.

mod ui;

use ui::*;

#[test]
fn config_command_updates_non_secret_env_without_a_model_request() {
    let rig = rig();
    let out = run_repl(
        &rig,
        &[],
        b"/config set ctx 65536\n/config set task-concurrency 8\n/quit\n",
    );
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let env = std::fs::read_to_string(rig.config.path().join(".env")).unwrap();
    assert!(
        env.contains("NOOB_BASE_URL="),
        "provider config was lost: {env}"
    );
    assert!(
        env.contains("NOOB_MODEL=mockmodel"),
        "model config was lost: {env}"
    );
    assert!(
        env.contains("NOOB_CTX=65536"),
        "context setting missing: {env}"
    );
    assert!(
        env.contains("NOOB_TASK_CONCURRENCY=8"),
        "task setting missing: {env}"
    );
    assert!(
        rig.api_requests().is_empty(),
        "/config must not invoke the model"
    );
}

#[test]
fn unsetting_base_url_explains_that_autodetect_runs_after_restart() {
    let rig = rig();
    let out = run_repl(&rig, &[], b"/config unset base-url\n/quit\n");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("restart noob to run localhost autodetect"),
        "{stdout}"
    );
    let env = std::fs::read_to_string(rig.config.path().join(".env")).unwrap();
    assert!(!env.contains("NOOB_BASE_URL="), "{env}");
    assert!(rig.api_requests().is_empty());
}

/// `/context` answers from the same estimate the model-callable context tool
/// reports, without any model round-trip.
#[test]
fn context_command_reports_usage_without_a_model_call() {
    let rig = rig();
    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"/context\r");
    pty.wait_for("context: ~");
    pty.wait_for("automatic compaction starts near");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();
    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    assert!(
        rig.api_requests().is_empty(),
        "/context must not call the model"
    );
}

/// M8 (skills on the fly): a session that started with NO skills installs one
/// with `/skills add` and immediately uses it. The `skill` tool must be
/// registered mid-session (absent at bootstrap), and the skill body must load.
#[test]
fn skills_add_registers_the_tool_and_the_skill_loads() {
    let rig = rig();
    // A source skill outside every discovery path, so it is not present until
    // it is installed.
    write_skill_md(
        &rig.work.path().join("src-demo"),
        "demo",
        "demo skill for the test",
        "STEP-ONE: do the demo thing.",
    );
    // The "use demo" turn: the model loads the skill, then answers.
    rig.server
        .enqueue_stream_toolcalls(&[("c1", "skill", r#"{"name":"demo"}"#)], None);
    rig.server.enqueue_stream_completion("used the demo skill");

    let mut pty = spawn_pty(&rig); // classic REPL: per-prompt RAW_READY sync
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"/skills add src-demo\r");
    pty.wait_for("installed skill demo");
    pty.wait_for(RAW_READY); // back at the prompt, skill now registered
    pty.send(b"use demo\r");
    pty.wait_for("used the demo skill");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let reqs = rig.api_requests();
    assert_eq!(
        reqs.len(),
        2,
        "the tool-call round and the completion round"
    );
    // The skill tool was registered mid-session: the first request already
    // carries it, though the session booted with no skills.
    let tools = reqs[0]["tools"].as_array().expect("tools array");
    assert!(
        tools.iter().any(|t| t["function"]["name"] == "skill"),
        "the skill tool must be registered after /skills add"
    );
    // The in-band announcement reached the model, and the skill body loaded.
    assert!(
        all_content(&reqs[0]).contains("[skills updated]"),
        "missing the in-band note"
    );
    assert!(
        all_content(&reqs[1]).contains("STEP-ONE"),
        "the skill body did not load"
    );
    rig.server.assert_clean();
}

/// M8: removing a skill mid-session announces it and the `skill` tool then
/// rejects loading it (the staleness backstop: the frozen prompt-head index
/// still lists it, but the in-band note and the tool's own check correct that).
#[test]
fn skills_remove_announces_and_the_tool_rejects_the_gone_skill() {
    let rig = rig();
    // Boot WITH the skill installed (a discovery path), so the tool exists.
    write_skill_md(
        &rig.work.path().join(".noob/skills/demo"),
        "demo",
        "demo skill for the test",
        "STEP-ONE: do the demo thing.",
    );
    // After removal the model still tries to load it (the head is stale); the
    // tool must reject, and the model then answers.
    rig.server
        .enqueue_stream_toolcalls(&[("c1", "skill", r#"{"name":"demo"}"#)], None);
    rig.server.enqueue_stream_completion("the skill is gone");

    let mut pty = spawn_pty(&rig);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"/skills remove demo\r");
    pty.wait_for("removed demo");
    pty.wait_for(RAW_READY);
    pty.send(b"use demo\r");
    pty.wait_for("the skill is gone");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2);
    // The removal was announced in-band so the model's working set is corrected.
    assert!(
        all_content(&reqs[0]).contains("no longer available"),
        "the removal must be announced to the model"
    );
    // The tool structurally rejected the gone skill (the hard backstop).
    assert!(
        all_content(&reqs[1]).contains("unknown skill"),
        "the skill tool must reject a removed skill"
    );
    assert!(
        !rig.work.path().join(".noob/skills/demo").exists(),
        "the skill dir must be gone"
    );
    rig.server.assert_clean();
}

#[test]
fn dock_canceled_skill_clone_restores_queued_input() {
    use std::os::unix::fs::PermissionsExt;

    let rig = rig();
    let bin = rig.work.path().join("fake-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let git = bin.join("git");
    std::fs::write(&git, "#!/bin/sh\nexec /bin/sleep 30\n").unwrap();
    std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = bin.to_string_lossy().into_owned();
    let envs = [("NOOB_DOCK", "1"), ("PATH", path.as_str())];

    let mut pty = spawn_pty_with(&rig, &envs);
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"/skills add https://example.invalid/demo.git\r");
    pty.wait_for("Working");
    pty.send(b"keep skill draft");
    pty.wait_for("keep skill draft");
    pty.send(&[0x03]);
    pty.wait_for("skill installation canceled by user");
    pty.wait_for("keep skill draft");
    pty.send(&[0x15]);
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    assert!(
        rig.api_requests().is_empty(),
        "the restored draft must not auto-run"
    );
}

/// `/mcp add` installs a server on the fly: the entry persists to the project
/// mcp.json, the two MCP tools register mid-session (absent at bootstrap), the
/// in-band `[mcp updated]` note reaches the model, `/mcp connect` lists the
/// catalog for the human, and the model can immediately mcp_call. `/mcp
/// remove` then drops it and announces the removal.
#[test]
fn mcp_add_registers_the_tools_connects_and_removes() {
    let rig = rig();
    let mcp_server = noob_testkit::mcp::McpHttpServer::start(noob_testkit::mcp::echo_tools());

    // The "use echo" turn: the model calls the freshly added server, then answers.
    rig.server.enqueue_stream_toolcalls(
        &[(
            "m1",
            "mcp_call",
            r#"{"server":"mock","tool":"echo","args":{"text":"hola"}}"#,
        )],
        None,
    );
    rig.server.enqueue_stream_completion("echo went through");

    let mut pty = spawn_pty(&rig); // classic REPL: per-prompt RAW_READY sync
    pty.wait_for("type a task");
    pty.wait_for(RAW_READY);
    pty.send(b"/mcp\r");
    pty.wait_for("no MCP servers configured");
    pty.wait_for(RAW_READY);
    pty.send(format!("/mcp add mock {}\r", mcp_server.url()).as_bytes());
    pty.wait_for("MCP tools available: server tools registered");
    pty.wait_for("mcp: added mock");
    pty.wait_for(RAW_READY);
    pty.send(b"/mcp connect mock\r");
    pty.wait_for("connected mock");
    pty.wait_for("1 tools: echo");
    pty.wait_for(RAW_READY);
    pty.send(b"use echo\r");
    pty.wait_for("echo went through");
    pty.wait_for(RAW_READY);
    pty.send(b"/mcp remove mock\r");
    pty.wait_for("mcp: removed mock");
    pty.wait_for(RAW_READY);
    pty.send(&[0x04]);
    pty.wait_for("resume with");
    let status = pty.finish();

    assert!(status.success(), "repl exit: {status:?};\n{}", pty.seen());
    // The added server persisted to the project file, then remove dropped it.
    let cfg = std::fs::read_to_string(rig.work.path().join(".noob/mcp.json")).unwrap();
    assert!(
        !cfg.contains("mock"),
        "remove must drop the entry from .noob/mcp.json: {cfg}"
    );
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2, "the mcp_call round and the completion round");
    // The MCP tools were registered mid-session: the first request carries
    // them although the session booted with no mcp.json.
    let tools = reqs[0]["tools"].as_array().expect("tools array");
    for name in ["mcp_connect", "mcp_call"] {
        assert!(
            tools.iter().any(|t| t["function"]["name"] == name),
            "{name} must be registered after /mcp add"
        );
    }
    assert!(
        all_content(&reqs[0]).contains("[mcp updated]"),
        "missing the in-band note"
    );
    // The tool result the model saw carries the echoed payload.
    assert!(
        all_content(&reqs[1]).contains("hola"),
        "the mcp_call result did not reach the model"
    );
    mcp_server.assert_clean();
    rig.server.assert_clean();
}
