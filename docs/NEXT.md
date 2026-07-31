# What is left to build

Written for someone picking this up cold. Everything here was checked against
the tree it describes; where a claim rests on a specific line, the line is
named, and the lines were re-read at 0.8.0.

Two things remain on the roadmap, and neither blocks the other: native binaries
for the three desktop platforms, and letting the agent run containers.

The GPU front end is built. It shipped as NO0B at 0.7.0 and is task 2 below,
kept for the design record. It did not wait for native binaries after all: it
runs `noob serve` as a subprocess, so the Docker launcher was enough to build
against on Linux. What native binaries still buy it is macOS and Windows.

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
workspace is in `crates/noob/src/config/mod.rs` (four sites, at lines 11,
171, 487 and 518). Everything else assumes Unix unconditionally. So the work is not
"fix a few warnings", it is "decide, per subsystem, what the Windows path is".

**Two calls are Linux-only, not merely Unix-only.** These will fail on macOS as
well as Windows, so they are the first thing to look at:

- `crates/noob/src/tools/bash.rs:331` uses `prctl(PR_SET_CHILD_SUBREAPER)` so
  orphaned grandchildren of a shell command reparent here and can be reaped.
- `crates/noob/src/subagent/mod.rs:391` uses `prctl(PR_SET_PDEATHSIG, SIGTERM)`
  so a detached sub-agent dies with its parent instead of outliving it.

Neither exists on macOS. The usual macOS substitute is a process group per job
plus `killpg`, with `kqueue`/`NOTE_EXIT` if you need the death notification.
Windows has a genuinely better primitive for both: a Job Object with
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, which kills the whole tree when the handle
closes. Treat "kill the whole process tree, reliably, when we go away" as one
capability with three implementations, not as three unrelated fixes.

**The terminal layer is raw termios.** `crates/noob/src/ui/prompt.rs` holds 24
`libc` calls: `tcgetattr`/`tcsetattr` for raw mode (lines 994, 1007 and 1092),
`read` on `STDIN_FILENO`, and `ioctl(TIOCGWINSZ)` for the terminal size (lines
542 and 557). Windows needs the console API or a crate over it. Decide early
whether to introduce a dependency here, because `dev.sh size-check` enforces a
45-crate runtime graph and an 8 MiB binary, and a terminal crate is not free.

**Signals.** `crates/noob/src/main.rs` installs `sigaction` handlers for SIGINT
(line 1665, and SIGWINCH at 1686) and blocks signals with `pthread_sigmask` (line 1690);
`crates/noob/src/ui/dock.rs:379` unblocks in the dock thread; SIGWINCH drives
resize. Windows has no signals in this sense. Ctrl-C is a console control
handler, and there is no SIGWINCH, so resize has to come from console events.

**The sandbox story changes meaning.** `crates/noob/src/tools/guard.rs:48`
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

## Task 2: the GPU front end. Built, at 0.7.0

Kept because the reasoning is worth not relearning, not because there is work
here.

It shipped as NO0B: `gui/`, its own cargo workspace, `no0b` on PATH,
`./dev.sh gui` to run it and `./dev.sh gui-install` to install it. Its budgets
are 40 MiB and 400 crates against the CLI's 8 MiB and 45, gated by
`./dev.sh gui-check`, and it currently uses 13.4 MiB and 147 crates.
`gui/README.md` is what a user reads.

**It did not need a library target**, which was the standing assumption before
it started: `crates/noob` was going to grow a `lib.rs` and make its modules
public. It runs `noob serve` as a subprocess instead and reads frames off its
stdout, so the two halves share no code and either can be replaced whole.

**The stream was pinned down before the window was opened**, which was the
recorded first step and turned out to be the right one. `crates/noob-proto` is
the contract: newline-delimited JSON, `Event` outward and `Command` inward,
every frame carrying `VERSION`, an `Unknown` variant on both enums so a newer
agent degrades to missing features rather than a dead stream, and one `call_id`
from a tool's start to its end. Serialization is written out by hand because
`derive(Serialize)` is five crates against a 45-crate cap.

The three gaps recorded here before it was built are all closed by that crate:
a `tool` event with no id, only four event kinds reaching stdout, and nothing
versioned or schema-backed.

`noob exec -p "<prompt>" --json` still writes the older, looser four-kind
stream, which is the scripting surface and is unversioned on purpose. A front
end uses `serve`.

The design in the old `README.md` plan that did not ship: one isolated surface
per concern (plan, multi-agent runner, agent management, main window) each
talking to the others over schema-validated data, plus a dedicated code-stream
surface showing each generated file as it is written. What shipped is one window
with eight views on a capped 2x2 grid, and one contract-isolated layer,
`gui/layers/text-geometry`.

What is still queued for the window, in order, none of it blocking anything
else: the classic preset with a pinned single-line input, the orb as a launcher
panel, putting the sandbox mode on the wire, a bridge that lets the `noob`
command talk to a window that is already open, and a files tree.

## Task 3: let the agent run containers

The sandbox has no `docker` binary and no socket, so an agent asked to start
anything containerized has no path at all. It does not fail cleanly either: it
tries pip, then a public instance, then a source install, and burns the
fifty-round cap (`TURN_CAP`, `crates/noob/src/agent/mod.rs:32`) before saying so.
Web search was the case that surfaced this, and it got a targeted fix upstream
(`websearch searxng up` installs SearXNG as a plain process instead), but the
general gap is still there for databases, message queues, and anything else the
agent might reasonably want to stand up.

What is already true and worth not rediscovering:

- **A detached process survives.** `setsid` plus stdio redirected off the
  inherited pipe outlives the group kill that ends every bash call, and earns
  neither straggler warning. Pinned by
  `a_detached_daemon_survives_with_a_clean_result` in
  `crates/noob/src/tools/bash.rs`. So long-running servers are already possible;
  containers specifically are not.
- **The bash tool is foreground-only by design** (`crates/noob/src/tools/bash.rs`,
  the group-kill at the end of `run_inner`). That is what keeps a tool call
  synchronous, and it should stay. Any container support has to work with it,
  not around it.
- **`compose.yml` already uses `network_mode: host`**, so a container started as
  a sibling on the host is reachable from the agent on loopback with no extra
  wiring.

The decision to make first is not technical. Mounting `/var/run/docker.sock`
gives the sandboxed agent root-equivalent control of the host, which dissolves
the thing the sandbox exists to be. Rootless Docker or Podman, a socket proxy
restricted to a few endpoints, or a per-run nested runtime are the alternatives,
and each trades isolation against setup cost differently. Pick that first, then
the tool work is small: install the client in the runtime stage of
`docker/Dockerfile` and say so in a skill so the agent knows the capability
exists.

## Constraints that are not obvious from the code

- **The fixed prompt has a hard ceiling of 2,000 tokens** and measured 1,938
  against the qwen tokenizer on 2026-07-28, with both shipped skills, MCP
  configured, and all fourteen tools registered. That leaves room for about one
  more short skill index line. `crates/noob/tests/budget.rs` guards this with
  ceilings measured through tiktoken, which no served model here uses, so it
  will not catch a real overrun on its own: the same artifact is 1,874 o200k
  tokens against a 1,900 ceiling. Measure the real thing with
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
