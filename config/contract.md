# config/

The default source for the `/config` bind mount. It contains the committed
`.env.example`, a websearch MCP example, and the bundled web-search skill.

At runtime the mounted directory can hold `.env`, `mcp.json`, an optional
`AGENTS.md`, `skills/`, caches, and `sessions/`. Provider keys in `.env` are
re-read for each model request. Context, sandbox, task budgets, MCP config,
and an autodetected endpoint are process-start settings.

Invariants: never commit real secrets. `NOOB_API_KEY` is read from `.env` only
and is not copied into the process environment. State lives in this mount,
never in the image.
