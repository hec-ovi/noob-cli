# noob

contractVersion: 1.0.0

## Purpose

The agent binary. `crates/noob/src/main.rs` is the composition root: argv
dispatch, bootstrap wiring, and the REPL's slash commands; every capability
behind it lives in a box with its own contract.

## The command surface

```
noob                       interactive REPL (flags forwarded: --plan,
                           --yolo, --resume <id>, --restore <id>, --model,
                           --base-url, --verbose, ...)
noob exec -p "<prompt>"    one turn, text out; --json for the JSONL stream
noob serve ...             the front-end surface (see src/serve/CONTRACT.md)
noob child                 internal: the sub-agent entry point
noob sessions              list saved sessions
noob tokens ...            token counting utilities
noob doctor                environment and endpoint checks
noob debug ...             prompt and wiring inspection
noob --version
```

Unknown commands name the available ones and exit 2. `exec` exits nonzero
when the turn failed; `child` writes exactly one JSON result line to stdout
(`{"status", "result", "turns", "usage"}`) with progress on stderr.

## The boxes behind it

agent, ui, tools, exec, term, session, skills, mcp, subagent, emit, config,
serve (each `src/<box>/CONTRACT.md`), over the workspace crates noob-proto,
noob-provider, noob-testkit. `docs/INDEX.md` maps them.

## Tests

`crates/noob/tests/`: per-box boundary suites (e2e_*, ui_*, session
recovery, install bundle) plus the budget and egress gates. One command
runs everything: `./dev.sh test-all`.
