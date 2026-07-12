# noob/src/task

The task tool spawns `current_exe() child` with one JSON task on stdin. The child gets a fresh context, sends progress to stderr, and writes exactly one JSON result line to stdout. Only its result field enters the parent transcript.

Defaults are four concurrent children, 25 inference rounds, 300 seconds, and recursion depth two. Children default to read-only tools and have no TTY.

The parent starts the wall clock before sending input. Child stdin is nonblocking, checks cancellation and deadline while writing, and closes on completion. The child result is read without an output-length cap. Optional progress retention is bounded at 64 KiB while stderr continues to drain. Timeout and cancellation kill and reap the process group.

These are execution budgets, not model output-token limits.
