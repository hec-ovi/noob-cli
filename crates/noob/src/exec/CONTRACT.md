# exec

contractVersion: 1.0.0

## Purpose

The one process runner: run a child command to completion, collect its
merged output bounded, and leave nothing behind. Every tool that shells out
goes through here; the caller owns the argv and the verdict, this box owns
the process lifecycle.

## Public surface

```rust
pub(crate) fn run(
    command: Command,
    program: &str,          // names the binary in the spawn-failure message
    timeout_s: u64,
    head: usize,            // truncation budget for the collected output
    tail: usize,
    progress: Option<emit::Progress>,   // live tap; None costs one branch
) -> Result<Run, RunError>;

pub(crate) struct Run { pub code: i32, pub body: String, pub elapsed: Duration }
pub(crate) enum RunError {
    Canceled { body: String, elapsed: Duration },
    TimedOut { body: String, timeout_s: u64 },
    Spawn(String),
}

pub(crate) fn kill_group(child: &mut Child);
    // for long-lived children spawned into their own group: kill the whole
    // tree, then reap the leader so no zombie remains
```

## The capability, platform-neutral

What callers may rely on, however a platform implements it:

- The child and every process it starts form one killable unit; timeout,
  cancellation, and end-of-call cleanup terminate the whole unit.
- stdout and stderr arrive merged, interleaved as a terminal would show
  them, bounded to the head+tail budget with an honest omission marker.
- A fast producer can never deadlock the runner, and an abandoned output
  stream can never grow memory without bound.
- The leader's real exit code survives cleanup.
- The result says what really happened: output left behind by surviving
  background processes earns an explicit note, killed stragglers earn
  another, and neither claim is ever false.

The implementation here is unix: a new session per child so `kill(-pgid)`
reaches the tree, exit detected with `waitid(WNOWAIT)` so the zombie pins
the pgid and a group kill can never hit a recycled group, reparented group
members reaped after the leader. A process that leaves the group (`setsid`)
with its stdio redirected is the one supported way to keep a daemon running.

## Errors

`RunError` is the closed set: `Canceled` and `TimedOut` carry the partial
output because both mean the verdict is unknowable, and `Spawn` names the
program and the reason. Everything else is a normal `Run` with the child's
exit code.

## Dependencies

Contracts: [`crates/noob/src/emit/CONTRACT.md`](../emit/CONTRACT.md) (the
optional live tap; bytes go to the buffer first, the tap after, so the
model's copy never depends on whether anybody is watching). Internal:
`buffer.rs`, the streaming head+tail collector.

## Tests

`buffer.rs` carries the collector's tests. The runner's behavior is proven
through its real callers: the bash tool's suite in
`crates/noob/src/tools/bash.rs` (group kill, straggler notes, zombie
reaping, the detached-daemon path, exit-code survival) and the websearch
tool's error paths.
