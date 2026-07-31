# noob-provider

contractVersion: 1.0.0

## Purpose

Take a transcript and a tool set, run one model turn against an
OpenAI-compatible endpoint over either wire shape, stream events as they
arrive, and hand back the assembled turn.

This box is the sole owner of ureq: no other crate in the project opens an
HTTP connection to a model server.

## Surface

The types named below all live in `src/types.rs`; that file is the schema.
There is no JSON schema directory: the typed Rust surface is checked by the
compiler at every call site, and the wire behavior behind it is enforced at
the test boundary by replaying captured byte transcripts (see What the tests
prove).

Primary entry points, in `src/lib.rs`:

```rust
pub fn resolve_endpoint(config_dir: &Path, ov: &Overrides)
    -> Result<Endpoint, ProviderError>;

pub fn run_turn(client: &http::Client, config_dir: &Path, ov: &Overrides,
    req: &TurnRequest, on: &mut dyn FnMut(Event))
    -> Result<Turn, ProviderError>;

pub fn run_turn_ref(client: &http::Client, config_dir: &Path, ov: &Overrides,
    req: TurnRequestRef<'_>, on: &mut dyn FnMut(Event))
    -> Result<Turn, ProviderError>;
```

`run_turn` resolves the endpoint fresh (see Settings), picks the adapter by
`Endpoint.style`, streams every `Event` through `on` in arrival order, and
returns the assembled `Turn`. `run_turn_ref` is the same call on a borrowed
request (`TurnRequest::borrowed()` converts), so an agent loop does not clone
the transcript every round.

Module surface, for callers that need one layer down:

| Module | Public items | Use it for |
|---|---|---|
| `http` | `Client::new(Timeouts)`, `Client::with_retry(Timeouts, RetryPolicy)`, `Client::ctl() -> WatchdogCtl`, `post_json`, `post_json_stream`, `post_json_stream_with`, `StreamBody`, `WatchdogCtl::interrupt()`, `static INTERRUPTED: AtomicBool`, `probe(url, timeout) -> bool`, `get_status(url, api_key, timeout)` | The watchdog transport. `post_json_stream_with` takes extra request headers (MCP Streamable HTTP needs them). `probe` is the loopback autodetect check; `get_status` is the doctor's bounded GET (256 KiB). |
| `chat` | `stream`, `stream_ref`, `wire_tools(&[ToolSpec]) -> Value` | Chat Completions adapter. `wire_tools` is the exact serialized tools array, exposed so debug output cannot drift from what is sent. |
| `responses` | `stream`, `stream_ref` | Responses adapter. |
| `sse` | `SseParser::new()`, `feed(&[u8], &mut Vec<SseEvent>)`, `finish(&mut Vec<SseEvent>)`, `SseEvent { event: Option<String>, data: String }` | The only SSE parser in the binary; MCP reuses it. |
| `assemble` | `Assembler::{new, on_chunk, mark_severed, finish}`, `repair_args(&str) -> String` | Chat delta state machine, pure function of parsed chunk JSON. |
| `envfile` | `parse(&str)`, `load(&Path)`, both `-> Result<HashMap<String, String>, String>` | Flat `KEY=VALUE` parser: `#` comments, optional `export` prefix, matched quotes stripped, trailing comments cut, later keys win, no interpolation. Errors name the first bad line. |

## Settings

`resolve_endpoint` opens `{config_dir}/.env`, parses it, and drops it, inside
every request build. A missing file is fine; an unreadable or malformed one is
`Config`. Precedence for non-secret keys: `Overrides` (CLI flag) > process
environment > `.env`; empty env or file values count as unset.

| Key | Meaning |
|---|---|
| `NOOB_BASE_URL` | Required. The `/v1` base; trailing slashes are trimmed. Missing is `Config`. |
| `NOOB_API_STYLE` | `chat` or `responses`; any other value is `Config`. Unset: `responses` when the base URL contains `api.openai.com`, else `chat`. |
| `NOOB_MODEL` | Model name; defaults to `default`. |
| `NOOB_API_KEY` | Read from `.env` only, never the process environment, so bash subprocesses and child agents cannot see it. Empty sends no `Authorization` header; otherwise `Bearer <key>`. |
| `NOOB_REASONING` | `on`/`true`/`yes`/`1` or `off`/`false`/`no`/`0` (case-insensitive, trimmed); anything else is `Config`. Unset sends no thinking field at all. No `Overrides` field; env and `.env` only. |

## Inputs

| Type | Shape |
|---|---|
| `Overrides` | `{ base_url, model, api_style: Option<String> }`, highest precedence. |
| `TurnRequest` | `{ system: Option<String>, items: Vec<Item>, tools: Vec<ToolSpec> }`. |
| `Item` | `User(String)`, `Assistant { text, tool_calls, raw_items }`, `ToolResult { call_id, content }`. `raw_items` are captured Responses output items, replayed verbatim when present; empty on chat-captured turns. |
| `ToolSpec` | `{ name, description, parameters: Value }`, parameters is the JSON Schema of the arguments object. Each adapter wraps it in its wire shape. |

## Outputs

Events, streamed through `on` in arrival order:

| Event | Meaning |
|---|---|
| `Text(String)` | A visible-output delta. |
| `Reasoning(String)` | A thinking delta. |
| `ToolCallStart { index, id, name }` | A call opened; `index` is emission order. |
| `ToolArgsDelta { index, delta }` | An arguments fragment for that call. |
| `Usage(Usage)` | Token counts when the server reports them. |
| `Done(Finish)` | Exactly once per `Ok` turn, last, carrying the same `Finish` as the returned `Turn`. An `Err` return emits no `Done`. |

The assembled `Turn`: `{ text, reasoning: Option<String>, tool_calls,
usage: Option<Usage>, finish, raw_items }`. `Usage` is `{ prompt_tokens,
completion_tokens, cached_prompt_tokens }`. `raw_items` is always empty on
the chat shape. `Finish` is `Stop`, `ToolCalls`, `Length`, `ContentFilter`,
or `Error(String)`; output is never capped, so `Length` means the context is
full. Turn-level failures (an in-band error payload, `response.failed`, a
severed stream) come back as `Finish::Error` inside an `Ok` turn with
whatever partial output arrived; transport-level failures are
`Err(ProviderError)`.

## Wire shapes

Chat Completions, `POST {base_url}/chat/completions`:

- Body: `model`, `messages`, `stream: true`,
  `stream_options: {"include_usage": true}`; `tools` (nested under
  `function`) only when non-empty. Reasoning on adds
  `chat_template_kwargs: {"enable_thinking": true}`; off adds
  `enable_thinking: false` plus `reasoning_effort: "none"`; unset adds
  neither, keeping the default body byte-identical for servers that reject
  unknown fields. Never `parallel_tool_calls`, never a `max_tokens`-family
  key.
- Transcript mapping: `system` role message first, `user` messages,
  `assistant` messages (content `null` when empty, calls nested under
  `function`), `tool` role with `tool_call_id`. Reasoning is never sent back.
- The stream ends at the `[DONE]` sentinel; a `finish_reason` on the last
  chunk also closes cleanly for servers that skip `[DONE]`. After `[DONE]`
  the residual bytes are drained (up to 250 ms and 16 KiB) so the connection
  returns to the pool.
- Quirks absorbed by the assembler: deltas without `index` (attributed to the
  last open call), indexless deltas with a new id (opened as a new call),
  the whole message repeated in a final non-delta chunk (deduplicated for
  both text and calls), `arguments` sent as a JSON object (re-serialized to
  a string), `"usage": null` and `"error": null` filler, in-band error
  payloads (become `Finish::Error`), reasoning under `reasoning_content` or
  `reasoning`, usage from `prompt_tokens`/`completion_tokens`/
  `prompt_tokens_details.cached_tokens`.

Responses, `POST {base_url}/responses`:

- Body: `model`, `input`, `store: false`, `stream: true`; `instructions`
  from `system`; flattened tools (no `function` nesting);
  `include: ["reasoning.encrypted_content"]` only when the base URL contains
  `api.openai.com` (other servers 400 on it).
- Stateless full-input replay: never `previous_response_id`. An assistant
  turn with `raw_items` goes back verbatim; without them (a turn captured on
  the chat shape) equivalent `message` and `function_call` items are
  reconstructed. Tool results become `function_call_output`.
- Events route on the payload `type`, with the SSE `event:` field as
  fallback; unknown types are ignored so a growing vocabulary cannot crash
  the client. Handled: `response.output_item.added`/`.done`,
  `response.function_call_arguments.delta`/`.done` (done is authoritative
  over accumulated deltas), `response.output_text.delta`,
  `response.reasoning_text.delta`, `response.reasoning_summary_text.delta`,
  `response.completed`, `response.failed`, `response.incomplete`, `error`.
  A stray `[DONE]` is ignored; the stream ends at EOF.
- `response.completed`'s `output` array is the authoritative `raw_items` and
  fills in any call or message text a server only sends there. Usage maps
  from `input_tokens`/`output_tokens`/`input_tokens_details.cached_tokens`.

Both adapters share a content-type guard: a 200 answering
`application/json` to a `stream: true` request (several proxies and older
servers) is parsed as one whole completion or response object, statuses
included, and replayed as the same uniform event sequence.

## SSE handling

`sse::SseParser` is byte-exact and incremental: fed raw TCP bytes in
whatever chunks arrive, it emits complete events. Chunk boundaries can fall
anywhere, including inside a `data:` keyword or a multibyte UTF-8 codepoint
(line terminators are ASCII, so buffering raw bytes to a terminator makes
codepoint splits safe). It tolerates CRLF, LF and bare CR, a BOM on the
first line, comment keepalives (`: OPENROUTER PROCESSING`), optional space
after the colon, and multiple `data:` lines joined with `\n`; `event:` is
captured, `id:` and `retry:` are dropped. `finish()` dispatches a terminated
trailing event missing only its blank line, and drops an unterminated tail
as truncation.

## Errors

The closed set, `ProviderError`:

| Variant | When |
|---|---|
| `Config(String)` | Bad or missing settings at resolve time: unreadable `.env`, missing `NOOB_BASE_URL`, invalid `NOOB_API_STYLE` or `NOOB_REASONING`. |
| `Connect(String)` | The endpoint could not be reached (refused, DNS, TLS). |
| `Http { status: u16, body: String }` | Non-2xx after retries and compat strips, body already read. |
| `Timeout(Connect)` | The connect budget ran out. |
| `Timeout(Send)` | The server accepted the connection but stopped reading the request for 30 s. |
| `Timeout(FirstByte)` | No response headers, or no first body byte, within the first-byte budget. |
| `Timeout(Idle)` | Silence between body bytes past the idle budget. |
| `Interrupted` | Ctrl-C or `WatchdogCtl::interrupt` during any phase, backoff sleeps included. |
| `Wire(String)` | Bytes that do not parse as the expected shape: bad JSON in SSE data or a completion, no `choices[0]`, an auxiliary body past its bound. |
| `Unsupported(String)` | In the set for callers to type against; no path in this box constructs it. |

Rendered messages name the likely fix (which `.env` key to check, when to
retry).

## Invariants

1. Hot reload: the `.env` file is opened, parsed, and dropped inside every
   request build, so an edited key applies on the very next call with no
   restart.
2. Secrets stay in the file: the API key is never read from the process
   environment and never enters it.
3. No output cap: no `max_tokens`-family key ever leaves this box, in either
   shape, and completion bodies are read without a length bound. Only
   auxiliary bodies (HTTP error bodies) are bounded, at 8 MiB.
4. Retries happen only before the first streamed content byte: connect and
   TLS failures, and statuses 408, 425, 429, 5xx. Default schedule: one
   attempt plus retries after 1, 2, 4 s, full jitter in (0, delay];
   `Retry-After` in seconds is honored up to 60 s and outranks the schedule.
   After the first content byte a failure surfaces as an error, never a
   silent retry, because a retry after output would duplicate it.
5. Reactive compat: a 400 whose body quotes a field this request actually
   sent, from the set `stream_options`, `store`, `include`,
   `parallel_tool_calls`, `chat_template_kwargs`, `reasoning_effort`, gets
   one immediate retry without that field, and the strip is remembered per
   request URL for the client's lifetime. Core fields are never stripped.
6. The watchdog is the only clock: 1 s socket ticks. Default budgets:
   connect 10 s, first byte 300 s (covering headers and the silent
   prompt-processing window llama.cpp spends on a long prompt), idle 90 s
   between body bytes, 30 s no-progress ceiling on the request send. Header
   bytes never start the idle clock; the first body byte does, even when it
   arrives in the same read as the headers.
7. An interrupt (the `http::INTERRUPTED` static or
   `Client::ctl().interrupt()`) lands within about one tick in every phase:
   connect, send, silent wait, mid-stream, or a backoff sleep.
8. A severed stream is never a clean answer: EOF without a terminal signal
   finishes as `Finish::Error` with the partial output kept, and a tool call
   whose arguments do not parse never finishes as `ToolCalls`.
9. Every started tool call ends with an id, a name, and arguments. A missing
   id is synthesized as `call_<seq>_<index>`, unique for the process, so
   replayed transcripts cannot collide; empty arguments become `{}`; fenced
   JSON gets exactly one mechanical repair; still-invalid arguments are
   returned as-is for the caller to reject with a useful error.
10. Responses replay is byte-identical: `store: false` always, captured
    `raw_items` returned verbatim in the next `input` so reasoning items
    survive, and a synthesized call id is patched into the captured item so
    every `function_call_output` still pairs with a `function_call`.
11. Serialization is deterministic: identical input produces identical
    bytes, which is what append-only prefix caching rests on.
12. Proxies are off regardless of environment variables, and DNS resolution
    is bounded at 5 s.
13. One in-flight request per `Client`; connections are kept alive and
    reused across turns.

## Dependencies

No contract dependencies: this box reads no other box's contract.

Crate dependencies: `ureq` and `serde_json`. ureq is pinned exactly to
`=3.3.0` in the root `Cargo.toml` because the watchdog transport is built on
ureq's `unversioned` connector API, which is explicitly outside its semver
promises; any version bump must revisit the `Transport` impl in
`src/http.rs` and rerun the watchdog tests. Dev-only: `noob-testkit` (the
scripted mock server and fixture chunk loader) and `tempfile`.

## What the tests prove

`tests/watchdog.rs`: reads resume across 1 s ticks and a dripped body
arrives intact; idle and first-byte stalls trip their typed timeouts
promptly; the idle clock starts only at the first body byte; an interrupt
aborts a silent read and a blocked send within about a tick; connection
refused is a typed `Connect` error naming the URL.

`tests/streaming.rs`: text arrives as ordered deltas that concatenate to the
turn; the JSON-200 guard parses instead of feeding SSE; a completion past
8 MiB is not capped; 5xx retries then succeeds and exhaustion surfaces
`Http`; `Retry-After` outranks the backoff schedule; no retry after the
first content byte; the 400 compat strip works and is remembered per
endpoint URL, and an ordinary 400 is not retried; a severed EOF is a turn
error; a mid-stream interrupt returns partial output; keep-alive reuses one
TCP connection across turns; the idle clock engages when body bytes ride
with the headers; a Responses round trip asserts `store: false`, flattened
tools, and verbatim replay; `hot_reload_env` proves a key rotation applies
on the next call; `no_output_cap` scans every recorded body for the
`max_tokens` family.

`tests/fixtures.rs` with `testdata/sse/`: captured real transcripts replayed
through the real adapters over the mock server, `%%CHUNK%%` sentinels
forcing the nastiest TCP splits (mid-`data:`, mid-JSON-key,
mid-codepoint). `llamacpp-qwen-toolcall.sse`: one streamed call with usage;
`llamacpp-qwen-parallel.sse`: two calls, distinct ids, emission order;
`llamacpp-responses-toolcall.sse`: typed events, verbatim `raw_items`, and
the request-shape asserts; `openrouter-keepalive-error.sse`: comment
keepalives, mid-codepoint splits reassembled exactly, and an in-band error
becoming `Finish::Error`. A property test re-splits the toolcall capture at
every byte offset and requires the identical assembled turn.

`tests/hot_env.rs`: `resolve_endpoint` semantics: fresh reads, the missing
`NOOB_BASE_URL` error names the file and the fix, style defaults by host and
accepts the override, every `NOOB_REASONING` spelling resolves, bad values
are `Config` errors naming the valid ones, trailing slashes are trimmed.

`tests/live_smoke.rs`: ignored by default; `./dev.sh smoke` drives both wire
shapes against a live local llama.cpp, including the tool-result replay leg.

## How to modify this blackbox safely

Server quirks live in `src/assemble.rs` (chat) and the `State` machine in
`src/responses.rs`. When a new server misbehaves, capture its bytes into
`testdata/sse/` with `%%CHUNK%%` splits at the ugly spots and make the
fixture pass end to end; the resplit property test then holds the fix
against every framing. Extending the Responses event vocabulary is additive,
unknown types are already ignored. Adding an operation or event variant is a
minor bump; changing the meaning of `Turn.raw_items`, the `Finish` set, or a
settings key is breaking, so add the new shape alongside and migrate callers
before removing the old one. Callers never parse wire JSON themselves; if a
caller needs something the wire carries and the `Turn` does not, the fix is
a new field or event here.
