# What is left to build

Written for someone picking this up cold. Everything here was checked against
the tree it describes; where a claim rests on a specific line, the line is
named. Two things remain on the roadmap, in this order: native binaries for the
three desktop platforms, then the GPU front end. The second cannot start before
the first, because there is no per-platform binary for a window to sit in front
of yet.

## Where the project actually is

noob is a terminal coding agent in Rust that talks to any OpenAI-compatible
endpoint. It is developed against a local llama.cpp server (`http://localhost:8080/v1`,
model alias `llm`). `README.md` covers what it does; `ARCHITECTURE.md` covers how.

What ships today is one Linux static binary living inside a Docker image, plus a
shell launcher on the host. `install.sh` builds the `noob:local` image and copies
`scripts/noob` to `~/.local/bin/noob`. That launcher is a bash script: it runs
`docker run` with the working directory bind-mounted at `/work` and the config
directory at `/config`. `docker/Dockerfile` cross-builds
`x86_64-unknown-linux-musl` or `aarch64-unknown-linux-musl` depending on
`TARGETARCH`. So "installing noob" today means "installing Docker and a script
that calls it".

## Task 1: native binaries for macOS, Windows, and Linux

The goal is a binary a person downloads and runs, on all three, with no Docker.

### What has to change, in the order the compiler will force

**The tree has almost no platform gating.** The only `#[cfg(unix)]` in the whole
workspace is in `crates/noob/src/config/mod.rs` (four sites, around lines 11,
170, 485, 516). Everything else assumes Unix unconditionally. So the work is not
"fix a few warnings", it is "decide, per subsystem, what the Windows path is".

**Two calls are Linux-only, not merely Unix-only.** These will fail on macOS as
well as Windows, so they are the first thing to look at:

- `crates/noob/src/tools/bash.rs:524` uses `prctl(PR_SET_CHILD_SUBREAPER)` so
  orphaned grandchildren of a shell command reparent here and can be reaped.
- `crates/noob/src/subagent/mod.rs:376` uses `prctl(PR_SET_PDEATHSIG, SIGTERM)`
  so a detached sub-agent dies with its parent instead of outliving it.

Neither exists on macOS. The usual macOS substitute is a process group per job
plus `killpg`, with `kqueue`/`NOTE_EXIT` if you need the death notification.
Windows has a genuinely better primitive for both: a Job Object with
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, which kills the whole tree when the handle
closes. Treat "kill the whole process tree, reliably, when we go away" as one
capability with three implementations, not as three unrelated fixes.

**The terminal layer is raw termios.** `crates/noob/src/ui/prompt.rs` holds 24
`libc` calls: `tcgetattr`/`tcsetattr` for raw mode (around lines 1007 and 1092),
`read` on `STDIN_FILENO`, and `ioctl(TIOCGWINSZ)` for the terminal size (lines
542 and 557). Windows needs the console API or a crate over it. Decide early
whether to introduce a dependency here, because `dev.sh size-check` enforces a
45-crate runtime graph and an 8 MiB binary, and a terminal crate is not free.

**Signals.** `crates/noob/src/main.rs` installs `sigaction` handlers for SIGINT
(line 1461 and nearby) and blocks signals with `pthread_sigmask` (line 1486);
`crates/noob/src/ui/dock.rs:379` unblocks in the dock thread; SIGWINCH drives
resize. Windows has no signals in this sense. Ctrl-C is a console control
handler, and there is no SIGWINCH, so resize has to come from console events.

**The sandbox story changes meaning.** `crates/noob/src/tools/guard.rs:46`
defines two modes: `Container`, where tools run unrestricted because the
container is the boundary, and `Workspace`, where write and edit refuse paths
outside the workspace. A native binary has no container, so `Workspace` becomes
the only real boundary and it needs to hold up. Read that code before assuming
it does; it was written when a container was always there behind it.

**The test suite assumes a pty.** `crates/noob/tests/e2e_ui.rs`,
`e2e_p3.rs`, `e2e_p5.rs`, and `vt.rs` drive the real binary through a pseudo
terminal and assert on rendered screens. That is the most valuable part of the
suite and the least portable. Plan for how these run on Windows before porting,
not after.

### Suggested order

1. macOS first. It is Unix, so only the two `prctl` calls and any Linux-only
   file behaviour block it. This gets you a second platform cheaply and forces
   the process-supervision abstraction into existence.
2. Introduce that abstraction properly: one internal capability for spawning a
   supervised child and killing its tree, with per-platform implementations
   behind it. Do this before Windows, because on Windows it is the whole job.
3. Windows last: terminal, signals, process tree, path handling.
4. Only then work out distribution (per-platform archives, checksums). Note the
   repository rule: do not add or edit anything under `.github/workflows`.

## Task 2: the GPU Vulkan front end

The plan in `README.md` is a separate Rust binary rendering the UI through
Vulkan, with each surface (plan, multi-agent runner, agent management, main
window, code stream) isolated and talking over schema-validated data rather than
shared code.

**It does not need a library target.** An earlier assumption in this project was
that `crates/noob` had to grow a `lib.rs` and make its modules public first.
That is the wrong shape and contradicts the design above. noob already emits a
machine-readable stream, so the front end runs `noob` as a subprocess and reads
it.

### What exists to build against, verified by running it

`noob exec -p "<prompt>" --json` writes one JSON object per line to stdout.
These are real lines from a run against the live server:

```json
{"args":{"path":"/tmp/.../note.txt"},"name":"read","t":"tool"}
{"err":false,"id":"XpNVcDsI7xqjW9tevvflynQxrhJkeb4w","t":"result"}
{"d":"The","t":"text"}
{"t":"done","usage":{"cached_prompt":1334,"completion":2,"prompt":1832}}
```

Emitted from `crates/noob/src/ui/mod.rs`: `text` at line 581, `tool` at 672,
`result` at 722, `done` at 1078.

The session log is a second, durable stream, one JSON object per line under
`<config>/sessions/<id>.jsonl`, documented at the top of
`crates/noob/src/session/mod.rs`: `meta`, `item` (one transcript item), `reset`
(compaction replaced the transcript), and `usage` (one request's cost, as
computed prefill and generated tokens).

### Known gaps in that stream, before you design against it

- **A `tool` event carries no id, but its `result` does.** Pairing a call with
  its result from stdout alone is positional. A front end that wants to show one
  panel per running tool needs the id on both.
- **Only four event kinds reach stdout.** Reasoning deltas, notes, and errors go
  to stderr in this mode, so a front end reading stdout only will silently miss
  them.
- **Nothing is versioned or schema-backed.** There is no schema file and no
  version field on the stream. The repository's stated architecture is
  contract-isolated layers connected by versioned JSON Schema, and this stream
  does not meet that bar yet.

### First step

Do not open a window first. Pin the event stream down as a contract: write the
schema, add the missing ids, decide what belongs on stdout, version it, and put
a test on it. That is one self-contained commit, it is useful on its own for
anyone scripting noob, and it is the thing the front end will be built on top
of. A GUI written against today's unversioned stream will encode its accidents.

## Constraints that are not obvious from the code

- **The fixed prompt has a hard ceiling of 2,000 tokens** and currently measures
  1,938 against the qwen tokenizer, with both shipped skills, MCP configured,
  and all thirteen tools registered. That leaves room for about one more short
  skill index line. `crates/noob/tests/budget.rs` guards this with lower
  ceilings measured through a different tokenizer, so it will not catch a real
  overrun on its own. Measure the real thing with
  `noob debug prompt --json` and the server's `/tokenize` endpoint.
- **Never cap model output.** No `max_tokens` and no word or sentence limits in
  prompts. `crates/noob/tests/budget.rs` enforces both.
- **Every behavioural change ships with a test**, run locally. Do not add or
  modify CI workflows.
- **The repository is not rustfmt-clean.** Running `cargo fmt --all` reformats
  files unrelated to your change. Format only the files you touched, and only if
  they were clean before.
- **The model comparison against opencode is finished.** Do not restart it. The
  token result stands: noob's system prompt is about a quarter of opencode's and
  it used fewer total tokens on two of three tasks. The correctness result does
  not stand: at 35B neither tool reliably produced a browser game that runs, and
  that is a model limit, not something to fix here.
