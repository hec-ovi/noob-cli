//! MCP client (P4): lazy to the bone. Session start connects nothing; the
//! prompt carries one line of server names. `mcp_connect` runs initialize +
//! tools/list and caches the catalog; `mcp_call` validates args against the
//! cached schema before anything touches the wire. The tools array never
//! changes when servers connect, so the cache prefix survives MCP entirely.

pub mod config;
pub mod http;
pub mod proto;
pub mod schema;
pub mod stdio;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use config::{ServerConfig, TransportConfig};

/// One tool from a server's catalog, cached at connect time.
#[derive(Clone, Debug)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

/// What `mcp_connect` reports back.
#[derive(Debug)]
pub struct ConnectInfo {
    pub protocol: String,
    pub tools: Vec<ToolDef>,
}

enum Transport {
    Stdio(stdio::StdioTransport),
    Http(http::HttpTransport),
}

impl Transport {
    fn ensure_ready(&self) -> Result<String, String> {
        match self {
            Transport::Stdio(t) => t.ensure_ready(),
            Transport::Http(t) => t.ensure_ready(),
        }
    }

    fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        match self {
            Transport::Stdio(t) => t.request(method, params),
            Transport::Http(t) => t.request(method, params),
        }
    }
}

pub struct Connection {
    transport: Transport,
    tools: Mutex<Vec<ToolDef>>,
}

impl Connection {
    /// The cached catalog snapshot (empty until a connect succeeded).
    pub fn tools(&self) -> Vec<ToolDef> {
        self.tools.lock().unwrap().clone()
    }
}

/// Session-scoped manager. Interior mutability throughout: `mcp_connect` is
/// a read-only call and may run on the scheduler's scoped threads.
pub struct Mcp {
    servers: Vec<ServerConfig>,
    conns: Mutex<HashMap<String, Arc<Connection>>>,
}

/// tools/list pagination bound: a server streaming endless cursors is
/// misbehaving; cap and carry on with what arrived.
const MAX_LIST_PAGES: usize = 16;

impl Mcp {
    pub fn new(servers: Vec<ServerConfig>) -> Mcp {
        Mcp { servers, conns: Mutex::new(HashMap::new()) }
    }

    /// Configured server names, sorted (config::load sorts).
    pub fn names(&self) -> Vec<&str> {
        self.servers.iter().map(|s| s.name.as_str()).collect()
    }

    /// The connection for `name`, if a connect succeeded this session.
    pub fn connection(&self, name: &str) -> Option<Arc<Connection>> {
        self.conns.lock().unwrap().get(name).cloned()
    }

    /// initialize (idempotent) + tools/list; caches and returns the catalog.
    /// Reconnecting an already-connected server refreshes its catalog.
    pub fn connect(&self, name: &str) -> Result<ConnectInfo, String> {
        let Some(cfg) = self.servers.iter().find(|s| s.name == name) else {
            return Err(format!(
                "unknown MCP server {name:?}; configured servers: {}",
                self.names().join(", ")
            ));
        };
        let conn = {
            let mut conns = self.conns.lock().unwrap();
            conns
                .entry(name.to_string())
                .or_insert_with(|| {
                    let transport = match &cfg.transport {
                        TransportConfig::Http { url } => {
                            Transport::Http(http::HttpTransport::new(url, cfg.timeout))
                        }
                        TransportConfig::Stdio { command, args } => Transport::Stdio(
                            stdio::StdioTransport::new(command, args, cfg.timeout),
                        ),
                    };
                    Arc::new(Connection { transport, tools: Mutex::new(Vec::new()) })
                })
                .clone()
        };
        let protocol = conn.transport.ensure_ready()?;
        let tools = list_tools(&conn.transport)?;
        *conn.tools.lock().unwrap() = tools.clone();
        Ok(ConnectInfo { protocol, tools })
    }

    /// tools/call against a connected server. The caller (the mcp_call
    /// tool) has already resolved the connection and validated the args.
    pub fn call(&self, conn: &Connection, tool: &str, args: &Value) -> Result<Value, String> {
        conn.transport
            .request("tools/call", json!({"name": tool, "arguments": args}))
    }
}

fn list_tools(transport: &Transport) -> Result<Vec<ToolDef>, String> {
    let mut tools = Vec::new();
    let mut cursor: Option<String> = None;
    for _page in 0..MAX_LIST_PAGES {
        let params = match &cursor {
            Some(c) => json!({"cursor": c}),
            None => json!({}),
        };
        let result = transport.request("tools/list", params)?;
        if let Some(items) = result.get("tools").and_then(Value::as_array) {
            for item in items {
                let Some(name) = item.get("name").and_then(Value::as_str) else {
                    continue; // a tool without a name is uncallable; skip
                };
                tools.push(ToolDef {
                    name: name.to_string(),
                    description: item
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    schema: item.get("inputSchema").cloned().unwrap_or(Value::Null),
                });
            }
        }
        cursor = result
            .get("nextCursor")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    Ok(tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn manager_with(server_url: &str) -> Mcp {
        Mcp::new(vec![ServerConfig {
            name: "mock".to_string(),
            transport: TransportConfig::Http { url: server_url.to_string() },
            timeout: Duration::from_secs(5),
        }])
    }

    #[test]
    fn connect_caches_the_catalog_and_call_round_trips() {
        let server = noob_testkit::mcp::McpHttpServer::start(noob_testkit::mcp::echo_tools());
        let mcp = manager_with(&server.url());
        assert!(mcp.connection("mock").is_none(), "lazy: nothing before connect");
        let info = mcp.connect("mock").unwrap();
        assert_eq!(info.protocol, "2025-11-25");
        assert_eq!(info.tools.len(), 1);
        assert_eq!(info.tools[0].name, "echo");
        let conn = mcp.connection("mock").expect("cached after connect");
        assert_eq!(conn.tools()[0].name, "echo");
        let result = mcp
            .call(&conn, "echo", &serde_json::json!({"text": "hola"}))
            .unwrap();
        assert!(result["content"][0]["text"].as_str().unwrap().contains("hola"));
        server.assert_clean();
    }

    #[test]
    fn unknown_server_lists_the_configured_ones() {
        let mcp = manager_with("http://127.0.0.1:9");
        let err = mcp.connect("ghost").unwrap_err();
        assert!(err.contains("unknown MCP server \"ghost\""), "{err}");
        assert!(err.contains("mock"), "{err}");
    }

    #[test]
    fn reconnect_refreshes_instead_of_erroring() {
        let server = noob_testkit::mcp::McpHttpServer::start(noob_testkit::mcp::echo_tools());
        let mcp = manager_with(&server.url());
        mcp.connect("mock").unwrap();
        let again = mcp.connect("mock").unwrap();
        assert_eq!(again.tools.len(), 1);
        // Still exactly one initialize: the transport handshake is idempotent.
        assert_eq!(server.initialize_count(), 1);
        server.assert_clean();
    }
}
