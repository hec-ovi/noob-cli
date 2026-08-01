# link

contractVersion: 1.0.0

## Purpose

The agent as a supervised child: `noob serve` with piped stdio, commands
down stdin, frames up stdout, and the guarantee that neither pipe ever
blocks the interface. Nothing here knows what a frame means.

## Public surface

```rust
pub enum Incoming;           // one decoded Event frame, or Trouble
pub struct Link;
impl Link {
    pub fn spawn(...) -> Link;             // start noob serve
    pub fn send(&mut self, command: Cmd);  // one Command frame
    pub fn drain(&mut self) -> Vec<Incoming>;  // everything since last
    pub fn is_alive(&self) -> bool;
    pub fn shutdown(&mut self);
}
pub fn command_for(...);     // which noob binary, and its argv
```

## Invariants

1. Reading happens on its own thread over a channel: a burst of frames or
   a dead child can never block a frame of the interface.
2. Malformed lines are skipped (the proto contract's tolerant reader);
   trouble surfaces as `Incoming::Trouble`, not a panic.
3. Shutdown ends the child and the reader; no zombie survives the window.

## Dependencies

Contracts: [`noob-proto`](../../../../crates/noob-proto/CONTRACT.md)
(frames), [`serve`](../../../../crates/noob/src/serve/CONTRACT.md) (the
surface it drives; nothing deeper in the CLI).

## Tests

Inline: frame decode paths, trouble surfacing, a missing binary (3 tests).
