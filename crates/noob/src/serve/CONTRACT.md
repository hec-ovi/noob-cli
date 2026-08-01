# serve

contractVersion: 1.0.0

## Purpose

The surface a front end drives: `noob serve` reads noob-proto Command frames
on stdin and writes Event frames on stdout, one JSON object per line. No
terminal is involved; stdin and stdout are the whole session.

## Invocation

```
noob serve [--resume <id> | --session <id>] [--model <name>]
           [--base-url <url>] [--plan] [--verbose] [--yolo]
```

Frame shapes are the noob-proto contract's; this box owns which commands the
agent answers and when.

## Behavior

- One session per process: bootstrap runs once, `session.start` announces
  it, and every prompt drives a full agent turn whose ui events stream out
  as frames.
- Unknown or unanswerable frames are ignored, never refused: a front end
  built against a newer agent loses a feature, not its session.
- `turn.cancel` sets the shared interrupt; the running turn winds down
  exactly as a Ctrl-C would, and the flag resets before the next prompt.
- Exactly one frame stream: stdout carries frames and nothing else; human
  diagnostics go to stderr.
- Every frame is also recorded beside the session
  (`<id>.frames.jsonl`), the prompts included as `user.echo` (record-only:
  the front end that sent one already echoed it). At resume the record
  streams back first, lifecycle frames excepted, so a front end rebuilds
  every pane from what already happened; a session that predates the
  record replays its transcript mapped to frames instead, and gains a
  record from then on. A record that cannot be written costs the replay,
  never the session.

## Errors

A bootstrap failure prints one human line to stderr and exits nonzero. After
a successful start the process survives malformed input by skipping it (the
proto contract's tolerant-reader rule) and ends when stdin closes.

## Dependencies

Contracts: [`noob-proto`](../../../noob-proto/CONTRACT.md) (every frame),
the agent box (the turn), [`emit`](../emit/CONTRACT.md) (stdout is the
emitter's sink here), [`config`](../config/CONTRACT.md) (bootstrap). The
GUI's link box consumes this contract and nothing deeper.

## Tests

`crates/noob/tests/e2e_serve.rs`: the frame stream of real sessions against
the real binary, including cancel, resume, and the degradation rule.
