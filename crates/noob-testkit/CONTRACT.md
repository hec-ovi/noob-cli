# noob-testkit

contractVersion: 1.0.0

## Purpose

A dev-only test rig: a scriptable mock OpenAI server and a scriptable mock MCP
server that record every request and run the wire assertions automatically, so
every e2e inherits the wire invariants without restating them.

It is a `[dev-dependencies]` entry of `crates/noob` (the e2e suites) and
`crates/noob-provider` (the transport tests), and of nothing else. The surface
is typed Rust, the text-geometry way: the types are the shapes, checked by the
compiler at the boundary, not re-validated per call.

## Surface: the OpenAI mock

`MockServer` is a hand-rolled HTTP/1.1 server on a `std::net::TcpListener`,
bound to `127.0.0.1:0`, one thread per connection, 30 s read timeout. Tests
enqueue scripted responses, point the binary at `base_url()`, and end with
`assert_clean()`. Scripts answer any path; the wire assertions run only on
requests whose path ends with `/chat/completions` or `/responses`.

| Operation | Signature | Behavior |
|---|---|---|
| Start | `MockServer::start() -> MockServer` | Binds and serves immediately. |
| Address | `base_url(&self) -> String` | `http://127.0.0.1:PORT/v1`, the `/v1` included. |
| Address, raw | `url(&self, path: &str) -> String` | `http://127.0.0.1:PORT` + `path`. |
| Enqueue completion | `enqueue_completion(&self, text: &str)` | One non-streamed chat completion answering `text`. |
| Enqueue JSON | `enqueue_json(&self, status: u16, body: Value)` | Any status and body; connection stays alive. |
| Enqueue raw | `enqueue_raw(&self, steps: Vec<RawStep>)` | Bytes and sleeps; the connection closes when the steps end. |
| Enqueue raw, keep-alive | `enqueue_raw_keepalive(&self, steps: Vec<RawStep>)` | Same, but the connection stays open; the bytes must be self-delimiting (chunked encoding or a content-length). |
| Enqueue raw, routed | `enqueue_raw_for(&self, matcher: RequestMatch, steps: Vec<RawStep>)` | Raw steps served only to a matching request. |
| Enqueue SSE | `enqueue_sse(&self, datas: &[&str])` | Each entry one `data:` event, one write each, close-delimited like a real streaming endpoint. |
| Enqueue stream | `enqueue_stream_completion(&self, text: &str)` | Streamed chat completion: role chunk, per-word deltas, finish chunk, usage chunk, `[DONE]`. |
| Enqueue stream, routed | `enqueue_stream_completion_for(&self, matcher: RequestMatch, text: &str)` | Same frames, routed. |
| Enqueue tool calls | `enqueue_stream_toolcalls(&self, calls: &[(&str, &str, &str)], usage: Option<(u64, u64)>)` | Streamed tool calls, llama.cpp shape (id and name first, argument fragments split mid-JSON, `finish_reason: "tool_calls"`). `calls` entries are (id, name, arguments); `usage` overrides the (prompt, completion) token counts, default (10, 5), for tests that force compaction. |
| Enqueue tool calls, routed | `enqueue_stream_toolcalls_for(&self, matcher: RequestMatch, calls: &[(&str, &str, &str)], usage: Option<(u64, u64)>)` | Same frames, routed. |
| Sanction a prefix break | `expect_prefix_break(&self)` | Arms one allowance for a message-prefix mismatch (compaction, plan-mode entry or exit, a fresh session). Call N times to allow N. |
| Sanction a tools change | `expect_tools_change(&self)` | Arms one allowance for a tools-array change. |
| Fan-out mode | `allow_interleaving(&self)` | Parent and children share the server, so the cross-request assertions are off; the per-request ones still run. |
| Connection count | `connection_count(&self) -> usize` | TCP connections accepted, for keep-alive assertions (two requests, one connection). |
| Recorded requests | `recorded(&self) -> Vec<Recorded>` | Every request, in arrival order. |
| Verdict | `assert_clean(&self)` | Panics listing every collected violation. Every test ends with it. |

Routing: `RequestMatch` is `Any`, `HasTool(String)`, `LacksTool(String)`, or
`UserPrompt(String)`. `HasTool`/`LacksTool` look for the tool name in the
request's `tools` array (top-level `name` or `function.name`). `UserPrompt`
matches an exact user-role `content` string in `messages` or `input`, and
deliberately not prompt text embedded in a tool call's arguments, so concurrent
sub-agent requests can be told apart. A routed script is preferred over an
earlier `Any` script; FIFO order holds within each group. The plain enqueue
methods use `Any`.

`Recorded` is `{ method: String, path: String, headers: Vec<(String, String)>,
body: Vec<u8>, arrived: Instant }`, header names lowercased, plus
`header(&self, name: &str) -> Option<&str>` (case-insensitive) and
`json(&self) -> Option<Value>`. `arrived` is when the request head came in;
fan-out tests assert overlap with it.

`RawStep` is `Bytes(Vec<u8>)` or `SleepMs(u64)`, for timeout and stall
scenarios.

### Response builders (free functions)

| Function | Returns |
|---|---|
| `http_response(status: u16, content_length: Option<usize>) -> Vec<u8>` | An HTTP/1.1 response head with `content-type: application/json`. |
| `sse_headers() -> Vec<u8>` | The head of a close-delimited SSE stream. |
| `chunked_sse_response(datas: &[&str]) -> Vec<u8>` | A complete chunked-encoded SSE response, one chunk per event, properly terminated; pair with `enqueue_raw_keepalive`. |
| `chat_stream_datas(text: &str) -> Vec<String>` | The `data:` payloads of a streamed chat completion; the deltas reassemble to exactly `text`. |
| `chat_stream_toolcalls_datas(calls: &[(&str, &str, &str)], usage: Option<(u64, u64)>) -> Vec<String>` | The `data:` payloads of a streamed tool-call answer. |
| `raw_top_level_value(body: &[u8], key: &str) -> Option<Vec<u8>>` | The raw bytes of one top-level key's value in a JSON object body: a string/escape/depth aware scanner, not a parser, so assertions can compare the exact bytes the client serialized. |

### Spawn helpers

| Item | Behavior |
|---|---|
| `NOOB_ENV_VARS: &[&str]` | The 22 `NOOB_*` variables the compiled binary reads from its environment. |
| `scrub_noob_env(cmd: &mut std::process::Command) -> &mut std::process::Command` | Removes every name in `NOOB_ENV_VARS`, then sets `NOOB_WEBSEARCH=off` (unset means "probe PATH for the websearch CLI", which would register an extra tool on a developer's machine and change what the suite asserts). |
| `load_fixture_chunks(path: impl AsRef<Path>) -> Vec<Vec<u8>>` | Splits a fixture file into TCP-chunk byte vectors at every `%%CHUNK%%` sentinel. The sentinel is removed and nothing else; mid-line and mid-codepoint splits are legal and intended. |

## Surface: the MCP mock

`mcp::McpHttpServer` speaks MCP Streamable HTTP, protocol `2025-11-25`, on its
own loopback listener. It serves `initialize`, `notifications/*` (202, empty
body), `tools/list`, and `tools/call` from the tool set given at start, assigns
and enforces `Mcp-Session-Id` (`initialize` mints `sess-N`; any other method
with an unknown session gets 404), and collects wire violations the way the
OpenAI mock does.

| Operation | Signature | Behavior |
|---|---|---|
| Start | `McpHttpServer::start(tools: Vec<Value>) -> McpHttpServer` | Binds and serves the given tool catalog. |
| Address | `url(&self) -> String` | `http://127.0.0.1:PORT`. |
| SSE mode | `sse_mode(&self)` | From now on every response is a one-event SSE stream, close-delimited, preceded by a keepalive comment so clients prove they skip them. |
| Drop session | `drop_session_once(&self)` | Invalidates the session before the next non-initialize request (one 404), forcing the client's one re-initialize retry. |
| Trickle | `trickle_next_call(&self)` | The next `tools/call` answers with keepalive comments forever and never the response: the wedged-server shape a per-call deadline must survive. |
| Oversize | `oversize_next_call(&self)` | The next `tools/call` answers with one unterminated SSE data line of 129 x 64 KiB of `x` bytes, then closes. |
| Script a result | `enqueue_call_result(&self, result: Value)` | The next `tools/call` returns this result; an empty queue falls back to an echo result. |
| Initialize count | `initialize_count(&self) -> usize` | How many `initialize` requests arrived. |
| Calls | `calls(&self) -> Vec<Value>` | Every `tools/call` params object, in arrival order. |
| Requests | `requests(&self) -> Vec<Recorded>` | Every raw request. |
| Verdict | `assert_clean(&self)` | Panics listing every collected violation. |

Catalog helpers: `mcp::tool(name: &str, description: &str, schema: Value) ->
Value` builds one tool definition; `mcp::echo_tools() -> Vec<Value>` is the
default set, one `echo` tool with a required string arg.

## The automatic wire assertions

Run on every OpenAI-mock request to `/chat/completions` or `/responses`,
violations collected into the list `assert_clean` checks:

1. The body is valid JSON.
2. No output cap: no key containing both `max` and `token` (case-insensitive)
   anywhere in the body, at any depth.
3. Prefix stability: the serialized `messages` (or `input` on `/responses`)
   array is a byte-exact prefix extension of the previous request on the same
   path. Byte-exact because llama.cpp KV reuse is byte-sensitive: a serializer
   that merely reorders keys between turns is a real cache bust and is caught.
   `expect_prefix_break` sanctions one mismatch; the allowance is consumed only
   when a mismatch actually happens, so it can be armed before spawning a
   binary that compacts mid-run.
4. Tools stability: the raw `tools` bytes match the most recent request on the
   same path that carried a `tools` key at all, checked independently of the
   prefix allowance (a compaction break must never swallow tools drift, and a
   toolless summarizer request must not blind the comparison).
   `expect_tools_change` sanctions one change.
5. Transcript validity, per request: on `messages`, every `tool` result pairs
   with the oldest pending `tool_calls` id in emission order, no other message
   arrives while calls are pending, every call has an id, and the transcript
   does not end with unanswered calls; on `input`, `function_call` and
   `function_call_output` pair by `call_id` in order and none is left
   unanswered.

`allow_interleaving` turns off 3 and 4 only.

The MCP mock checks every request: `Accept` must offer both `application/json`
and `text/event-stream`; `MCP-Protocol-Version` must be present on every
post-initialize request; `content-type`, when present, must start with
`application/json`.

## Errors

The closed set, all of them deliberate test outcomes:

- OpenAI mock, no script matches the request: HTTP 500 with
  `{"error":"mock server script is empty"}`, plus a recorded violation.
- MCP mock, unknown session on a non-initialize method: HTTP 404.
- MCP mock, unknown method: JSON-RPC error `-32601`.
- Panics: `start` on either server when the loopback bind fails,
  `load_fixture_chunks` when the fixture cannot be read, and `assert_clean`
  when violations were collected.

Nothing else fails. Wire violations are never panicked from a server thread (a
panic there would vanish); they wait for `assert_clean`.

## Dependencies

- Contracts: none. This box depends on no other box in the repo.
- Crates: `serde_json` only; the servers are `std::net`.
- Protocol shapes it mirrors, not depends on: the OpenAI Chat Completions and
  Responses wire formats with their SSE framing as llama.cpp and OpenAI emit
  it, and MCP Streamable HTTP `2025-11-25`.

## Invariants

1. Dev-only. It appears only under `[dev-dependencies]` and never in a runtime
   dependency graph.
2. Everything binds `127.0.0.1:0`. A test run touches nothing beyond loopback
   and no fixed port, so suites run in parallel.
3. A test that skips `assert_clean` has asserted nothing: violations are
   collected, not raised at the point of failure.
4. An allowance is consumed only by an actual mismatch, never by a clean
   request, so arming them up front is safe.
5. The stream builders emit the real frame sequences (role chunk first, usage
   chunk with `prompt_tokens_details`, `[DONE]` last, argument fragments split
   mid-JSON) and `chat_stream_datas` cuts on character boundaries, so consumer
   suites can treat them as the reference shapes.
6. JSON scripts keep the connection alive, raw scripts close it, keep-alive
   raw scripts require self-delimiting bytes; `connection_count` is how a test
   tells these apart.
7. `scrub_noob_env` is the whole answer to env leakage: every `NOOB_*` name the
   binary reads is in `NOOB_ENV_VARS`, so a host-exported setting never reaches
   an assertion.

## How to modify this blackbox safely

The assertions are the product. Loosening one, or changing a builder's frame
sequence, is a breaking change for every e2e that inherits it: add the new
shape alongside and migrate callers. Adding an enqueue method, a matcher arm,
or an MCP scenario toggle is additive, minor bump.

When the binary grows a new `NOOB_*` variable, add it to `NOOB_ENV_VARS` in the
same change, or host settings leak into every spawning suite.
