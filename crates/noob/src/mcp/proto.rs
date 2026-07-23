//! JSON-RPC 2.0 message building and response classification, shared by
//! both transports. Hand-rolled and tiny: requests, notifications, and the
//! three inbound shapes (response, server request, notification).

use serde_json::{Value, json};

/// The protocol revision this client speaks. Sent in `initialize` and, on
/// the HTTP transport, as the `MCP-Protocol-Version` header once negotiated.
pub const PROTOCOL_VERSION: &str = "2025-11-25";

pub fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

pub fn notification(method: &str) -> Value {
    json!({"jsonrpc": "2.0", "method": method})
}

pub fn initialize_params() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": {"name": "noob", "version": env!("CARGO_PKG_VERSION")}
    })
}

/// Reply for a server-to-client request we do not implement (tools-only
/// client): a wedge-proof polite refusal instead of silence.
pub fn method_not_found(id: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id,
        "error": {"code": -32601, "message": "this client supports tools only"}})
}

/// Cap on a server-controlled `error.message` before it is rendered into an
/// error string (and from there into transcripts): without it a hostile
/// server could spend the whole 8 MiB inbound line bound on one message.
const ERROR_MESSAGE_CHAR_CAP: usize = 300;

fn cap_error_message(message: &str) -> std::borrow::Cow<'_, str> {
    let total = message.chars().count();
    if total <= ERROR_MESSAGE_CHAR_CAP {
        return std::borrow::Cow::Borrowed(message);
    }
    let cut: String = message.chars().take(ERROR_MESSAGE_CHAR_CAP).collect();
    std::borrow::Cow::Owned(format!(
        "{cut}… [error message truncated; {total} chars total]"
    ))
}

/// One inbound JSON-RPC message, classified.
#[derive(Debug)]
pub enum Inbound {
    /// A response to our request `id`: Ok(result) or Err(rendered error).
    Response {
        id: u64,
        outcome: Result<Value, String>,
    },
    /// A server-to-client request (has both id and method); needs a reply.
    ServerRequest { id: Value },
    /// A notification (no id) or anything else safely ignorable.
    Other,
}

pub fn classify(msg: &Value) -> Inbound {
    let has_method = msg.get("method").is_some();
    match msg.get("id") {
        None | Some(Value::Null) => Inbound::Other,
        Some(id) if has_method => Inbound::ServerRequest { id: id.clone() },
        Some(id) => {
            // Some servers echo numeric ids as strings; accept both.
            let Some(id) = id
                .as_u64()
                .or_else(|| id.as_str().and_then(|s| s.parse().ok()))
            else {
                return Inbound::Other;
            };
            let outcome = if let Some(err) = msg.get("error") {
                let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
                let message = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                Err(format!(
                    "server error {code}: {}",
                    cap_error_message(message)
                ))
            } else {
                Ok(msg.get("result").cloned().unwrap_or(Value::Null))
            };
            Inbound::Response { id, outcome }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_notification_shapes() {
        let r = request(3, "tools/list", json!({}));
        assert_eq!(r["jsonrpc"], "2.0");
        assert_eq!(r["id"], 3);
        assert_eq!(r["method"], "tools/list");
        let n = notification("notifications/initialized");
        assert!(n.get("id").is_none());
    }

    #[test]
    fn classify_covers_all_inbound_shapes() {
        match classify(&json!({"jsonrpc":"2.0","id":7,"result":{"ok":true}})) {
            Inbound::Response {
                id: 7,
                outcome: Ok(v),
            } => assert_eq!(v["ok"], true),
            other => panic!("{other:?}"),
        }
        match classify(&json!({"jsonrpc":"2.0","id":"7","result":1})) {
            Inbound::Response { id: 7, .. } => {}
            other => panic!("string id must parse: {other:?}"),
        }
        match classify(&json!({"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"boom"}})) {
            Inbound::Response {
                id: 2,
                outcome: Err(e),
            } => {
                assert!(e.contains("-32000") && e.contains("boom"));
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            classify(
                &json!({"jsonrpc":"2.0","id":9,"method":"sampling/createMessage","params":{}})
            ),
            Inbound::ServerRequest { .. }
        ));
        assert!(matches!(
            classify(&json!({"jsonrpc":"2.0","method":"notifications/progress","params":{}})),
            Inbound::Other
        ));
        // A result missing entirely still resolves (Null), never wedges.
        match classify(&json!({"jsonrpc":"2.0","id":1})) {
            Inbound::Response {
                outcome: Ok(Value::Null),
                ..
            } => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn oversized_error_messages_are_capped_with_an_elision_marker() {
        let huge = "e".repeat(100_000);
        match classify(&json!({"jsonrpc":"2.0","id":4,"error":{"code":-1,"message":huge}})) {
            Inbound::Response {
                outcome: Err(rendered),
                ..
            } => {
                assert!(rendered.chars().count() < 400, "{}", rendered.len());
                assert!(
                    rendered.contains("[error message truncated; 100000 chars total]"),
                    "{rendered}"
                );
            }
            other => panic!("{other:?}"),
        }
        // Short messages pass through whole (the existing "boom" test above
        // pins the uncapped shape).
        assert_eq!(cap_error_message("short"), "short");
    }
}
