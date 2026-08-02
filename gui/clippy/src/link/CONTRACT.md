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
    pub fn send(&mut self, command: Cmd);  // one Command frame, at the agent's version
    pub fn drain(&mut self) -> Vec<Incoming>;  // everything since last
    pub fn speaks(&self) -> Option<u16>;   // the agent's protocol version, once it has said
    pub fn is_alive(&self) -> bool;
    pub fn shutdown(&mut self);
}
pub fn command_for(...);     // which noob binary, and its argv
pub fn env_command(program: &str, workspace: &Path, clear: &[&str]) -> Command;
pub fn env_from(ok: bool, stdout: &[u8], stderr: &[u8]) -> Result<Vec<String>, String>;
                             // noob debug env, run and read the way serve
                             // is started, for the panel's environment block
pub fn doctor_command(program: &str, workspace: &Path, clear: &[&str]) -> Command;
pub fn online_from(stdout: &[u8]) -> bool;
                             // noob doctor, run the same way, read down to
                             // one answer: did the endpoint answer
```

## Invariants

1. Reading happens on its own thread over a channel: a burst of frames or
   a dead child can never block a frame of the interface.
2. Malformed lines are skipped (the proto contract's tolerant reader);
   trouble surfaces as `Incoming::Trouble`, not a panic.
3. Shutdown ends the child and the reader; no zombie survives the window.
4. Commands go out at the version the agent speaks, learned from its first
   frame, and one typed before that is held until it arrives. An agent one
   release behind reads every command instead of dropping it unread.
5. An agent ahead of this window is the one skip that is said out loud: its
   frames cannot be read, so a `Trouble` line names both versions once.

## Dependencies

Contracts: [`noob-proto`](../../../../crates/noob-proto/CONTRACT.md)
(frames), [`serve`](../../../../crates/noob/src/serve/CONTRACT.md) (the
surface it drives; nothing deeper in the CLI).

## Tests

Inline: frame decode paths, trouble surfacing, a missing binary, the env
command and its parsing, a command written at an older agent's version,
the connection check's reading of a doctor report (7 tests).
