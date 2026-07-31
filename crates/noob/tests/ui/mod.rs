//! Shared glue for the ui_* suites: the mock-server rig, noob-flavored pty
//! spawns, stream scripting, and screen-assertion helpers. The pty driver and
//! the Vt screen emulator live in noob-testkit; this module only knows noob.
//! Each ui_* test crate compiles its own copy, so unused helpers are expected.
#![allow(dead_code)]

use std::io::Write;
use std::process::Command;

use noob_testkit::MockServer;
use serde_json::Value;

pub use noob_testkit::{Pty, Vt};

fn write_env(dir: &std::path::Path, base_url: &str) {
    std::fs::write(
        dir.join(".env"),
        format!("NOOB_BASE_URL={base_url}\nNOOB_MODEL=mockmodel\n"),
    )
    .unwrap();
}

pub fn noob(config_dir: &std::path::Path, workspace: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noob"));
    cmd.env("NOOB_CONFIG_DIR", config_dir)
        .current_dir(workspace)
        .env_remove("NOOB_BASE_URL")
        .env_remove("NOOB_MODEL")
        .env_remove("NOOB_API_STYLE")
        .env_remove("NOOB_CTX")
        .env_remove("NOOB_SANDBOX")
        // Hermetic: a developer machine with the CLI installed must not
        // register an extra tool and change what these assert on.
        .env("NOOB_WEBSEARCH", "off");
    cmd
}

pub struct Rig {
    pub server: MockServer,
    pub config: tempfile::TempDir,
    pub work: tempfile::TempDir,
}

pub fn rig() -> Rig {
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
    pub fn api_requests(&self) -> Vec<Value> {
        self.server
            .recorded()
            .iter()
            .filter(|r| r.path.ends_with("/chat/completions"))
            .map(|r| r.json().unwrap())
            .collect()
    }

    pub fn responses_requests(&self) -> Vec<Value> {
        self.server
            .recorded()
            .iter()
            .filter(|request| request.path.ends_with("/responses"))
            .map(|request| request.json().unwrap())
            .collect()
    }
}

/// The last user message in a recorded chat request: the line the editor
/// actually submitted.
pub fn last_user(req: &Value) -> String {
    req["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The sequence the editor writes right after `tcsetattr(raw)` succeeds.
/// Waiting for it proves the terminal is raw, so editing keys sent afterward
/// are handled by the editor and not the cooked line discipline (which would
/// treat Ctrl-U/Ctrl-C/Ctrl-D as VKILL/VINTR/VEOF).
pub const RAW_READY: &str = "\x1b[?2004h";

pub const DOCK: &[(&str, &str)] = &[("NOOB_DOCK", "1")];

/// Spawn the REPL on a fresh pty with the classic per-prompt editor
/// (`NOOB_DOCK=0`). The dock is the product default and has its own
/// whole-turn suites; these spawns exercise the classic editor explicitly.
pub fn spawn_pty(rig: &Rig) -> Pty {
    spawn_pty_with(rig, &[("NOOB_DOCK", "0")])
}

/// Spawn with exactly the requested UI environment. An empty slice exercises
/// the default dock; `NOOB_DOCK=0` is the classic escape hatch.
pub fn spawn_pty_with(rig: &Rig, envs: &[(&str, &str)]) -> Pty {
    spawn_pty_sized(rig, envs, None, &[])
}

/// Spawn with a specific terminal size. `size = Some((rows, cols))` sets the
/// pty winsize so scrolling behavior on a small screen is reproducible; noob
/// reads only `cols` (via TIOCGWINSZ) and is otherwise row-agnostic, so the
/// row count matters only to the emulator that replays the captured bytes.
pub fn spawn_pty_sized(
    rig: &Rig,
    envs: &[(&str, &str)],
    size: Option<(u16, u16)>,
    args: &[&str],
) -> Pty {
    // Force the themed color surface on regardless of the host's TERM, so the
    // pty tests exercise the real interactive path (a color terminal) and the
    // thinking scanner engages deterministically.
    let mut cmd = noob(rig.config.path(), rig.work.path());
    cmd.env("COLORTERM", "truecolor").env_remove("NO_COLOR");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.args(args);
    Pty::spawn(cmd, size)
}

/// Run the REPL with args and piped stdin; return its output.
pub fn run_repl(rig: &Rig, args: &[&str], input: &[u8]) -> std::process::Output {
    let mut child = noob(rig.config.path(), rig.work.path())
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

/// Write a session transcript file so a resume can replay it. `items` are the
/// per-item JSON objects (the user/assistant/tool shapes the session log uses);
/// each is wrapped as one `{"t":"item","item":...}` line under a meta header.
pub fn write_session(config: &std::path::Path, id: &str, items: &[Value]) {
    let dir = config.join("sessions");
    std::fs::create_dir_all(&dir).unwrap();
    let mut out = format!(
        "{}\n",
        serde_json::json!({"t":"meta","v":1,"id":id,"created_ms":0})
    );
    for item in items {
        out.push_str(&format!(
            "{}\n",
            serde_json::json!({"t":"item","item":item})
        ));
    }
    std::fs::write(dir.join(format!("{id}.jsonl")), out).unwrap();
}

/// Drop every SGR escape so an assertion can key on the plain text a human sees.
pub fn strip_ansi(s: &str) -> String {
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

/// Write a SKILL.md (name + description + body) at `dir`.
pub fn write_skill_md(dir: &std::path::Path, name: &str, desc: &str, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {desc}\n---\n{body}\n"),
    )
    .unwrap();
}

/// Every message's content across a recorded request, joined, for substring
/// assertions on what the model was actually sent.
pub fn all_content(req: &Value) -> String {
    req["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap_or("").to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Cross a boundary that has no byte marker (turn teardown to the next
/// prompt's reader). Generous next to the epilogue's sub-millisecond cost.
pub fn settle() {
    std::thread::sleep(std::time::Duration::from_millis(400));
}

/// End the session from the idle prompt with Ctrl-D. A byte sent during turn
/// teardown is consumed as an in-turn key and dropped, and the in-turn input
/// hint ("type a message; Enter queues it") shares its prefix with the idle
/// one, so neither a sleep nor a marker wait places the boundary reliably.
/// Drain repaints until the last hint on the wire is the idle form, then quit.
pub fn quit_at_idle(pty: &mut Pty) {
    for _ in 0..40 {
        pty.drain(std::time::Duration::from_millis(200));
        if let Some(at) = pty.seen().rfind("type a message")
            && !pty.seen()[at..].starts_with("type a message; Enter queues")
        {
            pty.send(&[0x04]);
            pty.wait_for("resume with");
            return;
        }
    }
    panic!("the idle prompt never settled; saw:\n{}", pty.seen());
}

/// Chunked-transfer frames for a run of SSE `data:` payloads (one frame per
/// event, no terminator), for scripting a stream that stalls mid-reply.
pub fn sse_frames(datas: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for d in datas {
        let event = format!("data: {d}\n\n");
        out.extend_from_slice(format!("{:x}\r\n", event.len()).as_bytes());
        out.extend_from_slice(event.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// A run whose stream sends `head_words` deltas, then holds `stall_ms`, then
/// (optionally) sends the rest and closes. `chat_stream_datas` splits on
/// whitespace, so head_words counts role delta + that many words.
pub fn stalled_stream(
    text: &str,
    head_deltas: usize,
    stall_ms: u64,
    resume: bool,
) -> Vec<noob_testkit::RawStep> {
    let datas = noob_testkit::chat_stream_datas(text);
    let mut head =
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n"
            .to_vec();
    head.extend_from_slice(&sse_frames(&datas[..head_deltas]));
    let mut steps = vec![
        noob_testkit::RawStep::Bytes(head),
        noob_testkit::RawStep::SleepMs(stall_ms),
    ];
    if resume {
        let mut tail = sse_frames(&datas[head_deltas..]);
        tail.extend_from_slice(b"0\r\n\r\n");
        steps.push(noob_testkit::RawStep::Bytes(tail));
    }
    steps
}

pub fn responses_completion_stream(text: &str, stall_ms: u64) -> Vec<noob_testkit::RawStep> {
    let message = serde_json::json!({
        "id": "message-1",
        "type": "message",
        "role": "assistant",
        "content": [{"type":"output_text","text":text}]
    });
    let events = [
        serde_json::json!({"type":"response.output_text.delta","item_id":"message-1","delta":text}),
        serde_json::json!({
            "type":"response.completed",
            "response": {"status":"completed","output":[message],"usage":{"input_tokens":10,"output_tokens":5}}
        }),
    ];
    let mut steps = vec![noob_testkit::RawStep::Bytes(noob_testkit::sse_headers())];
    if stall_ms > 0 {
        steps.push(noob_testkit::RawStep::SleepMs(stall_ms));
    }
    for event in events {
        steps.push(noob_testkit::RawStep::Bytes(
            format!("data: {event}\n\n").into_bytes(),
        ));
    }
    steps
}

pub fn responses_toolcall_stream(
    call_id: &str,
    name: &str,
    arguments: &str,
) -> Vec<noob_testkit::RawStep> {
    let item = serde_json::json!({
        "id": "function-1",
        "type": "function_call",
        "call_id": call_id,
        "name": name,
        "arguments": arguments
    });
    let events = [
        serde_json::json!({"type":"response.output_item.added","item":item}),
        serde_json::json!({"type":"response.output_item.done","item":item}),
        serde_json::json!({
            "type":"response.completed",
            "response": {"status":"completed","output":[item],"usage":{"input_tokens":10,"output_tokens":5}}
        }),
    ];
    let mut bytes = noob_testkit::sse_headers();
    for event in events {
        bytes.extend_from_slice(format!("data: {event}\n\n").as_bytes());
    }
    vec![noob_testkit::RawStep::Bytes(bytes)]
}

/// The U+203A input marker the dock's input row always leads with.
pub const MARKER: &str = "\u{203a}";

/// Every glyph an in-progress `[~]` row may show: the raw glyph plus the four
/// animation frames the dock substitutes on its comet cadence (during turns
/// AND at the idle prompt).
pub const SPINNER_FRAMES: [&str; 5] = ["[~]", "[|]", "[/]", "[-]", "[\\]"];

/// Find the dock's three rows in a rendered screen: the "Working" top rule, the
/// "Esc Esc to cancel" bottom rule, and the input row between them. Returns the
/// row indices if the top and bottom rules are both present.
pub fn dock_rows(screen: &[String]) -> Option<(usize, usize)> {
    let top = screen.iter().rposition(|r| r.contains("Working"))?;
    let bottom = screen
        .iter()
        .rposition(|r| r.contains("Esc Esc to cancel"))?;
    Some((top, bottom))
}

/// The live input row in a rendered screen: the one leading with the U+203A
/// marker. The greeting banner carries the command names too but never the
/// marker, so this isolates the editable row from the banner.
pub fn input_row(screen: &[String]) -> Option<&String> {
    screen.iter().find(|r| r.contains(MARKER))
}

/// Rows of a reflowed screen that carry a rule-sized run of `─` (20 or more):
/// the box/frame rules. The 12-dash logo underline and `── plan` heads stay
/// under the threshold, so any extra hit is a stale fragment of a badly
/// erased frame (the tested screens are 60 columns wide, so even a wrapped
/// remainder of a 100-col rule is a 40-dash run).
pub fn rule_row_indices(rows: &[String]) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let mut run = 0usize;
            let mut best = 0usize;
            for ch in row.chars() {
                if ch == '─' {
                    run += 1;
                    best = best.max(run);
                } else {
                    run = 0;
                }
            }
            (best >= 20).then_some(index)
        })
        .collect()
}

/// Scrollback must hold no archived frame garbage: no rule-sized `─` run (a
/// stale frame copy) and no screen-height run of blank rows (the gap a
/// viewport reset used to archive below the frame). Legitimate history (the
/// banner, scrolled transcript) passes; the pre-fix resize path fails on its
/// first reset, which VTE-family terminals turn into one archived garbage
/// screen per resize.
pub fn assert_scrollback_clean(vt: &Vt, screen_rows: usize, label: &str) {
    let back = vt.scrollback().to_vec();
    let stale_rules = rule_row_indices(&back);
    assert!(
        stale_rules.is_empty(),
        "{label}: archived frame rules in scrollback at rows {stale_rules:?}:\n{back:#?}"
    );
    let mut run = 0usize;
    let mut worst = 0usize;
    for row in &back {
        if row.is_empty() {
            run += 1;
            worst = worst.max(run);
        } else {
            run = 0;
        }
    }
    assert!(
        worst < screen_rows,
        "{label}: a {worst}-row blank gap was archived into scrollback:\n{back:#?}"
    );
}
