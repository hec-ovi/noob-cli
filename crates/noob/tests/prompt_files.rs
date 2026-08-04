//! The one user-owned prompt file, proven through the real binary: the system
//! prompt is AGENTS.md (the file when present, the shipped default when
//! absent) fenced under its heading, then the runtime layers. `noob debug
//! prompt` prints exactly what a session sends; `noob debug env` prints only
//! the runtime tail.

use std::process::Command;

const AGENTS_DEFAULT: &str = include_str!("../prompts/agents-default.md");

fn debug(config_dir: &std::path::Path, workspace: &std::path::Path, what: &str) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noob"));
    noob_testkit::scrub_noob_env(&mut cmd);
    let out = cmd
        .env("NOOB_CONFIG_DIR", config_dir)
        .env("NOOB_SANDBOX", "container")
        .current_dir(workspace)
        .args(["debug", what])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn debug_prompt(config_dir: &std::path::Path, workspace: &std::path::Path) -> String {
    debug(config_dir, workspace, "prompt")
}

#[test]
fn an_absent_file_falls_back_to_the_embedded_default_byte_identically() {
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let system = debug_prompt(config.path(), work.path());
    let expected = format!(
        "# Agent\n<instructions>\n{}\n</instructions>\n\n<env>",
        AGENTS_DEFAULT.trim_end()
    );
    assert!(
        system.starts_with(&expected),
        "the default prompt must be the embedded text verbatim, fenced:\n{system}"
    );
}

#[test]
fn present_agents_md_replaces_the_default_wholesale() {
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(config.path().join("AGENTS.md"), "my main prompt\n").unwrap();
    let system = debug_prompt(config.path(), work.path());
    assert!(
        system.starts_with("# Agent\n<instructions>\nmy main prompt\n</instructions>\n\n<env>"),
        "{system}"
    );
    // The file is the whole prompt, not an appended layer: nothing the binary
    // ships survives it, tool guidance included.
    assert!(!system.contains("You are noob"));
    assert!(!system.contains("Call the plan tool"));
    assert!(!system.contains("# Global instructions"));
}

#[test]
fn debug_env_prints_exactly_the_runtime_tail() {
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    // Every runtime layer present: project instructions, a skill, MCP.
    std::fs::write(work.path().join("AGENTS.md"), "project rule\n").unwrap();
    let skill = work.path().join(".noob/skills/tail-probe");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: tail-probe\ndescription: a probe skill for the tail test\n---\nbody\n",
    )
    .unwrap();
    std::fs::write(
        config.path().join("mcp.json"),
        r#"{"servers": {"websearch": {"url": "http://localhost:8000"}}}"#,
    )
    .unwrap();

    let system = debug_prompt(config.path(), work.path());
    let tail = debug(config.path(), work.path(), "env");
    assert!(
        system.ends_with(&tail),
        "debug env must be the byte-exact tail of debug prompt:\n{tail}"
    );
    assert!(tail.starts_with("<env>\ncwd: "), "{tail}");
    for layer in [
        "# Project instructions\n<instructions>\nproject rule\n</instructions>",
        "# Skills\n<available_skills>",
        "- tail-probe: a probe skill for the tail test",
        "# MCP servers\n<mcp_servers>\nConnect with mcp_connect: websearch",
    ] {
        assert!(tail.contains(layer), "missing {layer:?} in:\n{tail}");
    }
    // Nothing more: the shipped text stays out of the tail.
    assert!(!tail.contains("You are noob"));
    assert!(!tail.contains("Call the plan tool"));
}
