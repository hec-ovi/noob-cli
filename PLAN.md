# noob-cli status and release plan

Status date: 2026-07-12.

noob-cli is one static Rust binary in a Docker runtime, targeting OpenAI-compatible endpoints. [ARCHITECTURE.md](ARCHITECTURE.md) describes the runtime design; this file tracks release gates and open items.

## Verification

| Gate | Result |
|---|---|
| Strict workspace Clippy | clean |
| Offline suite (host and Docker) | 586 pass |
| Pty interaction suite | 41 pass |
| Opt-in live suite | 8 pass |
| Static musl binary | within the 8 MiB limit |
| Runtime crate graph | 40 crates, limit 45 |
| Host installer and wrapper | covered |
| Standalone `websearch web-search` | covered |
| stdio `websearch mcp` handshake | covered |

## Remaining

- An arm64 hardware smoke remains advisable before publishing an arm64 release artifact; target selection and the Docker build path are implemented and exercised on amd64.
- Version bump, release tag, and GitHub release are pending until requested.
- Telegram integration is opt-in.
- Interface enhancements such as history navigation and terminal resize handling remain in [docs/UI_PLAN.md](docs/UI_PLAN.md).

## Release invariants

- No lint or test failures.
- No protocol change to piped REPL, `exec`, JSONL, or child output.
- No request-side output limit and no application cap on model or child final output.
- No unbounded retention for tool, progress, diagnostic, or hostile integration streams.
- Sessions remain provider-valid after interruption or persistence failure.
- Release binary stays below 8 MiB with no more than 45 runtime crates.
