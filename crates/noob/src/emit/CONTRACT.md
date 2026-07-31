# emit

contractVersion: 1.0.0

## Purpose

The `NOOB_EMIT` side channel: `noob-proto` Event frames written beside the
session for anything watching, off by default and byte-invisible to every
human surface when off.

## Turning on

`Emitter::from_env()` reads `NOOB_EMIT`. Unset or empty means off. A value is
a path, opened create-append; a path that cannot be opened means off, never a
failed session. `Emitter::to(sink)` turns on against any `Write + Send` sink,
which is how `serve` streams frames over its own stdout and how tests watch a
buffer.

## Public surface

```rust
pub const EMIT_VAR: &str = "NOOB_EMIT";

pub fn as_call<T>(call_id: &str, body: impl FnOnce() -> T) -> T;
    // run body as that tool call: frames built inside carry the id
pub fn current_call() -> Option<String>;    // the call this thread is inside

pub struct Emitter;   // Clone + Default; Default is off, clones share the sink
impl Emitter {
    pub fn from_env() -> Emitter;                       // NOOB_EMIT path, or off
    pub fn to(sink: Box<dyn Write + Send>) -> Emitter;  // on, to any sink
    pub fn is_on(&self) -> bool;
    pub fn at_ms(&self) -> u64;                         // ms since the stream opened
    pub fn send(&self, event: Event);                   // one frame, flushed
    pub fn metrics(&self, group: &str, samples: Vec<Sample>);
        // one metrics frame stamped with at_ms
    pub fn cancel_open_calls(&self);   // tool.end for every call still open
}

pub struct Progress;   // live lines from one producer, never blocking it
impl Progress {
    pub fn for_current_call(emitter: &Emitter) -> Option<Progress>;
        // frames as tool.progress under this thread's call id
    pub fn for_agent(emitter: &Emitter, agent_id: &str) -> Option<Progress>;
        // frames as agent.output
    pub fn feed(&mut self, bytes: &[u8]);   // one raw chunk in, complete lines out
    pub fn flush(&mut self);                // send the trailing unterminated line
    pub fn finish(self);                    // flush, then wait for queued lines to land
}

pub fn file_edit(path: String, before: &str, after: &str, call_id: Option<String>) -> Event;
    // a file.edit frame carrying only the changed middle,
    // its span 1-based inclusive in the written file's coordinates
```

Test rig, in-crate only: `#[cfg(test)] pub struct Buf` (a shared readback
buffer implementing `Write`, with `text()` and `frames()`) and
`pub fn watched() -> (Emitter, Buf)`, so any test in the crate can watch what
its subject emitted.

## Off is a no-op

`Emitter::default()` is off. When off, `send`, `metrics` and
`cancel_open_calls` return without writing, opening or tracking anything, and
both `Progress` constructors return `None`. `for_current_call` also returns
`None` outside a call, on or off.

## Errors

None cross this surface: no `Result`, no panic on the emit path. Every failure
is absorbed where it happens:

- a `NOOB_EMIT` path that cannot be opened: emission is off (`from_env`)
- a failed write or a poisoned sink lock: the frame is dropped (`send`)
- a full progress queue: the line is dropped and the count of dropped lines is
  sent as its own line once there is room
- a progress writer thread that could not spawn or has exited: the line is
  dropped

## Invariants

1. Off is the default and off writes nothing, so byte identity of every human
   surface is a property of the type. The boundary test asserts stdout and
   stderr are byte-identical with and without the sink.
2. One frame per line, written and flushed per frame: the two interrupt exits
   call `libc::_exit`, which flushes nothing, so a buffered frame would be
   lost.
3. Under `NOOB_EMIT` frames go to their own append-opened file, never to the
   session's stdout or stderr; under `serve` stdout is the frame stream
   itself, handed to `to()`.
4. The emitter tracks every `tool.start` until its `tool.end`.
   `cancel_open_calls` closes each call still open with summary `canceled`,
   `elapsed_ms` 0 and an error of kind `canceled`, message `canceled by user`;
   a second call closes nothing.
5. `metrics` stamps `at_ms` as milliseconds since the stream opened, and every
   clone reports the same epoch, so a consumer builds a time series without a
   clock of its own.
6. Feeding a `Progress` never blocks on the consumer. Lines queue up to 512
   behind a writer thread; when the queue is full lines are dropped and the
   count is reported as its own line, never silently.
7. Progress framing: `\r` ends a line like `\n`; blank lines are not sent; a
   line is clipped to 400 characters plus an ellipsis mark; an unbroken
   partial line past 8192 bytes is sent rather than buffered; a UTF-8
   character split across two chunks survives; after 5000 lines one notice
   line is sent and the tap goes quiet.
8. `finish` flushes and then joins the writer, so a call's live lines are all
   written before the caller sends the frame that closes the call.
9. `file_edit` trims the common prefix and suffix and carries only the changed
   middle, with the span 1-based inclusive in the written file's coordinates;
   an unchanged file is an empty region, not a resend.
10. The call id is per thread: set for the duration of `as_call`, cleared on
    the way out, and never inherited by a spawned thread.

## Dependencies

Contracts: [`crates/noob-proto/CONTRACT.md`](../../../noob-proto/CONTRACT.md)
for `Event`, `Span`, `Sample`, `ToolError` and `encode`. Crates: none beyond
std (`serde_json` appears only in tests, to read frames back).

## Boundary test

`crates/noob/tests/e2e_emit.rs`, against the compiled binary and the mock
server: off by default, byte identity when on, the full frame stream of a
turn. Cargo requires integration tests to live in `crates/noob/tests/`, so box
tests for in-crate boxes live there by convention, named for the box.
