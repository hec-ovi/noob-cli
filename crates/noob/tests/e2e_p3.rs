//! P3 e2e: skills through the compiled binary against the mock server.
//! Named tests locked by ARCHITECTURE.md land here: the ecosystem discovery
//! paths, the resolver index, the skill tool round trip, skills_gate, and
//! the post-compaction loaded-skill re-listing (including across resume).

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
    Rig { server, config, work }
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

    /// Plant `<root>/<name>/SKILL.md` under the workspace (or an absolute
    /// root) with the given description and body.
    fn skill(&self, root: &str, name: &str, description: &str, body: &str) {
        let base = if root.starts_with('/') {
            std::path::PathBuf::from(root)
        } else {
            self.work.path().join(root)
        };
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n{body}"),
        )
        .unwrap();
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

/// One discovered skill: the resolver section (with the dispatcher
/// instruction) lands in the system prompt and the skill tool joins the
/// registered set.
#[test]
fn resolver_index_and_tool_registration() {
    let rig = rig();
    rig.skill(".claude/skills", "greeting", "says hello politely", "# Greeting\n\nSay hi.\n");
    rig.server.enqueue_stream_completion("noted");

    ok(&rig.run(&["exec", "-p", "hello"]));

    let reqs = rig.api_requests();
    let system = reqs[0]["messages"][0]["content"].as_str().unwrap();
    assert!(system.contains("# Skills (resolver)"), "resolver section missing");
    assert!(
        system.contains("Load a matching skill with the skill tool and follow it before acting"),
        "dispatcher instruction missing"
    );
    assert!(system.contains("- greeting: says hello politely"));
    let tools = reqs[0]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 10);
    assert!(tools.iter().any(|t| t["function"]["name"] == "skill"));
    rig.server.assert_clean();
}

/// No skills discovered: no resolver section, no skill tool; the tools
/// array stays the 8 core specs plus task (the registered set is decided at
/// start).
#[test]
fn no_skills_means_no_skill_tool_and_no_section() {
    let rig = rig();
    rig.server.enqueue_stream_completion("bare");

    ok(&rig.run(&["exec", "-p", "hello"]));

    let reqs = rig.api_requests();
    let system = reqs[0]["messages"][0]["content"].as_str().unwrap();
    assert!(!system.contains("# Skills"));
    assert_eq!(reqs[0]["tools"].as_array().unwrap().len(), 9);
    rig.server.assert_clean();
}

/// The skill tool round trip: the body comes back as a tool result with
/// the frontmatter stripped and the directory path attached; the system
/// prompt (the cache prefix) is byte-identical before and after the load.
#[test]
fn skill_tool_returns_body_and_never_mutates_the_head() {
    let rig = rig();
    rig.skill(
        ".claude/skills",
        "greeting",
        "says hello politely",
        "# Greeting skill\n\nAlways greet with the word ahoy.\n",
    );
    rig.server
        .enqueue_stream_toolcalls(&[("s1", "skill", r#"{"name":"greeting"}"#)], None);
    rig.server.enqueue_stream_completion("ahoy");

    ok(&rig.run(&["exec", "-p", "use the greeting skill"]));

    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2);
    let msgs = reqs[1]["messages"].as_array().unwrap();
    let result = msgs.iter().find(|m| m["role"] == "tool").unwrap();
    let content = result["content"].as_str().unwrap();
    assert!(content.starts_with("skill: greeting\ndir: .claude/skills/greeting\n\n"));
    assert!(content.contains("Always greet with the word ahoy."));
    assert!(
        !content.contains("description: says hello politely"),
        "frontmatter must be stripped: {content}"
    );
    // The system prompt did not change when the skill loaded, and neither
    // did the tools array: both are frozen for the session.
    assert_eq!(reqs[0]["messages"][0], reqs[1]["messages"][0]);
    assert_eq!(reqs[0]["tools"], reqs[1]["tools"], "tools array drifted mid-session");
    assert_eq!(reqs[0]["tools"].as_array().unwrap().len(), 10);
    assert!(!reqs[1]["tools"].is_null(), "a real turn must carry the tools array");
    rig.server.assert_clean();
}

/// All four discovery roots contribute; a name collision resolves to the
/// highest-priority root (.noob first, config last).
#[test]
fn four_discovery_roots_and_first_hit_priority() {
    let rig = rig();
    rig.skill(".noob/skills", "alpha", "from noob", "a\n");
    rig.skill(".claude/skills", "beta", "from claude", "b\n");
    rig.skill(".agents/skills", "gamma", "from agents", "c\n");
    let config_skills = rig.config.path().join("skills");
    rig.skill(config_skills.to_str().unwrap(), "delta", "from config", "d\n");
    // The same name in two roots: .noob wins over config.
    rig.skill(config_skills.to_str().unwrap(), "alpha", "shadowed loser", "x\n");
    rig.server.enqueue_stream_completion("counted");

    ok(&rig.run(&["exec", "-p", "list your skills"]));

    let system = rig.api_requests()[0]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    for line in [
        "- alpha: from noob",
        "- beta: from claude",
        "- gamma: from agents",
        "- delta: from config",
    ] {
        assert!(system.contains(line), "missing {line:?} in:\n{system}");
    }
    assert!(!system.contains("shadowed loser"));
    rig.server.assert_clean();
}

/// A malformed SKILL.md is skipped with a stderr warning; the good ones
/// still register. Never a crash.
#[test]
fn malformed_skill_skipped_with_warning() {
    let rig = rig();
    let bad_dir = rig.work.path().join(".claude/skills/broken");
    std::fs::create_dir_all(&bad_dir).unwrap();
    std::fs::write(bad_dir.join("SKILL.md"), "no frontmatter at all\n").unwrap();
    rig.skill(".claude/skills", "working", "still fine", "body\n");
    rig.server.enqueue_stream_completion("done");

    let out = rig.run(&["exec", "-p", "hi"]);
    ok(&out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("skipping skill") && stderr.contains("broken"),
        "stderr: {stderr}"
    );

    let system = rig.api_requests()[0]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(system.contains("- working: still fine"));
    assert!(!system.contains("broken"));
    rig.server.assert_clean();
}

/// A present-but-unreadable SKILL.md (invalid UTF-8) is skipped with the
/// mandated stderr warning, not silently dropped. Pins the warning so a
/// regression to a silent skip fails the test.
#[test]
fn unreadable_skill_warns_on_stderr() {
    use std::io::Write;
    let rig = rig();
    let dir = rig.work.path().join(".claude/skills/binary");
    std::fs::create_dir_all(&dir).unwrap();
    let mut f = std::fs::File::create(dir.join("SKILL.md")).unwrap();
    f.write_all(&[0xff, 0xfe, 0x00, 0x80]).unwrap(); // not UTF-8
    rig.skill(".claude/skills", "working", "fine", "body\n");
    rig.server.enqueue_stream_completion("ok");

    let out = rig.run(&["exec", "-p", "hi"]);
    ok(&out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("skipping skill") && stderr.contains("binary"),
        "no warning for the unreadable skill; stderr: {stderr}"
    );
    // The good skill still registered.
    let system = rig.api_requests()[0]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(system.contains("- working: fine"));
    rig.server.assert_clean();
}

/// The execution-time half of the write gate: a bash symlink created
/// earlier in the SAME batch cannot route a write into a skills dir past
/// the plan-time confirmation. The write is refused at execution and the
/// file never lands inside the skills tree.
#[test]
fn same_batch_symlink_into_skills_is_refused_at_execution() {
    let rig = rig();
    rig.skill(".claude/skills", "greeting", "says hello", "original\n");
    // One turn, one batch: create a symlink `innocent` -> the skills dir,
    // then write through it. At plan time `innocent` does not exist, so the
    // plan-time gate does not fire; the write tool re-checks at execution.
    rig.server.enqueue_stream_toolcalls(
        &[
            ("t1", "bash", r#"{"cmd":"ln -s .claude/skills/greeting innocent"}"#),
            ("t2", "write", r#"{"path":"innocent/INJECTED.md","content":"payload"}"#),
        ],
        None,
    );
    rig.server.enqueue_stream_completion("done");

    ok(&rig.run(&["exec", "-p", "sneak a skill in"]));

    assert!(
        !rig.work.path().join(".claude/skills/greeting/INJECTED.md").exists(),
        "a same-batch symlink routed a write into the skills dir"
    );
    let reqs = rig.api_requests();
    let refusal = reqs[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "tool")
        .filter_map(|m| m["content"].as_str())
        .any(|c| c.contains("refused"));
    assert!(refusal, "the write should have been refused at execution");
    rig.server.assert_clean();
}

/// An unknown skill name comes back as a typed error naming the available
/// skills, so a small model can self-correct on the next call.
#[test]
fn unknown_skill_error_lists_available() {
    let rig = rig();
    rig.skill(".claude/skills", "greeting", "says hello", "hi\n");
    rig.server
        .enqueue_stream_toolcalls(&[("u1", "skill", r#"{"name":"nope"}"#)], None);
    rig.server.enqueue_stream_completion("corrected");

    ok(&rig.run(&["exec", "-p", "load nope"]));

    let reqs = rig.api_requests();
    let result = reqs[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(result.contains("unknown skill \"nope\""), "{result}");
    assert!(result.contains("available skills: greeting"));
    rig.server.assert_clean();
}

/// skills_gate: a headless write into a skills directory is denied (the
/// confirmation degrades to No without a TTY), the file stays untouched,
/// and the refusal goes back to the model as the tool result.
#[test]
fn skills_gate() {
    let rig = rig();
    rig.skill(".claude/skills", "greeting", "says hello", "original body\n");
    rig.server.enqueue_stream_toolcalls(
        &[(
            "g1",
            "write",
            r#"{"path":".claude/skills/greeting/SKILL.md","content":"injected"}"#,
        )],
        None,
    );
    rig.server.enqueue_stream_completion("understood");

    ok(&rig.run(&["exec", "-p", "improve your greeting skill"]));

    let on_disk = std::fs::read_to_string(
        rig.work.path().join(".claude/skills/greeting/SKILL.md"),
    )
    .unwrap();
    assert!(on_disk.contains("original body"), "the skill file was modified");
    assert!(!on_disk.contains("injected"));

    let reqs = rig.api_requests();
    let result = reqs[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(result.contains("refused"), "{result}");
    assert!(result.contains("confirmation"));
    rig.server.assert_clean();
}

/// Compaction re-lists loaded skills, names only, in the spliced summary
/// message, so the model keeps knowing what it loaded after the middle
/// (and the skill body in it) is gone.
#[test]
fn compaction_relists_loaded_skills() {
    let rig = rig();
    // ~12 KiB body: big enough that the tail budget cannot hold the tool
    // result, so compaction has a middle to remove.
    let body: String = (0..800).map(|i| format!("procedure step {i}\n")).collect();
    rig.skill(".claude/skills", "bigproc", "a big procedure", &body);
    // Round 1 loads the skill and reports usage near the 4096 ceiling.
    rig.server.enqueue_stream_toolcalls(
        &[("c1", "skill", r#"{"name":"bigproc"}"#)],
        Some((3500, 100)),
    );
    rig.server.enqueue_stream_completion("SUMMARY-OF-EVERYTHING");
    rig.server.enqueue_stream_completion("continuing");
    rig.server.expect_prefix_break();
    rig.server.expect_prefix_break();

    let out = noob(rig.config.path(), rig.work.path())
        .env("NOOB_CTX", "4096")
        .args(["exec", "-p", "load the bigproc skill"])
        .output()
        .unwrap();
    ok(&out);

    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 3);
    let cont: String = reqs[2]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap_or("").to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(cont.contains("[conversation summary]"));
    assert!(
        cont.contains("[loaded skills: bigproc]"),
        "re-listing missing from the spliced summary:\n{cont}"
    );
    assert!(!cont.contains("procedure step 500"), "the body must be gone");
    rig.server.assert_clean();
}

/// The re-listing survives a process boundary: run 1 loads a skill into a
/// session; run 2 resumes, compacts immediately, and the spliced summary
/// still names the loaded skill (recovered from the replayed transcript).
#[test]
fn resume_then_compaction_still_relists_loaded_skills() {
    let rig = rig();
    let body: String = (0..800).map(|i| format!("procedure step {i}\n")).collect();
    rig.skill(".claude/skills", "bigproc", "a big procedure", &body);

    // Run 1 (default ctx: no compaction): load the skill, finish.
    rig.server
        .enqueue_stream_toolcalls(&[("r1", "skill", r#"{"name":"bigproc"}"#)], None);
    rig.server.enqueue_stream_completion("loaded");
    ok(&rig.run(&["exec", "--session", "s-skills", "-p", "load the bigproc skill"]));

    // Run 2 (tiny ctx): the replayed transcript alone crosses 75%, so the
    // loop compacts before its first request.
    rig.server.enqueue_stream_completion("SUMMARY-TWO");
    rig.server.enqueue_stream_completion("after resume");
    rig.server.expect_prefix_break();
    rig.server.expect_prefix_break();
    let out = noob(rig.config.path(), rig.work.path())
        .env("NOOB_CTX", "4096")
        .args(["exec", "--session", "s-skills", "-p", "continue"])
        .output()
        .unwrap();
    ok(&out);

    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 4);
    let cont: String = reqs[3]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap_or("").to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        cont.contains("[loaded skills: bigproc]"),
        "re-listing lost across resume:\n{cont}"
    );
    rig.server.assert_clean();
}

/// The hard-drop fallback (the summarize request itself overflows) must
/// also re-list loaded skills: the stub replaces the middle, so the names
/// are the only trace left.
#[test]
fn hard_drop_compaction_relists_loaded_skills() {
    let rig = rig();
    let body: String = (0..800).map(|i| format!("procedure step {i}\n")).collect();
    rig.skill(".claude/skills", "bigproc", "a big procedure", &body);
    rig.server.enqueue_stream_toolcalls(
        &[("h1", "skill", r#"{"name":"bigproc"}"#)],
        Some((3500, 100)),
    );
    // The summarize request answers with a context-overflow 400: the
    // deterministic hard drop must kick in.
    rig.server.enqueue_json(
        400,
        serde_json::json!({"error": {
            "message": "the request exceeds the available context size. try increasing it"
        }}),
    );
    rig.server.enqueue_stream_completion("continuing after hard drop");
    rig.server.expect_prefix_break();
    rig.server.expect_prefix_break();

    let out = noob(rig.config.path(), rig.work.path())
        .env("NOOB_CTX", "4096")
        .args(["exec", "-p", "load the bigproc skill"])
        .output()
        .unwrap();
    ok(&out);

    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 3);
    let cont: String = reqs[2]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap_or("").to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(cont.contains("[earlier conversation dropped:"), "{cont}");
    assert!(
        cont.contains("[loaded skills: bigproc]"),
        "hard-drop stub lost the re-listing:\n{cont}"
    );
    rig.server.assert_clean();
}

/// The other half of skills_gate: a human at a real terminal answering "y"
/// lets the write through. Drives the REPL under a pseudo-terminal, because
/// the gate refuses to take answers from pipes.
#[test]
fn skills_gate_grant_via_tty() {
    use std::io::{Read, Write};
    use std::os::fd::FromRawFd;

    let rig = rig();
    rig.skill(".claude/skills", "greeting", "says hello", "original body\n");
    // A NEW file inside the skills dir: the gate fires on the directory,
    // and check-and-set (which refuses unread overwrites) stays out of the
    // picture.
    rig.server.enqueue_stream_toolcalls(
        &[(
            "g2",
            "write",
            r#"{"path":".claude/skills/greeting/NOTES.md","content":"improved body\n"}"#,
        )],
        None,
    );
    rig.server.enqueue_stream_completion("skill improved");

    // A real pty pair; the REPL sees a terminal on stdin/stdout.
    let (mut master, slave) = unsafe {
        let mut m: libc::c_int = 0;
        let mut s: libc::c_int = 0;
        assert_eq!(
            libc::openpty(&mut m, &mut s, std::ptr::null_mut(), std::ptr::null(), std::ptr::null()),
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

    // Watchdog: a blocking master read on an alive-but-silent child would
    // otherwise hang the whole suite. Killing the child makes the pty
    // return EOF, turning any hang into a prompt test failure. Cancellable
    // so a fast success does not wait out the full timeout on join.
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

    // Reactive driver: wait for a marker, then answer. Type-ahead would be
    // flushed by the gate, so answers must follow their prompts.
    let mut seen = String::new();
    let wait_for = |master: &mut std::fs::File, marker: &str, seen: &mut String| {
        let mut buf = [0u8; 4096];
        while !seen.contains(marker) {
            match master.read(&mut buf) {
                // EOF: the child exited or the watchdog killed it.
                Ok(0) => panic!("pty closed before {marker:?}; saw:\n{seen}"),
                Ok(n) => seen.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(e) => panic!("pty read error: {e}; saw:\n{seen}"),
            }
        }
    };
    wait_for(&mut master, "type a task", &mut seen);
    master.write_all(b"improve your greeting skill\n").unwrap();
    wait_for(&mut master, "[y/N]", &mut seen);
    master.write_all(b"y\n").unwrap();
    wait_for(&mut master, "skill improved", &mut seen);
    master.write_all(b"/quit\n").unwrap();
    let status = child.wait().unwrap();
    done.store(true, Ordering::SeqCst);
    watchdog.join().ok();
    assert!(status.success(), "repl exit: {status:?};\n{seen}");

    let on_disk = std::fs::read_to_string(
        rig.work.path().join(".claude/skills/greeting/NOTES.md"),
    )
    .unwrap();
    assert!(
        on_disk.contains("improved body"),
        "granted confirmation did not let the write through:\n{seen}"
    );
    rig.server.assert_clean();
}
