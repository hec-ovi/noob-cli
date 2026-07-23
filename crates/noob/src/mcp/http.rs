//! MCP Streamable HTTP transport (protocol 2025-11-25): JSON-RPC over POST
//! with `Accept: application/json, text/event-stream`, both response body
//! types handled, `Mcp-Session-Id` captured and replayed, one transparent
//! re-initialize when the server answers 404 (expired session), and
//! `MCP-Protocol-Version` on every post-initialize request. All HTTP goes
//! through noob-provider's client (the egress invariant) with retries off:
//! a tools/call may have side effects, so a silent replay is never safe.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

use noob_provider::http::{Client, RetryPolicy, StreamBody, Timeouts};
use noob_provider::sse::SseParser;
use noob_provider::types::ProviderError;

use super::proto::{self, Inbound};

const MAX_MCP_JSON_BODY: usize = 8 * 1024 * 1024;

pub struct HttpTransport {
    url: String,
    timeout: Duration,
    client: Client,
    state: Mutex<HttpState>,
}

struct HttpState {
    session: Option<String>,
    /// Negotiated protocol version; None until the initialize handshake ran.
    protocol: Option<String>,
    next_id: u64,
}

type PostOutcome = (Option<Result<Value, String>>, Option<String>);

impl HttpTransport {
    pub fn new(url: &str, timeout: Duration) -> HttpTransport {
        let timeouts = Timeouts {
            connect: Duration::from_secs(10).min(timeout),
            first_byte: timeout,
            idle: timeout,
        };
        HttpTransport {
            url: url.to_string(),
            timeout,
            client: Client::with_retry(timeouts, RetryPolicy::none()),
            state: Mutex::new(HttpState {
                session: None,
                protocol: None,
                next_id: 1,
            }),
        }
    }

    /// Initialize handshake if not done yet; returns the negotiated version.
    pub fn ensure_ready(&self) -> Result<String, String> {
        let mut state = self.state.lock().unwrap();
        self.ensure_locked(&mut state)?;
        Ok(state.protocol.clone().expect("initialized"))
    }

    /// One JSON-RPC request. A 404 (expired session) triggers exactly one
    /// re-initialize and one retry of the request.
    pub fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let mut state = self.state.lock().unwrap();
        self.ensure_locked(&mut state)?;
        match self.rpc_locked(&mut state, method, params.clone()) {
            Err(RpcFailure::SessionGone) => {
                state.session = None;
                state.protocol = None;
                self.ensure_locked(&mut state)?;
                self.rpc_locked(&mut state, method, params)
                    .map_err(RpcFailure::into_message)?
            }
            other => other.map_err(RpcFailure::into_message)?,
        }
    }

    fn ensure_locked(&self, state: &mut HttpState) -> Result<(), String> {
        if state.protocol.is_some() {
            return Ok(());
        }
        let id = state.next_id;
        state.next_id += 1;
        let init = proto::request(id, "initialize", proto::initialize_params());
        let (outcome, session) = self
            .post_and_parse(state, &init, Some(id))
            .map_err(RpcFailure::into_message)?;
        let result =
            outcome.ok_or_else(|| "the server sent no response to initialize".to_string())??;
        if let Some(sess) = session {
            state.session = Some(sess);
        }
        state.protocol = Some(
            result
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(proto::PROTOCOL_VERSION)
                .to_string(),
        );
        // notifications/initialized completes the handshake; servers answer
        // 202 with no body. A failed note leaves the server side
        // uninitialized, so the local half rolls back too: keeping
        // protocol/session here would make every later call skip the
        // handshake against a session the server never completed.
        let note = proto::notification("notifications/initialized");
        if let Err(e) = self.post_and_parse(state, &note, None) {
            state.protocol = None;
            state.session = None;
            return Err(RpcFailure::into_message(e));
        }
        Ok(())
    }

    fn rpc_locked(
        &self,
        state: &mut HttpState,
        method: &str,
        params: Value,
    ) -> Result<Result<Value, String>, RpcFailure> {
        let id = state.next_id;
        state.next_id += 1;
        let msg = proto::request(id, method, params);
        let (outcome, _) = self.post_and_parse(state, &msg, Some(id))?;
        outcome.ok_or_else(|| {
            RpcFailure::Other(format!(
                "the MCP server at {} closed the response without answering {method}",
                self.url
            ))
        })
    }

    /// POST one JSON-RPC message and parse whichever body shape comes back.
    /// Returns (response outcome if one was expected, session header).
    fn post_and_parse(
        &self,
        state: &HttpState,
        msg: &Value,
        want_id: Option<u64>,
    ) -> Result<PostOutcome, RpcFailure> {
        let mut headers: Vec<(String, String)> = vec![(
            "accept".to_string(),
            "application/json, text/event-stream".to_string(),
        )];
        if let Some(protocol) = &state.protocol {
            headers.push(("mcp-protocol-version".to_string(), protocol.clone()));
        }
        if let Some(session) = &state.session {
            headers.push(("mcp-session-id".to_string(), session.clone()));
        }
        let mut body = msg.clone();
        let mut stream = self
            .client
            .post_json_stream_with(&self.url, &headers, &mut body)
            .map_err(|e| self.map_error(e))?;
        let session = stream.header("mcp-session-id").map(str::to_string);

        // 202/204: an accepted notification; there is no body to parse.
        if matches!(stream.status(), 202 | 204) {
            return Ok((None, session));
        }
        let media = stream.media_type();
        if media == "text/event-stream" {
            let Some(want) = want_id else {
                // A notification answered with a stream: drain nothing, done.
                return Ok((None, session));
            };
            let outcome = self.scan_sse(&mut stream, want)?;
            return Ok((Some(outcome), session));
        }
        // Default: a single JSON body.
        let bytes = self.read_json_bounded(&mut stream)?;
        if want_id.is_none() {
            return Ok((None, session));
        }
        let parsed: Value = serde_json::from_slice(&bytes).map_err(|e| {
            RpcFailure::Other(format!(
                "the MCP server at {} sent unparseable JSON ({e}); is the url \
                 really an MCP endpoint?",
                self.url
            ))
        })?;
        match proto::classify(&parsed) {
            Inbound::Response { id, outcome } if Some(id) == want_id => {
                Ok((Some(outcome), session))
            }
            _ => Err(RpcFailure::Other(format!(
                "the MCP server at {} answered with a message that is not the \
                 response to this request",
                self.url
            ))),
        }
    }

    /// Read a plain-JSON body to its end under the same ABSOLUTE per-call
    /// deadline scan_sse enforces: the provider watchdog's idle clock resets
    /// on every byte, so a server trickling the body one byte at a time
    /// would otherwise pin the call far past its timeout.
    fn read_json_bounded(&self, stream: &mut StreamBody) -> Result<Vec<u8>, RpcFailure> {
        let deadline = Instant::now() + self.timeout;
        let mut bytes = Vec::new();
        let mut buf = [0u8; 8 * 1024];
        loop {
            if Instant::now() >= deadline {
                return Err(RpcFailure::Other(format!(
                    "the MCP call timed out after {}s: the server at {} kept trickling \
                     the response without finishing it; retry or raise timeout_s in \
                     mcp.json",
                    self.timeout.as_secs(),
                    self.url
                )));
            }
            let n = stream.read(&mut buf).map_err(|e| self.map_error(e))?;
            if n == 0 {
                return Ok(bytes);
            }
            if bytes.len().saturating_add(n) > MAX_MCP_JSON_BODY {
                return Err(RpcFailure::Other(format!(
                    "the MCP server at {} sent more than 8 MiB in one response; ask \
                     the tool for less data or fix the server response",
                    self.url
                )));
            }
            bytes.extend_from_slice(&buf[..n]);
        }
    }

    /// Read SSE events until the response with `want` arrives. Server
    /// requests over the stream are ignored (tools-only client). An
    /// ABSOLUTE deadline bounds the whole scan: the provider watchdog's
    /// idle clock resets on every byte, so a server trickling keepalive
    /// comments forever would otherwise never trip it (the stdio transport
    /// has the same absolute guarantee via its recv deadline).
    fn scan_sse(
        &self,
        stream: &mut StreamBody,
        want: u64,
    ) -> Result<Result<Value, String>, RpcFailure> {
        let deadline = Instant::now() + self.timeout;
        let mut parser = SseParser::new();
        let mut events = Vec::new();
        let mut buf = [0u8; 8 * 1024];
        let mut received = 0usize;
        loop {
            if Instant::now() >= deadline {
                return Err(RpcFailure::Other(format!(
                    "the MCP call timed out after {}s: the server at {} kept the \
                     stream alive without answering; retry or raise timeout_s in \
                     mcp.json",
                    self.timeout.as_secs(),
                    self.url
                )));
            }
            let n = stream.read(&mut buf).map_err(|e| self.map_error(e))?;
            received = received.saturating_add(n);
            if received > MAX_MCP_JSON_BODY {
                return Err(RpcFailure::Other(format!(
                    "the MCP server at {} sent more than 8 MiB without answering; ask the tool \
                     for less data or fix the server response",
                    self.url
                )));
            }
            if n == 0 {
                parser.finish(&mut events);
            } else {
                parser.feed(&buf[..n], &mut events);
            }
            for ev in events.drain(..) {
                let Ok(msg) = serde_json::from_str::<Value>(&ev.data) else {
                    continue;
                };
                if let Inbound::Response { id, outcome } = proto::classify(&msg)
                    && id == want
                {
                    return Ok(outcome);
                }
            }
            if n == 0 {
                return Err(RpcFailure::Other(format!(
                    "the MCP server at {} ended the stream without answering",
                    self.url
                )));
            }
        }
    }

    fn map_error(&self, e: ProviderError) -> RpcFailure {
        match e {
            ProviderError::Http { status: 404, .. } => RpcFailure::SessionGone,
            ProviderError::Http { status, body } => {
                let shown: String = body.trim().chars().take(300).collect();
                RpcFailure::Other(format!(
                    "the MCP server at {} returned HTTP {status}: {shown}",
                    self.url
                ))
            }
            ProviderError::Timeout(_) => RpcFailure::Other(format!(
                "the MCP call timed out after {}s; the server at {} may be stuck, \
                 retry or raise timeout_s in mcp.json",
                self.timeout.as_secs(),
                self.url
            )),
            other => RpcFailure::Other(format!("MCP server {}: {other}", self.url)),
        }
    }
}

enum RpcFailure {
    /// HTTP 404: the session expired; the caller re-initializes once.
    SessionGone,
    Other(String),
}

impl RpcFailure {
    fn into_message(self) -> String {
        match self {
            // A 404 that survives the one re-initialize retry.
            RpcFailure::SessionGone => {
                "the MCP server returned 404 even after a fresh initialize; check \
                 that the url in mcp.json points at an MCP endpoint"
                    .to_string()
            }
            RpcFailure::Other(msg) => msg,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use noob_testkit::mcp::{McpHttpServer, echo_tools};
    use serde_json::json;

    fn transport(server: &McpHttpServer) -> HttpTransport {
        HttpTransport::new(&server.url(), Duration::from_secs(5))
    }

    #[test]
    fn handshake_list_call_with_session_and_version_headers() {
        let server = McpHttpServer::start(echo_tools());
        let t = transport(&server);
        assert_eq!(t.ensure_ready().unwrap(), "2025-11-25");
        let listed = t.request("tools/list", json!({})).unwrap();
        assert_eq!(listed["tools"][0]["name"], "echo");
        let result = t
            .request(
                "tools/call",
                json!({"name": "echo", "arguments": {"text": "hi"}}),
            )
            .unwrap();
        assert!(
            result["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("hi")
        );
        // The session assigned at initialize was replayed on every request.
        let reqs = server.requests();
        assert!(reqs.len() >= 4);
        for r in &reqs[1..] {
            assert_eq!(
                r.header("mcp-session-id"),
                Some("sess-0"),
                "session not replayed"
            );
        }
        server.assert_clean();
    }

    #[test]
    fn expired_session_reinitializes_exactly_once_and_retries() {
        let server = McpHttpServer::start(echo_tools());
        let t = transport(&server);
        t.ensure_ready().unwrap();
        server.drop_session_once();
        let result = t
            .request(
                "tools/call",
                json!({"name": "echo", "arguments": {"text": "again"}}),
            )
            .unwrap();
        assert!(
            result["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("again")
        );
        assert_eq!(server.initialize_count(), 2, "exactly one re-initialize");
        server.assert_clean();
    }

    #[test]
    fn sse_response_bodies_are_parsed() {
        let server = McpHttpServer::start(echo_tools());
        server.sse_mode();
        let t = transport(&server);
        t.ensure_ready().unwrap();
        let listed = t.request("tools/list", json!({})).unwrap();
        assert_eq!(listed["tools"][0]["name"], "echo");
        server.assert_clean();
    }

    #[test]
    fn server_side_tool_error_is_a_typed_error() {
        let server = McpHttpServer::start(echo_tools());
        let t = transport(&server);
        t.ensure_ready().unwrap();
        let err = t.request("no/such-method", json!({})).unwrap_err();
        assert!(err.contains("-32601"), "{err}");
    }

    #[test]
    fn unreachable_server_names_the_url() {
        let t = HttpTransport::new("http://127.0.0.1:9", Duration::from_secs(1));
        let err = t.ensure_ready().unwrap_err();
        assert!(err.contains("127.0.0.1:9"), "{err}");
    }

    #[test]
    fn endless_keepalive_trickle_hits_the_absolute_deadline() {
        // A server that keeps the stream alive with comments but never
        // answers: the idle watchdog resets on every byte, so only the
        // absolute per-call deadline can save the loop.
        let server = McpHttpServer::start(echo_tools());
        let t = HttpTransport::new(&server.url(), Duration::from_secs(1));
        t.ensure_ready().unwrap();
        server.trickle_next_call();
        let started = std::time::Instant::now();
        let err = t
            .request(
                "tools/call",
                json!({"name": "echo", "arguments": {"text": "x"}}),
            )
            .unwrap_err();
        assert!(err.contains("timed out after 1s"), "{err}");
        assert!(err.contains("kept the stream alive"), "{err}");
        // Bounded by deadline + one idle-window read, never unbounded.
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "took {:?}",
            started.elapsed()
        );
        // The transport stays usable for the next call.
        let ok = t
            .request(
                "tools/call",
                json!({"name": "echo", "arguments": {"text": "back"}}),
            )
            .unwrap();
        assert!(ok["content"][0]["text"].as_str().unwrap().contains("back"));
    }

    /// Minimal raw-TCP MCP endpoint: answers initialize with framed plain
    /// JSON, 202s notifications, and trickles every other response body one
    /// byte at a time forever over a close-delimited application/json
    /// response. That is the shape only an absolute per-call deadline can
    /// bound: the provider idle watchdog resets on every byte.
    fn json_trickle_server() -> String {
        use std::io::{Read as _, Write as _};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                std::thread::spawn(move || {
                    loop {
                        let mut head = Vec::new();
                        let mut byte = [0u8; 1];
                        while !head.ends_with(b"\r\n\r\n") {
                            match stream.read(&mut byte) {
                                Ok(1) => head.extend_from_slice(&byte),
                                _ => return,
                            }
                        }
                        let head_text = String::from_utf8_lossy(&head).to_ascii_lowercase();
                        let len: usize = head_text
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        let mut body = vec![0u8; len];
                        if stream.read_exact(&mut body).is_err() {
                            return;
                        }
                        let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                        match body.get("method").and_then(Value::as_str).unwrap_or("") {
                            "initialize" => {
                                let msg = json!({"jsonrpc": "2.0", "id": body["id"],
                                    "result": {"protocolVersion": "2025-11-25"}})
                                .to_string();
                                let reply = format!(
                                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                                     content-length: {}\r\n\r\n{msg}",
                                    msg.len()
                                );
                                if stream.write_all(reply.as_bytes()).is_err() {
                                    return;
                                }
                            }
                            m if m.starts_with("notifications/") => {
                                let reply = "HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\n\r\n";
                                if stream.write_all(reply.as_bytes()).is_err() {
                                    return;
                                }
                            }
                            _ => {
                                let head =
                                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n";
                                if stream.write_all(head.as_bytes()).is_err() {
                                    return;
                                }
                                loop {
                                    if stream.write_all(b"x").is_err() || stream.flush().is_err() {
                                        return;
                                    }
                                    std::thread::sleep(Duration::from_millis(25));
                                }
                            }
                        }
                    }
                });
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn plain_json_byte_trickle_hits_the_absolute_deadline() {
        let url = json_trickle_server();
        let t = HttpTransport::new(&url, Duration::from_secs(1));
        t.ensure_ready().unwrap();
        let started = Instant::now();
        let err = t
            .request(
                "tools/call",
                json!({"name": "echo", "arguments": {"text": "x"}}),
            )
            .unwrap_err();
        assert!(err.contains("timed out after 1s"), "{err}");
        assert!(err.contains("trickling"), "{err}");
        // Bounded by the deadline plus one read, never per-byte-reset idle.
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn failed_initialized_note_rolls_back_for_a_full_re_handshake() {
        let server = McpHttpServer::start(echo_tools());
        let t = transport(&server);
        // The note is the first non-initialize request; dropping the session
        // 404s it, so the handshake fails after initialize succeeded.
        server.drop_session_once();
        assert!(t.ensure_ready().is_err());
        // Half-committed state must not survive: the next attempt re-runs
        // the whole handshake instead of reporting ready off a session the
        // server never completed.
        assert_eq!(t.ensure_ready().unwrap(), "2025-11-25");
        assert_eq!(
            server.initialize_count(),
            2,
            "the retry must re-initialize, not reuse half-committed state"
        );
        server.assert_clean();
    }

    #[test]
    fn oversized_sse_is_rejected_before_parser_memory_can_grow_without_bound() {
        let server = McpHttpServer::start(echo_tools());
        let t = transport(&server);
        t.ensure_ready().unwrap();
        server.oversize_next_call();
        let err = t
            .request(
                "tools/call",
                json!({"name": "echo", "arguments": {"text": "x"}}),
            )
            .unwrap_err();
        assert!(err.contains("more than 8 MiB"), "{err}");

        let ok = t
            .request(
                "tools/call",
                json!({"name": "echo", "arguments": {"text": "back"}}),
            )
            .unwrap();
        assert!(ok["content"][0]["text"].as_str().unwrap().contains("back"));
    }
}
