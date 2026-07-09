# crates/noob-provider

Transcript in, events out. The only crate that touches the network, and the
sole owner of ureq (pinned =3.3.0; we build on its unversioned transport API).

Public surface: `http::Client` (blocking, watchdog-guarded, streaming via
`StreamBody`), `resolve_endpoint` + `run_turn` (lazy per-request `.env`
resolution, events through a callback), the neutral types (`TurnRequest`,
`Item`, `ToolSpec`, `Turn`, `Event`, `ToolCall`, `ProviderError`), the
`envfile` parser, the byte-exact SSE parser (`sse`, the only one in the
binary; MCP Streamable HTTP reuses it in P4), and the chat delta assembler
(`assemble`). Adapters per wire shape: `chat` and `responses`, both
complete with the quirk matrix from ARCHITECTURE.md.

Invariants:
- `.env` is opened, parsed, and dropped inside every request build; nothing
  is cached, so key edits apply on the next call. Secrets never enter the
  process environment.
- Typed timeouts (connect 10 s, DNS 5 s, request send 30 s, first-byte 300 s,
  idle 90 s) via a 1 s tick-read watchdog on a custom transport; the idle
  clock starts only at the first body byte, so long llama.cpp prompt
  processing never trips it. Interrupts abort reads within about one tick
  (DNS and connect are bounded but not tick-interruptible), including
  between retry attempts.
- Retries happen only before the first streamed content byte: connect/TLS
  errors and 408/425/429/5xx, three backoff slots (1/2/4 s, full jitter),
  `Retry-After` honored up to 60 s. Mid-stream death is a turn error, never
  a silent retry.
- Reactive compat: a 400 naming a strippable non-core field we sent (in
  practice `stream_options`) gets one immediate retry without it,
  remembered per client lifetime; no persisted quirk registry.
- A 200 answering `application/json` to a `stream: true` request is parsed
  as a whole completion, not fed to the SSE parser.
- Proxy env vars (HTTP_PROXY and friends) are explicitly ignored: noob talks
  only to the configured endpoints.
- No request ever carries a max_tokens-family key.
- Responses requests are stateless replays: `store: false` always, never
  `previous_response_id`; prior output items replay verbatim from their
  captured wire form (`Turn::raw_items`).
- Reasoning is surfaced as events and kept for display, never serialized
  back on the chat shape.
- Typed errors only, and every rendered message states its remedy.
- No knowledge of files, tools, or the agent loop.
