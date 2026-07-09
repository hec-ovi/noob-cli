# noob/src/mcp

MCP client (P4): JSON-RPC framing, stdio and Streamable HTTP transports,
protocol 2025-11-25, tools only.

Lazy to the bone: startup connects nothing; `mcp_connect` does initialize +
tools/list and returns a compact catalog as a tool result; `mcp_call`
validates args client-side against the cached schema before sending.

Invariants: the tools array never changes when servers connect; MCP tool
descriptions and results are untrusted input, wrapped in delimiters; per-call
timeouts kill the process group so a wedged server can never block the loop;
all HTTP goes through noob-provider.
