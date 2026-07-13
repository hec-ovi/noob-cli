use std::process::{Command, Stdio};

fn noob(config_dir: &std::path::Path, workspace: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_noob"));
    command
        .env("NOOB_CONFIG_DIR", config_dir)
        .env("NOOB_DOCK", "0")
        .env("NO_COLOR", "1")
        .env_remove("NOOB_BASE_URL")
        .env_remove("NOOB_MODEL")
        .env_remove("NOOB_API_STYLE")
        .env_remove("NOOB_AUTODETECT")
        .current_dir(workspace)
        .stdin(Stdio::null());
    command
}

fn combined_output(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn compiled_binary_warns_once_for_skipped_replay_records_only() {
    let config = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(
        config.path().join(".env"),
        "NOOB_BASE_URL=http://127.0.0.1:1/v1\nNOOB_MODEL=replay-test\nNOOB_AUTODETECT=0\n",
    )
    .unwrap();
    let sessions = config.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(
        sessions.join("recover.jsonl"),
        concat!(
            "{\"t\":\"meta\",\"v\":1,\"id\":\"recover\"}\n",
            "{\"t\":\"item\",\"item\":{\"role\":\"user\",\"text\":\"before\"}}\n",
            "{\"t\":\"reset\",\"items\":[{\"role\":\"user\",\"text\":\"summary\"},{\"role\":\"tool\",\"id\":\"missing-content\"}]}\n",
            "GARBAGE\n",
            "{\"t\":\"future-record\"}\n",
            "{\"t\":\"item\",\"item\":{\"role\":\"future-role\"}}\n",
            "{\"t\":\"reset\",\"items\":\"not-an-array\"}\n",
            "{\"t\":\"item\",\"item\":{\"role\":\"user\",\"text\":\"after\"}}\n",
        ),
    )
    .unwrap();

    let recovered = noob(config.path(), workspace.path())
        .args(["--resume", "recover"])
        .output()
        .unwrap();
    let recovered_output = combined_output(&recovered);
    let warning = "session recovery warning: skipped 5 unreadable or malformed session records; restored valid history";
    assert!(
        recovered.status.success(),
        "recovered output: {recovered_output}"
    );
    assert_eq!(recovered_output.matches(warning).count(), 1);

    std::fs::write(
        sessions.join("clean.jsonl"),
        concat!(
            "{\"t\":\"meta\",\"v\":1,\"id\":\"clean\"}\n",
            "{\"t\":\"item\",\"item\":{\"role\":\"user\",\"text\":\"valid\"}}\n",
        ),
    )
    .unwrap();
    let clean = noob(config.path(), workspace.path())
        .args(["--resume", "clean"])
        .output()
        .unwrap();
    let clean_output = combined_output(&clean);
    assert!(clean.status.success(), "clean output: {clean_output}");
    assert!(!clean_output.contains("session recovery warning:"));
}
