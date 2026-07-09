# noob/src/config

Config-dir resolution and, from P2, endpoint autodetect and mcp.json loading.

Directory precedence: `NOOB_CONFIG_DIR` > `/config` (the container mount) >
`~/.config/noob`. Setting precedence: CLI flag > process env (non-secret keys
only) > `/config/.env`.

The keys themselves are read lazily per request inside noob-provider; this
module never caches values. Autodetect (P2) probes localhost candidates only,
and only when no base URL is configured.
