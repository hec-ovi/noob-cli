# crates/noob-provider

Transcript in, events out. The only crate that touches the network, and the
sole owner of ureq (pinned =3.3.0; we build on its unversioned transport API).

Public surface: `http::Client` (blocking, watchdog-guarded),
`resolve_endpoint` + `run_turn` (lazy per-request `.env` resolution), the
neutral types (`Turn`, `Event`, `ToolCall`, `ProviderError`), and the
`envfile` parser. Adapters per wire shape: `chat` now, `responses` in P1.

Invariants:
- `.env` is opened, parsed, and dropped inside every request build; nothing
  is cached, so key edits apply on the next call. Secrets never enter the
  process environment.
- Three timeouts (connect 10 s, first-byte 300 s, idle 90 s) via a 1 s
  tick-read watchdog on a custom transport; the idle clock starts only at the
  first body byte, so long llama.cpp prompt processing never trips it.
  Interrupts abort within about one tick.
- No request ever carries a max_tokens-family key.
- Typed errors only, and every rendered message states its remedy.
- No knowledge of files, tools, or the agent loop.
