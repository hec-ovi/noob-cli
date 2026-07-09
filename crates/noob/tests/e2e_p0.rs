//! P0 e2e: the compiled binary against the mock server. Spawns
//! env!("CARGO_BIN_EXE_noob") with NOOB_CONFIG_DIR pointing at a temp dir
//! whose .env targets the mock; asserts stdout, exit codes, recorded wire
//! bytes, and the mock's automatic assertions.

use std::io::Read;
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
    server.assert_clean();
}

/// Exported proxy env vars (common in corp and WSL shells) must not reroute
/// or break requests: noob talks only to the configured endpoints.
#[test]
fn proxy_env_vars_are_ignored() {
    let server = MockServer::start();
    server.enqueue_completion("proxy-free answer");
    let dir = tempfile::tempdir().unwrap();
    write_env(dir.path(), &server.base_url(), "", "m");

    let out = noob(dir.path())
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .args(["exec", "-p", "hi"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "proxy-free answer");
    server.assert_clean();
}

/// A flag without its value is a usage error, never a silent misconfig.
#[test]
fn flag_without_value_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["exec", "-p", "hi", "--model"],
        vec!["exec", "--model", "-p", "hi"],
        vec!["exec", "-p"],
    ] {
        let out = noob(dir.path()).args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "args: {args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("needs a value"), "args: {args:?} stderr: {stderr}");
    }
}

/// Precedence: CLI flag > process env > .env for non-secret settings; the
/// API key comes from .env only and process env NOOB_API_KEY is ignored.
#[test]
fn config_precedence_flag_env_file() {
    let server = MockServer::start();
    server.enqueue_completion("one");
    server.enqueue_completion("two");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".env"),
        format!(
            "NOOB_BASE_URL={}\nNOOB_API_KEY=file-key\nNOOB_MODEL=file-model\n",
            server.base_url()
        ),
    )
    .unwrap();

    // Flag beats process env beats file.
    let out = noob(dir.path())
        .env("NOOB_MODEL", "proc-model")
        .env("NOOB_API_KEY", "proc-key")
        .args(["exec", "-p", "hi", "--model", "flag-model"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));

    // Process env beats file; the secret still comes from the file.
    let out2 = noob(dir.path())
        .env("NOOB_MODEL", "proc-model")
        .env("NOOB_API_KEY", "proc-key")
        .args(["exec", "-p", "hi"])
        .output()
        .unwrap();
    assert!(out2.status.success(), "stderr={}", String::from_utf8_lossy(&out2.stderr));

    let recorded = server.recorded();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].json().unwrap()["model"], "flag-model");
    assert_eq!(recorded[0].header("authorization"), Some("Bearer file-key"));
    assert_eq!(recorded[1].json().unwrap()["model"], "proc-model");
    assert_eq!(recorded[1].header("authorization"), Some("Bearer file-key"));
    server.assert_clean();
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
        .stderr(Stdio::piped())
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
    // The graceful watchdog path prints its message; the hard second-Ctrl-C
    // _exit path prints nothing. Asserting the message pins the right path.
    let mut stderr = String::new();
    child.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();
    assert!(stderr.contains("interrupted"), "stderr: {stderr}");
    server.assert_clean();
}
