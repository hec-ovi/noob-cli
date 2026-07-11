# noob/src/mcp

Tools-only MCP client for protocol 2025-11-25 over stdio and Streamable HTTP.

Startup connects nothing. `mcp_connect` initializes one named server and caches its catalog. `mcp_call` validates arguments against the cached schema before transport. Server content is untrusted, bounded, and wrapped before transcript insertion.

stdio uses newline-delimited JSON-RPC, bounded line reads, nonblocking writes, 50 ms interrupt polling, absolute per-call timeout, and process-group kill plus reap on timeout, cancellation, or drop. A subsequent call respawns the server.

Streamable HTTP accepts JSON and event-stream responses, carries session and protocol headers, retries initialization once after a 404 session loss, and applies an absolute call deadline. All HTTP uses noob-provider.
