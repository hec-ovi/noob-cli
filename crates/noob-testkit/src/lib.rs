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

pub mod mcp;

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::Ordering;
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
    /// Like Raw, but keeps the connection open for the next request (the
    /// bytes must be a self-delimiting response, e.g. chunked encoding).
    RawKeepAlive(Vec<RawStep>),
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
    /// How many prefix MISMATCHES are sanctioned. Consumed only when a
    /// mismatch actually happens, so tests can arm allowances up front
    /// (before spawning a binary that compacts mid-run).
    allowed_prefix_breaks: std::sync::atomic::AtomicUsize,
    /// Separate from prefix breaks: the tools array must stay byte-stable
    /// even across sanctioned message breaks (compaction). Plan mode (P5)
    /// is the first legitimate consumer.
    allowed_tools_changes: std::sync::atomic::AtomicUsize,
    connections: std::sync::atomic::AtomicUsize,
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
            allowed_prefix_breaks: std::sync::atomic::AtomicUsize::new(0),
            allowed_tools_changes: std::sync::atomic::AtomicUsize::new(0),
            connections: std::sync::atomic::AtomicUsize::new(0),
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

    /// Enqueue raw bytes that leave the connection open afterwards. The
    /// bytes must form a self-delimiting response (chunked encoding or a
    /// content-length), or the client will wait forever for the body end.
    pub fn enqueue_raw_keepalive(&self, steps: Vec<RawStep>) {
        self.shared
            .script
            .lock()
            .unwrap()
            .push_back(Scripted::RawKeepAlive(steps));
    }

    /// How many TCP connections the server has accepted. Lets tests assert
    /// keep-alive reuse (two requests, one connection).
    pub fn connection_count(&self) -> usize {
        self.shared.connections.load(Ordering::SeqCst)
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

    /// Sanction one future prefix mismatch (compaction, plan-mode entry or
    /// exit, a fresh session against the same server). Call N times to
    /// allow N; the allowance is consumed only when a mismatch happens.
    pub fn expect_prefix_break(&self) {
        self.shared
            .allowed_prefix_breaks
            .fetch_add(1, Ordering::SeqCst);
    }

    /// Sanction one future tools-array change (plan-mode entry or exit,
    /// P5). Consumed only when a change happens.
    pub fn expect_tools_change(&self) {
        self.shared
            .allowed_tools_changes
            .fetch_add(1, Ordering::SeqCst);
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

/// A complete chunked-encoded SSE response (the framing llama.cpp and
/// OpenAI actually use): one chunk per event, terminated properly, so the
/// connection can be kept alive. Pair with `enqueue_raw_keepalive`.
pub fn chunked_sse_response(datas: &[&str]) -> Vec<u8> {
    let mut out = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n"
        .to_vec();
    for d in datas {
        let event = format!("data: {d}\n\n");
        out.extend_from_slice(format!("{:x}\r\n", event.len()).as_bytes());
        out.extend_from_slice(event.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"0\r\n\r\n");
    out
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

/// The `data:` payloads of a streamed chat completion that answers with
/// tool calls (llama.cpp shape: id+name first, argument fragments after,
/// `finish_reason: "tool_calls"`). `usage` overrides the prompt/completion
/// token counts, for tests that force compaction.
pub fn chat_stream_toolcalls_datas(
    calls: &[(&str, &str, &str)],
    usage: Option<(u64, u64)>,
) -> Vec<String> {
    fn chunk(delta: Value, finish: Value) -> String {
        json!({"id": "chatcmpl-mock", "object": "chat.completion.chunk", "created": 0,
            "model": "mock",
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
        .to_string()
    }
    let mut datas = vec![chunk(json!({"role": "assistant", "content": null}), Value::Null)];
    for (i, (id, name, args)) in calls.iter().enumerate() {
        datas.push(chunk(
            json!({"tool_calls": [{"index": i, "id": id, "type": "function",
                "function": {"name": name, "arguments": ""}}]}),
            Value::Null,
        ));
        // Argument fragments split mid-JSON, the way real servers stream.
        let (a, b) = args.split_at(args.len() / 2);
        for frag in [a, b] {
            if !frag.is_empty() {
                datas.push(chunk(
                    json!({"tool_calls": [{"index": i,
                        "function": {"arguments": frag}}]}),
                    Value::Null,
                ));
            }
        }
    }
    datas.push(chunk(json!({}), json!("tool_calls")));
    let (p, c) = usage.unwrap_or((10, 5));
    datas.push(
        json!({"id": "chatcmpl-mock", "object": "chat.completion.chunk", "created": 0,
            "model": "mock", "choices": [],
            "usage": {"prompt_tokens": p, "completion_tokens": c,
                "prompt_tokens_details": {"cached_tokens": 0}}})
        .to_string(),
    );
    datas.push("[DONE]".to_string());
    datas
}

/// Enqueue a streamed completion calling the given tools.
impl MockServer {
    pub fn enqueue_stream_toolcalls(
        &self,
        calls: &[(&str, &str, &str)],
        usage: Option<(u64, u64)>,
    ) {
        let datas = chat_stream_toolcalls_datas(calls, usage);
        self.enqueue_sse(&datas.iter().map(String::as_str).collect::<Vec<_>>());
    }
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
    shared.connections.fetch_add(1, Ordering::SeqCst);
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
                if !write_steps(&mut stream, steps) {
                    return;
                }
                return; // raw scripts close the connection
            }
            Some(Scripted::RawKeepAlive(steps)) => {
                if !write_steps(&mut stream, steps) {
                    return;
                }
                // fall through: keep reading on this connection
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

fn write_steps(stream: &mut TcpStream, steps: Vec<RawStep>) -> bool {
    for step in steps {
        match step {
            RawStep::Bytes(b) => {
                if stream.write_all(&b).is_err() || stream.flush().is_err() {
                    return false;
                }
            }
            RawStep::SleepMs(ms) => std::thread::sleep(Duration::from_millis(ms)),
        }
    }
    true
}

pub(crate) fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
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
        // Prefix stability is byte-exact on the serialized array: llama.cpp
        // KV reuse is byte-sensitive, so a serializer that merely reorders
        // keys between turns is a real cache bust and must be caught.
        // Tools-array drift is the same class of bust (the schemas sit
        // before the transcript in the rendered prompt).
        let mismatch = {
            let recorded = shared.recorded.lock().unwrap();
            let prev = recorded.iter().rev().find(|r| r.path == req.path);
            prev.and_then(|prev| {
                let prev_raw = raw_top_level_value(&prev.body, array_key);
                let next_raw = raw_top_level_value(&req.body, array_key);
                if let (Some(p), Some(n)) = (&prev_raw, &next_raw) {
                    if let Some(v) = check_byte_prefix(p, n, idx) {
                        return Some(v);
                    }
                }
                // Structural fallback for a readable message when the raw
                // scan is unavailable.
                prev.json().and_then(|prev| {
                    prev.get(array_key)
                        .and_then(Value::as_array)
                        .and_then(|prev_items| check_prefix(prev_items, items, idx))
                })
            })
        };
        if let Some(v) = mismatch {
            // A sanctioned break burns one allowance; otherwise it is a
            // violation.
            let allowed = self_dec(&shared.allowed_prefix_breaks);
            if !allowed {
                violations.push(v);
            }
        }
        // Tools-array stability is checked INDEPENDENTLY of the messages
        // prefix: a prefix-break allowance (compaction) must never swallow
        // tools drift, and the baseline is the most recent request on this
        // path that carried a tools key at all (the summarizer sends none,
        // which must not blind the comparison across it).
        let tools_drift = {
            let recorded = shared.recorded.lock().unwrap();
            raw_top_level_value(&req.body, "tools").and_then(|next| {
                recorded
                    .iter()
                    .rev()
                    .filter(|r| r.path == req.path)
                    .find_map(|r| raw_top_level_value(&r.body, "tools"))
                    .and_then(|prevt| {
                        (prevt != next).then(|| {
                            format!(
                                "request #{idx}: the tools array changed since the last \
                                 request that carried one (cache prefix broken)"
                            )
                        })
                    })
            })
        };
        if let Some(v) = tools_drift {
            if !self_dec(&shared.allowed_tools_changes) {
                violations.push(v);
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

/// The raw byte slice of one top-level key's value in a JSON object body.
/// A tiny scanner (string/escape/depth aware), NOT a parser: it exists so
/// the prefix assertion can compare the exact bytes the client serialized,
/// which is what llama.cpp's KV prefix cache sees.
pub fn raw_top_level_value(body: &[u8], key: &str) -> Option<Vec<u8>> {
    let n = body.len();
    let mut i = 0;
    while i < n && body[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= n || body[i] != b'{' {
        return None;
    }
    i += 1;
    loop {
        while i < n && body[i].is_ascii_whitespace() {
            i += 1;
        }
        match body.get(i)? {
            b'}' => return None,
            b',' => {
                i += 1;
                continue;
            }
            b'"' => {}
            _ => return None,
        }
        let (k, after) = scan_string(body, i)?;
        i = after;
        while i < n && body[i].is_ascii_whitespace() {
            i += 1;
        }
        if body.get(i) != Some(&b':') {
            return None;
        }
        i += 1;
        while i < n && body[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        let end = scan_value(body, i)?;
        if k == key.as_bytes() {
            return Some(body[start..end].to_vec());
        }
        i = end;
    }
}

/// Returns (contents without quotes, index just past the closing quote).
fn scan_string(b: &[u8], start: usize) -> Option<(&[u8], usize)> {
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some((&b[start + 1..i], i + 1)),
            _ => i += 1,
        }
    }
    None
}

/// Index just past one JSON value starting at `start`.
fn scan_value(b: &[u8], start: usize) -> Option<usize> {
    match b.get(start)? {
        b'"' => scan_string(b, start).map(|(_, e)| e),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut i = start;
            while i < b.len() {
                match b[i] {
                    b'"' => {
                        i = scan_string(b, i)?.1;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(i + 1);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            None
        }
        _ => {
            let mut i = start;
            while i < b.len()
                && !matches!(b[i], b',' | b'}' | b']')
                && !b[i].is_ascii_whitespace()
            {
                i += 1;
            }
            Some(i)
        }
    }
}

/// Byte-exact prefix rule for a serialized JSON array: the next request's
/// array must be the previous one with zero or more items appended (same
/// bytes up to the previous closing bracket).
fn check_byte_prefix(prev: &[u8], next: &[u8], idx: usize) -> Option<String> {
    if prev == next || prev == b"[]" {
        return None;
    }
    if prev.last() == Some(&b']') && next.first() == Some(&b'[') {
        let prefix = &prev[..prev.len() - 1];
        if next.starts_with(prefix) && next.get(prefix.len()) == Some(&b',') {
            return None;
        }
    }
    let common = prev
        .iter()
        .zip(next.iter())
        .take_while(|(a, b)| a == b)
        .count();
    Some(format!(
        "request #{idx}: the serialized conversation array is not a byte-prefix \
         extension of the previous request (diverges at byte {common}); a \
         re-serialization that only reorders keys still busts the KV cache"
    ))
}

/// Atomically consume one allowance; false when none are left.
fn self_dec(counter: &std::sync::atomic::AtomicUsize) -> bool {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok()
}

/// The first prefix mismatch between consecutive requests, if any.
fn check_prefix(prev: &[Value], next: &[Value], idx: usize) -> Option<String> {
    if next.len() < prev.len() {
        return Some(format!(
            "request #{idx}: conversation array shrank from {} to {} items (prefix broken)",
            prev.len(),
            next.len()
        ));
    }
    for (i, item) in prev.iter().enumerate() {
        if &next[i] != item {
            return Some(format!(
                "request #{idx}: conversation item {i} changed since the previous request \
                 (prefix broken); prev={item} next={}",
                next[i]
            ));
        }
    }
    None
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
            allowed_prefix_breaks: std::sync::atomic::AtomicUsize::new(0),
            allowed_tools_changes: std::sync::atomic::AtomicUsize::new(0),
            connections: std::sync::atomic::AtomicUsize::new(0),
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
        // Same change again, but declared: clean, and the allowance is
        // consumed only by the mismatch.
        s.violations.lock().unwrap().clear();
        s.allowed_prefix_breaks.store(2, Ordering::SeqCst);
        run_assertions(
            &s,
            &req("/v1/chat/completions", json!({"messages": [{"role":"user","content":"CHANGED"}]})),
        );
        assert!(s.violations.lock().unwrap().is_empty());
        assert_eq!(s.allowed_prefix_breaks.load(Ordering::SeqCst), 1);
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
    fn raw_scanner_finds_the_real_key_not_a_string_decoy() {
        // The system prompt CONTAINS the text "messages":[ inside a string;
        // the scanner must skip it and return the real top-level array.
        let body = json!({
            "model": "m",
            "note": "docs say \"messages\":[{}] goes here",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        })
        .to_string();
        let raw = raw_top_level_value(body.as_bytes(), "messages").unwrap();
        assert_eq!(
            String::from_utf8(raw).unwrap(),
            r#"[{"content":"hi","role":"user"}]"#
        );
        assert!(raw_top_level_value(body.as_bytes(), "tools").is_none());
        assert_eq!(
            raw_top_level_value(body.as_bytes(), "stream").unwrap(),
            b"true"
        );
    }

    #[test]
    fn byte_prefix_catches_key_reordering_that_value_compare_misses() {
        // Same structural items, different key order: a real KV-cache bust.
        let prev = br#"[{"content":"a","role":"user"}]"#;
        let next_ok = br#"[{"content":"a","role":"user"},{"content":"b","role":"user"}]"#;
        let next_reordered = br#"[{"role":"user","content":"a"},{"content":"b","role":"user"}]"#;
        assert!(check_byte_prefix(prev, next_ok, 1).is_none());
        assert!(check_byte_prefix(prev, prev, 1).is_none());
        let v = check_byte_prefix(prev, next_reordered, 1).unwrap();
        assert!(v.contains("byte-prefix"), "{v}");
    }

    #[test]
    fn tools_array_drift_is_flagged() {
        let s = shared();
        let mk = |tools: Value| {
            req(
                "/v1/chat/completions",
                json!({"messages": [{"role":"user","content":"a"}], "tools": tools}),
            )
        };
        let first = mk(json!([{"type":"function","function":{"name":"read"}}]));
        run_assertions(&s, &first);
        s.recorded.lock().unwrap().push(first);
        // Same messages, different tools serialization: violation.
        run_assertions(&s, &mk(json!([{"type":"function","function":{"name":"write"}}])));
        let v: Vec<String> = std::mem::take(&mut s.violations.lock().unwrap());
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].contains("tools array changed"), "{}", v[0]);
    }

    #[test]
    fn tools_drift_is_caught_across_a_toolless_request_and_a_message_break() {
        // The compaction shape: a summarizer request with NO tools key and
        // a full message-prefix break sits between two normal requests. The
        // prefix allowance must not swallow tools drift, and the toolless
        // request must not blind the comparison.
        let s = shared();
        let normal = |tools_name: &str, msg: &str| {
            req(
                "/v1/chat/completions",
                json!({"messages": [{"role":"user","content":msg}],
                       "tools": [{"type":"function","function":{"name":tools_name}}]}),
            )
        };
        let first = normal("read", "a");
        run_assertions(&s, &first);
        s.recorded.lock().unwrap().push(first);
        // Summarizer: different messages (sanctioned break), no tools key.
        s.allowed_prefix_breaks.store(2, Ordering::SeqCst);
        let summarize = req(
            "/v1/chat/completions",
            json!({"messages": [{"role":"user","content":"summarize"}]}),
        );
        run_assertions(&s, &summarize);
        s.recorded.lock().unwrap().push(summarize);
        // Continuation: another sanctioned message break, but the tools
        // array DRIFTED vs request 1. That must still be a violation.
        run_assertions(&s, &normal("write", "b"));
        let v = s.violations.lock().unwrap();
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].contains("tools array changed"), "{}", v[0]);
    }

    #[test]
    fn declared_tools_change_is_accepted_once() {
        let s = shared();
        let mk = |name: &str| {
            req(
                "/v1/chat/completions",
                json!({"messages": [{"role":"user","content":"a"}],
                       "tools": [{"type":"function","function":{"name":name}}]}),
            )
        };
        let first = mk("read");
        run_assertions(&s, &first);
        s.recorded.lock().unwrap().push(first);
        s.allowed_tools_changes.store(1, Ordering::SeqCst);
        run_assertions(&s, &mk("plan_mode_set"));
        let v = s.violations.lock().unwrap();
        assert!(v.is_empty(), "{v:?}");
        assert_eq!(s.allowed_tools_changes.load(Ordering::SeqCst), 0);
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
