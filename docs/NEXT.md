# What is left to build

Written for someone picking this up cold. Everything here was checked against
the tree it describes; where a claim rests on a specific line, the line is
named, and the lines were re-read at 0.8.0.

Two things remain on the roadmap: native binaries for macOS and Windows,
and letting the agent run containers. The tree itself is
contract-carrying boxes: every box is a folder with a CONTRACT.md, outsiders
read the contract and never the code, and `docs/INDEX.md` maps them.

The GPU front end is built. It shipped as NO0B at 0.7.0 and is task 2 below,
kept for the design record. It runs `noob serve` as a subprocess, so the two
halves share no code; what native binaries still buy it is macOS and Windows.

## Where the project actually is

noob is a terminal coding agent in Rust that talks to any OpenAI-compatible
endpoint. It is developed against a local llama.cpp server (`http://localhost:8080/v1`,
model alias `llm`). `README.md` covers what it does; `ARCHITECTURE.md` covers how.

What ships today is a native Linux package: a deb and a tarball per
architecture on GitHub Releases, one static musl binary inside
(`x86_64-unknown-linux-musl` or `aarch64-unknown-linux-musl`), built and
published by the tag-triggered release workflow. Every command the model
types is folder-locked with Landlock (the exec box's `Lockdown`).
Development runs in containers through `./dev.sh`; users never touch them.

## Task 1: native binaries for macOS and Windows

The goal is a binary a person downloads and runs, with the same folder lock
the Linux package carries.

### Where the port stands

The terminal backend (`crates/noob/src/term`) and the process runner
(`crates/noob/src/exec`) are boxes behind platform-neutral contracts. Their
unix arms build for macOS, and `./dev.sh check-macos` type-checks both
workspaces for `aarch64-apple-darwin`. The terminal carries its Windows
console arm; the runner does not yet.

### What remains, in order

1. macOS: a machine to run the suite on; the folder lock's Seatbelt arm
   behind the exec contract's `Lockdown`; packaging (macOS wants layered
   icon artwork it masks into a squircle itself, and ships unsigned for
   now with the one-time approval step documented); a mac leg in the
   release workflow.
2. Windows: the runner's arm. A Job Object with
   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` kills the whole tree when the
   handle closes, which covers the group-kill half of the contract in one
   primitive. Then the signal story: `crates/noob/src/main.rs` installs
   `sigaction` handlers for SIGINT and SIGWINCH and blocks signals with
   `pthread_sigmask`, `crates/noob/src/ui/dock.rs` unblocks in the dock
   thread; on Windows Ctrl-C is a console control handler and resize is a
   console event, both already surfaced by the term arm. Path handling,
   and the closest folder-scoping mechanism, close it out.
3. The pty suite: `crates/noob/tests/e2e_ui.rs`, `e2e_p3.rs`, `e2e_p5.rs`,
   and `vt.rs` drive the real binary through a pseudo terminal and assert
   on rendered screens. That is the most valuable part of the suite and
   the least portable. Plan for how these run on Windows before porting,
   not after.
4. Web search inside the binary, in Rust, so no platform asks the user's
   machine for Python or uv.

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
  will not catch a real overrun on its own: the same artifact is 1,901 o200k
  tokens against a 1,925 ceiling. Measure the real thing with
  `noob debug prompt --json` and the server's `/tokenize` endpoint.
- **Never cap model output.** No `max_tokens` and no word or sentence limits in
  prompts. `crates/noob/tests/budget.rs` enforces both.
- **Every behavioural change ships with a test**, run locally. The one CI
  surface is the release workflow that builds and publishes binaries on tag;
  tests never move to CI.
- **The repository is not rustfmt-clean.** Running `cargo fmt --all` reformats
  files unrelated to your change. Format only the files you touched, and only if
  they were clean before.
- **The model comparison against opencode is finished.** Do not restart it. The
  token result stands: noob's system prompt is about a quarter of opencode's and
  it used fewer total tokens on two of three tasks. The correctness result does
  not stand: at 35B neither tool reliably produced a browser game that runs, and
  that is a model limit, not something to fix here.
