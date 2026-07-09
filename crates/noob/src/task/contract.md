# noob/src/task

Multi-agent (P6): the `task` tool spawns the binary itself
(`current_exe() child`) with a JSON task on stdin; the child runs a fresh
scoped context and writes exactly one JSON result line to stdout, progress to
stderr. Only the result string enters the parent transcript.

Caps, enforced by both sides: concurrency (default 4), per-child turn cap
(default 25), 300 s wall clock, recursion depth 2. Children default to
read-only tools and have no TTY, so ask-gated actions degrade to deny.
argv + stdin + stdout is the whole IPC surface.
