//! e2e for the agentic `plan` checklist tool through the compiled binary
//! against the mock server. Drives a model turn that emits a `plan` tool
//! call, then a second call that advances a status, and asserts the rendered
//! checklist (header, `[x]`/`[~]`/`[ ]` glyphs, item contents) reaches the
//! transcript, updates on the second call, and lands as a tool-role result.

use std::process::Command;

use noob_testkit::MockServer;
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
}

/// Drop every SGR escape so an assertion keys on the plain text, never a color.
fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The tool-role result content for a specific tool call id (a later request
/// replays every earlier result, so match by id, not "the first tool message").
fn tool_result(req: &Value, call_id: &str) -> String {
    req["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool" && m["tool_call_id"] == call_id)
        .unwrap_or_else(|| panic!("no tool result for {call_id}"))["content"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The model lays out a three-item plan, then advances two items on the next
/// call. Each call's rendered checklist (header + glyphs + items) reaches the
/// transcript as a tool result, and the second call re-renders the new status.
#[test]
fn todo_tool_renders_and_updates_a_visible_checklist() {
    let rig = rig();
    let first = r#"{"todos":[{"content":"research the codebase","status":"completed"},{"content":"write the todo tool","status":"in_progress"},{"content":"add tests","status":"pending"}]}"#;
    let second = r#"{"todos":[{"content":"research the codebase","status":"completed"},{"content":"write the todo tool","status":"completed"},{"content":"add tests","status":"in_progress"}]}"#;
    rig.server
        .enqueue_stream_toolcalls(&[("p1", "plan", first)], None);
    rig.server
        .enqueue_stream_toolcalls(&[("p2", "plan", second)], None);
    rig.server.enqueue_stream_completion("plan complete");

    let out = rig.run(&["exec", "-p", "do the multi-step task"]);
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let progress = String::from_utf8_lossy(&out.stderr);
    assert!(
        progress.contains("plan: 1/3 done · 0.0s"),
        "missing action duration: {progress}"
    );

    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 3, "two todo rounds plus the completion round");

    // The first call's rendered checklist reaches the model verbatim: header,
    // every glyph, every item content.
    let r1 = strip_ansi(&tool_result(&reqs[1], "p1"));
    assert!(
        r1.starts_with("plan (1/3 done):"),
        "header/progress wrong:\n{r1}"
    );
    assert!(
        r1.lines().next().unwrap().contains("·"),
        "plan elapsed time missing:\n{r1}"
    );
    assert!(
        r1.contains("[x] research the codebase"),
        "completed glyph/item:\n{r1}"
    );
    assert!(
        r1.contains("[~] write the todo tool"),
        "in-progress glyph/item:\n{r1}"
    );
    assert!(r1.contains("[ ] add tests"), "pending glyph/item:\n{r1}");

    // The second call overwrites the list and re-renders with the new status.
    let r2 = strip_ansi(&tool_result(&reqs[2], "p2"));
    assert!(
        r2.starts_with("plan (2/3 done):"),
        "progress did not advance:\n{r2}"
    );
    assert!(
        r2.contains("[x] write the todo tool"),
        "status did not update to completed:\n{r2}"
    );
    assert!(
        r2.lines()
            .find(|line| line.contains("[x] write the todo tool"))
            .is_some_and(|line| line.contains("·")),
        "completed action elapsed time missing:\n{r2}"
    );
    assert!(
        r2.contains("[~] add tests"),
        "status did not update to in_progress:\n{r2}"
    );

    // The compact summary surfaced to the user (activity line on stderr).
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        strip_ansi(&stderr).contains("plan: 1/3 done"),
        "summary line missing from stderr:\n{stderr}"
    );

    rig.server.assert_clean();
}

#[test]
fn context_tool_gives_the_model_its_live_budget() {
    let rig = rig();
    rig.server
        .enqueue_stream_toolcalls(&[("c1", "context", r#"{}"#)], None);
    rig.server.enqueue_stream_completion("context understood");

    let out = rig.run(&["exec", "-p", "check your context budget"]);
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let reqs = rig.api_requests();
    assert_eq!(reqs.len(), 2);
    let result = tool_result(&reqs[1], "c1");
    assert!(
        result.contains("/ 131.1k tokens"),
        "total context missing: {result}"
    );
    assert!(
        result.contains("automatic compaction starts near 98.3k (75%)"),
        "{result}"
    );
    rig.server.assert_clean();
}
