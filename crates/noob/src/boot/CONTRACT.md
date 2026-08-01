# boot

contractVersion: 1.0.0

## Purpose

The session bootstrap every surface shares: flags to an assembled `Agent`.
The REPL, exec, child and serve all call the same `bootstrap`, so what a
session is made of is decided in exactly one place.

## Public surface

```rust
pub(crate) struct BootArgs;   // overrides, sandbox, plan/read-only/web-only,
                              // verbose, excluded skills, session choice,
                              // emitter override; BootArgs::new for the
                              // common shape, fields set directly after
pub(crate) fn bootstrap(boot: BootArgs, ui: &mut Ui)
    -> Result<(Agent, bool), String>;   // the agent, and whether an explicit
                              // --resume missed (display-only)
pub(crate) fn value_for(flag: &str, next: Option<&String>, usage: &str)
    -> Result<String, String>;          // one flag value, refusing another flag
pub(crate) fn model_label(config_dir: &Path, ov: &Overrides) -> String;
pub(crate) fn current_depth() -> u32;   // NOOB_DEPTH, 0 for the user's agent
```

## Errors

`bootstrap` returns one human line when the session cannot start (config,
endpoint, session store); it never panics. `value_for` refuses a missing or
flag-shaped value with the caller's usage line.

## Dependencies

Contracts: [`config`](../config/CONTRACT.md) (dir, settings, sandbox),
[`session`](../session/CONTRACT.md) (resume), [`skills`](../skills/CONTRACT.md)
(discovery), [`mcp`](../mcp/CONTRACT.md) (servers), [`tools`](../tools/CONTRACT.md)
(registry and ToolCtx), the agent box (Agent, prompt assembly),
[`emit`](../emit/CONTRACT.md) (the frame emitter).

## Tests

The surfaces test it end to end: every pty and e2e suite boots real sessions
through this path (`crates/noob/tests/`).
