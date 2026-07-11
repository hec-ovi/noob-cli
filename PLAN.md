# noob-cli status and release plan

Status date: 2026-07-11.

## Completed

- Audited the codebase, contracts, user feedback, current Zero and Codex source, and the current websearch package.
- Removed request-side model output limits and full-transcript request cloning.
- Hardened provider writes, retries, finish states, MCP transports, Bash collection, file reads, child-agent pipes, and Git skill installation.
- Bounded retained tool, progress, diagnostic, catalog, JSON, SSE, and stdio-queue data without capping model completions or child final results.
- Made session replay streaming and persistence failures visible, with dangling-call repair and provider-valid interruption handling.
- Made parallel scheduling report real start and completion order while preserving transcript emission order.
- Rebuilt the interactive path around one terminal owner, a default three-row dock, typeahead, queueing, confirmations, cancellation, Markdown, JSON, tables, sanitization, and four themes.
- Added an isolated Linux installer and `noob` launcher with `noob --restore <session>`, caller UID/GID mapping, workspace and config mounts, and safe overwrite behavior.
- Added amd64 and arm64 musl target selection plus Python 3, uv, and `websearch-skill==0.1.0` to the runtime image.
- Seeded both standalone web-search skill instructions and lazy stdio MCP configuration from the same package.
- Kept host API keys out of the container environment and documented exact reload boundaries.
- Made endpoint autodetection controllable and the live runner endpoint-overridable.
- Removed parallel-test interference from the process-wide interrupt flag.
- Organized the work as eleven logical commits on `main`, pushed continuously to `origin/main`.

## Verification

| Gate | Result |
|---|---|
| Strict workspace Clippy | pass |
| Host offline suite | 522 pass |
| Docker offline suite | 522 pass |
| Real PTY interaction suite | 24 pass |
| Opt-in live suite | 8 pass |
| Static musl binary | 3,748,736 bytes, limit 8 MiB |
| Runtime crate graph | 40 crates, limit 45 |
| Runtime image | 90,275,508 bytes on amd64 |
| Actual host installer and wrapper | pass |
| Standalone live `websearch web-search` | pass |
| stdio `websearch mcp` initialize | pass, protocol 2025-11-25 |

The final live suite used the running Qwen3.6 27B endpoint (`obliterated-27b-mtp`) at `http://localhost:8080/v1` with a 131072-token context, plus an isolated Streamable HTTP websearch endpoint. An earlier interactive Qwen3.6 35B drive covered headings, emphasis, JSON fences, tables, typeahead, queueing, active Bash status, and cancellation.

## Remaining

No implementation requirement from the current request remains open.

- An arm64 hardware smoke remains advisable before publishing an arm64 release artifact; target selection and the Docker build path are implemented, while the final image measurement above is amd64.
- Version bump, release tag, and GitHub release are intentionally pending until requested.
- Telegram integration remains opt-in and was not changed.
- Interface enhancements such as history navigation, slash completion, and resize handling remain in [docs/UI_PLAN.md](docs/UI_PLAN.md).

## Release invariants

- No lint or test failures.
- No protocol change to piped REPL, `exec`, JSONL, or child output.
- No request-side output limit and no application cap on model or child final output.
- No unbounded retention for tool, progress, diagnostic, or hostile integration streams.
- Sessions remain provider-valid after interruption or persistence failure.
- Release binary stays below 8 MiB with no more than 45 runtime crates.
