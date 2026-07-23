//! Live P3 smoke (opt-in: `./dev.sh smoke`): a real skill
//! load-and-use round trip against qwen, through the shipped binary. This
//! is gauntlet item 5: the resolver index routes the model to the skill,
//! the body arrives as a tool result, and its content shapes the answer.

use std::process::Command;

use serde_json::Value;

fn live_base_url() -> String {
    std::env::var("NOOB_LIVE_BASE_URL").unwrap_or_else(|_| "http://localhost:8090/v1".to_string())
}

fn rig() -> (tempfile::TempDir, tempfile::TempDir) {
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(
        config.path().join(".env"),
        format!(
            "NOOB_BASE_URL={}\nNOOB_API_KEY=noauth\nNOOB_MODEL=qwen3.6-35b-a3b\n",
            live_base_url()
        ),
    )
    .unwrap();
    (config, work)
}

fn noob(config: &std::path::Path, work: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_noob"))
        .env("NOOB_CONFIG_DIR", config)
        .current_dir(work)
        .env_remove("NOOB_BASE_URL")
        .args(args)
        .output()
        .unwrap()
}

/// Live smoke: the model must load a skill to learn a fact that exists
/// nowhere else, and use it. Proves the whole chain on a small model:
/// resolver index -> skill tool call -> body as tool result -> answer.
#[test]
#[ignore = "live: needs qwen at :8090 (./dev.sh smoke)"]
fn live_skill_load_and_use() {
    let (config, work) = rig();
    let dir = work.path().join(".claude/skills/release-tags");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: release-tags\ndescription: how to tag release notes in this project\n---\n\
         # Release tags\n\nEvery release note in this project must end with the exact \
         tag ZANZIBAR-7. This is the only approved tag.\n",
    )
    .unwrap();

    let out = noob(
        config.path(),
        work.path(),
        &[
            "exec",
            "--json",
            "-p",
            "Load the release-tags skill, then answer in one line: what exact tag \
             must every release note end with?",
        ],
    );
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let events: Vec<Value> = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    // The model actually went through the skill tool...
    assert!(
        events
            .iter()
            .any(|e| e["t"] == "tool" && e["name"] == "skill"),
        "no skill tool call in: {stdout}"
    );
    // ...and the loaded body shaped the answer.
    let text: String = events
        .iter()
        .filter(|e| e["t"] == "text")
        .filter_map(|e| e["d"].as_str())
        .collect();
    assert!(
        text.contains("ZANZIBAR-7"),
        "the skill content did not reach the answer: {text}"
    );
}
