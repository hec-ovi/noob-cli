# noob-cli

An extremely lightweight agentic coding CLI: one static Rust binary that lives
in a Docker sandbox, works on a bind-mounted `/work` folder, and speaks both
OpenAI Chat Completions and Responses APIs against any base URL.

## Build, test, run

Everything runs inside Docker; the host needs docker and nothing else.

- `make test` runs the whole offline suite (unit + e2e against an in-process
  mock server). There is no CI; this is the whole story.
- `make smoke` runs the live suite against local endpoints (opt-in).
- `make docker` builds the runtime image; `make build` just the static binary.
- `docker compose run --rm noob` is the REPL,
  `docker compose run --rm noob exec -p "..."` a one-shot.
- `make size-check` enforces the footprint budgets (binary <= 8 MB, runtime
  crate graph <= 45).

## Layout

- `crates/` the cargo workspace (see its contract.md for the crate map)
- `config/` the committed `.env.example`; the default `/config` mount source
- `docker/` the two-stage musl Dockerfile
- `docs/` design record (research, architecture); not runtime input
- `compose.yml` the one-command entry point

Every folder ships a `contract.md`: what it does, its interface, its
invariants, nothing about the rest of the system.

## Repo-wide invariants

- No request ever carries a max_tokens-family key; output length is never
  capped. The mock server fails any test that tries.
- The binary talks only to the configured endpoints; no telemetry, no update
  checks.
- Commits are small and conventional; tests accompany every behavioral change.
