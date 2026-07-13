# noob/src/subagent

The subagent tool spawns `current_exe() child` with one JSON task on stdin. The child gets a fresh context, sends progress to stderr, and writes exactly one JSON result line to stdout.

The default interactive dock detaches every call through a session-scoped hub. Each original call receives one running-job result immediately. Final output later enters once as a framed, untrusted synthetic user report, never as a second result for the call ID. A fixed FIFO worker pool enforces the child cap, the pending queue is bounded, and `/agents` supports per-job cancellation. Any ready result is deliverable without waiting for unrelated jobs. Background workers never touch the parent transcript, session file, provider, or UI.

Full-tool children can use Bash, MCP, web search, and file tools. Each child `write`, `edit`, or `bash` call takes the workspace directory lease, so leased calls do not overlap while inference and read-only work continue concurrently. A child waits for the lease for a bounded interval; a root mutation reports a conflict instead of waiting behind a child. The lease covers one tool call, not a whole child task, and ends when that call returns. Exec, piped, classic-prompt, and nested-child calls remain inline. On cancellation or parent death, Linux cleanup signals the child process group and kills descendants still attached to it before reaping. Shutdown also cancels, reaps, and joins every child.

Defaults are four concurrent children, 25 inference rounds, 300 seconds, and recursion depth two. A depth-1 child retains nested delegation but runs one nested child at a time, preventing each root child from multiplying the configured inference fan-out. Children default to read-only tools and have no TTY.

The parent starts the wall clock before sending input. Child stdin is nonblocking, checks cancellation and deadline while writing, and closes on completion. The child result is read without an output-length cap. The dock retains a bounded recent progress window for the persistent Tab view while stderr continues to drain. Timeout and cancellation kill and reap the process group.

These are execution budgets, not model output-token limits.
