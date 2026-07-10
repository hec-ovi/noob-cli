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
/// registered set as the 8th tool.
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
    assert_eq!(tools.len(), 8);
    assert!(tools.iter().any(|t| t["function"]["name"] == "skill"));
    rig.server.assert_clean();
}

/// No skills discovered: no resolver section, no skill tool; the tools
/// array stays the 7 core specs (the registered set is decided at start).
#[test]
fn no_skills_means_no_skill_tool_and_no_section() {
    let rig = rig();
    rig.server.enqueue_stream_completion("bare");

    ok(&rig.run(&["exec", "-p", "hello"]));

    let reqs = rig.api_requests();
    let system = reqs[0]["messages"][0]["content"].as_str().unwrap();
    assert!(!system.contains("# Skills"));
    assert_eq!(reqs[0]["tools"].as_array().unwrap().len(), 7);
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
    // The system prompt did not change when the skill loaded.
    assert_eq!(reqs[0]["messages"][0], reqs[1]["messages"][0]);
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
