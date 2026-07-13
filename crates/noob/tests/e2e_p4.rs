//! P4 e2e: MCP through the compiled binary. The lazy-connect discipline is
//! the tested product: with servers configured the prompt gains one line and
//! the tools array two entries at session start, and NOTHING else ever
//! changes (catalogs and results arrive as tool results; the head and tools
//! array stay byte-stable through connect and call).

use std::process::Command;

use noob_testkit::MockServer;
use noob_testkit::mcp::{McpHttpServer, echo_tools};
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
    Rig {
        server,
        config,
        work,
    }
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

    fn mcp_json(&self, dir: &std::path::Path, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("mcp.json"), body).unwrap();
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

/// Write the tiny POSIX-shell MCP stdio server into the workspace and
/// return its absolute path. Answers initialize / tools/list / tools/call
/// (an `echo` tool) over newline-delimited JSON-RPC.
fn shell_server(dir: &std::path::Path) -> String {
    let script = r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"mock","version":"0"}}}\n' "$id" ;;
    *'"method":"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"echoes text back","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}}]}}\n' "$id" ;;
    *'"method":"tools/call"'*)
      text=$(printf '%s' "$line" | sed -n 's/.*"text":"\([^"]*\)".*/\1/p')
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"echo: %s"}],"isError":false}}\n' "$id" "$text" ;;
    *) : ;;
  esac
done
"#;
    let path = dir.join("mcp-server.sh");
    std::fs::write(&path, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_str().unwrap().to_string()
}

/// Configured servers put one line in the prompt and two tools in the
/// array; without configuration neither exists. Nothing connects at start.
#[test]
fn mcp_line_and_tool_registration() {
    let rig = rig();
    rig.mcp_json(
        rig.config.path(),
        r#"{"servers": {"beta": {"url": "http://127.0.0.1:1"}, "alpha": {"url": "http://127.0.0.1:2"}}}"#,
    );
    rig.server.enqueue_stream_completion("noted");

    ok(&rig.run(&["exec", "-p", "hello"]));

    let reqs = rig.api_requests();
    let system = reqs[0]["messages"][0]["content"].as_str().unwrap();
    assert!(
        system.contains("MCP servers (use mcp_connect): alpha, beta"),
        "prompt line missing or unsorted:\n{system}"
    );
    // Nothing was probed at session start: both URLs are dead ports and the
    // run still succeeded (lazy to the bone).
    let tools = reqs[0]["tools"].as_array().unwrap();
    assert_eq!(
        tools.len(),
        12,
        "9 core + subagent + mcp_connect + mcp_call"
    );
    assert!(tools.iter().any(|t| t["function"]["name"] == "mcp_connect"));
    assert!(tools.iter().any(|t| t["function"]["name"] == "mcp_call"));
    rig.server.assert_clean();
}

#[test]
fn no_mcp_config_means_no_line_and_no_tools() {
    let rig = rig();
    rig.server.enqueue_stream_completion("bare");
    ok(&rig.run(&["exec", "-p", "hello"]));
    let reqs = rig.api_requests();
    let system = reqs[0]["messages"][0]["content"].as_str().unwrap();
    assert!(!system.contains("MCP servers"));
    assert_eq!(reqs[0]["tools"].as_array().unwrap().len(), 10);
    rig.server.assert_clean();
}

/// The full loop against a stdio server: connect returns the catalog inside
/// untrusted delimiters, call returns the wrapped result, and neither the
/// system prompt nor the tools array moved a byte.
#[test]
fn connect_and_call_stdio_through_the_loop() {
    let rig = rig();
    let cmd = shell_server(rig.work.path());
    rig.mcp_json(
        rig.config.path(),
        &format!(r#"{{"servers": {{"mock": {{"command": "{cmd}"}}}}}}"#),
    );
    rig.server
        .enqueue_stream_toolcalls(&[("m1", "mcp_connect", r#"{"server":"mock"}"#)], None);
    rig.server.enqueue_stream_toolcalls(
        &[(
            "m2",
            "mcp_call",
            r#"{"server":"mock","tool":"echo","args":{"text":"ping"}}"#,
        )],
        None,
    );
    rig.server.enqueue_stream_completion("done");

    ok(&rig.run(&["exec", "-p", "use the mock server"]));

    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 3);
    let catalog = reqs[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        catalog.starts_with("connected to mock: 1 tools (protocol 2025-11-25)"),
        "{catalog}"
    );
    assert!(catalog.contains("[untrusted content from MCP server \"mock\""));
    assert!(catalog.contains("- echo(text: string): echoes text back"));

    let result = reqs[2]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rfind(|m| m["role"] == "tool")
        .unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(result.contains("echo: ping"), "{result}");
    assert!(result.starts_with("[untrusted content from MCP server \"mock\""));

    // The cache-prefix property through MCP use: identical head, identical
    // tools array, across all three requests.
    assert_eq!(reqs[0]["messages"][0], reqs[2]["messages"][0]);
    assert_eq!(reqs[0]["tools"], reqs[2]["tools"]);
    rig.server.assert_clean();
}

/// The same loop against a Streamable HTTP server (the testkit MCP mock).
#[test]
fn connect_and_call_http_through_the_loop() {
    let rig = rig();
    let mcp_server = McpHttpServer::start(echo_tools());
    rig.mcp_json(
        rig.config.path(),
        &format!(
            r#"{{"servers": {{"web": {{"url": "{}"}}}}}}"#,
            mcp_server.url()
        ),
    );
    rig.server
        .enqueue_stream_toolcalls(&[("h1", "mcp_connect", r#"{"server":"web"}"#)], None);
    rig.server.enqueue_stream_toolcalls(
        &[(
            "h2",
            "mcp_call",
            r#"{"server":"web","tool":"echo","args":{"text":"hola"}}"#,
        )],
        None,
    );
    rig.server.enqueue_stream_completion("done");

    ok(&rig.run(&["exec", "-p", "use the web server"]));

    let reqs = rig.api_requests();
    let result = reqs[2]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rfind(|m| m["role"] == "tool")
        .unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(result.contains("hola"), "{result}");
    assert_eq!(mcp_server.calls().len(), 1);
    mcp_server.assert_clean();
    rig.server.assert_clean();
}

/// mcp_call against an unconnected (and an unknown) server comes back as a
/// teaching error, locally, with zero wire traffic.
#[test]
fn call_before_connect_teaches_the_next_move() {
    let rig = rig();
    rig.mcp_json(
        rig.config.path(),
        r#"{"servers": {"mock": {"url": "http://127.0.0.1:1"}}}"#,
    );
    rig.server.enqueue_stream_toolcalls(
        &[
            (
                "e1",
                "mcp_call",
                r#"{"server":"mock","tool":"echo","args":{}}"#,
            ),
            (
                "e2",
                "mcp_call",
                r#"{"server":"ghost","tool":"echo","args":{}}"#,
            ),
        ],
        None,
    );
    rig.server.enqueue_stream_completion("understood");

    ok(&rig.run(&["exec", "-p", "call things"]));

    let reqs = rig.api_requests();
    let results: Vec<String> = reqs[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "tool")
        .map(|m| m["content"].as_str().unwrap().to_string())
        .collect();
    assert!(
        results[0].contains("connect first with mcp_connect"),
        "{}",
        results[0]
    );
    assert!(
        results[1].contains("unknown MCP server \"ghost\""),
        "{}",
        results[1]
    );
    rig.server.assert_clean();
}

/// A broken mcp.json warns on stderr and the session runs on without MCP;
/// a project .noob/mcp.json overrides a global entry of the same name.
#[test]
fn broken_config_warns_and_project_overrides_global() {
    let rig = rig();
    std::fs::write(rig.config.path().join("mcp.json"), "{ nope").unwrap();
    rig.server.enqueue_stream_completion("fine");
    let out = rig.run(&["exec", "-p", "hi"]);
    ok(&out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not valid JSON"), "stderr: {stderr}");
    assert_eq!(rig.api_requests()[0]["tools"].as_array().unwrap().len(), 10);

    // Second run: valid global + project override for the same name.
    rig.mcp_json(
        rig.config.path(),
        r#"{"servers": {"shared": {"url": "http://127.0.0.1:1"}, "global-only": {"url": "http://127.0.0.1:2"}}}"#,
    );
    rig.mcp_json(
        &rig.work.path().join(".noob"),
        r#"{"servers": {"shared": {"url": "http://127.0.0.1:3"}}}"#,
    );
    rig.server.expect_prefix_break(); // a fresh session against the same mock
    rig.server.expect_tools_change(); // ...now with the MCP pair registered
    rig.server.enqueue_stream_completion("listed");
    ok(&rig.run(&["exec", "-p", "hello again"]));
    let reqs = rig.api_requests();
    let system = reqs.last().unwrap()["messages"][0]["content"]
        .as_str()
        .unwrap();
    assert!(
        system.contains("MCP servers (use mcp_connect): global-only, shared"),
        "{system}"
    );
    rig.server.assert_clean();
}
