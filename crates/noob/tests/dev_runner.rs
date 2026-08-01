//! Dev-runner contract without needing a Docker daemon. A fake docker
//! executable records the build and run argv at the process boundary.

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .to_path_buf()
}

fn fake_docker(dir: &std::path::Path) -> String {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let docker = bin.join("docker");
    std::fs::write(
        &docker,
        "#!/bin/sh\nprintf 'CALL\\n' >> \"$DOCKER_LOG\"\n\
         if [ -n \"${NOOB_WORKSPACE:-}\" ]; then printf 'NOOB_WORKSPACE=%s\\n' \"$NOOB_WORKSPACE\" >> \"$DOCKER_LOG\"; fi\n\
         printf '%s\\n' \"$@\" >> \"$DOCKER_LOG\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755)).unwrap();
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

/// The bundled search tool reads a `.env` from its working directory and
/// exports every key in it. Inside the image the agent's working directory is
/// /work, the user's project, so a project `.env` would silently feed that
/// process: `WEBSEARCH_PROXY` there reroutes or kills every search, and the
/// rest of the file (database URLs, API keys) lands in the environment of a
/// process that opens sockets. The image points the tool's dotenv at /config
/// instead, where noob's own configuration lives.
#[test]
fn the_image_keeps_the_search_tool_dotenv_out_of_the_workspace() {
    let dockerfile = std::fs::read_to_string(repo_root().join("docker/Dockerfile")).unwrap();
    let value = dockerfile
        .lines()
        .find_map(|l| l.trim().strip_prefix("WEBSEARCH_ENV_FILE="))
        .expect("the runtime image must pin WEBSEARCH_ENV_FILE")
        .trim_end_matches(" \\")
        .to_string();
    assert!(
        value.starts_with("/config/"),
        "the search tool dotenv must live in /config, not the workspace: {value}"
    );
    assert!(dockerfile.contains("WORKDIR /work"), "{dockerfile}");
}

#[test]
fn live_runner_forwards_endpoint_overrides_to_docker() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("docker.log");
    let path = fake_docker(tmp.path());

    let output = Command::new("bash")
        .arg(repo_root().join("dev.sh"))
        .arg("smoke")
        .env("PATH", path)
        .env("DOCKER_LOG", &log)
        .env("NOOB_LIVE_BASE_URL", "http://localhost:8090/v1")
        .env("NOOB_LIVE_MODEL", "my-model")
        .env("NOOB_LIVE_MCP_URL", "http://localhost:18000/mcp")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "live runner failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let calls = std::fs::read_to_string(log).unwrap();
    assert!(
        calls.contains("build\n--build-arg\nTARGETARCH=x86_64\n--target\ndev\n"),
        "{calls}"
    );
    assert!(calls.contains("-e\nNOOB_LIVE_BASE_URL\n"), "{calls}");
    assert!(calls.contains("-e\nNOOB_LIVE_MODEL\n"), "{calls}");
    assert!(calls.contains("-e\nNOOB_LIVE_MCP_URL\n"), "{calls}");
    assert!(calls.contains("--ignored\n--test-threads=1\n"), "{calls}");
    // Gating is `--ignored` alone; nothing reads a NOOB_LIVE switch.
    assert!(!calls.contains("NOOB_LIVE=1\n"), "{calls}");
}

#[test]
fn dev_runner_creates_and_mounts_an_isolated_default_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let checkout = tmp.path().join("checkout");
    std::fs::create_dir(&checkout).unwrap();
    std::fs::copy(repo_root().join("dev.sh"), checkout.join("dev.sh")).unwrap();
    let log = tmp.path().join("docker.log");
    let path = fake_docker(tmp.path());

    let output = Command::new("bash")
        .arg(checkout.join("dev.sh"))
        // Host-exported workspace overrides would defeat the isolation
        // default under test.
        .env_remove("NOOB_WORKSPACE")
        .env_remove("WORKSPACE")
        .env("PATH", path)
        .env("DOCKER_LOG", &log)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "dev runner failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let workspace = checkout.join("workspace").canonicalize().unwrap();
    let calls = std::fs::read_to_string(log).unwrap();
    assert!(
        calls.contains(&format!("NOOB_WORKSPACE={}\n", workspace.display())),
        "{calls}"
    );
    assert!(
        calls.contains("compose\nrun\n--build\n--rm\n--user\n"),
        "{calls}"
    );
}
