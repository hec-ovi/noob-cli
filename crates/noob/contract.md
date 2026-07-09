# crates/noob

The binary. `main.rs` is argv dispatch only: `repl` (default, P2) | `exec` |
`child` (P6) | `doctor` (P7) | `debug` (P2) | `--version`. Hand-rolled flag
parsing; no clap.

`exec -p "<prompt>"` is the headless one-shot: final text on stdout, progress
and errors on stderr, meaningful exit codes (0 ok, 1 failure, 2 usage,
130 interrupted).

Owns the SIGINT handler: first Ctrl-C sets the shared interrupt flag (an
in-flight request aborts within about one second), second Ctrl-C hard-exits.

Module map (each with its own contract): `agent/`, `tools/`, `skills/`,
`mcp/`, `task/`, `config/`, `session/`, `ui/`, plus `prompts/` and `tests/`.
