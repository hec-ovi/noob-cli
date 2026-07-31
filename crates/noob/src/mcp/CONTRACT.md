# mcp

contractVersion: 1.0.0

## Purpose

The MCP client, lazy to the bone: nothing connects at session start, the
prompt carries one line of server names, and a connect caches the tool
catalog so calls validate locally before anything touches the wire.

## Public surface

```rust
pub struct Mcp;                       // session-scoped manager
impl Mcp {
    pub fn new(servers: Vec<ServerConfig>) -> Mcp;
    pub fn names(&self) -> Vec<&str>;                     // configured, sorted
    pub fn connection(&self, name: &str) -> Option<Arc<Connection>>;
    pub fn connect(&self, name: &str) -> Result<ConnectInfo, String>;
        // initialize (idempotent) + tools/list; caches and returns the catalog
    pub fn call(&self, conn: &Connection, tool: &str, args: &Value)
        -> Result<Value, String>;                          // tools/call
}
pub struct Connection;  impl Connection { pub fn tools(&self) -> Vec<ToolDef> }
pub struct ToolDef { pub name: String, pub description: String, pub schema: Value }
pub struct ConnectInfo { pub protocol: String, pub tools: Vec<ToolDef> }

// config: mcp.json loading, spec parsing, edits
pub fn config::load(workspace: &Path, config_dir: &Path)
    -> (Vec<ServerConfig>, Vec<String>);   // servers sorted, plus warnings
pub fn config::project_path(workspace: &Path) -> PathBuf;  // .noob/mcp.json
pub fn config::parse_spec(spec: &str) -> Result<TransportConfig, String>;
pub fn config::add_server(path, name, transport) -> Result<(), String>;
pub fn config::remove_server(path, name) -> Result<bool, String>;
pub enum TransportConfig { Http { url }, Stdio { command, args } }
pub struct ServerConfig { name, transport, timeout }
pub const config::DEFAULT_TIMEOUT_S: u64 = 30;
pub const config::MAX_TIMEOUT_S: u64 = 600;

// schema: client-side validation of call args against the cached schema
pub fn schema::validate(schema: &Value, args: &Value) -> Result<(), String>;
    // every problem found, joined, so one retry fixes the whole call
pub fn schema::sketch(schema: &Value) -> String;   // compact catalog sketch

pub const proto::PROTOCOL_VERSION: &str = "2025-11-25";
```

The transports (`stdio`, `http`) are internal machinery behind `connect` and
`call`; their types are public for the manager's use, not part of this
surface.

## Errors

Every failure is a `String` the model can act on: an unknown server names
the configured ones; a transport failure names the server and the timeout
that fired; a validation miss returns every problem with the expected shape
attached; a malformed mcp.json entry is a warning in `load`'s second return,
never a crash.

## Invariants

1. Lazy: session start connects nothing, and the request tools array never
   changes when servers connect, so the prompt cache prefix survives MCP
   entirely.
2. A failed connect leaves the server looking unconnected: no phantom
   connection, `mcp_call` keeps teaching "connect first".
3. Parallel connects to one name share a single handshake; a reconnect
   refreshes the catalog without a second initialize.
4. A wedged server can never block the loop: every call runs under the
   per-server timeout (default 30 s, ceiling 600 s); on stdio a timeout
   kills the child's whole process group and the next call respawns and
   re-handshakes transparently.
5. Config merge: the project `.noob/mcp.json` wins over the global file per
   server name; edits rewrite atomically.
6. Client-side validation is deliberately shallow (required keys, top-level
   primitive types) and permissive about what it does not understand.
7. tools/list pagination is capped at 16 pages; a server streaming endless
   cursors is cut off and the catalog keeps what arrived.

## Dependencies

Contracts: [`crates/noob-provider/CONTRACT.md`](../../../noob-provider/CONTRACT.md)
for the HTTP client and the interrupt flag; the tools box for `atomic_write`
(mcp.json edits share the one atomic writer). Dev only:
[`crates/noob-testkit/CONTRACT.md`](../../../noob-testkit/CONTRACT.md) mock
MCP servers.

## Tests

Inline: connect/call round trips, phantom-connection refusal, parallel
handshake sharing, catalog refresh, config merge and edits, validation
shapes, transport timeouts. Boundary: `crates/noob/tests/e2e_p4.rs` through
the real binary, `crates/noob/tests/ui_commands.rs` for `/mcp`.
