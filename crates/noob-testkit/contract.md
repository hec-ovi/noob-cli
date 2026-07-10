# crates/noob-testkit

Dev-only. A hand-rolled mock OpenAI server on std::net::TcpListener serving
`/v1/chat/completions` and `/v1/responses`; it exists to test our own client
and nothing else.

Tests enqueue scripted responses (JSON completions or raw byte steps with
sleeps, for timeout scenarios) and read back every recorded request. Three
assertions run automatically on each API request: prefix stability (declare
sanctioned breaks with `expect_prefix_break`), no max_tokens-family key
anywhere, and transcript validity (every tool call paired with exactly one
result, in order).

Violations collect instead of panicking inside server threads; tests must end
with `assert_clean()`. Never a runtime dependency of the shipped binary.

`mcp` adds a mock MCP server over Streamable HTTP (initialize / tools/list /
tools/call from a configurable tool set): it assigns and enforces
`Mcp-Session-Id` (404 on unknown sessions, `drop_session_once` to force the
client's re-initialize), answers as plain JSON or single-event SSE
(`sse_mode`), records every request and tools/call, and collects its own
wire violations (missing `MCP-Protocol-Version` or Accept types) for
`assert_clean()`.
