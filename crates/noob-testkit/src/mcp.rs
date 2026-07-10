//! Dev-only mock MCP server over Streamable HTTP (protocol 2025-11-25).
//! Serves initialize / notifications/initialized / tools/list / tools/call
//! from a configurable tool set, assigns and enforces `Mcp-Session-Id`, and
//! collects wire violations (missing `MCP-Protocol-Version` header, missing
//! Accept types) the way the OpenAI mock does; tests end with
//! `assert_clean()`.

use std::collections::{HashSet, VecDeque};
use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::Recorded;

struct Shared {
    tools: Vec<Value>,
    /// Respond to requests as one-event SSE streams instead of plain JSON.
    sse_mode: AtomicBool,
    /// Invalidate the current session before the next non-initialize
    /// request, forcing the client's one re-initialize retry.
    drop_session_once: AtomicBool,
    /// Answer the next tools/call with an endless keepalive trickle.
    trickle_next_call: AtomicBool,
    initializes: AtomicUsize,
    session_counter: AtomicUsize,
    sessions: Mutex<HashSet<String>>,
    /// Scripted `tools/call` results; empty falls back to an echo result.
    call_results: Mutex<VecDeque<Value>>,
    /// Every `tools/call` params object, in arrival order.
    calls: Mutex<Vec<Value>>,
    requests: Mutex<Vec<Recorded>>,
    violations: Mutex<Vec<String>>,
}

pub struct McpHttpServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
}

/// One tool definition for the mock's catalog.
pub fn tool(name: &str, description: &str, schema: Value) -> Value {
    json!({"name": name, "description": description, "inputSchema": schema})
}

/// The default tool set: one `echo` tool with a required string arg.
pub fn echo_tools() -> Vec<Value> {
    vec![tool(
        "echo",
        "echoes text back",
        json!({"type": "object", "properties": {"text": {"type": "string"}},
               "required": ["text"]}),
    )]
}

impl McpHttpServer {
    pub fn start(tools: Vec<Value>) -> McpHttpServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mcp mock");
        let addr = listener.local_addr().unwrap();
        let shared = Arc::new(Shared {
            tools,
            sse_mode: AtomicBool::new(false),
            drop_session_once: AtomicBool::new(false),
            trickle_next_call: AtomicBool::new(false),
            initializes: AtomicUsize::new(0),
            session_counter: AtomicUsize::new(0),
            sessions: Mutex::new(HashSet::new()),
            call_results: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
            violations: Mutex::new(Vec::new()),
        });
        let accept_shared = shared.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let conn = accept_shared.clone();
                std::thread::spawn(move || handle(stream, conn));
            }
        });
        McpHttpServer { addr, shared }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Respond as SSE streams from now on.
    pub fn sse_mode(&self) {
        self.shared.sse_mode.store(true, Ordering::SeqCst);
    }

    /// Invalidate the session before the next request (one 404).
    pub fn drop_session_once(&self) {
        self.shared.drop_session_once.store(true, Ordering::SeqCst);
    }

    /// The next tools/call answers with an SSE stream that sends keepalive
    /// comments forever and never the response: the wedged-server shape a
    /// per-call deadline must survive.
    pub fn trickle_next_call(&self) {
        self.shared.trickle_next_call.store(true, Ordering::SeqCst);
    }

    /// Enqueue one scripted `tools/call` result value.
    pub fn enqueue_call_result(&self, result: Value) {
        self.shared.call_results.lock().unwrap().push_back(result);
    }

    pub fn initialize_count(&self) -> usize {
        self.shared.initializes.load(Ordering::SeqCst)
    }

    pub fn calls(&self) -> Vec<Value> {
        self.shared.calls.lock().unwrap().clone()
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.shared.requests.lock().unwrap().clone()
    }

    pub fn assert_clean(&self) {
        let violations = self.shared.violations.lock().unwrap();
        assert!(
            violations.is_empty(),
            "mcp mock wire violations:\n  {}",
            violations.join("\n  ")
        );
    }
}

fn handle(mut stream: TcpStream, shared: Arc<Shared>) {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
    loop {
        let Some(req) = crate::read_request(&mut stream) else { return };
        let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        let method = body.get("method").and_then(Value::as_str).unwrap_or("");
        let id = body.get("id").cloned();
        check_wire(&shared, &req, method);
        let session = req.header("mcp-session-id").map(str::to_string);
        shared.requests.lock().unwrap().push(req);

        // Session enforcement per the spec: a request against an unknown
        // session gets 404 (the client should re-initialize).
        if method != "initialize" {
            if shared.drop_session_once.swap(false, Ordering::SeqCst) {
                if let Some(s) = &session {
                    shared.sessions.lock().unwrap().remove(s);
                }
            }
            let known = session
                .as_ref()
                .is_some_and(|s| shared.sessions.lock().unwrap().contains(s));
            if !known {
                if write_simple(&mut stream, 404, "{}", None).is_err() {
                    return;
                }
                continue;
            }
        }

        let response = match method {
            "initialize" => {
                shared.initializes.fetch_add(1, Ordering::SeqCst);
                let sess = format!(
                    "sess-{}",
                    shared.session_counter.fetch_add(1, Ordering::SeqCst)
                );
                shared.sessions.lock().unwrap().insert(sess.clone());
                let result = json!({
                    "protocolVersion": "2025-11-25",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "mcp-mock", "version": "0"}
                });
                Some((rpc_result(&id, result), Some(sess)))
            }
            m if m.starts_with("notifications/") => {
                // Notifications get 202 Accepted with no body.
                if write_simple(&mut stream, 202, "", session.as_deref()).is_err() {
                    return;
                }
                continue;
            }
            "tools/list" => {
                Some((rpc_result(&id, json!({"tools": shared.tools})), None))
            }
            "tools/call" => {
                let params = body.get("params").cloned().unwrap_or(Value::Null);
                shared.calls.lock().unwrap().push(params.clone());
                if shared.trickle_next_call.swap(false, Ordering::SeqCst) {
                    // Keepalives forever; the write fails once the client
                    // gives up and closes, which ends this connection.
                    let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n";
                    if stream.write_all(head.as_bytes()).is_err() {
                        return;
                    }
                    loop {
                        if stream.write_all(b": keepalive\n\n").is_err()
                            || stream.flush().is_err()
                        {
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
                let scripted = shared.call_results.lock().unwrap().pop_front();
                let result = scripted.unwrap_or_else(|| {
                    json!({"content": [{"type": "text",
                        "text": format!("echo: {}", params.get("arguments").unwrap_or(&Value::Null))}],
                        "isError": false})
                });
                Some((rpc_result(&id, result), None))
            }
            other => Some((
                json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": -32601, "message": format!("unknown method {other}")}}),
                None,
            )),
        };

        let Some((msg, new_session)) = response else { return };
        if shared.sse_mode.load(Ordering::SeqCst) {
            // SSE bodies are close-delimited: one response per connection.
            let _ = write_sse(&mut stream, &msg, new_session.as_deref().or(session.as_deref()));
            return;
        }
        if write_simple(
            &mut stream,
            200,
            &msg.to_string(),
            new_session.as_deref().or(session.as_deref()),
        )
        .is_err()
        {
            return;
        }
    }
}

fn rpc_result(id: &Option<Value>, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id.clone().unwrap_or(Value::Null), "result": result})
}

fn check_wire(shared: &Shared, req: &Recorded, method: &str) {
    let mut violations = Vec::new();
    let accept = req.header("accept").unwrap_or("");
    if !accept.contains("application/json") || !accept.contains("text/event-stream") {
        violations.push(format!(
            "{method}: Accept must offer application/json and text/event-stream, got {accept:?}"
        ));
    }
    if method != "initialize" && req.header("mcp-protocol-version").is_none() {
        violations.push(format!(
            "{method}: MCP-Protocol-Version header missing on a post-initialize request"
        ));
    }
    if req.header("content-type").map(|c| !c.starts_with("application/json")) == Some(true) {
        violations.push(format!("{method}: content-type is not application/json"));
    }
    shared.violations.lock().unwrap().extend(violations);
}

fn write_simple(
    stream: &mut TcpStream,
    status: u16,
    body: &str,
    session: Option<&str>,
) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {status} MOCK\r\ncontent-type: application/json\r\n");
    if let Some(s) = session {
        head.push_str(&format!("mcp-session-id: {s}\r\n"));
    }
    head.push_str(&format!("content-length: {}\r\n\r\n", body.len()));
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}

/// One JSON-RPC message as a single-event SSE stream, close-delimited the
/// way real Streamable HTTP servers end a per-request stream.
fn write_sse(stream: &mut TcpStream, msg: &Value, session: Option<&str>) -> std::io::Result<()> {
    let mut head =
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\n"
            .to_string();
    if let Some(s) = session {
        head.push_str(&format!("mcp-session-id: {s}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    // A keepalive comment first, so clients prove they skip them.
    stream.write_all(b": keepalive\n\n")?;
    stream.write_all(format!("event: message\ndata: {msg}\n\n").as_bytes())?;
    stream.flush()?;
    stream.shutdown(std::net::Shutdown::Write)
}
