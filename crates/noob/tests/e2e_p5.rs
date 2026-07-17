//! P5 e2e: plan mode through the compiled binary. The gating is structural
//! (mutating schemas are absent from the request, so they cannot tempt the
//! model), the dispatcher refuses hallucinated mutations as defense in
//! depth, and the /go approval restores the full set in the same session.

use std::process::Command;

use noob_testkit::MockServer;
use serde_json::Value;

fn write_env(dir: &std::path::Path, base_url: &str) {
    std::fs::write(
        dir.join(".env"),
        format!("NOOB_BASE_URL={base_url}\nNOOB_MODEL=mockmodel\n"),
    )
    .unwrap();
}

fn noob(config_dir: &std::path::Path, workspace: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noob"));
    cmd.env("NOOB_CONFIG_DIR", config_dir)
        .current_dir(workspace)
        // This suite validates plan semantics. Dock interaction and command
        // queueing have dedicated PTY coverage in e2e_ui.
        .env("NOOB_DOCK", "0")
        .env_remove("NOOB_BASE_URL")
        .env_remove("NOOB_MODEL")
        .env_remove("NOOB_API_STYLE")
        .env_remove("NOOB_CTX")
        .env_remove("NOOB_SANDBOX");
    cmd
}

struct Rig {
    server: MockServer,
    config: tempfile::TempDir,
    work: tempfile::TempDir,
}

fn rig() -> Rig {
    let server = MockServer::start();
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    write_env(config.path(), &server.base_url());
    Rig {
        server,
        config,
        work,
    }
}

impl Rig {
    fn run(&self, args: &[&str]) -> std::process::Output {
        noob(self.config.path(), self.work.path())
            .args(args)
            .output()
            .unwrap()
    }

    fn api_requests(&self) -> Vec<Value> {
        self.server
            .recorded()
            .iter()
            .filter(|r| r.path.ends_with("/chat/completions"))
            .map(|r| r.json().unwrap())
            .collect()
    }
}

fn ok(out: &std::process::Output) -> String {
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn tool_names(req: &Value) -> Vec<String> {
    req["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap().to_string())
        .collect()
}

/// plan_gate: in `exec --plan` the request carries only the read-only
/// schemas plus the injected mode message; a mutating call the model
/// hallucinates anyway is refused by the dispatcher and nothing lands on
/// disk; the plan prints and the process exits 0.
#[test]
fn plan_gate() {
    let rig = rig();
    // A skill exists, so the plan set includes the skill tool (5 entries).
    let dir = rig.work.path().join(".claude/skills/notes");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: notes\ndescription: note taking rules\n---\nbody\n",
    )
    .unwrap();
    // The model tries to write during planning, then presents its plan.
    rig.server.enqueue_stream_toolcalls(
        &[(
            "p1",
            "write",
            r#"{"path":"sneak.txt","content":"early write"}"#,
        )],
        None,
    );
    rig.server
        .enqueue_stream_completion("1. read the code\n2. change it\n3. verify");

    let out = rig.run(&["exec", "--plan", "-p", "plan the feature"]);
    let stdout = ok(&out);
    assert!(
        stdout.contains("1. read the code"),
        "plan text missing: {stdout}"
    );

    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2);
    // Structural gating: exactly the read-only set, both requests.
    for req in &reqs {
        assert_eq!(
            tool_names(req),
            ["read", "grep", "glob", "ls", "context", "skill"],
            "plan mode must send only read-only schemas"
        );
    }
    // The injected mode message precedes the user prompt.
    let msgs = reqs[0]["messages"].as_array().unwrap();
    assert_eq!(
        msgs[1]["content"],
        "[plan mode] Read-only: write, edit, and bash are disabled until the user \
         approves with /go. Explore with the read-only tools, then present a \
         numbered implementation plan as plain text. If the request asks for a \
         change, plan it instead of attempting it."
    );
    assert_eq!(msgs[2]["content"], "plan the feature");
    // Defense in depth: the write was refused, not executed.
    assert!(!rig.work.path().join("sneak.txt").exists());
    let refusal = reqs[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(refusal.contains("plan mode is read-only"), "{refusal}");
    rig.server.assert_clean();
}

/// The review-then-approve flow across processes (the telegram-bot shape):
/// run 1 plans in a session; run 2 resumes it without --plan and the full
/// tool set is back, byte-extending the same transcript (one sanctioned
/// tools change, zero message-prefix breaks).
#[test]
fn plan_session_resume_restores_full_tools() {
    let rig = rig();
    rig.server
        .enqueue_stream_completion("1. create result.txt with the answer");
    ok(&rig.run(&["exec", "--plan", "--session", "plan-s1", "-p", "plan it"]));

    // Run 2: same session, no --plan; the model executes the plan.
    rig.server.expect_tools_change();
    rig.server.enqueue_stream_toolcalls(
        &[(
            "g1",
            "write",
            r#"{"path":"result.txt","content":"the answer"}"#,
        )],
        None,
    );
    rig.server.enqueue_stream_completion("done");
    ok(&rig.run(&["exec", "--session", "plan-s1", "-p", "go"]));

    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 3);
    assert_eq!(
        tool_names(&reqs[0]),
        ["read", "grep", "glob", "ls", "context"]
    );
    assert_eq!(tool_names(&reqs[1]).len(), 10, "full set after resume");
    // The resumed request byte-extends the plan-mode transcript (the mock's
    // automatic prefix assertion saw no unsanctioned break), and the write
    // actually executed this time.
    assert_eq!(
        std::fs::read_to_string(rig.work.path().join("result.txt")).unwrap(),
        "the answer"
    );
    rig.server.assert_clean();
}

/// The interactive surface: /plan mid-session shrinks the tool set, /go
/// restores it and appends the approval message, all in one process.
#[test]
fn repl_plan_then_go_flow() {
    use std::io::{Read, Write};
    use std::os::fd::FromRawFd;

    let rig = rig();
    // Turn 1 (normal), turn 2 (planning, read-only), turn 3 (after /go).
    rig.server.enqueue_stream_completion("hello");
    rig.server
        .enqueue_stream_completion("1. write greeting.txt");
    rig.server.enqueue_stream_toolcalls(
        &[(
            "w1",
            "write",
            r#"{"path":"greeting.txt","content":"hi there"}"#,
        )],
        None,
    );
    rig.server.enqueue_stream_completion("plan executed");
    rig.server.expect_tools_change(); // /plan entry
    rig.server.expect_tools_change(); // /go exit

    let (mut master, slave) = unsafe {
        let mut m: libc::c_int = 0;
        let mut s: libc::c_int = 0;
        assert_eq!(
            libc::openpty(
                &mut m,
                &mut s,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null()
            ),
            0,
            "openpty failed"
        );
        (std::fs::File::from_raw_fd(m), s)
    };
    let stdio = |fd: i32| unsafe { std::process::Stdio::from_raw_fd(libc::dup(fd)) };
    let mut child = noob(rig.config.path(), rig.work.path())
        .stdin(stdio(slave))
        .stdout(stdio(slave))
        .stderr(stdio(slave))
        .spawn()
        .unwrap();
    unsafe { libc::close(slave) };

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let child_pid = child.id() as i32;
    let done = Arc::new(AtomicBool::new(false));
    let wd_done = done.clone();
    let watchdog = std::thread::spawn(move || {
        for _ in 0..200 {
            if wd_done.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        unsafe { libc::kill(child_pid, libc::SIGKILL) };
    });

    let mut seen = String::new();
    let wait_for = |master: &mut std::fs::File, marker: &str, seen: &mut String| {
        let mut buf = [0u8; 4096];
        while !seen.contains(marker) {
            match master.read(&mut buf) {
                Ok(0) => panic!("pty closed before {marker:?}; saw:\n{seen}"),
                Ok(n) => seen.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(e) => panic!("pty read error: {e}; saw:\n{seen}"),
            }
        }
    };
    let wait_for_after =
        |master: &mut std::fs::File, marker: &str, offset: usize, seen: &mut String| {
            let mut buf = [0u8; 4096];
            while !seen[offset..].contains(marker) {
                match master.read(&mut buf) {
                    Ok(0) => panic!("pty closed before {marker:?}; saw:\n{seen}"),
                    Ok(n) => seen.push_str(&String::from_utf8_lossy(&buf[..n])),
                    Err(e) => panic!("pty read error: {e}; saw:\n{seen}"),
                }
            }
        };
    let prompt_ready = "\x1b[?2004h";
    wait_for(&mut master, "type a task", &mut seen);
    wait_for(&mut master, prompt_ready, &mut seen);
    let turn_start = seen.len();
    master.write_all(b"say hello\n").unwrap();
    // Wait for the next editor, not the word "hello": that word is also in
    // the submitted prompt and would race the terminal's raw-mode transition.
    wait_for_after(&mut master, prompt_ready, turn_start, &mut seen);
    assert!(seen[turn_start..].contains("hello"));
    let plan_start = seen.len();
    master.write_all(b"/plan\n").unwrap();
    wait_for_after(&mut master, "plan mode", plan_start, &mut seen);
    wait_for_after(&mut master, prompt_ready, plan_start, &mut seen);
    let planning_turn = seen.len();
    master.write_all(b"plan writing a greeting\n").unwrap();
    wait_for_after(&mut master, prompt_ready, planning_turn, &mut seen);
    assert!(
        seen[planning_turn..].contains("write greeting.txt"),
        "planning turn ended without the expected response: {}",
        &seen[planning_turn..]
    );
    let execution_turn = seen.len();
    master.write_all(b"/go\n").unwrap();
    wait_for_after(&mut master, "plan executed", execution_turn, &mut seen);
    wait_for_after(&mut master, prompt_ready, execution_turn, &mut seen);
    master.write_all(b"/quit\n").unwrap();
    let status = child.wait().unwrap();
    done.store(true, Ordering::SeqCst);
    watchdog.join().ok();
    assert!(status.success(), "repl exit: {status:?};\n{seen}");

    assert_eq!(
        std::fs::read_to_string(rig.work.path().join("greeting.txt")).unwrap(),
        "hi there"
    );
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 4);
    assert_eq!(tool_names(&reqs[0]).len(), 10, "normal turn: full set");
    assert_eq!(
        tool_names(&reqs[1]),
        ["read", "grep", "glob", "ls", "context"]
    );
    assert_eq!(tool_names(&reqs[2]).len(), 10, "after /go: full set");
    // The approval message /go appends is the last user message before the
    // execution turn.
    let msgs = reqs[2]["messages"].as_array().unwrap();
    let last_user = msgs.iter().rev().find(|m| m["role"] == "user").unwrap();
    assert_eq!(last_user["content"], "Plan approved. Execute it.");
    // And the transcript stayed append-only through both mode changes: any
    // message-prefix break would have tripped the mock (none sanctioned).
    rig.server.assert_clean();
}

/// /go without a plan and /plan twice degrade to notes, never to state
/// corruption; `exec --plan` without a session still exits 0 after the plan.
#[test]
fn plan_mode_edge_commands() {
    let rig = rig();
    rig.server.enqueue_stream_completion("just a plan");
    let out = rig.run(&["exec", "--plan", "-p", "quick plan"]);
    assert!(out.status.success());
    // Only the read-only set went out; exit code 0 (the plan is the product).
    let reqs = rig.api_requests();
    assert_eq!(
        tool_names(&reqs[0]),
        ["read", "grep", "glob", "ls", "context"]
    );
    rig.server.assert_clean();
}
