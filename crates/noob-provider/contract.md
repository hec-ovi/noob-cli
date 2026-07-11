# crates/noob-provider

Neutral transcript in, semantic events and a complete turn out. This is the only runtime crate that owns the HTTP dependency.

Public entry points include endpoint resolution, owned and borrowed turn requests, Chat and Responses adapters, the watchdog HTTP client, SSE parsing, and neutral item, tool, event, finish, usage, and error types.

Invariants:

- `.env` is opened and dropped during every request build; key changes apply to the next request.
- No request carries a model output-length field.
- Streamed and whole-JSON model completions have no application length cap.
- Request writes check interruption on one-second ticks and fail after 30 seconds without write progress.
- Connect, first-byte, idle, retry, and `Retry-After` behavior is typed and tested.
- Retries happen only before content reaches the caller.
- Responses use stateless replay and preserve captured raw items.
- Proxy environment variables are ignored.
- Errors and incomplete finishes are typed.
- The crate has no knowledge of files, tools, sessions, or terminal rendering.
