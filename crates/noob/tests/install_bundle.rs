//! Host installer contract without needing a Docker daemon. A fake docker
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

#[test]
fn installer_builds_image_installs_launcher_and_forwards_restore() {
    let tmp = tempfile::tempdir().unwrap();
    let prefix = tmp.path().join("prefix");
    let config = tmp.path().join("config");
    let log = tmp.path().join("docker.log");
    let path = fake_docker(tmp.path());

    let installed = Command::new("bash")
        .arg(repo_root().join("install.sh"))
        .args(["--prefix", prefix.to_str().unwrap()])
        .env("PATH", &path)
        .env("HOME", tmp.path().join("home"))
        .env("NOOB_CONFIG_HOME", &config)
        .env("DOCKER_LOG", &log)
        .output()
        .unwrap();
    assert!(
        installed.status.success(),
        "installer failed: {}",
        String::from_utf8_lossy(&installed.stderr)
    );

    let launcher = prefix.join("bin/noob");
    assert!(launcher.is_file());
    assert!(config.join("skills/web-search/SKILL.md").is_file());
    assert!(config.join("mcp.json").is_file());
    assert!(
        std::fs::read_to_string(config.join("mcp.json"))
            .unwrap()
            .contains("websearch")
    );

    let workspace = tmp.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let ran = Command::new(&launcher)
        .args(["--restore", "saved-session"])
        .current_dir(&workspace)
        .env("PATH", &path)
        .env("HOME", tmp.path().join("home"))
        .env("NOOB_CONFIG_HOME", &config)
        .env("NOOB_MODEL", "mock-model")
        .env("NOOB_API_KEY", "host-secret-must-not-be-forwarded")
        .env("WEBSEARCH_PROXY", "nordvpn")
        .env("NORDVPN_USER", "svc-user")
        .env("NORDVPN_PASS", "svc-pass")
        .env("NOOB_TOOL_CAPS", "0")
        .env("DOCKER_LOG", &log)
        .output()
        .unwrap();
    assert!(
        ran.status.success(),
        "launcher failed: {}",
        String::from_utf8_lossy(&ran.stderr)
    );

    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        calls
            .contains("build\n--build-arg\nTARGETARCH=amd64\n--target\nruntime\n--tag\nnoob:local"),
        "{calls}"
    );
    assert!(calls.contains("run\n--rm\n-i\n"), "{calls}");
    assert!(
        calls.contains(&format!("{}:/work", workspace.display())),
        "{calls}"
    );
    assert!(
        calls.contains(&format!("{}:/config", config.display())),
        "{calls}"
    );
    assert!(calls.contains("--env\nNOOB_MODEL\n"), "{calls}");
    assert!(
        !calls.contains("NOOB_API_KEY"),
        "the launcher must not expose a host API key to tools: {calls}"
    );
    // The websearch egress-proxy switch and the NordVPN service credentials it expands
    // are part of the fixed forward set, so `WEBSEARCH_PROXY=nordvpn noob` just works.
    assert!(calls.contains("--env\nWEBSEARCH_PROXY\n"), "{calls}");
    assert!(calls.contains("--env\nNORDVPN_USER\n"), "{calls}");
    assert!(calls.contains("--env\nNORDVPN_PASS\n"), "{calls}");
    // The truncation-caps switch forwards too, so `NOOB_TOOL_CAPS=0 noob`
    // lifts the caps inside the container without touching /config/.env.
    assert!(calls.contains("--env\nNOOB_TOOL_CAPS\n"), "{calls}");
    assert!(
        calls.contains("noob:local\n--restore\nsaved-session\n"),
        "{calls}"
    );
}

#[test]
fn installer_refuses_to_replace_an_unmanaged_command_without_force() {
    let tmp = tempfile::tempdir().unwrap();
    let prefix = tmp.path().join("prefix");
    std::fs::create_dir_all(prefix.join("bin")).unwrap();
    std::fs::write(prefix.join("bin/noob"), "unrelated command\n").unwrap();
    let log = tmp.path().join("docker.log");
    let path = fake_docker(tmp.path());

    let output = Command::new("bash")
        .arg(repo_root().join("install.sh"))
        .args(["--prefix", prefix.to_str().unwrap()])
        .env("PATH", path)
        .env("HOME", tmp.path().join("home"))
        .env("DOCKER_LOG", &log)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("refusing to replace unmanaged"));
    assert!(
        !log.exists(),
        "the image must not build after a safety refusal"
    );
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
        .env("NOOB_LIVE_BASE_URL", "http://localhost:8080/v1")
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
    assert!(calls.contains("-e\nNOOB_LIVE_MCP_URL\n"), "{calls}");
    assert!(calls.contains("--ignored\n--test-threads=1\n"), "{calls}");
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
