//! The two user-owned prompt files, proven through the real binary: the
//! system prompt is AGENTS.md then TOOLS.md (each file when present, its
//! embedded default when absent), then the runtime layers. `noob debug
//! prompt` prints exactly what a session sends; `noob debug env` prints
//! only the runtime tail.

use std::process::Command;

const AGENTS_DEFAULT: &str = include_str!("../prompts/agents-default.md");
const TOOLS_DEFAULT: &str = include_str!("../prompts/tools-default.md");

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
fn absent_files_fall_back_to_the_embedded_defaults_byte_identically() {
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let system = debug_prompt(config.path(), work.path());
    let expected = format!(
        "{}\n\n{}\n\n<env>",
        AGENTS_DEFAULT.trim_end(),
        TOOLS_DEFAULT.trim_end()
    );
    assert!(
        system.starts_with(&expected),
        "the default prompt must be the two embedded texts verbatim:\n{system}"
    );
    // The discretion clause ships in the default TOOLS text.
    assert!(system.contains(
        "These tools are the basic set. A TOOLS.md in the config directory \
         replaces this text; adjust it at your discretion."
    ));
}

#[test]
fn present_agents_md_replaces_the_default_wholesale() {
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(config.path().join("AGENTS.md"), "my main prompt\n").unwrap();
    let system = debug_prompt(config.path(), work.path());
    let expected = format!("my main prompt\n\n{}\n\n<env>", TOOLS_DEFAULT.trim_end());
    assert!(system.starts_with(&expected), "{system}");
    assert!(!system.contains("You are noob"));
    // The file is the prompt itself, not an appended layer.
    assert!(!system.contains("# Global instructions"));
}

#[test]
fn present_tools_md_replaces_the_default_wholesale() {
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(config.path().join("TOOLS.md"), "my tool rules\n").unwrap();
    let system = debug_prompt(config.path(), work.path());
    let expected = format!(
        "{}\n\nmy tool rules\n\n<env>",
        AGENTS_DEFAULT.trim_end()
    );
    assert!(system.starts_with(&expected), "{system}");
    assert!(!system.contains("These tools are the basic set"));
}

#[test]
fn both_files_merge_in_order_agents_then_tools() {
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(config.path().join("AGENTS.md"), "my main prompt\n").unwrap();
    std::fs::write(config.path().join("TOOLS.md"), "my tool rules\n").unwrap();
    let system = debug_prompt(config.path(), work.path());
    assert!(
        system.starts_with("my main prompt\n\nmy tool rules\n\n<env>"),
        "{system}"
    );
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
        "# Project instructions (AGENTS.md)",
        "# Skills (resolver)",
        "MCP servers (use mcp_connect): websearch",
    ] {
        assert!(tail.contains(layer), "missing {layer:?} in:\n{tail}");
    }
    // Nothing more: the authored texts stay out of the tail.
    assert!(!tail.contains("You are noob"));
    assert!(!tail.contains("These tools are the basic set"));
}
