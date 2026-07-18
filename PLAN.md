# noob-cli status and release plan

Status date: 2026-07-18.

noob 0.3.10 gives the idle prompt the active frame's exact layout: top rule, pinned plan/agents rows inside the frame, input row, bottom rule. Pinned rows used to sit above the top rule between turns, which read as the agents counter floating loose in the transcript. Queued rows are now styled like the record they become (green marker, plain text) with only the trailing [queued] tag in non-bold green. Resize remains the one known pending issue (stale idle frames and blank gaps in scrollback after repeated resizes; noted in the README).

noob-cli is one static Rust binary in a Docker runtime, targeting OpenAI-compatible endpoints. [ARCHITECTURE.md](ARCHITECTURE.md) describes the runtime design; this file tracks release gates and open items.

## Verification

| Gate | Result |
|---|---|
| Strict workspace Clippy | clean |
| Offline suite (host and Docker) | 730 pass |
| Interactive `e2e_ui` suite | 86 pass |
| Live qwen dock runs (queue, plan retire, mid-turn resize, idle layout, queued tag) | 5 pass |
| Opt-in live suite | 9 pass |
| Static musl binary | 4,326,272 bytes, limit 8 MiB |
| Runtime crate graph | 40 crates, limit 45 |
| Host installer and wrapper | covered |
| Standalone `websearch web-search` | covered |
| stdio `websearch mcp` handshake | covered |

## Remaining

- Resize is unstable (known issue, noted in the README): the viewport reset repaints the live frame, but repeated resizes leave stale idle frames and blank gaps in scrollback history.
- An arm64 hardware smoke remains advisable before publishing an arm64 release artifact; target selection and the Docker build path are implemented and exercised on amd64.
- Telegram integration is opt-in.
- Interface enhancements such as history navigation remain in [docs/UI_PLAN.md](docs/UI_PLAN.md).

## Release invariants

- No lint or test failures.
- No protocol change to piped REPL, `exec`, JSONL, or child output.
- No request-side output limit and no application cap on model or child final output.
- No unbounded retention for tool, progress, diagnostic, or hostile integration streams.
- Sessions remain provider-valid after interruption or persistence failure.
- Release binary stays below 8 MiB with no more than 45 runtime crates.
