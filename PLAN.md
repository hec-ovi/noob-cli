# noob-cli status and release plan

Status date: 2026-07-18.

noob 0.3.7 makes the dock keep its promises under interaction: a message typed during a turn queues ([queued]) instead of interrupting, so the turn, its plan, and every sub-agent keep running and the message is answered next; only double-Escape or Ctrl-C stops a turn. The plan checklist and the agents counter are pinned once above the input, across turns and at the idle prompt, and are no longer re-recorded into the transcript at every turn end. Resizing the terminal now erases the frame by its physical reflowed height (VTE-style rewrap aware), so shrinking the window repaints cleanly instead of shredding the screen with rule fragments.

noob-cli is one static Rust binary in a Docker runtime, targeting OpenAI-compatible endpoints. [ARCHITECTURE.md](ARCHITECTURE.md) describes the runtime design; this file tracks release gates and open items.

## Verification

| Gate | Result |
|---|---|
| Strict workspace Clippy | clean |
| Offline suite (host and Docker) | 726 pass |
| Interactive `e2e_ui` suite | 84 pass |
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
