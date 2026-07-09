//! Dev-only mock OpenAI server: a hand-rolled HTTP/1.1 server on
//! `std::net::TcpListener` serving `/v1/chat/completions` and `/v1/responses`.
//!
//! Tests enqueue scripted responses; the server records every raw request and
//! runs three assertions automatically on each one, so every future e2e
//! inherits the wire invariants for free:
//!   1. prefix stability: each request's message/input array extends the
//!      previous one (declare expected breaks with `expect_prefix_break`)
//!   2. no max_tokens-family key anywhere in the body
//!   3. transcript validity: every tool_call id paired with exactly one
//!      result, in emission order
//!
//! Violations are collected, not panicked (a panic in the server thread would
//! vanish); tests must end with `assert_clean()`.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

/// One step of a raw scripted response, for timeout and stall scenarios.
#[derive(Clone, Debug)]
pub enum RawStep {
    Bytes(Vec<u8>),
    SleepMs(u64),
}

#[derive(Clone, Debug)]
enum Scripted {
    Json { status: u16, body: String },
    Raw(Vec<RawStep>),
}

#[derive(Clone, Debug)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    /// Header names lowercased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Option<Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

struct Shared {
    script: Mutex<VecDeque<Scripted>>,
    recorded: Mutex<Vec<Recorded>>,
    violations: Mutex<Vec<String>>,
    allow_prefix_break: AtomicBool,
}

pub struct MockServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
}

impl MockServer {
    pub fn start() -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let addr = listener.local_addr().unwrap();
        let shared = Arc::new(Shared {
            script: Mutex::new(VecDeque::new()),
            recorded: Mutex::new(Vec::new()),
            violations: Mutex::new(Vec::new()),
            allow_prefix_break: AtomicBool::new(false),
        });
        let accept_shared = shared.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let conn_shared = accept_shared.clone();
                std::thread::spawn(move || handle_connection(stream, conn_shared));
            }
        });
        MockServer { addr, shared }
    }

    /// Base URL including the /v1 prefix, e.g. `http://127.0.0.1:39321/v1`.
    pub fn base_url(&self) -> String {
        format!("http://{}/v1", self.addr)
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    /// Enqueue a standard non-streamed chat completion answering with `text`.
    pub fn enqueue_completion(&self, text: &str) {
        self.enqueue_json(
            200,
            json!({
                "id": "cmpl-mock", "object": "chat.completion", "created": 0, "model": "mock",
                "choices": [{"index": 0, "finish_reason": "stop",
                    "message": {"role": "assistant", "content": text}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5}
            }),
        );
    }

    pub fn enqueue_json(&self, status: u16, body: Value) {
        self.shared.script.lock().unwrap().push_back(Scripted::Json {
            status,
            body: body.to_string(),
        });
    }

    /// Enqueue raw bytes and sleeps; the connection closes when the steps end.
    pub fn enqueue_raw(&self, steps: Vec<RawStep>) {
        self.shared
            .script
            .lock()
            .unwrap()
            .push_back(Scripted::Raw(steps));
    }

    /// Enqueue an SSE response: each entry becomes one `data:` event, sent
    /// as one write. The body is close-delimited (no content-length), like
    /// a real streaming endpoint that ends by closing.
    pub fn enqueue_sse(&self, datas: &[&str]) {
        let mut steps = vec![RawStep::Bytes(sse_headers())];
        for d in datas {
            steps.push(RawStep::Bytes(format!("data: {d}\n\n").into_bytes()));
        }
        self.enqueue_raw(steps);
    }

    /// Enqueue a standard streamed chat completion answering with `text`:
    /// role chunk, per-word content deltas, finish chunk, usage chunk, and
    /// the `[DONE]` sentinel, the way llama.cpp and OpenAI stream it.
    pub fn enqueue_stream_completion(&self, text: &str) {
        let datas = chat_stream_datas(text);
        self.enqueue_sse(&datas.iter().map(String::as_str).collect::<Vec<_>>());
    }

    /// Declare that the NEXT request is a sanctioned prefix break
    /// (compaction, plan-mode entry or exit).
    pub fn expect_prefix_break(&self) {
        self.shared.allow_prefix_break.store(true, Ordering::SeqCst);
    }

    pub fn recorded(&self) -> Vec<Recorded> {
        self.shared.recorded.lock().unwrap().clone()
    }

    /// Panics with every collected wire violation. Call at the end of each test.
    pub fn assert_clean(&self) {
        let violations = self.shared.violations.lock().unwrap();
        assert!(
            violations.is_empty(),
            "mock server wire violations:\n  {}",
            violations.join("\n  ")
        );
    }
}

/// HTTP/1.1 response head + body helper for raw scripts.
pub fn http_response(status: u16, content_length: Option<usize>) -> Vec<u8> {
    let mut head = format!("HTTP/1.1 {status} MOCK\r\ncontent-type: application/json\r\n");
    if let Some(len) = content_length {
        head.push_str(&format!("content-length: {len}\r\n"));
    }
    head.push_str("\r\n");
    head.into_bytes()
}

/// Response head for a close-delimited SSE stream.
pub fn sse_headers() -> Vec<u8> {
    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\n\r\n"
        .to_vec()
}

/// The `data:` payloads of a standard streamed chat completion.
pub fn chat_stream_datas(text: &str) -> Vec<String> {
    fn chunk(delta: Value, finish: Value) -> String {
        json!({"id": "chatcmpl-mock", "object": "chat.completion.chunk", "created": 0,
            "model": "mock",
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
        .to_string()
    }
    let mut datas = vec![chunk(json!({"role": "assistant", "content": null}), Value::Null)];
    // Split into word-ish deltas so tests exercise real reassembly.
    let mut rest = text;
    while !rest.is_empty() {
        let cut = rest
            .char_indices()
            .filter(|(i, c)| *i > 0 && c.is_whitespace())
            .map(|(i, _)| i + 1)
            .find(|&i| i < rest.len())
            .unwrap_or(rest.len());
        let (piece, tail) = rest.split_at(cut.min(rest.len()));
        datas.push(chunk(json!({"content": piece}), Value::Null));
        rest = tail;
    }
    datas.push(chunk(json!({}), json!("stop")));
    datas.push(
        json!({"id": "chatcmpl-mock", "object": "chat.completion.chunk", "created": 0,
            "model": "mock", "choices": [],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5,
                "prompt_tokens_details": {"cached_tokens": 0}}})
        .to_string(),
    );
    datas.push("[DONE]".to_string());
    datas
}

/// Load an SSE fixture and split it into TCP-chunk byte vectors at every
/// `%%CHUNK%%` sentinel. The sentinel is removed and NOTHING else: place it
/// exactly at the intended boundary (mid-line and mid-codepoint are legal
/// and intended; that is the point of the format).
pub fn load_fixture_chunks(path: impl AsRef<std::path::Path>) -> Vec<Vec<u8>> {
    let bytes = std::fs::read(path.as_ref())
        .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.as_ref().display()));
    const SENTINEL: &[u8] = b"%%CHUNK%%";
    let mut chunks = Vec::new();
    let mut rest = &bytes[..];
    while let Some(pos) = rest
        .windows(SENTINEL.len())
        .position(|w| w == SENTINEL)
    {
        chunks.push(rest[..pos].to_vec());
        rest = &rest[pos + SENTINEL.len()..];
    }
    chunks.push(rest.to_vec());
    chunks.retain(|c| !c.is_empty());
    chunks
}

fn handle_connection(mut stream: TcpStream, shared: Arc<Shared>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    loop {
        let Some(req) = read_request(&mut stream) else { return };
        let is_api = req.path.ends_with("/chat/completions") || req.path.ends_with("/responses");
        if is_api {
            run_assertions(&shared, &req);
        }
        shared.recorded.lock().unwrap().push(req);
        let next = shared.script.lock().unwrap().pop_front();
        match next {
            Some(Scripted::Json { status, body }) => {
                let mut out = http_response(status, Some(body.len()));
                out.extend_from_slice(body.as_bytes());
                if stream.write_all(&out).is_err() {
                    return;
                }
                // keep-alive: fall through and read the next request
            }
            Some(Scripted::Raw(steps)) => {
                for step in steps {
                    match step {
                        RawStep::Bytes(b) => {
                            if stream.write_all(&b).is_err() || stream.flush().is_err() {
                                return;
                            }
                        }
                        RawStep::SleepMs(ms) => std::thread::sleep(Duration::from_millis(ms)),
                    }
                }
                return; // raw scripts close the connection
            }
            None => {
                shared
                    .violations
                    .lock()
                    .unwrap()
                    .push("request arrived with an empty script queue".to_string());
                let body = br#"{"error":"mock server script is empty"}"#;
                let mut out = http_response(500, Some(body.len()));
                out.extend_from_slice(body);
                let _ = stream.write_all(&out);
                return;
            }
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).ok()?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((name, value)) = h.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    let len: usize = headers
        .iter()
        .find(|(n, _)| n == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    if len > 0 {
        reader.read_exact(&mut body).ok()?;
    }
    Some(Recorded { method, path, headers, body })
}

// ---------------------------------------------------------------------------
// Automatic wire assertions
// ---------------------------------------------------------------------------

fn run_assertions(shared: &Shared, req: &Recorded) {
    let mut violations = Vec::new();
    let idx = shared.recorded.lock().unwrap().len();

    let Some(body) = req.json() else {
        violations.push(format!("request #{idx}: body is not valid JSON"));
        shared.violations.lock().unwrap().extend(violations);
        return;
    };

    // 1. No output-length cap of any kind, anywhere in the body.
    scan_cap_keys(&body, "$", &mut violations);

    // 2 + 3 need the conversation array.
    let array_key = if req.path.ends_with("/responses") { "input" } else { "messages" };
    if let Some(items) = body.get(array_key).and_then(Value::as_array) {
        let allow_break = shared.allow_prefix_break.swap(false, Ordering::SeqCst);
        if !allow_break {
            let recorded = shared.recorded.lock().unwrap();
            if let Some(prev) = recorded
                .iter()
                .rev()
                .find(|r| r.path == req.path)
                .and_then(|r| r.json())
            {
                if let Some(prev_items) = prev.get(array_key).and_then(Value::as_array) {
                    check_prefix(prev_items, items, idx, &mut violations);
                }
            }
        }
        if array_key == "messages" {
            check_chat_transcript(items, idx, &mut violations);
        } else {
            check_responses_transcript(items, idx, &mut violations);
        }
    }

    shared.violations.lock().unwrap().extend(violations);
}

fn scan_cap_keys(v: &Value, path: &str, violations: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                let lower = k.to_ascii_lowercase();
                if lower.contains("max") && lower.contains("token") {
                    violations.push(format!(
                        "output cap key {path}.{k} present; output length must never be capped"
                    ));
                }
                scan_cap_keys(child, &format!("{path}.{k}"), violations);
            }
        }
        Value::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                scan_cap_keys(child, &format!("{path}[{i}]"), violations);
            }
        }
        _ => {}
    }
}

fn check_prefix(prev: &[Value], next: &[Value], idx: usize, violations: &mut Vec<String>) {
    if next.len() < prev.len() {
        violations.push(format!(
            "request #{idx}: conversation array shrank from {} to {} items (prefix broken)",
            prev.len(),
            next.len()
        ));
        return;
    }
    for (i, item) in prev.iter().enumerate() {
        if &next[i] != item {
            violations.push(format!(
                "request #{idx}: conversation item {i} changed since the previous request \
                 (prefix broken); prev={item} next={}",
                next[i]
            ));
            return;
        }
    }
}

fn check_chat_transcript(messages: &[Value], idx: usize, violations: &mut Vec<String>) {
    let mut pending: VecDeque<String> = VecDeque::new();
    for (i, msg) in messages.iter().enumerate() {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
        if role == "tool" {
            let id = msg
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            match pending.pop_front() {
                Some(expected) if expected == id => {}
                Some(expected) => violations.push(format!(
                    "request #{idx}: messages[{i}] tool result id {id:?} out of order \
                     (expected {expected:?})"
                )),
                None => violations.push(format!(
                    "request #{idx}: messages[{i}] tool result {id:?} has no pending call"
                )),
            }
            continue;
        }
        if !pending.is_empty() {
            violations.push(format!(
                "request #{idx}: messages[{i}] ({role}) arrived while tool calls \
                 {pending:?} still await results"
            ));
            pending.clear();
        }
        if role == "assistant" {
            if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    let id = call.get("id").and_then(Value::as_str).unwrap_or("");
                    if id.is_empty() {
                        violations.push(format!(
                            "request #{idx}: messages[{i}] has a tool call without an id"
                        ));
                    }
                    pending.push_back(id.to_string());
                }
            }
        }
    }
    if !pending.is_empty() {
        violations.push(format!(
            "request #{idx}: transcript ends with unanswered tool calls {pending:?}"
        ));
    }
}

fn check_responses_transcript(input: &[Value], idx: usize, violations: &mut Vec<String>) {
    let mut pending: VecDeque<String> = VecDeque::new();
    for (i, item) in input.iter().enumerate() {
        match item.get("type").and_then(Value::as_str).unwrap_or("") {
            "function_call" => {
                let id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
                pending.push_back(id.to_string());
            }
            "function_call_output" => {
                let id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
                match pending.pop_front() {
                    Some(expected) if expected == id => {}
                    Some(expected) => violations.push(format!(
                        "request #{idx}: input[{i}] output id {id:?} out of order \
                         (expected {expected:?})"
                    )),
                    None => violations.push(format!(
                        "request #{idx}: input[{i}] function_call_output {id:?} \
                         has no pending call"
                    )),
                }
            }
            _ => {}
        }
    }
    if !pending.is_empty() {
        violations.push(format!(
            "request #{idx}: input ends with unanswered function calls {pending:?}"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> Shared {
        Shared {
            script: Mutex::new(VecDeque::new()),
            recorded: Mutex::new(Vec::new()),
            violations: Mutex::new(Vec::new()),
            allow_prefix_break: AtomicBool::new(false),
        }
    }

    fn req(path: &str, body: Value) -> Recorded {
        Recorded {
            method: "POST".into(),
            path: path.into(),
            headers: vec![],
            body: body.to_string().into_bytes(),
        }
    }

    #[test]
    fn flags_any_max_token_key_recursively() {
        let s = shared();
        run_assertions(
            &s,
            &req(
                "/v1/chat/completions",
                json!({"messages": [], "nested": {"max_output_tokens": 5}}),
            ),
        );
        let v = s.violations.lock().unwrap();
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].contains("max_output_tokens"));
    }

    #[test]
    fn flags_prefix_break_and_accepts_declared_break() {
        let s = shared();
        let first = req("/v1/chat/completions", json!({"messages": [{"role":"user","content":"a"}]}));
        run_assertions(&s, &first);
        s.recorded.lock().unwrap().push(first);
        // Changed first element: violation.
        run_assertions(
            &s,
            &req("/v1/chat/completions", json!({"messages": [{"role":"user","content":"CHANGED"}]})),
        );
        assert_eq!(s.violations.lock().unwrap().len(), 1);
        // Same change again, but declared: clean.
        s.violations.lock().unwrap().clear();
        s.allow_prefix_break.store(true, Ordering::SeqCst);
        run_assertions(
            &s,
            &req("/v1/chat/completions", json!({"messages": [{"role":"user","content":"CHANGED"}]})),
        );
        assert!(s.violations.lock().unwrap().is_empty());
    }

    #[test]
    fn flags_orphan_and_out_of_order_tool_results() {
        let s = shared();
        run_assertions(
            &s,
            &req(
                "/v1/chat/completions",
                json!({"messages": [
                    {"role":"assistant","tool_calls":[
                        {"id":"a","function":{"name":"x","arguments":"{}"}},
                        {"id":"b","function":{"name":"y","arguments":"{}"}}]},
                    {"role":"tool","tool_call_id":"b","content":"r"},
                    {"role":"tool","tool_call_id":"a","content":"r"}
                ]}),
            ),
        );
        assert!(!s.violations.lock().unwrap().is_empty());
    }

    #[test]
    fn accepts_valid_chat_transcript() {
        let s = shared();
        run_assertions(
            &s,
            &req(
                "/v1/chat/completions",
                json!({"messages": [
                    {"role":"user","content":"go"},
                    {"role":"assistant","tool_calls":[
                        {"id":"a","function":{"name":"x","arguments":"{}"}}]},
                    {"role":"tool","tool_call_id":"a","content":"r"},
                    {"role":"assistant","content":"done"}
                ]}),
            ),
        );
        let v = s.violations.lock().unwrap();
        assert!(v.is_empty(), "{v:?}");
    }

    #[test]
    fn responses_input_pairing_checked() {
        let s = shared();
        run_assertions(
            &s,
            &req(
                "/v1/responses",
                json!({"input": [
                    {"type":"function_call","call_id":"c1","name":"x","arguments":"{}"}
                ]}),
            ),
        );
        assert!(!s.violations.lock().unwrap().is_empty());
    }
}
