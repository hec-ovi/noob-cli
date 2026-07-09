# config/

The default source for the `/config` bind mount, and home of the committed
`.env.example`.

At runtime the mounted directory holds: `.env` (flat KEY=VALUE, re-read on
every request, so edits apply with no restart), `mcp.json`, an optional
`AGENTS.md`, `skills/`, and `sessions/`. State lives here, never in the image.

Invariants: never commit real secrets to this folder; only `.env.example` and
this contract are tracked. Keys from `.env` never enter the process
environment.
