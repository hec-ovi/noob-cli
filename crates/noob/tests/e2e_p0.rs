//! P0 e2e: the compiled binary against the mock server. Spawns
//! env!("CARGO_BIN_EXE_noob") with NOOB_CONFIG_DIR pointing at a temp dir
//! whose .env targets the mock; asserts stdout, exit codes, recorded wire
//! bytes, and the mock's automatic assertions.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use noob_testkit::{MockServer, RawStep, http_response};

fn write_env(dir: &std::path::Path, base_url: &str, key: &str, model: &str) {
    std::fs::write(
        dir.join(".env"),
        format!("NOOB_BASE_URL={base_url}\nNOOB_API_KEY={key}\nNOOB_MODEL={model}\n"),
    )
    .unwrap();
}

fn noob(config_dir: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noob"));
    cmd.env("NOOB_CONFIG_DIR", config_dir);
    // Keep host/process env from leaking settings into assertions.
    cmd.env_remove("NOOB_BASE_URL")
        .env_remove("NOOB_MODEL")
        .env_remove("NOOB_API_STYLE");
    cmd
}

#[test]
fn exec_round_trip_against_mock() {
    let server = MockServer::start();
    server.enqueue_completion("hello from the mock");
    let dir = tempfile::tempdir().unwrap();
    write_env(dir.path(), &server.base_url(), "sekret-key", "mockmodel");

    let out = noob(dir.path())
        .args(["exec", "-p", "say hi"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello from the mock");

    let recorded = server.recorded();
    assert_eq!(recorded.len(), 1);
    let req = &recorded[0];
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/v1/chat/completions");
    assert_eq!(req.header("authorization"), Some("Bearer sekret-key"));
    let body = req.json().unwrap();
    assert_eq!(body["model"], "mockmodel");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "say hi");
    server.assert_clean();
}

#[test]
fn missing_base_url_fails_with_remedy() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), "# nothing configured\n").unwrap();

    let out = noob(dir.path()).args(["exec", "-p", "hi"]).output().unwrap();

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("NOOB_BASE_URL"), "stderr: {stderr}");
}

#[test]
fn http_error_body_reaches_the_user() {
    let server = MockServer::start();
    server.enqueue_json(500, serde_json::json!({"error": "kaboom-marker"}));
    let dir = tempfile::tempdir().unwrap();
    write_env(dir.path(), &server.base_url(), "", "m");

    let out = noob(dir.path()).args(["exec", "-p", "hi"]).output().unwrap();

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("500"), "stderr: {stderr}");
    assert!(stderr.contains("kaboom-marker"), "stderr: {stderr}");
}

#[test]
fn version_prints_and_exits_zero() {
    let out = Command::new(env!("CARGO_BIN_EXE_noob"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("noob "), "stdout: {stdout}");
}

#[test]
fn missing_prompt_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = noob(dir.path()).arg("exec").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage"));
}

/// SIGINT during a stalled response: the binary exits promptly (well before
/// the stall ends) with the interrupt exit code. Proves the watchdog makes
/// Ctrl-C responsive end to end.
#[test]
fn sigint_aborts_a_stalled_request() {
    let server = MockServer::start();
    server.enqueue_raw(vec![
        RawStep::Bytes(http_response(200, Some(100))),
        RawStep::SleepMs(20_000),
    ]);
    let dir = tempfile::tempdir().unwrap();
    write_env(dir.path(), &server.base_url(), "", "m");

    let mut child = noob(dir.path())
        .args(["exec", "-p", "hi"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // Give it time to connect and enter the stalled body read.
    std::thread::sleep(Duration::from_millis(1200));
    unsafe { libc::kill(child.id() as i32, libc::SIGINT) };

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "binary did not exit within 5s of SIGINT"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(!status.success());
    assert_eq!(status.code(), Some(130));
}
