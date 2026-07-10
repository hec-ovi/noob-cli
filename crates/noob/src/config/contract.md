# noob/src/config

Config-dir resolution, non-secret settings lookup, sandbox detection,
endpoint autodetect; mcp.json loading lands in P4.

Directory precedence: `NOOB_CONFIG_DIR` > `/config` (the container mount) >
`~/.config/noob`. Setting precedence: CLI flag > process env (non-secret keys
only) > `/config/.env`. API keys are never read here: they stay lazy inside
noob-provider and never enter the process environment.

Sandbox: explicit NOOB_SANDBOX wins, otherwise /.dockerenv decides;
`--yolo` lifts the workspace restriction. Autodetect probes localhost
candidates only (:8090, :8080, :11434, :1234, :8000, in that order, 500 ms
each), and only when no base URL is configured anywhere.
