# noob-cli status and release plan

Status date: 2026-07-18.

noob 0.3.8 cleans up the four remaining dock visuals caught on 0.3.7 screenshots. Resize handling drops the wrap-aware erase (it still shredded rows on real VTE) for a viewport reset: clear screen, home, repaint, with the old screen pushed into scrollback. The [queued] marker no longer goes stale: a queued message waits as a dim pinned row and enters the transcript as a plain record only at dispatch. Every erase-and-redraw cycle reaches the terminal as one batched write, so the pinned plan no longer blinks while output streams. A fully completed plan is retired at turn end into one timed transcript record and unpinned; unfinished plans stay pinned as before.

noob-cli is one static Rust binary in a Docker runtime, targeting OpenAI-compatible endpoints. [ARCHITECTURE.md](ARCHITECTURE.md) describes the runtime design; this file tracks release gates and open items.

## Verification

| Gate | Result |
|---|---|
| Strict workspace Clippy | clean |
| Offline suite (host and Docker) | 728 pass |
| Interactive `e2e_ui` suite | 85 pass |
| Live qwen dock runs (queue, plan retire, mid-turn resize) | 3 pass |
| Opt-in live suite | 9 pass |
| Static musl binary | 4,326,272 bytes, limit 8 MiB |
| Runtime crate graph | 40 crates, limit 45 |
| Host installer and wrapper | covered |
| Standalone `websearch web-search` | covered |
| stdio `websearch mcp` handshake | covered |

## Remaining

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
