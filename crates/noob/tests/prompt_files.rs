//! The two user-owned prompt files, proven through the real binary: the
//! system prompt is AGENTS.md then TOOLS.md (each file when present, its
//! embedded default when absent), then the runtime layers. `noob debug
//! prompt` prints exactly what a session sends.

use std::process::Command;

const AGENTS_DEFAULT: &str = include_str!("../prompts/agents-default.md");
const TOOLS_DEFAULT: &str = include_str!("../prompts/tools-default.md");

fn debug_prompt(config_dir: &std::path::Path, workspace: &std::path::Path) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noob"));
    noob_testkit::scrub_noob_env(&mut cmd);
    let out = cmd
        .env("NOOB_CONFIG_DIR", config_dir)
        .env("NOOB_SANDBOX", "container")
        .current_dir(workspace)
        .args(["debug", "prompt"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
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
